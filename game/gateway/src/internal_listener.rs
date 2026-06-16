use std::net::SocketAddr;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use base::net::{session_delegate::SessionDelegate, WriterMessage};

use crate::codec::{
    encode_backend_frame, encode_client_frame, try_extract_backend_frame,
};
use crate::context::GatewayContext;
use crate::protocol::*;

/// 后端服务连接 delegate（处理注册和消息回传）
pub struct InternalDelegate {
    ctx: GatewayContext,
    session_id: u32,
    tx: Option<mpsc::UnboundedSender<WriterMessage>>,
    /// 注册后的服务类型和实例ID
    registered: Option<(ServiceType, u32)>,
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

        // 从注册表移除
        if let Some((service_type, instance_id)) = self.ctx.registry.remove(self.session_id) {
            info!(
                "backend {:?} instance {} unregistered (disconnected)",
                service_type, instance_id
            );

            // 如果是战斗服，清理相关绑定
            if service_type == ServiceType::Battle {
                self.ctx.router.unbind_all_by_instance(instance_id);
            }

            // 广播 ServerOffline 给其他后端
            let notify = ServerOffline {
                service_type: service_type as u8,
                instance_id,
            };
            let data = encode_backend_frame(
                MSG_SERVER_OFFLINE,
                0, // 推送
                0,
                &notify.encode(),
            );
            self.ctx.registry.broadcast_except(self.session_id, data);

            // 通知所有客户端服务状态变更
            let status = ServerStatus {
                services: vec![(
                    service_type as u8,
                    self.ctx.registry.has_online(service_type),
                )],
            };
            let client_data = encode_client_frame(MSG_SERVER_STATUS, 0, &status.encode());
            self.ctx.sessions.broadcast_to_clients(client_data);
        }

        Ok(())
    }

    async fn on_try_extract_frame(
        &mut self,
        buffer: &mut BytesMut,
    ) -> anyhow::Result<Option<Bytes>> {
        // 后端使用 14 字节头帧格式
        match try_extract_backend_frame(buffer)? {
            Some(frame) => {
                // 打包 msg_id(2) + serial(4) + session_id(4) + payload
                let mut out = BytesMut::with_capacity(10 + frame.payload.len());
                out.extend_from_slice(&frame.msg_id.to_le_bytes());
                out.extend_from_slice(&frame.serial.to_le_bytes());
                out.extend_from_slice(&frame.session_id.to_le_bytes());
                out.extend_from_slice(&frame.payload);
                Ok(Some(out.freeze()))
            }
            None => Ok(None),
        }
    }

    async fn on_recv_frame(&mut self, frame: Bytes) -> anyhow::Result<()> {
        if frame.len() < 10 {
            return Ok(());
        }

        let msg_id = u16::from_le_bytes([frame[0], frame[1]]);
        let serial = i32::from_le_bytes([frame[2], frame[3], frame[4], frame[5]]);
        let session_id = u32::from_le_bytes([frame[6], frame[7], frame[8], frame[9]]);
        let payload = frame.slice(10..);

        // 处理内部协议
        if INTERNAL_MSG_RANGE.contains(&msg_id) {
            self.handle_internal(msg_id, session_id, payload).await?;
            return Ok(());
        }

        // 非内部协议：后端回包，转发给客户端（透传 serial）
        let client_data = encode_client_frame(msg_id, serial, &payload);
        if !self.ctx.sessions.send_to_client(session_id, client_data) {
            debug!(
                "client session {} not found, dropping response msg_id={}",
                session_id, msg_id
            );
        }

        Ok(())
    }
}

impl InternalDelegate {
    /// 处理网关内部协议消息
    async fn handle_internal(
        &mut self,
        msg_id: u16,
        _session_id: u32,
        payload: Bytes,
    ) -> anyhow::Result<()> {
        match msg_id {
            MSG_SERVER_REG_REQ => {
                let req = ServerRegReq::decode(payload)?;
                let service_type = ServiceType::from_u8(req.service_type);

                if let Some(stype) = service_type {
                    // 获取已注册列表（注册前）
                    let existing = self.ctx.registry.list_all();

                    // 注册
                    self.ctx.registry.register(
                        self.session_id,
                        stype,
                        req.instance_id,
                        self.tx.clone().unwrap(),
                    );
                    self.registered = Some((stype, req.instance_id));

                    info!(
                        "backend {:?} instance {} registered",
                        stype, req.instance_id
                    );

                    // 回复注册成功 + 已注册服务列表
                    let resp = ServerRegResp {
                        code: 0,
                        servers: existing,
                    };
                    let data = encode_backend_frame(MSG_SERVER_REG_RESP, 0, 0, &resp.encode());
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(WriterMessage::Send(data, true));
                    }

                    // 广播 ServerOnline 给其他后端
                    let notify = ServerOnline {
                        service_type: stype as u8,
                        instance_id: req.instance_id,
                    };
                    let broadcast_data =
                        encode_backend_frame(MSG_SERVER_ONLINE, 0, 0, &notify.encode());
                    self.ctx
                        .registry
                        .broadcast_except(self.session_id, broadcast_data);

                    // 通知所有客户端服务状态变更
                    let status = ServerStatus {
                        services: vec![(stype as u8, true)],
                    };
                    let client_data = encode_client_frame(MSG_SERVER_STATUS, 0, &status.encode());
                    self.ctx.sessions.broadcast_to_clients(client_data);

                    // 通知新注册后端所有当前在线的客户端
                    for sid in self.ctx.sessions.online_sessions() {
                        let online = SessionOnline { session_id: sid };
                        let notify_data =
                            encode_backend_frame(MSG_SESSION_ONLINE, 0, sid, &online.encode());
                        if let Some(tx) = &self.tx {
                            let _ = tx.send(WriterMessage::Send(notify_data, true));
                        }
                    }
                } else {
                    warn!("unknown service_type {} in ServerRegReq", req.service_type);
                    let resp = ServerRegResp {
                        code: 1,
                        servers: vec![],
                    };
                    let data = encode_backend_frame(MSG_SERVER_REG_RESP, 0, 0, &resp.encode());
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(WriterMessage::Send(data, true));
                    }
                }
            }

            MSG_BIND_BATTLE => {
                let req = BindBattle::decode(payload)?;
                self.ctx
                    .router
                    .bind_battle(req.session_id, req.battle_instance_id);
                debug!(
                    "bound session {} to battle instance {}",
                    req.session_id, req.battle_instance_id
                );
            }

            MSG_UNBIND_BATTLE => {
                let req = UnbindBattle::decode(payload)?;
                self.ctx.router.unbind_battle(req.session_id);
                debug!("unbound session {} from battle", req.session_id);
            }

            MSG_KICK_SESSION => {
                let req = KickSession::decode(payload)?;
                self.ctx.sessions.kick(req.session_id);
                self.ctx.router.cleanup_session(req.session_id);
                debug!("kicked session {}", req.session_id);
            }

            MSG_FORWARD_TO_SERVER => {
                let req = ForwardToServer::decode(payload)?;
                let target_type = ServiceType::from_u8(req.target_service_type);
                if let Some(stype) = target_type {
                    if let Some(tx) =
                        self.ctx.registry.find_by_instance(stype, req.target_instance_id)
                    {
                        // 直接将 payload 作为后端帧转发
                        let _ = tx.send(WriterMessage::Send(req.payload, true));
                    } else {
                        warn!(
                            "forward target {:?} instance {} not found",
                            stype, req.target_instance_id
                        );
                    }
                }
            }

            _ => {
                debug!("unhandled internal msg_id={}", msg_id);
            }
        }

        Ok(())
    }
}
