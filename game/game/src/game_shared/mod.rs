use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use protocol::gateway::KickSessionReq;
use protocol::message_map::MessageType;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use backend_framework::delegate::BackendDelegate;
use backend_framework::rpc::RpcError;
use backend_framework::server_source::ServerSource;
use backend_framework::session::BackendSession;
use backend_framework::session_delegate::SessionDelegate;

use crate::player::PlayerSessionDelegate;

// 数据访问和业务辅助函数（next_battle_id、DB 查询、max_character_count 等）
mod data;

// 游戏服的共享全局状态。
pub struct GameShared {
    pub session: Arc<BackendSession>,
    pub pool: PgPool,
    pub account_sessions: Mutex<HashMap<i64, u32>>,
    self_weak: OnceLock<Weak<Self>>,
    battle_id_seed: AtomicU32,
}

impl GameShared {
    pub fn new(pool: PgPool, session: Arc<BackendSession>) -> Arc<Self> {
        Arc::new_cyclic(|weak| {
            let s = Self {
                session,
                pool,
                account_sessions: Mutex::new(HashMap::new()),
                self_weak: OnceLock::new(),
                battle_id_seed: AtomicU32::new(1),
            };
            s.self_weak.set(weak.clone()).ok();
            s
        })
    }

    fn arc_self(&self) -> Arc<Self> {
        self.self_weak.get().unwrap().upgrade().unwrap()
    }

    // ── 网络收发（委托给 BackendSession） ───────────────────

    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        self.session.send_msg(msg, serial, session_id);
    }

    pub async fn request_gateway_timeout(
        &self,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        self.session.request_gateway_timeout(msg, timeout).await
    }

    pub async fn request_server_timeout(
        &self,
        target: ServerSource,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        self.session.request_server_timeout(target, msg, timeout).await
    }

    /// 向网关发 RPC 请求(默认超时 10s)。
    pub async fn request_gateway(
        &self,
        msg: MessageType,
    ) -> Result<MessageType, RpcError> {
        self.session.request_gateway(msg).await
    }

    /// 向其他服务发 RPC 请求(默认超时 10s)。
    pub async fn request_server(
        &self,
        target: ServerSource,
        msg: MessageType,
    ) -> Result<MessageType, RpcError> {
        self.session.request_server(target, msg).await
    }

    // ── 账号管理 ────────────────────────────────────────────

    pub fn bind_account(&self, session_id: u32, account_id: i64) {
        let old_binding = {
            let mut account_sessions = self.account_sessions.lock().unwrap();
            account_sessions.insert(account_id, session_id)
        };
        let kick_target = match old_binding {
            Some(old) if old != session_id => {
                let still_owner = {
                    let account_sessions = self.account_sessions.lock().unwrap();
                    account_sessions.get(&account_id).copied() == Some(session_id)
                };
                if still_owner { Some(old) } else { None }
            }
            _ => None,
        };
        if let Some(old_session_id) = kick_target {
            warn!(
                "account {} login from session {}, kicking old session {}",
                account_id, session_id, old_session_id
            );

            let msg = MessageType::GatewayKickSessionReq(KickSessionReq {
                session_id: old_session_id,
            });
            if let Err(err) = self.session.send_control_msg(&msg, 0) {
                warn!("failed to kick old session {}: {}", old_session_id, err);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// BackendDelegate 实现
// ═══════════════════════════════════════════════════════════════

#[async_trait]
impl BackendDelegate for GameShared {
    fn on_disconnected(&self) {
        self.account_sessions.lock().unwrap().clear();
    }

    fn create_session_delegate(
        &self,
        session_id: u32,
        _session: Arc<BackendSession>,
    ) -> Box<dyn SessionDelegate> {
        Box::new(PlayerSessionDelegate::new(session_id, self.arc_self()))
    }

    async fn on_server_request(
        &self,
        _source: ServerSource,
        msg: MessageType,
    ) -> anyhow::Result<MessageType> {
        warn!("unhandled server request");
        drop(msg);
        Err(anyhow::anyhow!("no handler"))
    }

    async fn on_server_push(&self, _source: ServerSource, msg: MessageType) -> anyhow::Result<()> {
        debug!("unhandled server push");
        drop(msg);
        Ok(())
    }

    async fn on_shutdown(&self) {
        info!("game server global shutdown cleanup");
        self.account_sessions.lock().unwrap().clear();
    }
}
