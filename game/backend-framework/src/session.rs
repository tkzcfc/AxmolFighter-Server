use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use bytes::BytesMut;
use dashmap::DashMap;
use protocol::gateway::{ForwardToServerReq, ServerPongResp};
use protocol::message_map::{MessageType, decode_message, encode_message};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::codec::{BackendFrame, encode_frame, try_extract_frame};
use crate::delegate::{BackendDelegate, common_error_response};
use crate::gateway_client::GatewaySender;
use crate::handler::MessageHandler;
use crate::rpc::{PendingResponse, RpcError, RpcManager};
use crate::server_source::ServerSource;
use crate::session_delegate::SessionDelegate;
use crate::wire::{CMD_BUSINESS, CMD_GATEWAY_CONTROL};

const SESSION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const CLIENT_MAILBOX_SIZE: usize = 256;
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(10);

// ═══════════════════════════════════════════════════════════════
// BackendSession — 后端会话(网络收发 + 协议路由 + session 管理)
//
// 框架管理:
// - per-session task:spawn on online / stop on offline(对标 base 的 net_session::run)
// - server_dispatcher task:串行处理服务间消息(on_server_request/push)
// - 读循环内联路由:控制帧(会话上下线、ping)内联;服务间帧入队;RPC 响应内联解析
// ═══════════════════════════════════════════════════════════════

struct SessionHandle {
    client_tx: mpsc::Sender<BackendFrame>,
    cancel_token: CancellationToken,
}

pub struct BackendSession {
    gateway_tx: Mutex<Option<GatewaySender>>,
    rpc: RpcManager,
    delegate: OnceLock<Arc<dyn BackendDelegate>>,
    service_id: u32,
    instance_id: u32,
    /// 获取 Arc<Self>(init_weak 后可用)
    self_weak: OnceLock<Weak<Self>>,
    /// 全部 per-session task
    sessions: DashMap<u32, SessionHandle>,
    /// 服务间消息分发通道
    server_dispatch_tx: OnceLock<mpsc::UnboundedSender<(ServerSource, BackendFrame)>>,
    /// 每个 session task 持 clone,退出时 drop;shutdown 先 take 自身再等 recv(None)
    shutdown_complete_tx: Mutex<Option<mpsc::Sender<()>>>,
    shutdown_complete_rx: tokio::sync::Mutex<mpsc::Receiver<()>>,
}

impl BackendSession {
    pub fn new(service_id: u32, instance_id: u32) -> Arc<Self> {
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel::<()>(1);
        Arc::new_cyclic(|weak| {
            let s = Self {
                gateway_tx: Mutex::new(None),
                rpc: RpcManager::new(),
                delegate: OnceLock::new(),
                service_id,
                instance_id,
                self_weak: OnceLock::new(),
                sessions: DashMap::new(),
                server_dispatch_tx: OnceLock::new(),
                shutdown_complete_tx: Mutex::new(Some(shutdown_complete_tx)),
                shutdown_complete_rx: tokio::sync::Mutex::new(shutdown_complete_rx),
            };
            s.self_weak.set(weak.clone()).ok();
            s
        })
    }

    fn arc_self(&self) -> Arc<Self> {
        self.self_weak.get().unwrap().upgrade().unwrap()
    }

    pub fn set_delegate(&self, delegate: Arc<dyn BackendDelegate>) {
        let _ = self.delegate.set(delegate);
    }

    fn delegate(&self) -> Option<&Arc<dyn BackendDelegate>> {
        self.delegate.get()
    }

