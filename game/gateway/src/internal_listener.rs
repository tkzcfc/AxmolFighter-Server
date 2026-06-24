use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::protocol::gateway::{
    GatewayErrorResp, ServerOfflinePush, ServerOnlinePush, ServerPingReq, ServerRegResp,
    ServerStatusPush, ServiceEndpoint, ServiceStatus as GatewayServiceStatus, SessionOnlinePush,
};
use ::protocol::message_map::{MessageType, decode_message, encode_message};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use base::net::{WriterMessage, session_delegate::SessionDelegate};

use crate::bind_manager::BindRequester;
use crate::codec::{encode_backend_frame, encode_client_frame, try_extract_backend_frame};
use crate::context::GatewayContext;
use crate::frame_cmd::*;
use crate::server_registry::RegisterResult;

// 注册成功
const SERVER_REG_OK: u32 = 0;
// 注册失败,同一个服务向网关发送了两次不同的注册请求
const SERVER_REG_ERR_INSTANCE_ALREADY_REGISTERED: u32 = 1;
// 注册失败,可能是服务配置重复了
const SERVER_REG_ERR_SESSION_ALREADY_REGISTERED: u32 = 2;

// 心跳间隔时间
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
// 心跳超时时间
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy)]
struct PendingPing {
    nonce: u64,
    sent_at: Instant,
}

#[derive(Default)]
struct HeartbeatState {
    pending: Option<PendingPing>,
}

impl HeartbeatState {
    fn mark_sent(&mut self, nonce: u64, sent_at: Instant) {
        self.pending = Some(PendingPing { nonce, sent_at });
    }

    fn acknowledge(&mut self, nonce: u64, now: Instant) -> Option<u32> {
        let pending = self.pending?;
        if pending.nonce != nonce {
            return None;
        }

        self.pending = None;
        Some(
            now.duration_since(pending.sent_at)
                .as_millis()
                .min(u32::MAX as u128) as u32,
        )
    }

    fn timed_out(&self, now: Instant, timeout: Duration) -> bool {
        self.pending
            .is_some_and(|pending| now.duration_since(pending.sent_at) >= timeout)
    }

    fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

pub struct InternalDelegate {
    ctx: GatewayContext,
    session_id: u32,
    tx: Option<mpsc::UnboundedSender<WriterMessage>>,
    registered: Option<(u8, u32)>,
    heartbeat: Arc<Mutex<HeartbeatState>>,
    heartbeat_task: Option<JoinHandle<()>>,
}

impl InternalDelegate {
    pub fn new(ctx: GatewayContext) -> Self {
        Self {
            ctx,
            session_id: 0,
            tx: None,
            registered: None,
            heartbeat: Arc::new(Mutex::new(HeartbeatState::default())),
            heartbeat_task: None,
        }
    }

    fn build_status(&self) -> ServerStatusPush {
        ServerStatusPush {
            services: self
                .ctx
                .registry
                .service_statuses()
                .into_iter()
                .map(|(service_id, instance_id)| GatewayServiceStatus {
                    service_id: service_id as u32,
                    instance_id,
                    online: true,
                })
                .collect(),
        }
    }

    fn broadcast_status_to_clients(&self) {
        let (msg_id, payload) =
            encode_message(&MessageType::GatewayServerStatusPush(self.build_status())).unwrap();
        let client_data = encode_client_frame(CMD_GATEWAY_CONTROL, msg_id as u16, 0, &payload);
        self.ctx.sessions.broadcast_to_clients(client_data);
    }

    fn encode_control_frame(&self, message: MessageType, serial: i32, session_id: u32) -> Bytes {
        let (msg_id, payload) = encode_message(&message).unwrap();
        encode_backend_frame(
            CMD_GATEWAY_CONTROL,
            msg_id as u16,
            serial,
            session_id,
            &payload,
        )
    }

    fn reply_control_error_if_request(&self, serial: i32, code: u32, message: &str) {
        if serial == 0 {
            return;
        }

        let Some(tx) = &self.tx else {
            warn!(
                "failed to reply control error before writer is ready, serial={}",
                serial
            );
            return;
        };

        let response_serial = if serial < 0 { -serial } else { serial };
        let resp = MessageType::GatewayGatewayErrorResp(GatewayErrorResp {
            code,
            message: message.to_string(),
        });
        let data = self.encode_control_frame(resp, response_serial, 0);
        let _ = tx.send(WriterMessage::Send(data, true));
    }

