use std::net::SocketAddr;
use std::time::Duration;

use ::protocol::gateway_client::{
    GatewayErrorResp, ServerStatusPush, ServiceStatus as GatewayServiceStatus,
};
use ::protocol::gateway_internal::{SessionOfflinePush, SessionOnlinePush};
use ::protocol::message_map::{MessageType, decode_message, encode_message};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use base::net::{WriterMessage, session_delegate::SessionDelegate};

use crate::codec::{encode_backend_frame, encode_client_frame, try_extract_client_frame};
use crate::context::GatewayContext;
use crate::frame_cmd::{
    BACKEND_CMD_BUSINESS, BACKEND_CMD_CONTROL, CMD_BUSINESS, CMD_GATEWAY_MESSAGE,
};
use crate::router::RouteTarget;

const SERVER_STATUS_REQ_MSG_ID: u16 =
    ::protocol::gateway_client::server_status_req::MsgId::Id as u16;

// 客户端业务帧和网关消息帧不能混用。
const GATEWAY_ERR_INVALID_CMD: u32 = 1;
// 路由命中了服务类型，但当前没有可用实例。
const GATEWAY_ERR_SERVICE_UNAVAILABLE: u32 = 2;
// 这条消息需要先绑定服务实例。
const GATEWAY_ERR_SERVICE_NOT_BOUND: u32 = 3;
// msg_id 没有命中 gateway.toml 里的任何路由。
const GATEWAY_ERR_UNKNOWN_ROUTE: u32 = 4;
// 客户端还没有请求服务器状态，不能转发业务消息。
const GATEWAY_ERR_UNAUTHENTICATED: u32 = 5;

pub struct ClientDelegate {
    ctx: GatewayContext,
    session_id: u32,
    tx: Option<mpsc::UnboundedSender<WriterMessage>>,
}

impl ClientDelegate {
    pub fn new(ctx: GatewayContext) -> Self {
        Self {
            ctx,
            session_id: 0,
            tx: None,
        }
    }

    fn service_visible_to_session(&self, service_id: u8, instance_id: u32) -> bool {
        self.ctx.router.service_has_unbound_route(service_id)
            || self
                .ctx
                .router
                .bound_instance(self.session_id, service_id)
                .is_some_and(|bound_instance_id| bound_instance_id == instance_id)
    }

    fn build_status(&self) -> ServerStatusPush {
        let services = self
            .ctx
            .registry
            .service_statuses()
            .into_iter()
            .filter(|(service_id, instance_id)| {
                self.service_visible_to_session(*service_id, *instance_id)
            })
            .map(|(service_id, instance_id)| GatewayServiceStatus {
                service_id: service_id as u32,
                instance_id,
                online: true,
            })
            .collect();

        ServerStatusPush { services }
    }

    fn encode_status_frame(&self, serial: i32) -> Option<Bytes> {
        let status = MessageType::GatewayClientServerStatusPush(self.build_status());
        let (msg_id, payload) = encode_message(&status)?;
        let response_serial = if serial < 0 { -serial } else { 0 };
        Some(encode_client_frame(
            CMD_GATEWAY_MESSAGE,
            msg_id as u16,
            response_serial,
            &payload,
        ))
    }

    fn broadcast_session_online(&self) {
        let notify = MessageType::GatewayInternalSessionOnlinePush(SessionOnlinePush {
            session_id: self.session_id,
        });
        let (msg_id, payload) = encode_message(&notify).unwrap();
        let data = encode_backend_frame(
            BACKEND_CMD_CONTROL,
            msg_id as u16,
            0,
            self.session_id,
            &payload,
        );
        self.ctx.registry.broadcast(data);
    }

    fn start_auth_timeout(&self, session_id: u32, tx: mpsc::UnboundedSender<WriterMessage>) {
        let sessions = self.ctx.sessions.clone();
        let timeout = Duration::from_secs(self.ctx.config.gateway.client_auth_timeout_secs);

        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if !sessions.is_authenticated(session_id) {
                warn!(
                    "client session {} auth timeout after {}s",
                    session_id,
                    timeout.as_secs()
                );
                let _ = tx.send(WriterMessage::Close);
            }
        });
    }

    fn handle_server_status_request(&self, serial: i32) {
        // 第一次请求服务器状态时，才把连接视为真正的游戏客户端。
        if self.ctx.sessions.authenticate(self.session_id) {
            self.broadcast_session_online();
        }

        if let Some(data) = self.encode_status_frame(serial)
            && let Some(tx) = &self.tx
        {
            let _ = tx.send(WriterMessage::Send(data, true));
        }
    }

    fn reply_error_if_request(&self, _msg_id: u16, serial: i32, code: u32, message: &str) {
        if serial >= 0 {
            warn!(
                "client session {} sent non-request serial {}, ignoring error reply",
                self.session_id, serial
            );
            return;
        }

        if let Some(tx) = &self.tx {
            let resp = GatewayErrorResp {
                code,
                message: message.to_string(),
            };
            let (gateway_msg_id, payload) =
                encode_message(&MessageType::GatewayClientGatewayErrorResp(resp)).unwrap();
            let data = encode_client_frame(
                CMD_GATEWAY_MESSAGE,
                gateway_msg_id as u16,
                -serial,
                &payload,
            );
            let _ = tx.send(WriterMessage::Send(data, true));
        }
    }
}

