use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use protocol::battle_internal::BattleResultReq;
use protocol::gateway_internal::{KickSessionReq, UnbindServiceReq};
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
pub(crate) use data::CharacterProgress;

#[derive(Clone, Copy)]
pub(crate) struct BattleSessionState {
    pub battle_id: u32,
    pub battle_instance_id: u32,
}

/// 玩家（账号）当前所在的进行中联网战斗。以 player_id 为键、独立于 session：
/// 客户端断线 / 关闭后 session 被清理，但战斗仍在 battle 服继续，该记录保留，
/// 玩家重新登录选角时据此重连而不是进城镇。战斗结束（BattleResultReq）或 battle 实例下线时清除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActivePlayerBattle {
    pub battle_id: u32,
    pub battle_instance_id: u32,
    pub room_id: i32,
    /// 参战时使用的角色，重连时强制选回该角色。
    pub character_id: i64,
}

/// 待定决斗：挑战者已发出邀请，等待目标回应。
#[derive(Clone, Copy)]
pub(crate) struct PendingDuel {
    pub challenger_session: u32,
    pub challenger_player_id: i64,
    pub target_session: u32,
    pub target_player_id: i64,
    pub created_at: std::time::Instant,
}

// 游戏服的共享全局状态。
pub struct GameShared {
    pub session: Arc<BackendSession>,
    pub pool: PgPool,
    pub account_sessions: Mutex<HashMap<i64, u32>>,
    battle_sessions: Mutex<HashMap<u32, BattleSessionState>>,
    battle_instance_sessions: Mutex<HashMap<u32, HashSet<u32>>>,
    player_battles: Mutex<HashMap<i64, ActivePlayerBattle>>,
    town_sessions: Mutex<HashMap<u32, u32>>,
    town_instance_sessions: Mutex<HashMap<u32, HashSet<u32>>>,
    pending_duels: Mutex<HashMap<u32, PendingDuel>>,
    selected_characters: Mutex<HashMap<u32, protocol::types::CharacterInfo>>,
    self_weak: OnceLock<Weak<Self>>,
    battle_id_seed: AtomicU32,
    random_seed_counter: AtomicU64,
}

