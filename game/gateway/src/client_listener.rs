use std::net::SocketAddr;

use ::protocol::gateway::{
    GatewayErrorResp, ServerStatusPush, ServiceStatus as GatewayServiceStatus,
};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use prost::Message;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use base::net::{WriterMessage, session_delegate::SessionDelegate};

use crate::codec::{encode_backend_frame, encode_client_frame, try_extract_client_frame};
use crate::context::GatewayContext;
use crate::protocol::{
    CMD_BUSINESS, CMD_GATEWAY_ERROR, CMD_SERVER_STATUS, CMD_SESSION_OFFLINE, CMD_SESSION_ONLINE,
    SessionOffline, SessionOnline,
};
use crate::router::RouteTarget;

/// 客户端发了网关不接受的 cmd，比如直接发内部控制命令。
const GATEWAY_ERR_INVALID_CMD: u32 = 1;
/// msg_id 能找到目标服务类型，但当前没有可用的服务实例。
const GATEWAY_ERR_SERVICE_UNAVAILABLE: u32 = 2;
/// 这个消息需要先绑定服务实例，但当前 session 还没绑定。
const GATEWAY_ERR_SERVICE_NOT_BOUND: u32 = 3;
/// msg_id 没命中 gateway.toml 里的任何路由范围。
const GATEWAY_ERR_UNKNOWN_ROUTE: u32 = 4;

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

    fn reply_error_if_request(&self, msg_id: u16, serial: i32, code: u32, message: &str) {
        if serial < 0 {
            if let Some(tx) = &self.tx {
                let resp = GatewayErrorResp {
                    code,
                    message: message.to_string(),
                };
                let data =
                    encode_client_frame(CMD_GATEWAY_ERROR, msg_id, -serial, &resp.encode_to_vec());
                let _ = tx.send(WriterMessage::Send(data, true));
            }
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

        debug!("client session {} connected", session_id);

        let notify = SessionOnline { session_id };
        let data = encode_backend_frame(CMD_SESSION_ONLINE, 0, 0, session_id, &notify.encode());
        self.ctx.registry.broadcast(data);

        let status_frame = encode_client_frame(
            CMD_SERVER_STATUS,
            0,
            0,
            &self.build_status().encode_to_vec(),
        );
        let _ = tx.send(WriterMessage::Send(status_frame, true));

        Ok(())
    }

    async fn on_session_close(&mut self) -> anyhow::Result<()> {
        debug!("client session {} disconnected", self.session_id);

        self.ctx.sessions.remove(self.session_id);
        self.ctx.router.cleanup_session(self.session_id);

        let notify = SessionOffline {
            session_id: self.session_id,
        };
        let data =
            encode_backend_frame(CMD_SESSION_OFFLINE, 0, 0, self.session_id, &notify.encode());
        self.ctx.registry.broadcast(data);

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

        if cmd != CMD_BUSINESS {
            warn!(
                "client session {} sent non-business cmd={}, ignored",
                self.session_id, cmd
            );
            self.reply_error_if_request(
                msg_id,
                serial,
                GATEWAY_ERR_INVALID_CMD,
                "client command is not allowed",
            );
            return Ok(());
        }

        match self.ctx.router.resolve(msg_id, self.session_id) {
            RouteTarget::Service(service_id) => {
                if let Some(tx) = self.ctx.registry.find_by_service(service_id) {
                    let data = encode_backend_frame(
                        CMD_BUSINESS,
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
                        CMD_BUSINESS,
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