    fn start_heartbeat(&mut self, tx: mpsc::UnboundedSender<WriterMessage>) {
        let session_id = self.session_id;
        let heartbeat = self.heartbeat.clone();

        self.heartbeat_task = Some(tokio::spawn(async move {
            let mut nonce = 0u64;

            loop {
                tokio::time::sleep(HEARTBEAT_INTERVAL).await;

                let now = Instant::now();
                {
                    let state = heartbeat.lock().await;
                    if state.timed_out(now, HEARTBEAT_TIMEOUT) {
                        warn!(
                            "internal session {} heartbeat timeout after {}s",
                            session_id,
                            HEARTBEAT_TIMEOUT.as_secs()
                        );
                        let _ = tx.send(WriterMessage::Close);
                        break;
                    }

                    if state.has_pending() {
                        continue;
                    }
                }

                nonce = nonce.wrapping_add(1);
                let message = MessageType::GatewayServerPingReq(ServerPingReq { nonce });
                let Some((msg_id, payload)) = encode_message(&message) else {
                    warn!("failed to encode ServerPingReq");
                    continue;
                };
                let data = encode_backend_frame(CMD_GATEWAY_CONTROL, msg_id as u16, 0, 0, &payload);

                {
                    let mut state = heartbeat.lock().await;
                    state.mark_sent(nonce, Instant::now());
                }

                if tx.send(WriterMessage::Send(data, true)).is_err() {
                    break;
                }
            }
        }));
    }
}

#[async_trait]
impl SessionDelegate for InternalDelegate {
    async fn on_session_start(
        &mut self,
        session_id: u32,
        addr: &SocketAddr,
        tx: mpsc::UnboundedSender<WriterMessage>,
    ) -> anyhow::Result<()> {
        self.session_id = session_id;
        self.tx = Some(tx.clone());
        self.start_heartbeat(tx);
        info!("internal connection {} from {}", session_id, addr);
        Ok(())
    }

    async fn on_session_close(&mut self) -> anyhow::Result<()> {
        debug!("internal connection {} closed", self.session_id);
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }

        if let Some((service_id, instance_id)) = self.ctx.registry.remove(self.session_id) {
            info!(
                "backend service_id={} instance {} unregistered (disconnected)",
                service_id, instance_id
            );

            self.ctx
                .router
                .unbind_all_by_instance(service_id, instance_id);

            let notify = MessageType::GatewayServerOfflinePush(ServerOfflinePush {
                service_id: service_id as u32,
                instance_id,
            });
            let data = self.encode_control_frame(notify, 0, 0);
            self.ctx.registry.broadcast_except(self.session_id, data);
            self.broadcast_status_to_clients();
        }

        Ok(())
    }

    async fn on_try_extract_frame(
        &mut self,
        buffer: &mut BytesMut,
    ) -> anyhow::Result<Option<Bytes>> {
        match try_extract_backend_frame(buffer)? {
            Some(frame) => {
                let mut out = BytesMut::with_capacity(11 + frame.payload.len());
                out.extend_from_slice(&[frame.cmd]);
                out.extend_from_slice(&frame.msg_id.to_be_bytes());
                out.extend_from_slice(&frame.serial.to_be_bytes());
                out.extend_from_slice(&frame.session_id.to_be_bytes());
                out.extend_from_slice(&frame.payload);
                Ok(Some(out.freeze()))
            }
            None => Ok(None),
        }
    }

    async fn on_recv_frame(&mut self, frame: Bytes) -> anyhow::Result<()> {
        if frame.len() < 11 {
            return Ok(());
        }

        let cmd = frame[0];
        let msg_id = u16::from_be_bytes([frame[1], frame[2]]);
        let serial = i32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
        let session_id = u32::from_be_bytes([frame[7], frame[8], frame[9], frame[10]]);
        let payload = frame.slice(11..);

        if cmd == CMD_BUSINESS {
            // 后端回客户端的业务包，网关只按 session 转回去。
            let client_data = encode_client_frame(CMD_BUSINESS, msg_id, serial, &payload);
            if !self.ctx.sessions.send_to_client(session_id, client_data) {
                debug!(
                    "client session {} not found, dropping response msg_id={}",
                    session_id, msg_id
                );
            }
            return Ok(());
        }

        self.handle_internal(cmd, msg_id, serial, session_id, payload)
            .await
    }
}