#[async_trait]
impl SessionDelegate for ClientDelegate {
    async fn on_session_start(
        &mut self,
        session_id: u32,
        _addr: &SocketAddr,
        tx: mpsc::UnboundedSender<WriterMessage>,
    ) -> anyhow::Result<()> {
        self.session_id = session_id;
        self.tx = Some(tx.clone());
        self.ctx.sessions.add(session_id, tx.clone());
        self.start_auth_timeout(session_id, tx);

        debug!("client session {} connected", session_id);
        Ok(())
    }

    async fn on_session_close(&mut self) -> anyhow::Result<()> {
        debug!("client session {} disconnected", self.session_id);

        let was_authenticated = self.ctx.sessions.remove(self.session_id);
        self.ctx.router.cleanup_session(self.session_id);

        if was_authenticated {
            let notify = MessageType::GatewayInternalSessionOfflinePush(SessionOfflinePush {
                session_id: self.session_id,
            });
            let (msg_id, payload) = encode_message(&notify).unwrap();
            let data = encode_backend_frame(
                BACKEND_CMD_CONTROL,
                msg_id as u16,
                0,
                self.session_id,
                &payload,
            );
            self.ctx.registry.broadcast(data);
        }

        Ok(())
    }

    async fn on_try_extract_frame(
        &mut self,
        buffer: &mut BytesMut,
    ) -> anyhow::Result<Option<Bytes>> {
        match try_extract_client_frame(buffer)? {
            Some(frame) => {
                let mut out = BytesMut::with_capacity(7 + frame.payload.len());
                out.extend_from_slice(&[frame.cmd]);
                out.extend_from_slice(&frame.msg_id.to_be_bytes());
                out.extend_from_slice(&frame.serial.to_be_bytes());
                out.extend_from_slice(&frame.payload);
                Ok(Some(out.freeze()))
            }
            None => Ok(None),
        }
    }

