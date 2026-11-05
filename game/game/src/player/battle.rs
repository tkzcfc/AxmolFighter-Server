use protocol::battle_internal::{BattleCreateReq, BattleRejoinReq};
use protocol::game::*;
use protocol::gateway_internal::{BindServiceReq, UnbindServiceReq};
use protocol::message_map::MessageType;
use protocol::types::BattlePlayerSpec;
use tracing::warn;

use crate::game_shared::ActivePlayerBattle;
use crate::player::town::town_appearance_from_character;
use crate::player::PlayerSessionDelegate;
use backend_framework::server_source::ServerSource;
use backend_framework::service_id::SERVICE_ID_BATTLE;

/// PVP 决斗房间 id（1v1 竞技场）。
const PVP_DUEL_ROOM_ID: i32 = 30002;

/// 重连尝试的结果。
pub(crate) enum RejoinOutcome {
    /// 已接回战斗，客户端应直接进入战斗界面而不是城镇。
    Rejoined(BattleRejoinInfo),
    /// 战斗记录已过期（已结束 / 不存在），已丢弃，按普通选角进城镇。
    Stale,
    /// 基础设施暂时不可用（gateway / battle 服无响应），记录保留，本次按普通选角进城镇。
    Unavailable,
}

/// 选角时的重连决定：账号有进行中的战斗则必须选回参战角色并重连（忽略请求的角色），
/// 否则按请求选角。返回最终角色 id 与需要重连的战斗。
pub(crate) fn resolve_select_character(
    requested_character_id: i64,
    active: Option<ActivePlayerBattle>,
) -> (i64, Option<ActivePlayerBattle>) {
    match active {
        Some(battle) => (battle.character_id, Some(battle)),
        None => (requested_character_id, None),
    }
}

impl PlayerSessionDelegate {
    // ── 决斗 ───────────────────────────────────────────────

    pub(crate) async fn handle_duel_invite(&self, req: DuelInviteReq) -> DuelInviteResp {
        let Some(account_id) = self.account_id() else {
            return Self::duel_invite_error(401, "please login first");
        };
        let Some(character) = self.selected_character() else {
            return Self::duel_invite_error(402, "please select character first");
        };

        if req.target_player_id == account_id {
            return Self::duel_invite_error(403, "cannot duel yourself");
        }

        let target_session = {
            let sessions = self.shared.account_sessions.lock().unwrap();
            sessions.get(&req.target_player_id).copied()
        };
        let Some(target_session) = target_session else {
            return Self::duel_invite_error(404, "target player not online");
        };

        if self.shared.session_in_battle(self.session_id)
            || self.shared.session_in_battle(target_session)
        {
            return Self::duel_invite_error(405, "player already in battle");
        }
        if !self.shared.session_in_town(self.session_id)
            || !self.shared.session_in_town(target_session)
        {
            return Self::duel_invite_error(406, "both players must be in town");
        }
        if self.shared.has_pending_duel_from(self.session_id) {
            return Self::duel_invite_error(407, "you already have a pending duel invite");
        }

        self.shared.add_pending_duel(crate::game_shared::PendingDuel {
            challenger_session: self.session_id,
            challenger_player_id: account_id,
            target_session,
            target_player_id: req.target_player_id,
            created_at: std::time::Instant::now(),
        });

        let push = MessageType::GameDuelInvitePush(DuelInvitePush {
            from_player_id: account_id,
            from_name: character.name.clone(),
            from_class_id: character.class_id,
        });
        self.shared.send_msg(&push, 0, target_session);

        DuelInviteResp {
            code: 0,
            message: String::new(),
        }
    }

    pub(crate) async fn handle_duel_respond(&self, req: DuelRespondReq) -> DuelRespondResp {
        let Some(pending) = self
            .shared
            .find_pending_duel_for_target(self.session_id)
        else {
            return Self::duel_respond_error(404, "no pending duel invite");
        };

        self.shared.remove_pending_duel(pending.challenger_session);

        if req.accept == 0 {
            let push = MessageType::GameDuelEndPush(DuelEndPush {
                reason: 1,
                message: "duel invite rejected".to_string(),
            });
            self.shared.send_msg(&push, 0, pending.challenger_session);
            return DuelRespondResp {
                code: 0,
                message: String::new(),
            };
        }

        if let Err(message) = self.start_duel_battle(&pending).await {
            warn!(
                "duel battle start failed challenger={} target={}: {}",
                pending.challenger_session, pending.target_session, message
            );
            let push = MessageType::GameDuelEndPush(DuelEndPush {
                reason: 3,
                message: message.clone(),
            });
            self.shared.send_msg(&push, 0, pending.challenger_session);
            self.shared.send_msg(&push, 0, pending.target_session);
            return Self::duel_respond_error(-1, &message);
        }

        DuelRespondResp {
            code: 0,
            message: String::new(),
        }
    }