impl InternalDelegate {
    async fn handle_internal(
        &mut self,
        cmd: u8,
        msg_id: u16,
        serial: i32,
        _session_id: u32,
        payload: Bytes,
    ) -> anyhow::Result<()> {
        if cmd != CMD_GATEWAY_CONTROL {
            debug!("unhandled internal cmd={}", cmd);
            return Ok(());
        }

        let message = match decode_message(msg_id as u32, &payload) {
            Ok(message) => message,
            Err(err) => {
                warn!(
                    "failed to decode internal control msg_id={} serial={} session_id={}: {}",
                    msg_id, serial, self.session_id, err
                );
                return Ok(());
            }
        };

        match message {
            MessageType::GatewayServerRegReq(req) => {
                info!(
                    "received register request service_id={} instance_id={} from internal session {}",
                    req.service_id, req.instance_id, self.session_id
                );
                let service_id = req.service_id as u8;
                let Some(tx) = self.tx.clone() else {
                    // 这种情况理论上不应该发生，因为注册请求来自连接的服务实例，而连接成功的前提是 writer 已经准备好。
                    warn!(
                        "register requested before writer is ready, internal session {}",
                        self.session_id
                    );
                    return Ok(());
                };

                let register_result = self.ctx.registry.register(
                    self.session_id,
                    service_id,
                    req.instance_id,
                    tx.clone(),
                );
                let (code, should_broadcast_online) = match register_result {
                    RegisterResult::Registered => {
                        self.ctx.registry.update_load(
                            self.session_id,
                            service_id,
                            req.instance_id,
                            req.load_score,
                            req.accepting_bindings,
                            req.load_message.clone(),
                        );
                        self.registered = Some((service_id, req.instance_id));

                        info!(
                            "backend service_id={} instance {} registered",
                            req.service_id, req.instance_id
                        );

                        (SERVER_REG_OK, true)
                    }
                    RegisterResult::AlreadyRegistered => {
                        // AlreadyRegistered 表示之前已经注册过一次了,又重复发送了注册请求但注册信息和第一次的一模一样
                        // 允许重复注册,重复注册只更新负载信息，不重复广播上线事件。
                        self.ctx.registry.update_load(
                            self.session_id,
                            service_id,
                            req.instance_id,
                            req.load_score,
                            req.accepting_bindings,
                            req.load_message.clone(),
                        );
                        self.registered = Some((service_id, req.instance_id));

                        debug!(
                            "backend service_id={} instance {} registration refreshed",
                            req.service_id, req.instance_id
                        );

                        (SERVER_REG_OK, false)
                    }
                    RegisterResult::SessionAlreadyRegistered => {
                        // SessionAlreadyRegistered 表示之前已经注册过一次了,又重复发送了注册请求并且注册信息和第一次的还不一样
                        // 同一连接上不允许注册多个服务实例，避免同一连接上既有游戏服又有聊天服等情况
                        warn!(
                            "internal session {} tried to register a second identity service_id={} instance {}",
                            self.session_id, req.service_id, req.instance_id
                        );

                        (SERVER_REG_ERR_SESSION_ALREADY_REGISTERED, false)
                    }
                    RegisterResult::InstanceAlreadyRegistered => {
                        // InstanceAlreadyRegistered 表示这个服务信息已经被其他连接注册过了,又重复发送了注册请求
                        // 可能是服务端重试了注册请求或者之前的连接断了但还没被网关清理掉,也可能是服务配置重复了,
                        // 总之同一服务实例不允许重复注册，避免重复绑定和负载计算错误
                        warn!(
                            "rejected duplicate backend service_id={} instance {} from internal session {}",
                            req.service_id, req.instance_id, self.session_id
                        );

                        (SERVER_REG_ERR_INSTANCE_ALREADY_REGISTERED, false)
                    }
                };

                // 注册结果回复
                let resp = ServerRegResp {
                    code,
                    servers: self
                        .ctx
                        .registry
                        .list_all_except(self.session_id)
                        .into_iter()
                        .map(|(service_id, instance_id)| ServiceEndpoint {
                            service_id: service_id as u32,
                            instance_id,
                        })
                        .collect(),
                };
                let data =
                    self.encode_control_frame(MessageType::GatewayServerRegResp(resp), serial, 0);
                let _ = tx.send(WriterMessage::Send(data, true));

                if !should_broadcast_online {
                    return Ok(());
                }

                let notify = MessageType::GatewayServerOnlinePush(ServerOnlinePush {
                    service_id: req.service_id,
                    instance_id: req.instance_id,
                });
                let broadcast_data = self.encode_control_frame(notify, 0, 0);
                self.ctx
                    .registry
                    .broadcast_except(self.session_id, broadcast_data);

                self.broadcast_status_to_clients();

                for sid in self.ctx.sessions.online_sessions() {
                    let online = MessageType::GatewaySessionOnlinePush(SessionOnlinePush {
                        session_id: sid,
                    });
                    let notify_data = self.encode_control_frame(online, 0, sid);
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(WriterMessage::Send(notify_data, true));
                    }
                }
            }

            MessageType::GatewayBindServiceReq(req) => {
                let Some(tx) = self.tx.clone() else {
                    // 这种情况理论上不应该发生，因为绑定请求来自已注册的服务实例，而注册成功的前提是 writer 已经准备好。
                    // 但万一发生了，也只能记录日志了。
                    warn!(
                        "bind requested before writer is ready, internal session {}",
                        self.session_id
                    );
                    return Ok(());
                };

                let Some((_requester_service_id, _requester_instance_id)) = self.registered else {
                    // 服务没有注册，无法处理绑定请求。
                    warn!(
                        "bind requested by unregistered internal session {}",
                        self.session_id
                    );
                    self.ctx.binds.reject_request(
                        &tx,
                        serial,
                        &req,
                        crate::bind_manager::REQUESTER_SERVICE_NOT_REGISTERED,
                        "requester not registered",
                    );
                    return Ok(());
                };

                self.ctx
                    .binds
                    .start(&self.ctx, BindRequester { serial, tx }, req);
            }

            MessageType::GatewayServiceLoadReportPush(report) => {
                let Some((service_id, instance_id)) = self.registered else {
                    warn!(
                        "ignored load report from unregistered session {}",
                        self.session_id
                    );
                    return Ok(());
                };

                // 负载来源以注册连接为准，避免服务端误填身份。
                let updated = self.ctx.registry.update_load(
                    self.session_id,
                    service_id,
                    instance_id,
                    report.load_score,
                    report.accepting_bindings,
                    report.message,
                );
                if !updated {
                    warn!(
                        "ignored load report from session {} service_id={} instance_id={}",
                        self.session_id, service_id, instance_id
                    );
                }
            }

            MessageType::GatewayServerPongResp(pong) => {
                let latency_ms = {
                    let mut heartbeat = self.heartbeat.lock().await;
                    heartbeat.acknowledge(pong.nonce, Instant::now())
                };

                if let Some(latency_ms) = latency_ms {
                    if self
                        .ctx
                        .registry
                        .update_latency(self.session_id, latency_ms)
                    {
                        debug!(
                            "backend session {} latency updated: {}ms",
                            self.session_id, latency_ms
                        );
                    }
                } else {
                    debug!(
                        "ignored unmatched pong nonce={} from internal session {}",
                        pong.nonce, self.session_id
                    );
                }
            }

            MessageType::GatewayUnbindServiceReq(req) => {
                let session_id = req.session_id;
                let service_id = req.service_id;
                self.ctx.router.unbind_service(session_id, service_id as u8);
                debug!(
                    "unbound session {} from service_id={}",
                    session_id, service_id
                );
                if serial < 0 {
                    let resp = MessageType::GatewayUnbindServiceResp(
                        protocol::gateway::UnbindServiceResp {
                            session_id,
                            service_id,
                            code: 0,
                            message: String::new(),
                        },
                    );
                    let data = self.encode_control_frame(resp, -serial, 0);
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(WriterMessage::Send(data, true));
                    }
                }
            }

