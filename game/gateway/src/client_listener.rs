use std::net::SocketAddr;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use base::net::{session_delegate::SessionDelegate, WriterMessage};

use crate::codec::{encode_backend_frame, try_extract_client_frame, encode_client_frame};
use crate::context::GatewayContext;
use crate::protocol::{self, ServiceType, ServerStatus, SessionOffline, SessionOnline};
use crate::router::RouteTarget;

/// 客户端连接 delegate
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

        // 注册到会话管理器
        self.ctx.sessions.add(session_id, tx.clone());

        debug!("client session {} connected", session_id);

        // 通知所有后端：客户端上线
        let notify = SessionOnline { session_id };
        let data = encode_backend_frame(
            protocol::MSG_SESSION_ONLINE,
            0, // 推送
            session_id,
            &notify.encode(),
        );
        self.ctx.registry.broadcast(data);

        // 向客户端发送当前服务可用状态
        let status = ServerStatus {
            services: vec![
                (ServiceType::Game as u8, self.ctx.registry.has_online(ServiceType::Game)),
                (ServiceType::Battle as u8, self.ctx.registry.has_online(ServiceType::Battle)),
            ],
        };
        let status_frame = encode_client_frame(protocol::MSG_SERVER_STATUS, 0, &status.encode());
        let _ = tx.send(WriterMessage::Send(status_frame, true));

        Ok(())
    }

    async fn on_session_close(&mut self) -> anyhow::Result<()> {
        debug!("client session {} disconnected", self.session_id);

        // 从会话管理器移除
        self.ctx.sessions.remove(self.session_id);

        // 清理路由绑定
        self.ctx.router.cleanup_session(self.session_id);

        // 通知所有后端：客户端下线
        let notify = SessionOffline {
            session_id: self.session_id,
        };
        let data = encode_backend_frame(
            protocol::MSG_SESSION_OFFLINE,
            0, // 推送
            self.session_id,
            &notify.encode(),
        );
        self.ctx.registry.broadcast(data);

        Ok(())
    }

    async fn on_try_extract_frame(
        &mut self,
        buffer: &mut BytesMut,
    ) -> anyhow::Result<Option<Bytes>> {
        // 提取客户端帧，转换为内部格式传递给 on_recv_frame
        match try_extract_client_frame(buffer)? {
            Some(frame) => {
                // 打包 msg_id(2) + serial(4) + payload
                let mut out = BytesMut::with_capacity(6 + frame.payload.len());
                out.extend_from_slice(&frame.msg_id.to_le_bytes());
                out.extend_from_slice(&frame.serial.to_le_bytes());
                out.extend_from_slice(&frame.payload);
                Ok(Some(out.freeze()))
            }
            None => Ok(None),
        }
    }

    async fn on_recv_frame(&mut self, frame: Bytes) -> anyhow::Result<()> {
        if frame.len() < 6 {
            return Ok(());
        }

        let msg_id = u16::from_le_bytes([frame[0], frame[1]]);
        let serial = i32::from_le_bytes([frame[2], frame[3], frame[4], frame[5]]);
        let payload = frame.slice(6..);

        // 路由
        match self.ctx.router.resolve(msg_id, self.session_id) {
            RouteTarget::GameServer => {
                // 转发到游戏服，注入 session_id，透传 serial
                if let Some(tx) = self.ctx.registry.find_by_type(ServiceType::Game) {
                    let data = encode_backend_frame(msg_id, serial, self.session_id, &payload);
                    let _ = tx.send(WriterMessage::Send(data, true));
                } else {
                    warn!("no game server registered, dropping msg_id={}", msg_id);
                }
            }
            RouteTarget::BattleServer(instance_id) => {
                // 转发到绑定的战斗服
                if let Some(tx) =
                    self.ctx.registry.find_by_instance(ServiceType::Battle, instance_id)
                {
                    let data = encode_backend_frame(msg_id, serial, self.session_id, &payload);
                    let _ = tx.send(WriterMessage::Send(data, true));
                } else {
                    warn!(
                        "battle instance {} not found, dropping msg_id={}",
                        instance_id, msg_id
                    );
                }
            }
            RouteTarget::BattleNotBound => {
                warn!(
                    "session {} has no battle binding, dropping msg_id={}",
                    self.session_id, msg_id
                );
            }
            RouteTarget::Internal => {
                // 客户端不应发送内部协议，忽略
                warn!(
                    "client session {} sent internal msg_id={}, ignored",
                    self.session_id, msg_id
                );
            }
            RouteTarget::Unknown => {
                warn!(
                    "unknown route for msg_id={} from session {}",
                    msg_id, self.session_id
                );
            }
        }

        Ok(())
    }
}