    async fn start_duel_battle(
        &self,
        pending: &crate::game_shared::PendingDuel,
    ) -> Result<(), String> {
        // 双方当前选中角色
        let challenger_char = self.character_for_session(pending.challenger_session)?;
        let target_char = self.character_for_session(pending.target_session)?;

        let battle_id = self.shared.next_battle_id();
        let random_seed = self.shared.next_random_seed();

        let create_req = MessageType::BattleInternalBattleCreateReq(BattleCreateReq {
            battle_id,
            room_id: PVP_DUEL_ROOM_ID,
            random_seed,
            pvp: true,
            players: build_duel_player_specs(
                pending.challenger_session,
                pending.challenger_player_id,
                &challenger_char,
                pending.target_session,
                pending.target_player_id,
                &target_char,
            ),
        });

        let create_resp = match self
            .shared
            .request_server(ServerSource::any_instance(SERVICE_ID_BATTLE), create_req)
            .await
        {
            Ok(MessageType::BattleInternalBattleCreateResp(resp)) => resp,
            Ok(_) => return Err("invalid battle create response".to_string()),
            Err(err) => return Err(format!("battle server unavailable: {}", err)),
        };

        if create_resp.code != 0 {
            return Err(if create_resp.message.is_empty() {
                "battle create failed".to_string()
            } else {
                create_resp.message
            });
        }

        // 双方绑定 battle 服务
        for session_id in [pending.challenger_session, pending.target_session] {
            self.bind_battle_service(session_id, create_resp.battle_instance_id)
                .await?;
            self.shared
                .bind_battle_session(session_id, battle_id, create_resp.battle_instance_id);
        }

        // 按 player_id 记录进行中的战斗：session 掉线后仍可据此重连。
        for (player_id, character) in [
            (pending.challenger_player_id, &challenger_char),
            (pending.target_player_id, &target_char),
        ] {
            self.shared.bind_player_battle(
                player_id,
                ActivePlayerBattle {
                    battle_id,
                    battle_instance_id: create_resp.battle_instance_id,
                    room_id: PVP_DUEL_ROOM_ID,
                    character_id: character.character_id,
                },
            );
        }

        // 双方离开城镇
        for session_id in [pending.challenger_session, pending.target_session] {
            if !self.leave_town_for_session(session_id).await {
                warn!("session {} failed to leave town scene for duel", session_id);
            }
        }

        // 按 session 下发 DuelStartPush（各自对手信息）
        for (session_id, opponent_player_id, opponent_name) in [
            (
                pending.challenger_session,
                pending.target_player_id,
                target_char.name.clone(),
            ),
            (
                pending.target_session,
                pending.challenger_player_id,
                challenger_char.name.clone(),
            ),
        ] {
            let push = MessageType::GameDuelStartPush(DuelStartPush {
                battle_id,
                room_id: PVP_DUEL_ROOM_ID,
                opponent_player_id,
                opponent_name,
            });
            self.shared.send_msg(&push, 0, session_id);
        }

        Ok(())
    }

    /// 请求 gateway 把会话绑定到指定 battle 实例（后续 20000–29999 的客户端消息据此路由）。
    async fn bind_battle_service(&self, session_id: u32, battle_instance_id: u32) -> Result<(), String> {
        let bind_resp = self
            .shared
            .request_gateway(MessageType::GatewayInternalBindServiceReq(BindServiceReq {
                session_id,
                service_id: SERVICE_ID_BATTLE,
                target_instance_id: battle_instance_id as i32,
            }))
            .await
            .map_err(|e| format!("bind battle service failed: {}", e))?;

        let code = match bind_resp {
            MessageType::GatewayInternalBindServiceResp(resp) => resp.code,
            _ => return Err("invalid bind service response".to_string()),
        };
        if code != 0 {
            return Err(format!("bind battle service rejected: code={}", code));
        }
        Ok(())
    }

