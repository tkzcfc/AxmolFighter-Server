use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use protocol::gateway_internal::KickSessionReq;
use protocol::message_map::MessageType;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use backend_framework::delegate::BackendDelegate;
use backend_framework::rpc::RpcError;
use backend_framework::server_source::ServerSource;
use backend_framework::service_id::{SERVICE_ID_BATTLE, SERVICE_ID_TOWN};
use backend_framework::session::BackendSession;
use backend_framework::session_delegate::SessionDelegate;

use crate::player::PlayerSessionDelegate;

// 数据访问和业务辅助函数（next_battle_id、DB 查询、max_character_count 等）
mod data;

#[derive(Clone, Copy)]
pub(crate) struct BattleSessionState {
    pub battle_id: u32,
    pub battle_instance_id: u32,
}

// 游戏服的共享全局状态。
pub struct GameShared {
    pub session: Arc<BackendSession>,
    pub pool: PgPool,
    pub account_sessions: Mutex<HashMap<i64, u32>>,
    battle_sessions: Mutex<HashMap<u32, BattleSessionState>>,
    battle_instance_sessions: Mutex<HashMap<u32, HashSet<u32>>>,
    town_sessions: Mutex<HashMap<u32, u32>>,
    town_instance_sessions: Mutex<HashMap<u32, HashSet<u32>>>,
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
                battle_sessions: Mutex::new(HashMap::new()),
                battle_instance_sessions: Mutex::new(HashMap::new()),
                town_sessions: Mutex::new(HashMap::new()),
                town_instance_sessions: Mutex::new(HashMap::new()),
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
        self.session
            .request_server_timeout(target, msg, timeout)
            .await
    }

    /// 向网关发 RPC 请求(默认超时 10s)。
    pub async fn request_gateway(&self, msg: MessageType) -> Result<MessageType, RpcError> {
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

    pub(crate) fn bind_battle_session(
        &self,
        session_id: u32,
        battle_id: u32,
        battle_instance_id: u32,
    ) {
        self.clear_battle_session(session_id);
        self.battle_sessions.lock().unwrap().insert(
            session_id,
            BattleSessionState {
                battle_id,
                battle_instance_id,
            },
        );
        self.battle_instance_sessions
            .lock()
            .unwrap()
            .entry(battle_instance_id)
            .or_default()
            .insert(session_id);
    }

    pub(crate) fn clear_battle_session(&self, session_id: u32) {
        let old = self.battle_sessions.lock().unwrap().remove(&session_id);
        if let Some(state) = old {
            let mut by_instance = self.battle_instance_sessions.lock().unwrap();
            if let Some(sessions) = by_instance.get_mut(&state.battle_instance_id) {
                sessions.remove(&session_id);
                if sessions.is_empty() {
                    by_instance.remove(&state.battle_instance_id);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn bind_town_session(&self, session_id: u32, town_instance_id: u32) {
        self.clear_town_session(session_id);
        self.town_sessions
            .lock()
            .unwrap()
            .insert(session_id, town_instance_id);
        self.town_instance_sessions
            .lock()
            .unwrap()
            .entry(town_instance_id)
            .or_default()
            .insert(session_id);
    }

    pub(crate) fn clear_town_session(&self, session_id: u32) {
        let old = self.town_sessions.lock().unwrap().remove(&session_id);
        if let Some(town_instance_id) = old {
            let mut by_instance = self.town_instance_sessions.lock().unwrap();
            if let Some(sessions) = by_instance.get_mut(&town_instance_id) {
                sessions.remove(&session_id);
                if sessions.is_empty() {
                    by_instance.remove(&town_instance_id);
                }
            }
        }
    }

    pub(crate) fn town_instance_for_session(&self, session_id: u32) -> Option<u32> {
        self.town_sessions.lock().unwrap().get(&session_id).copied()
    }

    pub(crate) fn clear_session_runtime_state(&self, session_id: u32) {
        self.clear_battle_session(session_id);
        self.clear_town_session(session_id);
    }

    fn clear_all_runtime_state(&self) {
        self.account_sessions.lock().unwrap().clear();
        self.battle_sessions.lock().unwrap().clear();
        self.battle_instance_sessions.lock().unwrap().clear();
        self.town_sessions.lock().unwrap().clear();
        self.town_instance_sessions.lock().unwrap().clear();
    }

    fn is_session_logged_in(&self, session_id: u32) -> bool {
        self.account_sessions
            .lock()
            .unwrap()
            .values()
            .any(|sid| *sid == session_id)
    }

    fn handle_battle_server_offline(&self, battle_instance_id: u32) {
        let session_ids: Vec<u32> = self
            .battle_instance_sessions
            .lock()
            .unwrap()
            .remove(&battle_instance_id)
            .map(|sessions| sessions.into_iter().collect())
            .unwrap_or_default();

        for session_id in session_ids {
            let state = self.battle_sessions.lock().unwrap().remove(&session_id);
            let Some(state) = state else {
                continue;
            };
            if !self.is_session_logged_in(session_id) {
                continue;
            }

            let push =
                MessageType::GameBattleServerOfflinePush(protocol::game::BattleServerOfflinePush {
                    battle_id: state.battle_id,
                    battle_instance_id,
                    reason: 1,
                    message: "battle server offline".to_string(),
                });
            self.send_msg(&push, 0, session_id);
        }
    }

    fn handle_town_server_offline(&self, town_instance_id: u32) {
        let session_ids: Vec<u32> = self
            .town_instance_sessions
            .lock()
            .unwrap()
            .remove(&town_instance_id)
            .map(|sessions| sessions.into_iter().collect())
            .unwrap_or_default();

        for session_id in session_ids {
            let removed = self.town_sessions.lock().unwrap().remove(&session_id);
            if removed != Some(town_instance_id) || !self.is_session_logged_in(session_id) {
                continue;
            }

            let push =
                MessageType::GameTownServerOfflinePush(protocol::game::TownServerOfflinePush {
                    town_instance_id,
                    reason: 1,
                    message: "town server offline".to_string(),
                });
            self.send_msg(&push, 0, session_id);
        }
    }

    // ── 账号管理 ────────────────────────────────────────────

    pub async fn bind_account(&self, session_id: u32, account_id: i64) {
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
            self.clear_session_runtime_state(old_session_id);

            // 先向旧客户端发送被踢下线的 Push 通知（原因：在其他地方登录）
            let push_msg = MessageType::GameAccountKickedPush(protocol::game::AccountKickedPush {
                reason: 1,
                message: "Your account logged in elsewhere.".to_string(),
            });
            self.send_msg(&push_msg, 0, old_session_id);

            // 再通知网关踢掉旧会话
            let msg = MessageType::GatewayInternalKickSessionReq(KickSessionReq {
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
        self.clear_all_runtime_state();
    }

    fn on_server_offline(&self, service_id: u32, instance_id: u32) {
        match service_id {
            SERVICE_ID_BATTLE => self.handle_battle_server_offline(instance_id),
            SERVICE_ID_TOWN => self.handle_town_server_offline(instance_id),
            _ => {}
        }
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
        self.clear_all_runtime_state();
    }
}