impl GameShared {
    pub fn new(pool: PgPool, session: Arc<BackendSession>) -> Arc<Self> {
        let seed_init = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        Arc::new_cyclic(|weak| {
            let s = Self {
                session,
                pool,
                account_sessions: Mutex::new(HashMap::new()),
                battle_sessions: Mutex::new(HashMap::new()),
                battle_instance_sessions: Mutex::new(HashMap::new()),
                player_battles: Mutex::new(HashMap::new()),
                town_sessions: Mutex::new(HashMap::new()),
                town_instance_sessions: Mutex::new(HashMap::new()),
                pending_duels: Mutex::new(HashMap::new()),
                selected_characters: Mutex::new(HashMap::new()),
                self_weak: OnceLock::new(),
                battle_id_seed: AtomicU32::new(1),
                random_seed_counter: AtomicU64::new(seed_init.max(1)),
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

    // ── 玩家进行中的战斗（跨 session，用于断线重连） ─────────

    pub(crate) fn bind_player_battle(&self, player_id: i64, battle: ActivePlayerBattle) {
        self.player_battles.lock().unwrap().insert(player_id, battle);
    }

    pub(crate) fn active_battle_for_player(&self, player_id: i64) -> Option<ActivePlayerBattle> {
        self.player_battles.lock().unwrap().get(&player_id).copied()
    }

    /// 仅当记录仍指向该战斗时清除（避免误删玩家后来开始的新战斗）。
    pub(crate) fn clear_player_battle(&self, player_id: i64, battle_id: u32) {
        let mut battles = self.player_battles.lock().unwrap();
        if battles.get(&player_id).map(|b| b.battle_id) == Some(battle_id) {
            battles.remove(&player_id);
        }
    }

    /// 战斗结束：清除所有指向该战斗的玩家记录，返回受影响的玩家。
    pub(crate) fn clear_player_battles_of(&self, battle_id: u32) -> Vec<i64> {
        let mut battles = self.player_battles.lock().unwrap();
        let players: Vec<i64> = battles
            .iter()
            .filter(|(_, b)| b.battle_id == battle_id)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in &players {
            battles.remove(pid);
        }
        players
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

    pub(crate) fn session_in_battle(&self, session_id: u32) -> bool {
        self.battle_sessions.lock().unwrap().contains_key(&session_id)
    }

    pub(crate) fn session_in_town(&self, session_id: u32) -> bool {
        self.town_sessions.lock().unwrap().contains_key(&session_id)
    }

    // ── 待定决斗 ───────────────────────────────────────────

    pub(crate) fn add_pending_duel(&self, pending: PendingDuel) {
        self.pending_duels
            .lock()
            .unwrap()
            .insert(pending.challenger_session, pending);
    }

    pub(crate) fn find_pending_duel_for_target(&self, target_session: u32) -> Option<PendingDuel> {
        self.pending_duels
            .lock()
            .unwrap()
            .values()
            .find(|p| p.target_session == target_session)
            .copied()
    }

    pub(crate) fn has_pending_duel_from(&self, challenger_session: u32) -> bool {
        self.pending_duels
            .lock()
            .unwrap()
            .contains_key(&challenger_session)
    }

    pub(crate) fn remove_pending_duel(&self, challenger_session: u32) -> Option<PendingDuel> {
        self.pending_duels
            .lock()
            .unwrap()
            .remove(&challenger_session)
    }

    pub(crate) fn remove_pending_duels_for_session(&self, session_id: u32) {
        let mut duels = self.pending_duels.lock().unwrap();
        duels.retain(|_, p| p.challenger_session != session_id && p.target_session != session_id);
    }

    // ── 选中角色缓存（跨 session 可读） ─────────────────────

    pub(crate) fn set_selected_character_for_session(
        &self,
        session_id: u32,
        character: protocol::types::CharacterInfo,
    ) {
        self.selected_characters
            .lock()
            .unwrap()
            .insert(session_id, character);
    }

    pub(crate) fn character_for_session(
        &self,
        session_id: u32,
    ) -> Option<protocol::types::CharacterInfo> {
        self.selected_characters
            .lock()
            .unwrap()
            .get(&session_id)
            .cloned()
    }

    /// 代指定 session 离开城镇场景（用于决斗等需要同时移动双方的流程）。
    pub(crate) async fn leave_town_for_session(&self, session_id: u32) -> bool {
        use protocol::town_internal::TownLeaveSceneReq;

        let Some(town_instance_id) = self.town_instance_for_session(session_id) else {
            return true;
        };
        let player_id = {
            let sessions = self.account_sessions.lock().unwrap();
            sessions
                .iter()
                .find(|(_, sid)| **sid == session_id)
                .map(|(pid, _)| *pid)
        };
        let Some(player_id) = player_id else {
            return false;
        };

        let leave_req = MessageType::TownInternalTownLeaveSceneReq(TownLeaveSceneReq {
            session_id,
            player_id,
        });
        match self
            .request_server(
                ServerSource::new(SERVICE_ID_TOWN, town_instance_id as i32),
                leave_req,
            )
            .await
        {
            Ok(MessageType::TownInternalTownLeaveSceneResp(resp)) => resp.code == 0,
            Ok(_) => {
                warn!("unexpected town leave scene response type");
                false
            }
            Err(err) => {
                warn!("town leave scene request failed: {}", err);
                false
            }
        }
    }

    /// 会话断开 / 被踢时清理。注意不清 `player_battles`：战斗在 battle 服继续，玩家可重连。
    pub(crate) fn clear_session_runtime_state(&self, session_id: u32) {
        self.clear_battle_session(session_id);
        self.clear_town_session(session_id);
        self.remove_pending_duels_for_session(session_id);
        self.selected_characters.lock().unwrap().remove(&session_id);
    }

    fn clear_all_runtime_state(&self) {
        self.account_sessions.lock().unwrap().clear();
        self.battle_sessions.lock().unwrap().clear();
        self.battle_instance_sessions.lock().unwrap().clear();
        self.player_battles.lock().unwrap().clear();
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

    async fn handle_battle_result(&self, req: BattleResultReq) {
        let session_ids: Vec<u32> = {
            let battles = self.battle_sessions.lock().unwrap();
            battles
                .iter()
                .filter(|(_, state)| state.battle_id == req.battle_id)
                .map(|(sid, _)| *sid)
                .collect()
        };

        // 玩家记录与 session 无关：断线中的玩家没有 session，也必须清掉，否则重新登录会误入已结束的战斗。
        let players = self.clear_player_battles_of(req.battle_id);
        if session_ids.is_empty() && players.is_empty() {
            warn!("battle result for unknown battle_id={}", req.battle_id);
            return;
        }

        info!(
            "battle result battle_id={} winner_camp={} reason={} end_frame={}",
            req.battle_id, req.winner_camp, req.reason, req.end_frame
        );

        for session_id in session_ids {
            let unbind = MessageType::GatewayInternalUnbindServiceReq(UnbindServiceReq {
                session_id,
                service_id: SERVICE_ID_BATTLE,
            });
            match self.request_gateway(unbind).await {
                Ok(MessageType::GatewayInternalUnbindServiceResp(resp)) if resp.code == 0 => {}
                Ok(MessageType::GatewayInternalUnbindServiceResp(resp)) => {
                    warn!(
                        "unbind battle service failed session={} code={} message={}",
                        session_id, resp.code, resp.message
                    );
                }
                Ok(_) => {
                    warn!("unexpected unbind battle service response session={}", session_id);
                }
                Err(err) => {
                    warn!("unbind battle service failed session={}: {}", session_id, err);
                }
            }
            self.clear_battle_session(session_id);
        }
    }

    fn handle_battle_server_offline(&self, battle_instance_id: u32) {
        self.player_battles
            .lock()
            .unwrap()
            .retain(|_, b| b.battle_instance_id != battle_instance_id);
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
        match msg {
            MessageType::BattleInternalBattleResultReq(req) => self.handle_battle_result(req).await,
            other => {
                debug!("unhandled server push");
                drop(other);
            }
        }
        Ok(())
    }

    async fn on_shutdown(&self) {
        info!("game server global shutdown cleanup");
        self.clear_all_runtime_state();
    }
}

#[cfg(test)]
mod tests {
    use backend_framework::service_id::SERVICE_ID_GAME;
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    fn test_shared() -> Arc<GameShared> {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1/test")
            .unwrap();
        GameShared::new(pool, BackendSession::new(SERVICE_ID_GAME, 1))
    }

    fn battle(battle_id: u32, instance: u32, character_id: i64) -> ActivePlayerBattle {
        ActivePlayerBattle {
            battle_id,
            battle_instance_id: instance,
            room_id: 30002,
            character_id,
        }
    }

    #[tokio::test]
    async fn player_battle_survives_session_disconnect() {
        let shared = test_shared();
        shared.bind_player_battle(100, battle(5, 1, 11));
        shared.bind_battle_session(7, 5, 1);

        // 客户端关闭：session 运行时状态被清理，但战斗记录必须保留以便重连。
        shared.clear_session_runtime_state(7);

        assert!(!shared.session_in_battle(7));
        assert_eq!(shared.active_battle_for_player(100), Some(battle(5, 1, 11)));
    }

    #[tokio::test]
    async fn battle_result_clears_player_records_even_when_offline() {
        let shared = test_shared();
        shared.bind_player_battle(100, battle(5, 1, 11));
        shared.bind_player_battle(200, battle(5, 1, 22));
        shared.bind_player_battle(300, battle(6, 1, 33));

        let mut cleared = shared.clear_player_battles_of(5);
        cleared.sort();

        assert_eq!(cleared, vec![100, 200]);
        assert_eq!(shared.active_battle_for_player(100), None);
        assert_eq!(shared.active_battle_for_player(200), None);
        assert_eq!(shared.active_battle_for_player(300), Some(battle(6, 1, 33)));
    }

    #[tokio::test]
    async fn stale_clear_does_not_drop_a_newer_battle() {
        let shared = test_shared();
        shared.bind_player_battle(100, battle(8, 1, 11));

        shared.clear_player_battle(100, 5);
        assert_eq!(shared.active_battle_for_player(100), Some(battle(8, 1, 11)));

        shared.clear_player_battle(100, 8);
        assert_eq!(shared.active_battle_for_player(100), None);
    }

    #[tokio::test]
    async fn battle_server_offline_drops_only_its_players() {
        let shared = test_shared();
        shared.bind_player_battle(100, battle(5, 1, 11));
        shared.bind_player_battle(200, battle(6, 2, 22));

        shared.handle_battle_server_offline(1);

        assert_eq!(shared.active_battle_for_player(100), None);
        assert_eq!(shared.active_battle_for_player(200), Some(battle(6, 2, 22)));
    }
}
