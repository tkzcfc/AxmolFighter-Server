use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use protocol::gateway::{ForwardToServerReq, KickSessionReq, ServerPongResp};
use protocol::message_map::MessageType;
use sqlx::PgPool;
use tracing::{debug, warn};

use crate::codec::BackendFrame;
use crate::error_code::ErrorCode;
use crate::gateway_client::GatewaySender;
use crate::player::{PlayerActor, PlayerCommand, PlayerRef, PlayerSendError};
use crate::server_source::ServerSource;
use crate::wire::{CMD_BUSINESS, CMD_GATEWAY_CONTROL};

// 数据访问和业务辅助函数（next_battle_id、DB 查询等）
mod data;
pub mod rpc;

use rpc::{RpcError, RpcManager};

// 游戏服的共享状态，所有玩家 Actor 通过 Arc 持有同一个实例。
pub struct GameShared {
    pub gateway_tx: Mutex<Option<GatewaySender>>,
    pub pool: PgPool,
    pub players: DashMap<u32, PlayerRef>,
    pub account_sessions: Mutex<HashMap<i64, u32>>,
    pub rpc: RpcManager,
    battle_id_seed: AtomicU32,
}

impl GameShared {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            gateway_tx: Mutex::new(None),
            pool,
            players: DashMap::new(),
            account_sessions: Mutex::new(HashMap::new()),
            rpc: RpcManager::new(),
            battle_id_seed: AtomicU32::new(1),
        })
    }

    // ── 网络收发 ────────────────────────────────────────────

    /// 发业务消息到客户端，失败只打日志不阻塞。
    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        if let Err(err) = self.try_send_frame_msg(CMD_BUSINESS, msg, serial, session_id) {
            warn!("failed to send business message: {}", err);
        }
    }

    /// 编码消息 → 组帧 → 通过 gateway channel 发出。
    fn try_send_frame_msg(
        &self,
        cmd: u8,
        msg: &MessageType,
        serial: i32,
        session_id: u32,
    ) -> anyhow::Result<()> {
        let tx = self.gateway_tx.lock().unwrap();
        let sender = tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("gateway not connected"))?;
        let (msg_id, payload) = protocol::message_map::encode_message(msg)
            .ok_or_else(|| anyhow::anyhow!("failed to encode message"))?;
        let data = crate::codec::encode_frame(cmd, msg_id as u16, serial, session_id, &payload);
        sender
            .send(data)
            .map_err(|_| anyhow::anyhow!("gateway channel closed"))?;
        Ok(())
    }

    /// 向网关发请求并等待回复
    pub async fn request_gateway(
        &self,
        msg: MessageType,
        session_id: u32,
        timeout: std::time::Duration,
    ) -> Result<MessageType, RpcError> {
        self.rpc
            .request_gateway(
                &|cmd, msg, serial, sid| self.try_send_frame_msg(cmd, msg, serial, sid),
                msg,
                session_id,
                timeout,
            )
            .await
    }

    /// 向其他服务发请求并等待回复
    pub async fn request_server(
        &self,
        target_service_id: u32,
        target_instance_id: i32,
        session_id: u32,
        msg: MessageType,
        timeout: std::time::Duration,
    ) -> Result<MessageType, RpcError> {
        self.rpc
            .request_server(
                &|cmd, msg, serial, sid| self.try_send_frame_msg(cmd, msg, serial, sid),
                target_service_id,
                target_instance_id,
                session_id,
                msg,
                timeout,
            )
            .await
    }

    /// 单向推送消息给其他服务，不等待回复。
    fn send_server_msg(
        &self,
        target: ServerSource,
        msg: &MessageType,
        serial: i32,
        session_id: u32,
    ) -> anyhow::Result<()> {
        let (msg_id, payload) = protocol::message_map::encode_message(msg)
            .ok_or_else(|| anyhow::anyhow!("failed to encode inner message"))?;
        let frame =
            crate::codec::encode_frame(CMD_BUSINESS, msg_id as u16, serial, session_id, &payload);
        let forward = MessageType::GatewayForwardToServerReq(ForwardToServerReq {
            target_service_id: target.service_id,
            target_instance_id: target.instance_id as i32,
            payload: frame.to_vec(),
            source_service_id: 0,
            source_instance_id: 0,
        });
        self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &forward, 0, session_id)
    }

    // ── 消息路由 ────────────────────────────────────────────

    /// 收到网关控制帧处理逻辑
    pub async fn dispatch_control_frame(self: &Arc<Self>, frame: BackendFrame) {
        // 请求回复,触发rpc pending匹配
        if frame.serial > 0 {
            if !self
                .rpc
                .resolve_pending_response(frame.serial, frame.msg_id, frame.payload)
            {
                debug!(
                    "ignored unmatched gateway control response serial={}, msg_id={}, session={}",
                    frame.serial, frame.msg_id, frame.session_id
                );
            }
            return;
        }

        match protocol::message_map::decode_message(frame.msg_id as u32, &frame.payload) {
            Ok(msg) => match msg {
                MessageType::GatewaySessionOnlinePush(push) => {
                    self.start_session_actor(push.session_id);
                }
                MessageType::GatewaySessionOfflinePush(push) => {
                    self.stop_session_actor(push.session_id);
                }
                MessageType::GatewayServerOnlinePush(push) => {
                    debug!(
                        "server online: type={}, instance={}",
                        push.service_id, push.instance_id
                    );
                }
                MessageType::GatewayServerOfflinePush(push) => {
                    debug!(
                        "server offline: type={}, instance={}",
                        push.service_id, push.instance_id
                    );
                }
                MessageType::GatewayForwardToServerReq(req) => {
                    self.dispatch_server_forward(req).await;
                }
                MessageType::GatewayServerPingReq(ping) => {
                    let message =
                        MessageType::GatewayServerPongResp(ServerPongResp { nonce: ping.nonce });
                    if let Err(err) =
                        self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &message, 0, frame.session_id)
                    {
                        warn!("failed to reply gateway ping nonce={}: {}", ping.nonce, err);
                    }
                }
                MessageType::GatewayBindServiceResp(_)
                | MessageType::GatewayUnbindServiceResp(_) => {}
                other => {
                    debug!("unhandled gateway control message");
                    drop(other);
                }
            },
            Err(err) => {
                debug!(
                    "failed to decode gateway control msg_id={} serial={} session={}: {}",
                    frame.msg_id, frame.serial, frame.session_id, err
                );
            }
        }
    }

    /// 客户端业务帧入口：投递到对应玩家的 Actor 邮箱。
    pub fn dispatch_client_frame(&self, frame: BackendFrame) {
        if frame.serial > 0 {
            if !self
                .rpc
                .resolve_pending_response(frame.serial, frame.msg_id, frame.payload)
            {
                debug!(
                    "ignored unmatched client response serial={} session={} msg_id={}",
                    frame.serial, frame.session_id, frame.msg_id
                );
            }
            return;
        }

        let Some(player_ref) = self
            .players
            .get(&frame.session_id)
            .map(|entry| entry.value().clone())
        else {
            debug!(
                "dropped message for missing player actor session={} msg_id={}",
                frame.session_id, frame.msg_id
            );
            return;
        };

        let serial = frame.serial;
        let msg_id = frame.msg_id;
        let session_id = frame.session_id;

        let cmd = PlayerCommand::ClientMessage {
            msg_id,
            serial,
            payload: frame.payload,
        };
        if let Err(err) = player_ref.try_send(cmd) {
            match err {
                PlayerSendError::Full(_) => {
                    warn!(
                        "player mailbox full session={} msg_id={}",
                        session_id, msg_id
                    );
                    if serial < 0 {
                        let resp = ErrorCode::ServerBusy.to_common_error_message();
                        self.send_msg(&resp, -serial, session_id);
                    }
                }
                PlayerSendError::Closed(_) => {
                    warn!(
                        "player mailbox closed session={} msg_id={}",
                        session_id, msg_id
                    );
                    self.stop_session_actor(session_id);
                }
            }
        }
    }

    /// 解包其他服务转发过来的帧，按 cmd 分流。
    async fn dispatch_server_forward(&self, req: ForwardToServerReq) {
        let source = ServerSource::new(req.source_service_id, req.source_instance_id);
        let mut buf = BytesMut::from(req.payload.as_slice());
        let Ok(Some(inner)) = crate::codec::try_extract_frame(&mut buf) else {
            debug!(
                "invalid forwarded backend frame from service_id={} instance_id={}",
                source.service_id, source.instance_id
            );
            return;
        };

        match inner.cmd {
            CMD_BUSINESS => {
                self.dispatch_server_message(
                    source,
                    inner.msg_id,
                    inner.serial,
                    inner.session_id,
                    inner.payload,
                )
                .await;
            }
            CMD_GATEWAY_CONTROL => {
                debug!("ignored forwarded gateway control msg_id={}", inner.msg_id)
            }
            _ => debug!("unhandled forwarded cmd={}", inner.cmd),
        }
    }

    /// 其他服务发来的消息入口：回复走 pending 匹配，请求/推送分发给 handler。
    pub async fn dispatch_server_message(
        &self,
        source: ServerSource,
        msg_id: u16,
        serial: i32,
        session_id: u32,
        payload: Bytes,
    ) {
        if serial > 0 {
            if !self.rpc.resolve_pending_response(serial, msg_id, payload) {
                debug!(
                    "ignored unmatched server response serial={} source_service_id={} source_instance_id={}",
                    serial, source.service_id, source.instance_id
                );
            }
            return;
        }

        let msg = match protocol::message_map::decode_message(msg_id as u32, &payload) {
            Ok(msg) => msg,
            Err(err) => {
                warn!(
                    "failed to decode server message msg_id={} serial={} source_service_id={} source_instance_id={}: {}",
                    msg_id, serial, source.service_id, source.instance_id, err
                );
                if serial < 0 {
                    self.reply_server_error(
                        source,
                        -serial,
                        session_id,
                        ErrorCode::DecodeMessageFailed,
                    );
                }
                return;
            }
        };

        if serial < 0 {
            match self
                .handle_server_request(source, msg_id, session_id, msg)
                .await
            {
                Ok(resp) => {
                    if let Err(err) = self.send_server_msg(source, &resp, -serial, session_id) {
                        warn!(
                            "failed to reply server request msg_id={} serial={} session={} source_service_id={} source_instance_id={}: {}",
                            msg_id, serial, session_id, source.service_id, source.instance_id, err
                        );
                    }
                }
                Err(err) => {
                    warn!(
                        "server request failed msg_id={} serial={} session={} source_service_id={} source_instance_id={}: {}",
                        msg_id, serial, session_id, source.service_id, source.instance_id, err
                    );
                    self.reply_server_error(source, -serial, session_id, ErrorCode::InternalError);
                }
            }
        } else if let Err(err) = self
            .handle_server_push(source, msg_id, session_id, msg)
            .await
        {
            warn!(
                "server push failed msg_id={} session={} source_service_id={} source_instance_id={}: {}",
                msg_id, session_id, source.service_id, source.instance_id, err
            );
        }
    }

    // ── 会话管理 ────────────────────────────────────────────

    /// 启动玩家 Actor，如果已有旧的就先停掉。
    pub fn start_session_actor(self: &Arc<Self>, session_id: u32) {
        if let Some((_, old_ref)) = self.players.remove(&session_id) {
            old_ref.stop();
        }

        let player_ref = PlayerActor::spawn(self.clone(), session_id);
        self.players.insert(session_id, player_ref);
        debug!("player session {} online", session_id);
    }

    /// 停掉玩家 Actor，解绑账号。
    pub fn stop_session_actor(&self, session_id: u32) {
        if let Some((_, player_ref)) = self.players.remove(&session_id) {
            player_ref.stop();
        }
        self.unbind_session_accounts(session_id);
        debug!("player session {} offline", session_id);
    }

    /// 网关断连时停掉所有 Actor。
    pub fn stop_all_session_actors(&self) {
        let session_ids: Vec<u32> = self.players.iter().map(|entry| *entry.key()).collect();
        for session_id in session_ids {
            if let Some((_, player_ref)) = self.players.remove(&session_id) {
                player_ref.stop();
            }
        }
        self.account_sessions.lock().unwrap().clear();
    }

    /// 绑定账号到 session，如果账号已在其他 session 登录就踢掉旧的。
    pub fn bind_account(&self, session_id: u32, account_id: i64) {
        let old_session_id = {
            let mut account_sessions = self.account_sessions.lock().unwrap();
            account_sessions.insert(account_id, session_id)
        };

        let Some(old_session_id) = old_session_id else {
            return;
        };
        if old_session_id == session_id {
            return;
        }

        warn!(
            "account {} login from session {}, kicking old session {}",
            account_id, session_id, old_session_id
        );
        self.kick_session(old_session_id);
        self.stop_session_actor(old_session_id);
    }

    /// 清理 session 关联的所有账号绑定。
    fn unbind_session_accounts(&self, session_id: u32) {
        let mut account_sessions = self.account_sessions.lock().unwrap();
        account_sessions.retain(|_, bound_session_id| *bound_session_id != session_id);
    }

    /// 通知网关踢掉指定 session。
    fn kick_session(&self, session_id: u32) {
        let msg = MessageType::GatewayKickSessionReq(KickSessionReq { session_id });
        if let Err(err) = self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &msg, 0, session_id) {
            warn!("failed to kick old session {}: {}", session_id, err);
        }
    }

    // ── 服务端消息处理（桩） ────────────────────────────────

    /// 处理其他服务的请求，默认返回“未实现”。
    async fn handle_server_request(
        &self,
        source: ServerSource,
        msg_id: u16,
        session_id: u32,
        msg: MessageType,
    ) -> anyhow::Result<MessageType> {
        warn!(
            "no server request handler for msg_id={} session={} source_service_id={} source_instance_id={}",
            msg_id, session_id, source.service_id, source.instance_id
        );
        drop(msg);
        Err(anyhow::anyhow!("no server request handler"))
    }

    /// 处理其他服务的推送，默认打日志忽略。
    async fn handle_server_push(
        &self,
        source: ServerSource,
        msg_id: u16,
        session_id: u32,
        msg: MessageType,
    ) -> anyhow::Result<()> {
        warn!(
            "no server push handler for msg_id={} session={} source_service_id={} source_instance_id={}",
            msg_id, session_id, source.service_id, source.instance_id
        );
        drop(msg);
        Err(anyhow::anyhow!("no server push handler"))
    }

    /// 给其他服务回复一个通用错误。
    fn reply_server_error(
        &self,
        target: ServerSource,
        serial: i32,
        session_id: u32,
        code: ErrorCode,
    ) {
        let resp = code.to_common_error_message();
        if let Err(err) = self.send_server_msg(target, &resp, serial, session_id) {
            warn!(
                "failed to reply server error serial={} session={} target_service_id={} target_instance_id={}: {}",
                serial, session_id, target.service_id, target.instance_id, err
            );
        }
    }
}
