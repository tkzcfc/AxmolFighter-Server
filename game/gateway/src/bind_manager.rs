use tokio::sync::mpsc;
use tracing::{debug, warn};

use base::net::WriterMessage;
use protocol::gateway::{BindServiceReq, BindServiceResp};
use protocol::message_map::{MessageType, encode_message};

use crate::codec::encode_backend_frame;
use crate::context::GatewayContext;
use crate::frame_cmd::CMD_GATEWAY_CONTROL;

const BIND_OK: u32 = 0;
// 没有可用服务实例。
const NO_AVAILABLE_SERVICE: u32 = 1;
// 路由命中服务类型，但当前没有可用服务实例。
const TARGET_SERVICE_UNREACHABLE: u32 = 2;
// 该消息需要绑定服务实例，但当前 session 尚未绑定。
pub const REQUESTER_SERVICE_NOT_REGISTERED: u32 = 3;

#[derive(Clone)]
pub struct BindRequester {
    pub serial: i32,
    pub tx: mpsc::UnboundedSender<WriterMessage>,
}

#[derive(Clone)]
pub struct BindManager;

impl BindManager {
    pub fn new() -> Self {
        Self
    }

    pub fn reject_request(
        &self,
        tx: &mpsc::UnboundedSender<WriterMessage>,
        serial: i32,
        req: &BindServiceReq,
        code: u32,
        message: &str,
    ) {
        Self::send_bind_result(tx, serial, req, 0, code, message);
    }

    pub fn start(&self, ctx: &GatewayContext, requester: BindRequester, req: BindServiceReq) {
        let service_id = req.service_id as u8;

        let selected = if req.target_instance_id >= 0 {
            let instance_id = req.target_instance_id as u32;
            ctx.registry
                .find_by_instance(service_id, instance_id)
                .map(|_| instance_id)
                .ok_or((
                    TARGET_SERVICE_UNREACHABLE,
                    "target service instance unreachable",
                ))
        } else {
            ctx.registry
                .select_lowest_load_instance(service_id, |service_id, instance_id| {
                    ctx.router.binding_count(service_id, instance_id)
                })
                .map(|candidate| candidate.instance_id)
                .ok_or((NO_AVAILABLE_SERVICE, "no available service"))
        };

        match selected {
            Ok(instance_id) => {
                ctx.router
                    .bind_service(req.session_id, service_id, instance_id);
                debug!(
                    "bound session {} to service_id={} instance {}",
                    req.session_id, req.service_id, instance_id
                );
                Self::send_bind_result(
                    &requester.tx,
                    requester.serial,
                    &req,
                    instance_id,
                    BIND_OK,
                    "",
                );
            }
            Err((code, message)) => {
                warn!(
                    "failed to bind session {} to service_id={} target_instance_id={}: {}",
                    req.session_id, req.service_id, req.target_instance_id, message
                );
                Self::send_bind_result(&requester.tx, requester.serial, &req, 0, code, message);
            }
        }
    }

    fn send_bind_result(
        tx: &mpsc::UnboundedSender<WriterMessage>,
        serial: i32,
        req: &BindServiceReq,
        instance_id: u32,
        code: u32,
        message: &str,
    ) {
        if serial == 0 {
            if code != 0 {
                warn!(
                    "bind failed without response session {} service_id={} code={} message={}",
                    req.session_id, req.service_id, code, message
                );
            }
            return;
        }

        let resp = BindServiceResp {
            session_id: req.session_id,
            service_id: req.service_id,
            instance_id,
            code,
            message: message.to_string(),
        };
        let response_serial = if serial < 0 { -serial } else { serial };
        let (msg_id, payload) = encode_message(&MessageType::GatewayBindServiceResp(resp)).unwrap();
        let data = encode_backend_frame(
            CMD_GATEWAY_CONTROL,
            msg_id as u16,
            response_serial,
            req.session_id,
            &payload,
        );
        let _ = tx.send(WriterMessage::Send(data, true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};
    use protocol::gateway::BindServiceReq;

    use crate::codec::try_extract_backend_frame;
    use crate::config::{GatewayConfig, GatewaySection};

    fn test_context() -> GatewayContext {
        GatewayContext::new(GatewayConfig {
            gateway: GatewaySection {
                client_listen: "127.0.0.1:0".to_string(),
                internal_listen: "127.0.0.1:0".to_string(),
            },
            route: vec![],
        })
    }

    fn recv_send(rx: &mut mpsc::UnboundedReceiver<WriterMessage>) -> Bytes {
        match rx.try_recv().expect("expected outgoing frame") {
            WriterMessage::Send(data, _) => data,
            _ => panic!("expected send message"),
        }
    }

    fn frame_serial(data: Bytes) -> i32 {
        let mut buf = BytesMut::from(data.as_ref());
        try_extract_backend_frame(&mut buf).unwrap().unwrap().serial
    }

    fn requester(tx: mpsc::UnboundedSender<WriterMessage>) -> BindRequester {
        BindRequester { serial: -100, tx }
    }

    #[test]
    fn specified_instance_binds_without_load_report() {
        let ctx = test_context();
        let (target_tx, _target_rx) = mpsc::unbounded_channel();
        let (requester_tx, mut requester_rx) = mpsc::unbounded_channel();
        ctx.registry.register(101, 1, 7, target_tx);

        ctx.binds.start(
            &ctx,
            requester(requester_tx),
            BindServiceReq {
                session_id: 9001,
                service_id: 1,
                target_instance_id: 7,
            },
        );

        assert_eq!(frame_serial(recv_send(&mut requester_rx)), 100);
        assert_eq!(ctx.router.binding_count(1, 7), 1);
    }

    #[test]
    fn specified_instance_missing_returns_unreachable() {
        let ctx = test_context();
        let (requester_tx, mut requester_rx) = mpsc::unbounded_channel();

        ctx.binds.start(
            &ctx,
            requester(requester_tx),
            BindServiceReq {
                session_id: 9001,
                service_id: 1,
                target_instance_id: 7,
            },
        );

        assert_eq!(frame_serial(recv_send(&mut requester_rx)), 100);
        assert_eq!(ctx.router.binding_count(1, 7), 0);
    }

    #[test]
    fn auto_bind_uses_lowest_load_instance() {
        let ctx = test_context();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (requester_tx, mut requester_rx) = mpsc::unbounded_channel();
        ctx.registry.register(101, 1, 1, tx1);
        ctx.registry.register(102, 1, 2, tx2);
        ctx.registry.update_load(101, 1, 1, 80, true, String::new());
        ctx.registry.update_load(102, 1, 2, 20, true, String::new());

        ctx.binds.start(
            &ctx,
            requester(requester_tx),
            BindServiceReq {
                session_id: 9001,
                service_id: 1,
                target_instance_id: -1,
            },
        );

        assert_eq!(frame_serial(recv_send(&mut requester_rx)), 100);
        assert_eq!(ctx.router.binding_count(1, 1), 0);
        assert_eq!(ctx.router.binding_count(1, 2), 1);
    }
}