    async fn unbind_battle_service(&self, session_id: u32) {
        let unbind = MessageType::GatewayInternalUnbindServiceReq(UnbindServiceReq {
            session_id,
            service_id: SERVICE_ID_BATTLE,
        });
        if let Err(err) = self.shared.request_gateway(unbind).await {
            warn!("unbind battle service failed session={}: {}", session_id, err);
        }
    }

    /// 选角时账号仍在进行中的战斗里：把本 session 重新接回 battle 实例。
    ///
    /// 顺序：先 gateway 绑定（失败无副作用）→ 再通知 battle 改绑槽位（失败则回滚绑定）→ 最后记录 session 绑定。
    /// 开始参数与快照不在这里推：客户端此刻还没进入战斗界面，推送会丢；
    /// 客户端进入战斗界面后发 `BattleReadyReq`，battle 服在 Running 状态下补发 `BattleStartPush` + `BattleSnapshotPush`。
    pub(crate) async fn rejoin_active_battle(
        &self,
        account_id: i64,
        active: ActivePlayerBattle,
    ) -> RejoinOutcome {
        if let Err(err) = self
            .bind_battle_service(self.session_id, active.battle_instance_id)
            .await
        {
            warn!(
                "rejoin battle={} player={}: {}",
                active.battle_id, account_id, err
            );
            return RejoinOutcome::Unavailable;
        }

        let rejoin_req = MessageType::BattleInternalBattleRejoinReq(BattleRejoinReq {
            battle_id: active.battle_id,
            player_id: account_id,
            session_id: self.session_id,
        });
        let resp = match self
            .shared
            .request_server(
                ServerSource::new(SERVICE_ID_BATTLE, active.battle_instance_id as i32),
                rejoin_req,
            )
            .await
        {
            Ok(MessageType::BattleInternalBattleRejoinResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected battle rejoin response type");
                self.unbind_battle_service(self.session_id).await;
                return RejoinOutcome::Unavailable;
            }
            Err(err) => {
                warn!("battle rejoin request failed: {}", err);
                self.unbind_battle_service(self.session_id).await;
                return RejoinOutcome::Unavailable;
            }
        };
        if resp.code != 0 {
            warn!(
                "battle {} rejected rejoin of player {}: code={} message={}",
                active.battle_id, account_id, resp.code, resp.message
            );
            self.unbind_battle_service(self.session_id).await;
            // 战斗已不存在 / 已结束 / 玩家不在其中：记录已过期，丢弃后正常进城镇。
            self.shared.clear_player_battle(account_id, active.battle_id);
            return RejoinOutcome::Stale;
        }

        self.shared.bind_battle_session(
            self.session_id,
            active.battle_id,
            active.battle_instance_id,
        );
        RejoinOutcome::Rejoined(BattleRejoinInfo {
            battle_id: active.battle_id,
            room_id: active.room_id,
        })
    }

    fn character_for_session(&self, session_id: u32) -> Result<protocol::types::CharacterInfo, String> {
        if session_id == self.session_id {
            return self
                .selected_character()
                .ok_or_else(|| "no selected character".to_string());
        }
        self.shared
            .character_for_session(session_id)
            .ok_or_else(|| "opponent has no selected character".to_string())
    }

    async fn leave_town_for_session(&self, session_id: u32) -> bool {
        if session_id == self.session_id {
            return self.leave_town_scene().await;
        }
        self.shared.leave_town_for_session(session_id).await
    }

    fn duel_invite_error(code: i32, message: &str) -> DuelInviteResp {
        DuelInviteResp {
            code,
            message: message.to_string(),
        }
    }

    fn duel_respond_error(code: i32, message: &str) -> DuelRespondResp {
        DuelRespondResp {
            code,
            message: message.to_string(),
        }
    }
}