    async fn on_recv_frame(&mut self, frame: Bytes) -> anyhow::Result<()> {
        if frame.len() < 7 {
            return Ok(());
        }

        let cmd = frame[0];
        let msg_id = u16::from_be_bytes([frame[1], frame[2]]);
        let serial = i32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
        let payload = frame.slice(7..);

        if msg_id == SERVER_STATUS_REQ_MSG_ID {
            if cmd != CMD_GATEWAY_MESSAGE {
                warn!(
                    "client session {} sent server status request with cmd={}, ignored",
                    self.session_id, cmd
                );
                self.reply_error_if_request(
                    msg_id,
                    serial,
                    GATEWAY_ERR_INVALID_CMD,
                    "gateway message command is required",
                );
                return Ok(());
            }

            match decode_message(msg_id as u32, &payload) {
                Ok(MessageType::GatewayClientServerStatusReq(_)) => {
                    self.handle_server_status_request(serial);
                }
                Ok(_) | Err(_) => {
                    self.reply_error_if_request(
                        msg_id,
                        serial,
                        GATEWAY_ERR_UNKNOWN_ROUTE,
                        "invalid server status request",
                    );
                }
            }
            return Ok(());
        }

        if cmd != CMD_BUSINESS {
            warn!(
                "client session {} sent non-business cmd={}, ignored",
                self.session_id, cmd
            );
            self.reply_error_if_request(
                msg_id,
                serial,
                GATEWAY_ERR_INVALID_CMD,
                "business command is required",
            );
            return Ok(());
        }

        if !self.ctx.sessions.is_authenticated(self.session_id) {
            warn!(
                "client session {} sent business msg_id={} before auth",
                self.session_id, msg_id
            );
            self.reply_error_if_request(
                msg_id,
                serial,
                GATEWAY_ERR_UNAUTHENTICATED,
                "client is not authenticated",
            );
            return Ok(());
        }

        match self.ctx.router.resolve(msg_id, self.session_id) {
            RouteTarget::Service(service_id) => {
                if let Some(tx) = self.ctx.registry.find_by_service(service_id) {
                    let data = encode_backend_frame(
                        BACKEND_CMD_BUSINESS,
                        msg_id,
                        serial,
                        self.session_id,
                        &payload,
                    );
                    let _ = tx.send(WriterMessage::Send(data, true));
                } else {
                    warn!(
                        "no service registered, service_id={}, msg_id={}",
                        service_id, msg_id
                    );
                    self.reply_error_if_request(
                        msg_id,
                        serial,
                        GATEWAY_ERR_SERVICE_UNAVAILABLE,
                        "target service unavailable",
                    );
                }
            }
            RouteTarget::BoundService {
                service_id,
                instance_id,
            } => {
                if let Some(tx) = self.ctx.registry.find_by_instance(service_id, instance_id) {
                    let data = encode_backend_frame(
                        BACKEND_CMD_BUSINESS,
                        msg_id,
                        serial,
                        self.session_id,
                        &payload,
                    );
                    let _ = tx.send(WriterMessage::Send(data, true));
                } else {
                    warn!(
                        "service instance not found, service_id={}, instance_id={}, msg_id={}",
                        service_id, instance_id, msg_id
                    );
                    self.reply_error_if_request(
                        msg_id,
                        serial,
                        GATEWAY_ERR_SERVICE_UNAVAILABLE,
                        "bound service instance unavailable",
                    );
                }
            }
            RouteTarget::ServiceNotBound(service_id) => {
                warn!(
                    "session {} has no binding for service_id={}, dropping msg_id={}",
                    self.session_id, service_id, msg_id
                );
                self.reply_error_if_request(
                    msg_id,
                    serial,
                    GATEWAY_ERR_SERVICE_NOT_BOUND,
                    "session has no bound service instance",
                );
            }
            RouteTarget::Unknown => {
                warn!(
                    "unknown route for msg_id={} from session {}",
                    msg_id, self.session_id
                );
                self.reply_error_if_request(
                    msg_id,
                    serial,
                    GATEWAY_ERR_UNKNOWN_ROUTE,
                    "unknown route",
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{try_extract_backend_frame, try_extract_client_frame};
    use crate::config::{GatewayConfig, GatewaySection, RouteEntry};
    use ::protocol::gateway_client::ServerStatusReq;
    use ::protocol::message_map::decode_message;

    fn test_context() -> GatewayContext {
        test_context_with_timeout(30)
    }

    fn test_context_with_timeout(client_auth_timeout_secs: u64) -> GatewayContext {
        GatewayContext::new(GatewayConfig {
            gateway: GatewaySection {
                client_listen: "127.0.0.1:0".to_string(),
                internal_listen: "127.0.0.1:0".to_string(),
                client_auth_timeout_secs,
            },
            route: vec![RouteEntry {
                service_id: 0,
                range: [1, 19999],
                require_binding: false,
            }],
        })
    }

    fn inbound_client_frame(msg_id: u16, serial: i32, payload: &[u8]) -> Bytes {
        inbound_frame(CMD_BUSINESS, msg_id, serial, payload)
    }

    fn inbound_gateway_frame(msg_id: u16, serial: i32, payload: &[u8]) -> Bytes {
        inbound_frame(CMD_GATEWAY_MESSAGE, msg_id, serial, payload)
    }

    fn inbound_frame(cmd: u8, msg_id: u16, serial: i32, payload: &[u8]) -> Bytes {
        let mut buf = BytesMut::with_capacity(7 + payload.len());
        buf.extend_from_slice(&[cmd]);
        buf.extend_from_slice(&msg_id.to_be_bytes());
        buf.extend_from_slice(&serial.to_be_bytes());
        buf.extend_from_slice(payload);
        buf.freeze()
    }

    fn recv_backend_message(rx: &mut mpsc::UnboundedReceiver<WriterMessage>) -> MessageType {
        let WriterMessage::Send(data, _) = rx.try_recv().expect("expected backend frame") else {
            panic!("expected backend send");
        };
        let mut buf = BytesMut::from(data.as_ref());
        let frame = try_extract_backend_frame(&mut buf).unwrap().unwrap();
        decode_message(frame.msg_id as u32, &frame.payload).unwrap()
    }

    fn recv_client_message(rx: &mut mpsc::UnboundedReceiver<WriterMessage>) -> (i32, MessageType) {
        let WriterMessage::Send(data, _) = rx.try_recv().expect("expected client frame") else {
            panic!("expected client send");
        };
        let mut buf = BytesMut::from(data.as_ref());
        let frame = try_extract_client_frame(&mut buf).unwrap().unwrap();
        (
            frame.serial,
            decode_message(frame.msg_id as u32, &frame.payload).unwrap(),
        )
    }

    #[tokio::test]
    async fn connect_does_not_notify_backend_until_status_request() {
        let ctx = test_context();
        let (backend_tx, mut backend_rx) = mpsc::unbounded_channel();
        ctx.registry.register(100, 0, 1, backend_tx);

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx.clone());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        assert!(backend_rx.try_recv().is_err());
        assert!(client_rx.try_recv().is_err());
        assert!(!ctx.sessions.is_authenticated(42));
    }

    #[tokio::test]
    async fn auth_timeout_closes_unauthenticated_client() {
        let ctx = test_context_with_timeout(0);
        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx);
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        let close = tokio::time::timeout(Duration::from_millis(50), client_rx.recv())
            .await
            .unwrap();
        assert!(matches!(close, Some(WriterMessage::Close)));
    }

    #[tokio::test]
    async fn unauthenticated_business_is_not_forwarded() {
        let ctx = test_context();
        let (backend_tx, mut backend_rx) = mpsc::unbounded_channel();
        ctx.registry.register(100, 0, 1, backend_tx);

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx.clone());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        delegate
            .on_recv_frame(inbound_client_frame(100, -3, &[]))
            .await
            .unwrap();

        assert!(backend_rx.try_recv().is_err());
        assert!(matches!(
            recv_client_message(&mut client_rx),
            (3, MessageType::GatewayClientGatewayErrorResp(resp))
                if resp.code == GATEWAY_ERR_UNAUTHENTICATED
        ));
    }

    #[tokio::test]
    async fn server_status_request_requires_gateway_command() {
        let ctx = test_context();
        let (backend_tx, mut backend_rx) = mpsc::unbounded_channel();
        ctx.registry.register(100, 0, 1, backend_tx);

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx.clone());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        let (msg_id, payload) = encode_message(&MessageType::GatewayClientServerStatusReq(
            ServerStatusReq {},
        ))
        .unwrap();
        delegate
            .on_recv_frame(inbound_client_frame(msg_id as u16, -7, &payload))
            .await
            .unwrap();

        assert!(!ctx.sessions.is_authenticated(42));
        assert!(backend_rx.try_recv().is_err());
        assert!(matches!(
            recv_client_message(&mut client_rx),
            (7, MessageType::GatewayClientGatewayErrorResp(resp))
                if resp.code == GATEWAY_ERR_INVALID_CMD
        ));
    }

