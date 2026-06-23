mod db;
pub(crate) mod framework;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicU32};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use sqlx::PgPool;
use tokio::sync::oneshot;

use protocol::gateway::{BindServiceResp, GatewayErrorResp, UnbindServiceResp};

use crate::gateway_client::GatewaySender;
use crate::handler::framework::PendingResponse;
use crate::player::PlayerRef;

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
    async fn on_bind_service_resp(&self, serial: i32, resp: BindServiceResp);

    async fn on_unbind_service_resp(&self, serial: i32, resp: UnbindServiceResp);

    async fn on_gateway_error_resp(&self, serial: i32, resp: GatewayErrorResp);

    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes);
}

/// 默认的游戏消息处理器
pub struct GameHandler {
    shared: Arc<GameShared>,
}

pub(super) struct GameShared {
    /// 网关发送通道
    gateway_tx: Mutex<Option<GatewaySender>>,
    /// 数据库连接池
    pub(super) pool: PgPool,
    /// 在线玩家 actor（session_id -> PlayerRef）
    players: DashMap<u32, PlayerRef>,
    /// 在线账号映射（account_id -> session_id）
    account_sessions: Mutex<HashMap<i64, u32>>,
    pending_requests: DashMap<i32, oneshot::Sender<PendingResponse>>,
    control_serial_seed: AtomicI32,
    battle_id_seed: AtomicU32,
}

impl GameHandler {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(GameShared {
                gateway_tx: Mutex::new(None),
                pool,
                players: DashMap::new(),
                account_sessions: Mutex::new(HashMap::new()),
                pending_requests: DashMap::new(),
                control_serial_seed: AtomicI32::new(1),
                battle_id_seed: AtomicU32::new(1),
            }),
        })
    }
}