    /// 启动服务间分发任务(对标 net_session::run)。
    pub fn start_server_dispatcher(
        self: &Arc<Self>,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<(ServerSource, BackendFrame)>();
        let _ = self.server_dispatch_tx.set(tx);
        let session = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased; // 加了 biased 后改为按书写顺序轮询——上面的分支优先
                    _ = shutdown.cancelled() => break,
                    item = rx.recv() => {
                        let Some((source, inner)) = item else { break };
                        session.process_server_message(source, inner).await;
                    }
                }
            }
            debug!("server dispatcher exited");
        })
    }

    /// 服务间消息处理(由 server_dispatcher 调用)。
    /// 解帧 → 调 delegate → 框架自动回包(request)或打日志(push)。
    async fn process_server_message(&self, source: ServerSource, inner: BackendFrame) {
        let serial = inner.serial;
        match decode_message(inner.msg_id as u32, &inner.payload) {
            Ok(msg) => {
                let Some(delegate) = self.delegate() else {
                    return;
                };
                if serial < 0 {
                    let resp = match delegate.on_server_request(source, msg).await {
                        Ok(resp) => resp,
                        Err(err) => {
                            error!("server request failed source={} : {}", source, err);
                            common_error_response(err)
                        }
                    };
                    if let Err(err) = self.send_server_msg(source, &resp, -serial) {
                        warn!("failed to reply server request serial={}: {}", serial, err);
                    }
                } else if serial > 0 {
                    // RPC 响应已在 read_loop 内联解析,不应到这里
                    debug!(
                        "ignored unmatched server push response serial={} source={}",
                        serial, source
                    );
                } else if let Err(err) = delegate.on_server_push(source, msg).await {
                    error!("server push failed source={}: {}", source, err);
                }
            }
            Err(err) => {
                error!(
                    "failed to decode server message msg_id={} source={}: {}",
                    inner.msg_id, source, err
                );
                if serial != 0 {
                    let resp =
                        common_error_response(format!("decode server message failed: {err}"));
                    let _ = self.send_server_msg(source, &resp, -serial);
                }
            }
        }
    }

    // ── per-session 管理 ───────────────────────────────────

    /// 创建 per-session task 并存入 sessions。
    /// 框架内部调用,顶替旧 session(若存在)。
    fn spawn_session(&self, sid: u32) {
        self.stop_session(sid);

        let Some(delegate) = self.delegate() else {
            return;
        };
        let session_delegate = delegate.create_session_delegate(sid, self.arc_self());

        let (client_tx, mut client_rx) = mpsc::channel::<BackendFrame>(CLIENT_MAILBOX_SIZE);
        let shutdown_complete = self.shutdown_complete_tx.lock().unwrap().clone();
        let bs = self.arc_self();

        let cancel_token = CancellationToken::new();
        let _handle = tokio::spawn({
            let cancel_token = cancel_token.clone();
            async move {
                session_delegate.on_start().await;
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel_token.cancelled() => break,
                        frame = client_rx.recv() => {
                            let Some(frame) = frame else { break };
                            bs.process_client_frame(&*session_delegate, frame).await;
                        }
                    }
                }
                session_delegate.on_stop().await;
                drop(shutdown_complete);
            }
        });

        self.sessions.insert(
            sid,
            SessionHandle {
                client_tx,
                cancel_token,
            },
        );
        debug!("session {} spawned", sid);
    }

    /// 强停 session(取消令牌,任务自然退出 → on_stop → drop shutdown_complete)。
    fn stop_session(&self, sid: u32) {
        if let Some((_, handle)) = self.sessions.remove(&sid) {
            handle.cancel_token.cancel();
            debug!("session {} cancel signalled", sid);
        }
    }

    fn stop_all_sessions(&self) {
        let sids: Vec<u32> = self.sessions.iter().map(|e| *e.key()).collect();
        for sid in sids {
            self.stop_session(sid);
        }
    }

    /// 全服优雅关闭入口(由 BackendRuntime::shutdown 调用)。
    ///
    /// 流程: 取消所有 session → 等全部退出 → 清理 → delegate.on_shutdown
    pub async fn shutdown(&self) {
        info!("shutdown: notifying all sessions");
        self.stop_all_sessions();
        // take 并立即 drop 掉服务端的 sender,这样 recv() 才能在所有 session 退出后返回 None
        drop(self.shutdown_complete_tx.lock().unwrap().take());
        // 等待所有 session 退出,超时则强制退出
        let result = {
            let mut rx = self.shutdown_complete_rx.lock().await;
            tokio::time::timeout(SESSION_SHUTDOWN_TIMEOUT, rx.recv()).await
        };
        if result.is_err() {
            warn!("graceful session shutdown timed out, force aborting");
        } else {
            info!("all sessions exited gracefully");
        }
        if let Some(delegate) = self.delegate() {
            delegate.on_shutdown().await;
        }
        info!("backend session shutdown complete");
    }

    /// 处理 per-session task 收到的客户端帧。
    /// 解帧 → 调 delegate → 框架自动回包(request)或打日志(push)。
    async fn process_client_frame(&self, d: &dyn SessionDelegate, frame: BackendFrame) {
        match decode_message(frame.msg_id as u32, &frame.payload) {
            Ok(msg) => {
                if frame.serial == 0 {
                    if let Err(err) = d.on_client_push(msg).await {
                        warn!(
                            "client push error session={} msg_id={}: {}",
                            frame.session_id, frame.msg_id, err
                        );
                    }
                } else {
                    let resp = match d.on_client_request(msg).await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!(
                                "client request error session={} msg_id={}: {}",
                                frame.session_id, frame.msg_id, err
                            );
                            common_error_response(err)
                        }
                    };
                    self.send_msg(&resp, -frame.serial, frame.session_id);
                }
            }
            Err(err) => {
                warn!(
                    "failed to decode client msg msg_id={} session={}: {}",
                    frame.msg_id, frame.session_id, err
                );
            }
        }
    }

    // ── 连接生命周期 ────────────────────────────────────────

    pub fn set_gateway_tx(&self, tx: GatewaySender) {
        *self.gateway_tx.lock().unwrap() = Some(tx);
    }

    pub fn clear_gateway_tx(&self) {
        *self.gateway_tx.lock().unwrap() = None;
        self.rpc.clear_pending();
    }

    // ── 发送 ────────────────────────────────────────────────

    fn try_send_frame_msg(
        &self,
        cmd: u8,
        msg: &MessageType,
        serial: i32,
        session_id: u32,
    ) -> anyhow::Result<()> {
        let (msg_id, payload) =
            encode_message(msg).ok_or_else(|| anyhow::anyhow!("failed to encode message"))?;
        let data = encode_frame(cmd, msg_id as u16, serial, session_id, &payload);
        let tx = self.gateway_tx.lock().unwrap();
        let sender = tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("gateway not connected"))?;
        sender
            .send(data)
            .map_err(|_| anyhow::anyhow!("gateway channel closed"))?;
        Ok(())
    }

    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        if let Err(err) = self.try_send_frame_msg(CMD_BUSINESS, msg, serial, session_id) {
            warn!("failed to send business message: {}", err);
        }
    }

    pub fn send_control_msg(&self, msg: &MessageType, serial: i32) -> anyhow::Result<()> {
        self.try_send_frame_msg(CMD_GATEWAY_CONTROL, msg, serial, 0)
    }

    pub async fn request_gateway_timeout(
        &self,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        let request_id = self.rpc.next_request_id();
        let (tx, rx) = oneshot::channel::<PendingResponse>();
        self.rpc.insert_pending(request_id, tx);
        if let Err(err) = self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &msg, -request_id, 0) {
            self.rpc.remove_pending(request_id);
            return Err(RpcError::Send(err.to_string()));
        }
        self.rpc.wait_response(request_id, rx, timeout).await
    }

    pub async fn request_server_timeout(
        &self,
        target: ServerSource,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        let request_id = self.rpc.next_request_id();
        let (tx, rx) = oneshot::channel::<PendingResponse>();
        self.rpc.insert_pending(request_id, tx);
        if let Err(err) = self.send_server_msg(target, &msg, -request_id) {
            self.rpc.remove_pending(request_id);
            return Err(RpcError::Send(err.to_string()));
        }
        self.rpc.wait_response(request_id, rx, timeout).await
    }

    /// 向网关发 RPC 请求并等待回复(默认超时 10s)。
    pub async fn request_gateway(&self, msg: MessageType) -> Result<MessageType, RpcError> {
        self.request_gateway_timeout(msg, DEFAULT_RPC_TIMEOUT).await
    }

    /// 向其他服务发 RPC 请求并等待回复(默认超时 10s)。
    pub async fn request_server(
        &self,
        target: ServerSource,
        msg: MessageType,
    ) -> Result<MessageType, RpcError> {
        self.request_server_timeout(target, msg, DEFAULT_RPC_TIMEOUT)
            .await
    }

    pub fn send_server_msg(
        &self,
        target: ServerSource,
        msg: &MessageType,
        serial: i32,
    ) -> anyhow::Result<()> {
        let (msg_id, payload) =
            encode_message(msg).ok_or_else(|| anyhow::anyhow!("failed to encode inner message"))?;
        let frame = encode_frame(CMD_BUSINESS, msg_id as u16, serial, 0, &payload);
        let forward = MessageType::GatewayForwardToServerReq(ForwardToServerReq {
            target_service_id: target.service_id,
            target_instance_id: target.instance_id,
            payload: frame.to_vec(),
            source_service_id: self.service_id,
            source_instance_id: self.instance_id,
        });
        self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &forward, serial, 0)
    }

    // ── 控制帧处理(读循环内联,同步) ────────────────────────

    fn handle_control_frame(&self, frame: BackendFrame) {
        if frame.serial > 0 {
            if !self
                .rpc
                .resolve_pending_response(frame.serial, frame.msg_id, frame.payload)
            {
                debug!(
                    "ignored unmatched gateway control response serial={} msg_id={}",
                    frame.serial, frame.msg_id
                );
            }
            return;
        }

        let msg = match decode_message(frame.msg_id as u32, &frame.payload) {
            Ok(msg) => msg,
            Err(err) => {
                debug!(
                    "failed to decode gateway control msg_id={}: {}",
                    frame.msg_id, err
                );
                return;
            }
        };

        match msg {
            MessageType::GatewaySessionOnlinePush(push) => {
                self.spawn_session(push.session_id);
            }
            MessageType::GatewaySessionOfflinePush(push) => {
                self.stop_session(push.session_id);
            }
            MessageType::GatewayServerOnlinePush(push) => {
                debug!(
                    "server online: type={} instance={}",
                    push.service_id, push.instance_id
                );
            }
            MessageType::GatewayServerOfflinePush(push) => {
                debug!(
                    "server offline: type={} instance={}",
                    push.service_id, push.instance_id
                );
            }
            MessageType::GatewayForwardToServerReq(req) => {
                self.route_forwarded_frame(req);
            }
            MessageType::GatewayServerPingReq(ping) => {
                let msg = MessageType::GatewayServerPongResp(ServerPongResp { nonce: ping.nonce });
                if let Err(err) =
                    self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &msg, 0, frame.session_id)
                {
                    warn!("failed to reply gateway ping nonce={}: {}", ping.nonce, err);
                }
            }
            other => {
                debug!("unhandled gateway control message");
                drop(other);
            }
        }
    }

    /// 拆解 ForwardToServerReq:内层 serial>0 → 内联 RPC 响应解析;
    /// 内层 serial≤0 → 入队给 server_dispatcher。
    fn route_forwarded_frame(&self, req: ForwardToServerReq) {
        let source = ServerSource::new(req.source_service_id, req.source_instance_id as i32);
        let mut buf = BytesMut::from(req.payload.as_slice());
        let Ok(Some(inner)) = try_extract_frame(&mut buf) else {
            debug!("invalid forwarded backend frame from {}", source);
            return;
        };

        if inner.serial > 0 {
            if !self
                .rpc
                .resolve_pending_response(inner.serial, inner.msg_id, inner.payload)
            {
                debug!(
                    "ignored unmatched server response serial={} source={}",
                    inner.serial, source
                );
            }
            return;
        }

        if let Some(tx) = self.server_dispatch_tx.get()
            && tx.send((source, inner)).is_err()
        {
            warn!("server dispatch channel closed");
        }
    }

    // ── 业务帧处理(读循环内联,同步) ────────────────────────

    pub fn dispatch_business_frame(&self, frame: BackendFrame) {
        if frame.serial > 0 {
            if !self
                .rpc
                .resolve_pending_response(frame.serial, frame.msg_id, frame.payload)
            {
                debug!(
                    "ignored unmatched client response serial={} session={}",
                    frame.serial, frame.session_id
                );
            }
            return;
        }

        if let Some(handle) = self
            .sessions
            .get(&frame.session_id)
            .map(|e| e.value().client_tx.clone())
        {
            let sid = frame.session_id;
            match handle.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(frame)) => {
                    warn!("session {} mailbox full", sid);
                    if frame.serial != 0 {
                        self.send_msg(&common_error_response("mailbox full"), -frame.serial, sid);
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!("session {} channel closed", sid);
                }
            }
        } else {
            warn!(
                "dropped client frame for unknown session {}",
                frame.session_id
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// MessageHandler 实现
// ═══════════════════════════════════════════════════════════════

impl MessageHandler for BackendSession {
    fn on_gateway_connected(&self, tx: GatewaySender) {
        self.set_gateway_tx(tx);
        if let Some(delegate) = self.delegate() {
            delegate.on_connected();
        }
    }

    fn on_gateway_disconnected(&self) {
        if let Some(delegate) = self.delegate() {
            delegate.on_disconnected();
        }
        self.stop_all_sessions();
        self.clear_gateway_tx();
    }

    fn on_gateway_control_frame(&self, frame: BackendFrame) {
        self.handle_control_frame(frame);
    }

    fn on_business_frame(&self, frame: BackendFrame) {
        self.dispatch_business_frame(frame);
    }
}