    #[tokio::test]
    async fn first_status_request_authenticates_and_notifies_backend_once() {
        let ctx = test_context();
        let (backend_tx, mut backend_rx) = mpsc::unbounded_channel();
        ctx.registry.register(100, 0, 1, backend_tx);

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx.clone());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        let (msg_id, payload) = encode_message(&MessageType::GatewayClientServerStatusReq(
            ServerStatusReq {},
        ))
        .unwrap();
        delegate
            .on_recv_frame(inbound_gateway_frame(msg_id as u16, -7, &payload))
            .await
            .unwrap();

        assert!(ctx.sessions.is_authenticated(42));
        assert!(matches!(
            recv_backend_message(&mut backend_rx),
            MessageType::GatewayInternalSessionOnlinePush(push) if push.session_id == 42
        ));
        assert!(matches!(
            recv_client_message(&mut client_rx),
            (7, MessageType::GatewayClientServerStatusPush(push)) if push.services.len() == 1
        ));

        delegate
            .on_recv_frame(inbound_gateway_frame(msg_id as u16, -8, &payload))
            .await
            .unwrap();

        assert!(backend_rx.try_recv().is_err());
        assert!(matches!(
            recv_client_message(&mut client_rx),
            (8, MessageType::GatewayClientServerStatusPush(_))
        ));
    }

    #[tokio::test]
    async fn authenticated_close_notifies_backend_offline() {
        let ctx = test_context();
        let (backend_tx, mut backend_rx) = mpsc::unbounded_channel();
        ctx.registry.register(100, 0, 1, backend_tx);

        let (client_tx, _client_rx) = mpsc::unbounded_channel();
        let mut delegate = ClientDelegate::new(ctx.clone());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        delegate
            .on_session_start(42, &addr, client_tx)
            .await
            .unwrap();

        let (msg_id, payload) = encode_message(&MessageType::GatewayClientServerStatusReq(
            ServerStatusReq {},
        ))
        .unwrap();
        delegate
            .on_recv_frame(inbound_gateway_frame(msg_id as u16, -7, &payload))
            .await
            .unwrap();
        let _ = recv_backend_message(&mut backend_rx);

        delegate.on_session_close().await.unwrap();

        assert!(matches!(
            recv_backend_message(&mut backend_rx),
            MessageType::GatewayInternalSessionOfflinePush(push) if push.session_id == 42
        ));
    }
}
