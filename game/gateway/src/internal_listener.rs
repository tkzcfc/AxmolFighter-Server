use std::net::SocketAddr;

use ::protocol::gateway::{
    ServerOfflinePush, ServerOnlinePush, ServerRegResp, ServerStatusPush, ServiceEndpoint,
    ServiceStatus as GatewayServiceStatus, SessionOnlinePush,
};
use ::protocol::message_map::{MessageType, decode_message, encode_message};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use base::net::{WriterMessage, session_delegate::SessionDelegate};

use crate::codec::{encode_backend_frame, encode_client_frame, try_extract_backend_frame};
use crate::context::GatewayContext;
use crate::protocol::*;

pub struct InternalDelegate {
    ctx: GatewayContext,
    session_id: u32,
    tx: Option<mpsc::UnboundedSender<WriterMessage>>,
    registered: Option<(u8, u32)>,
}

impl InternalDelegate {
    pub fn new(ctx: GatewayContext) -> Self {
        Self {
            ctx,
            session_id: 0,
            tx: None,
            registered: None,
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
        self.tx = Some(tx);
        debug!("internal connection {} from {}", session_id, addr);
        Ok(())
    }

    async fn on_session_close(&mut self) -> anyhow::Result<()> {
        debug!("internal connection {} closed", self.session_id);

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
            let client_data = encode_client_frame(CMD_BUSINESS, msg_id, serial, &payload);
            if !self.ctx.sessions.send_to_client(session_id, client_data) {
                debug!(
                    "client session {} not found, dropping response msg_id={}",
                    session_id, msg_id
                );
            }
            return Ok(());
        }

        self.handle_internal(cmd, msg_id, session_id, payload).await
    }
}

impl InternalDelegate {
    async fn handle_internal(
        &mut self,
        cmd: u8,
        msg_id: u16,
        _session_id: u32,
        payload: Bytes,
    ) -> anyhow::Result<()> {
        if cmd != CMD_GATEWAY_CONTROL {
            debug!("unhandled internal cmd={}", cmd);
            return Ok(());
        }

        match decode_message(msg_id as u32, &payload)? {
            MessageType::GatewayServerRegReq(req) => {
                let existing = self.ctx.registry.list_all();
                let service_id = req.service_id as u8;

                self.ctx.registry.register(
                    self.session_id,
                    service_id,
                    req.instance_id,
                    self.tx.clone().unwrap(),
                );
                self.registered = Some((service_id, req.instance_id));

                info!(
                    "backend service_id={} instance {} registered",
                    req.service_id, req.instance_id
                );

                let resp = ServerRegResp {
                    code: 0,
                    servers: existing
                        .into_iter()
                        .map(|(service_id, instance_id)| ServiceEndpoint {
                            service_id: service_id as u32,
                            instance_id,
                        })
                        .collect(),
                };
                let data = self.encode_control_frame(MessageType::GatewayServerRegResp(resp), 0, 0);
                if let Some(tx) = &self.tx {
                    let _ = tx.send(WriterMessage::Send(data, true));
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
                let service_id = req.service_id as u8;
                let selected = if req.target_instance_id < 0 {
                    self.ctx.registry.least_loaded_instance(
                        service_id,
                        |service_id, instance_id| {
                            self.ctx.router.binding_count(service_id, instance_id)
                        },
                    )
                } else {
                    let instance_id = req.target_instance_id as u32;
                    self.ctx
                        .registry
                        .find_by_instance(service_id, instance_id)
                        .map(|tx| (instance_id, tx))
                };

                if let Some((instance_id, _)) = selected {
                    self.ctx
                        .router
                        .bind_service(req.session_id, service_id, instance_id);
                    debug!(
                        "bound session {} to service_id={} instance {}",
                        req.session_id, req.service_id, instance_id
                    );
                } else {
                    warn!(
                        "failed to bind session {} to service_id={} target_instance_id={}",
                        req.session_id, req.service_id, req.target_instance_id
                    );
                }
            }

            MessageType::GatewayUnbindServiceReq(req) => {
                self.ctx
                    .router
                    .unbind_service(req.session_id, req.service_id as u8);
                debug!(
                    "unbound session {} from service_id={}",
                    req.session_id, req.service_id
                );
            }

            MessageType::GatewayKickSessionReq(req) => {
                self.ctx.sessions.kick(req.session_id);
                self.ctx.router.cleanup_session(req.session_id);
                debug!("kicked session {}", req.session_id);
            }

            MessageType::GatewayForwardToServerReq(req) => {
                if let Some(tx) = self
                    .ctx
                    .registry
                    .find_by_instance(req.target_service_id as u8, req.target_instance_id)
                {
                    let _ = tx.send(WriterMessage::Send(Bytes::from(req.payload), true));
                } else {
                    warn!(
                        "forward target service_id={} instance {} not found",
                        req.target_service_id, req.target_instance_id
                    );
                }
            }

            _ => {
                debug!("unhandled gateway control msg_id={}", msg_id);
            }
        }

        Ok(())
    }
}