/// 构造 1v1 决斗的玩家描述：挑战者槽位 0（阵营 2=Our），目标槽位 1（阵营 8=Enemy）。
/// 双方必须分属互相敌对的阵营位（Our / Enemy），中立阵营的特效无法命中，战斗世界引导会拒绝。
fn build_duel_player_specs(
    challenger_session: u32,
    challenger_player_id: i64,
    challenger: &protocol::types::CharacterInfo,
    target_session: u32,
    target_player_id: i64,
    target: &protocol::types::CharacterInfo,
) -> Vec<BattlePlayerSpec> {
    vec![
        BattlePlayerSpec {
            session_id: challenger_session,
            player_id: challenger_player_id,
            character_id: challenger.character_id,
            name: challenger.name.clone(),
            class_id: challenger.class_id,
            slot: 0,
            camp: 2,
            level: challenger.level,
            appearance: Some(town_appearance_from_character(challenger)),
        },
        BattlePlayerSpec {
            session_id: target_session,
            player_id: target_player_id,
            character_id: target.character_id,
            name: target.name.clone(),
            class_id: target.class_id,
            slot: 1,
            camp: 8,
            level: target.level,
            appearance: Some(town_appearance_from_character(target)),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(id: i64, name: &str, class_id: i32, level: i32) -> protocol::types::CharacterInfo {
        protocol::types::CharacterInfo {
            character_id: id,
            name: name.to_string(),
            class_id,
            level,
            ..Default::default()
        }
    }

    fn active(battle_id: u32, character_id: i64) -> ActivePlayerBattle {
        ActivePlayerBattle {
            battle_id,
            battle_instance_id: 1,
            room_id: PVP_DUEL_ROOM_ID,
            character_id,
        }
    }

    #[test]
    fn select_character_without_active_battle_keeps_requested_character() {
        let (character_id, rejoin) = resolve_select_character(7, None);
        assert_eq!(character_id, 7);
        assert!(rejoin.is_none());
    }

    #[test]
    fn select_character_in_active_battle_rejoins_with_battle_character() {
        let battle = active(9, 11);
        let (character_id, rejoin) = resolve_select_character(7, Some(battle));
        assert_eq!(character_id, 11, "must select the character that is in the battle");
        assert_eq!(rejoin, Some(battle));
    }

    #[test]
    fn duel_specs_carry_each_players_appearance() {
        let mut challenger = character(11, "Alice", 3, 42);
        challenger.active_pet_id = 17102;
        challenger.armament_appearance_id = 55;
        challenger.fashions.push(protocol::types::FashionInfo {
            position: 2,
            config_id: 900,
            worn: true,
            ..Default::default()
        });
        challenger.fashions.push(protocol::types::FashionInfo {
            position: 3,
            config_id: 901,
            worn: false,
            ..Default::default()
        });
        let target = character(22, "Bob", 5, 37);

        let specs = build_duel_player_specs(100, 1000, &challenger, 200, 2000, &target);

        let a = specs[0].appearance.as_ref().expect("challenger appearance");
        assert_eq!(a.class_id, 3);
        assert_eq!(a.name, "Alice");
        assert_eq!(a.pet_id, 17102);
        assert_eq!(a.armament_id, 55);
        assert_eq!(a.fashions.len(), 1, "only worn fashions are carried");
        assert_eq!((a.fashions[0].position, a.fashions[0].config_id), (2, 900));

        let b = specs[1].appearance.as_ref().expect("target appearance");
        assert_eq!(b.class_id, 5);
        assert_eq!(b.name, "Bob");
        assert!(b.fashions.is_empty());
    }

    #[test]
    fn duel_player_specs_assign_slots_camps_and_attributes() {
        let challenger = character(11, "Alice", 3, 42);
        let target = character(22, "Bob", 5, 37);

        let specs = build_duel_player_specs(100, 1000, &challenger, 200, 2000, &target);

        assert_eq!(specs.len(), 2);

        assert_eq!(specs[0].session_id, 100);
        assert_eq!(specs[0].player_id, 1000);
        assert_eq!(specs[0].character_id, 11);
        assert_eq!(specs[0].name, "Alice");
        assert_eq!(specs[0].class_id, 3);
        assert_eq!(specs[0].slot, 0);
        assert_eq!(specs[0].camp, 2);
        assert_ne!(specs[0].camp, specs[1].camp);
        assert_eq!(specs[0].level, 42);

        assert_eq!(specs[1].session_id, 200);
        assert_eq!(specs[1].player_id, 2000);
        assert_eq!(specs[1].character_id, 22);
        assert_eq!(specs[1].name, "Bob");
        assert_eq!(specs[1].class_id, 5);
        assert_eq!(specs[1].slot, 1);
        assert_eq!(specs[1].camp, 8);
        assert_eq!(specs[1].level, 37);
    }
}
