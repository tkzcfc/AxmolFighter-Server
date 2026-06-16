use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Mutex;
use tracing::{debug, info, warn};

use protocol::game::*;
use protocol::message_map::{decode_message, MessageType};

use crate::codec::encode_frame;
use crate::gateway_client::GatewaySender;

/// 消息处理 trait，业务逻辑实现该 trait
#[async_trait]
pub trait MessageHandler: Send + Sync {
    /// 网关连接建立
    fn on_gateway_connected(&self, tx: GatewaySender);

    /// 网关连接断开
    fn on_gateway_disconnected(&self);

    /// 客户端上线
    async fn on_session_online(&self, session_id: u32);

    /// 客户端下线
    async fn on_session_offline(&self, session_id: u32);

    /// 收到客户端业务消息
    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes);
}

/// 默认的游戏消息处理器
pub struct GameHandler {
    /// 网关发送通道
    gateway_tx: Mutex<Option<GatewaySender>>,
}

impl GameHandler {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            gateway_tx: Mutex::new(None),
        })
    }

    /// 向指定客户端发送消息（经网关转发）
    /// serial: <0 请求, >0 回复, =0 推送
    pub fn send_to_client(&self, msg_id: u16, serial: i32, session_id: u32, payload: &[u8]) {
        let tx = self.gateway_tx.lock().unwrap();
        if let Some(sender) = tx.as_ref() {
            let data = encode_frame(msg_id, serial, session_id, payload);
            if sender.send(data).is_err() {
                warn!("failed to send to gateway (channel closed)");
            }
        } else {
            debug!("gateway not connected, dropping msg_id={} to session={}", msg_id, session_id);
        }
    }

    /// 发送 protobuf 消息给客户端（自动编码）
    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        if let Some((msg_id, payload)) = protocol::message_map::encode_message(msg) {
            self.send_to_client(msg_id as u16, serial, session_id, &payload);
        }
    }

    /// 向网关发送原始帧（用于内部协议如 BindBattle）
    pub fn send_raw(&self, data: Bytes) {
        let tx = self.gateway_tx.lock().unwrap();
        if let Some(sender) = tx.as_ref() {
            let _ = sender.send(data);
        }
    }

    // ========== 业务消息处理 ==========

    fn handle_login(&self, serial: i32, session_id: u32, req: LoginReq) {
        info!("login request from session={}: account={}", session_id, req.account);

        // TODO: 实际的账号验证逻辑
        let resp = LoginResp {
            code: 0,
            message: String::new(),
            player_id: session_id as i64, // 临时用 session_id 作为 player_id
            nickname: req.account.clone(),
        };
        self.send_msg(&MessageType::GameLoginResp(resp), -serial, session_id);
    }

    fn handle_register(&self, serial: i32, session_id: u32, req: RegisterReq) {
        info!("register request from session={}: account={}", session_id, req.account);

        // TODO: 实际的注册逻辑
        let resp = RegisterResp {
            code: 0,
            message: String::new(),
            player_id: session_id as i64,
        };
        self.send_msg(&MessageType::GameRegisterResp(resp), -serial, session_id);
    }
}

#[async_trait]
impl MessageHandler for GameHandler {
    fn on_gateway_connected(&self, tx: GatewaySender) {
        let mut guard = self.gateway_tx.lock().unwrap();
        *guard = Some(tx);
    }

    fn on_gateway_disconnected(&self) {
        let mut guard = self.gateway_tx.lock().unwrap();
        *guard = None;
    }

    async fn on_session_online(&self, session_id: u32) {
        debug!("player session {} online", session_id);
    }

    async fn on_session_offline(&self, session_id: u32) {
        debug!("player session {} offline", session_id);
    }

    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes) {
        // 使用 protocol crate 解码消息
        match decode_message(msg_id as u32, &payload) {
            Ok(msg) => match msg {
                MessageType::GameLoginReq(req) => self.handle_login(serial, session_id, req),
                MessageType::GameRegisterReq(req) => self.handle_register(serial, session_id, req),
                other => {
                    debug!("unhandled message type, msg_id={}", msg_id);
                    // 未处理的消息，如果是请求则回复空
                    if serial < 0 {
                        warn!("no handler for msg_id={}, session={}", msg_id, session_id);
                    }
                    drop(other);
                }
            },
            Err(e) => {
                warn!("failed to decode msg_id={} from session={}: {}", msg_id, session_id, e);
            }
        }
    }
}