            MessageType::GatewayKickSessionReq(req) => {
                self.ctx.sessions.kick(req.session_id);
                self.ctx.router.cleanup_session(req.session_id);
                debug!("kicked session {}", req.session_id);
                if serial < 0 {
                    let resp =
                        MessageType::GatewayKickSessionRsp(protocol::gateway::KickSessionRsp {
                            session_id: req.session_id,
                            code: 0,
                        });
                    let data = self.encode_control_frame(resp, -serial, 0);
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(WriterMessage::Send(data, true));
                    }
                }
            }

            MessageType::GatewayForwardToServerReq(req) => {
                // 跨服转发也是控制消息，用 msg_id 区分具体协议。
                self.handle_server_forward(serial, req);
            }

            _ => {
                debug!("unhandled gateway control msg_id={}", msg_id);
            }
        }

        Ok(())
    }

    fn handle_server_forward(&self, serial: i32, mut req: ::protocol::gateway::ForwardToServerReq) {
        let Some((source_service_id, source_instance_id)) = self.registered else {
            warn!(
                "forward requested by unregistered internal session {}",
                self.session_id
            );
            self.reply_control_error_if_request(serial, 3, "requester service not registered");
            return;
        };

        let target_service_id = req.target_service_id as u8;
        let selected = if req.target_instance_id >= 0 {
            let instance_id = req.target_instance_id as u32;
            self.ctx
                .registry
                .find_by_instance(target_service_id, instance_id)
                .map(|tx| (instance_id, tx))
                .ok_or((2, "target service instance unreachable"))
        } else {
            self.ctx
                .registry
                .select_lowest_load_instance(target_service_id, |service_id, instance_id| {
                    self.ctx.router.binding_count(service_id, instance_id)
                })
                .map(|candidate| (candidate.instance_id, candidate.tx))
                .ok_or((1, "no available service"))
        };

        match selected {
            Ok((instance_id, tx)) => {
                // 来源只信网关记录，不能信调用方自己填。
                req.target_instance_id = instance_id as i32;
                req.source_service_id = source_service_id as u32;
                req.source_instance_id = source_instance_id;
                let Some((forward_msg_id, forward_payload)) =
                    encode_message(&MessageType::GatewayForwardToServerReq(req))
                else {
                    warn!("failed to encode ForwardToServerReq");
                    return;
                };
                let forward_data = encode_backend_frame(
                    CMD_GATEWAY_CONTROL,
                    forward_msg_id as u16,
                    0, // 转发请求不需要回复，serial=0,实际序号在payload里面自己解析
                    0,
                    &forward_payload,
                );

                if tx.send(WriterMessage::Send(forward_data, true)).is_err() {
                    warn!(
                        "forward to service_id={} instance_id={} failed: channel closed",
                        target_service_id, instance_id
                    );
                    self.reply_control_error_if_request(
                        serial,
                        2,
                        "target service instance unreachable",
                    );
                }
            }
            Err((code, message)) => {
                warn!(
                    "forward target service_id={} instance {} failed: code={} {}",
                    req.target_service_id, req.target_instance_id, code, message
                );
                self.reply_control_error_if_request(serial, code, message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::try_extract_backend_frame;
    use crate::config::{GatewayConfig, GatewaySection};
    use ::protocol::gateway::UnbindServiceReq;
    use ::protocol::message_map::{MessageType, decode_message, encode_message};

    #[test]
    fn heartbeat_acknowledges_matching_nonce() {
        let start = Instant::now();
        let mut heartbeat = HeartbeatState::default();
        heartbeat.mark_sent(7, start);

        assert_eq!(
            heartbeat.acknowledge(7, start + Duration::from_millis(42)),
            Some(42)
        );
        assert!(!heartbeat.has_pending());
    }

    #[test]
    fn heartbeat_ignores_unmatched_nonce() {
        let start = Instant::now();
        let mut heartbeat = HeartbeatState::default();
        heartbeat.mark_sent(7, start);

        assert_eq!(
            heartbeat.acknowledge(8, start + Duration::from_millis(42)),
            None
        );
        assert!(heartbeat.has_pending());
    }

    #[test]
    fn heartbeat_detects_timeout() {
        let start = Instant::now();
        let mut heartbeat = HeartbeatState::default();
        heartbeat.mark_sent(7, start);

        assert!(!heartbeat.timed_out(start + Duration::from_secs(14), HEARTBEAT_TIMEOUT));
        assert!(heartbeat.timed_out(start + Duration::from_secs(15), HEARTBEAT_TIMEOUT));
    }

    #[tokio::test]
    async fn unbind_service_request_replies_with_response() {
        let ctx = GatewayContext::new(GatewayConfig {
            gateway: GatewaySection {
                client_listen: "127.0.0.1:0".to_string(),
                internal_listen: "127.0.0.1:0".to_string(),
            },
            route: vec![],
        });
        ctx.router.bind_service(42, 1, 7);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut delegate = InternalDelegate::new(ctx.clone());
        delegate.session_id = 9001;
        delegate.tx = Some(tx);

        let (msg_id, payload) =
            encode_message(&MessageType::GatewayUnbindServiceReq(UnbindServiceReq {
                session_id: 42,
                service_id: 1,
            }))
            .unwrap();

        delegate
            .handle_internal(
                CMD_GATEWAY_CONTROL,
                msg_id as u16,
                -123,
                42,
                Bytes::from(payload),
            )
            .await
            .unwrap();

        assert_eq!(ctx.router.binding_count(1, 7), 0);

        let WriterMessage::Send(data, _) = rx.recv().await.unwrap() else {
            panic!("expected response frame");
        };
        let mut buf = BytesMut::from(data.as_ref());
        let frame = try_extract_backend_frame(&mut buf).unwrap().unwrap();
        assert_eq!(frame.serial, 123);
        assert_eq!(frame.session_id, 42);
        assert!(matches!(
            decode_message(frame.msg_id as u32, &frame.payload).unwrap(),
            MessageType::GatewayUnbindServiceResp(resp)
                if resp.session_id == 42 && resp.service_id == 1 && resp.code == 0
        ));
    }
}
