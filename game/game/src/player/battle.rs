use protocol::battle_internal::BattleCreateReq;
use protocol::game::*;
use protocol::gateway_internal::BindServiceReq;
use protocol::message_map::MessageType;
use protocol::types::BattlePlayerSpec;
use tracing::warn;

use crate::player::PlayerSessionDelegate;
use backend_framework::rpc::RpcError;
use backend_framework::server_source::ServerSource;
use backend_framework::service_id::SERVICE_ID_BATTLE;

impl PlayerSessionDelegate {
    pub(crate) async fn handle_battle_join(&self, req: BattleJoinReq) -> BattleJoinResp {
        let Some(account_id) = self.account_id() else {
            return BattleJoinResp {
                code: 401,
                message: "please login first".to_string(),
                battle_id: 0,
                server_frame: 0,
                world_dump: vec![],
                actor_entity_id: 0,
                random_seed: 0,
            };
        };

        let Some(character) = self.selected_character() else {
            return BattleJoinResp {
                code: 402,
                message: "please select character first".to_string(),
                battle_id: 0,
                server_frame: 0,
                world_dump: vec![],
                actor_entity_id: 0,
                random_seed: 0,
            };
        };

        let battle_id = self.shared.next_battle_id();
        let random_seed = self.shared.next_random_seed();
        let create_req = MessageType::BattleInternalBattleCreateReq(BattleCreateReq {
            battle_id,
            map_id: if req.map_id <= 0 { 1 } else { req.map_id },
            players: vec![BattlePlayerSpec {
                session_id: self.session_id,
                player_id: account_id,
                class_id: character.class_id,
                name: character.name.clone(),
                character_id: character.character_id,
            }],
            requester_service_id: 0,
            requester_instance_id: 0,
            random_seed,
        });

        let create_resp = match self
            .shared
            .request_server(ServerSource::any_instance(SERVICE_ID_BATTLE), create_req)
            .await
        {
            Ok(MessageType::BattleInternalBattleCreateResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected battle create response type");
                return Self::battle_join_error(-1, "invalid battle create response");
            }
            Err(RpcError::Gateway { code, message }) => {
                warn!(
                    "gateway rejected battle create request: code={} message={}",
                    code, message
                );
                return Self::battle_join_error(
                    code as i32,
                    if message.is_empty() {
                        "battle server unavailable"
                    } else {
                        &message
                    },
                );
            }
            Err(err) => {
                warn!("battle create request failed: {}", err);
                return Self::battle_join_error(-1, "battle server unavailable");
            }
        };

        if create_resp.code != 0 {
            return BattleJoinResp {
                code: create_resp.code,
                message: create_resp.message,
                battle_id: 0,
                server_frame: 0,
                world_dump: vec![],
                actor_entity_id: 0,
                random_seed: 0,
            };
        }

        let bind_resp = match self
            .shared
            .request_gateway(MessageType::GatewayInternalBindServiceReq(BindServiceReq {
                session_id: self.session_id,
                service_id: SERVICE_ID_BATTLE,
                target_instance_id: create_resp.battle_instance_id as i32,
            }))
            .await
        {
            Ok(MessageType::GatewayInternalBindServiceResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected bind service response type");
                return Self::battle_join_error(-1, "invalid bind service response");
            }
            Err(RpcError::Gateway { code, message }) => {
                warn!(
                    "gateway rejected bind service request: code={} message={}",
                    code, message
                );
                return Self::battle_join_error(
                    code as i32,
                    if message.is_empty() {
                        "battle server unavailable"
                    } else {
                        &message
                    },
                );
            }
            Err(err) => {
                warn!("bind battle service failed: {}", err);
                return Self::battle_join_error(-1, "battle server unavailable");
            }
        };

        if bind_resp.code != 0 {
            return BattleJoinResp {
                code: bind_resp.code as i32,
                message: if bind_resp.message.is_empty() {
                    "battle server unavailable".to_string()
                } else {
                    bind_resp.message
                },
                battle_id: 0,
                server_frame: 0,
                world_dump: vec![],
                actor_entity_id: 0,
                random_seed: 0,
            };
        }

        self.shared
            .bind_battle_session(self.session_id, battle_id, create_resp.battle_instance_id);

        if !self.leave_town_scene().await {
            warn!(
                "session {} joined battle {}, but failed to leave town scene cleanly",
                self.session_id, battle_id
            );
        }

        BattleJoinResp {
            code: 0,
            message: String::new(),
            battle_id,
            server_frame: create_resp.server_frame,
            world_dump: create_resp.world_dump,
            actor_entity_id: create_resp.actor_entity_id,
            random_seed,
        }
    }

    fn battle_join_error(code: i32, message: &str) -> BattleJoinResp {
        BattleJoinResp {
            code,
            message: message.to_string(),
            battle_id: 0,
            server_frame: 0,
            world_dump: vec![],
            actor_entity_id: 0,
            random_seed: 0,
        }
    }

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
        let map_id = 1;

        let create_req = MessageType::BattleInternalBattleCreateReq(BattleCreateReq {
            battle_id,
            map_id,
            players: vec![
                BattlePlayerSpec {
                    session_id: pending.challenger_session,
                    player_id: pending.challenger_player_id,
                    class_id: challenger_char.class_id,
                    name: challenger_char.name.clone(),
                    character_id: challenger_char.character_id,
                },
                BattlePlayerSpec {
                    session_id: pending.target_session,
                    player_id: pending.target_player_id,
                    class_id: target_char.class_id,
                    name: target_char.name.clone(),
                    character_id: target_char.character_id,
                },
            ],
            requester_service_id: 0,
            requester_instance_id: 0,
            random_seed,
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
            let bind_resp = self
                .shared
                .request_gateway(MessageType::GatewayInternalBindServiceReq(BindServiceReq {
                    session_id,
                    service_id: SERVICE_ID_BATTLE,
                    target_instance_id: create_resp.battle_instance_id as i32,
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

            self.shared
                .bind_battle_session(session_id, battle_id, create_resp.battle_instance_id);
        }

        // 双方离开城镇
        for session_id in [pending.challenger_session, pending.target_session] {
            if !self.leave_town_for_session(session_id).await {
                warn!("session {} failed to leave town scene for duel", session_id);
            }
        }

        // 按 session 下发 DuelStartPush（各自 actor_entity_id）
        for session_id in [pending.challenger_session, pending.target_session] {
            let actor_entity_id = create_resp
                .players
                .iter()
                .find(|p| p.session_id == session_id)
                .map(|p| p.actor_entity_id)
                .unwrap_or(0);

            let push = MessageType::GameDuelStartPush(DuelStartPush {
                battle_id,
                server_frame: create_resp.server_frame,
                world_dump: create_resp.world_dump.clone(),
                actor_entity_id,
                map_id,
                random_seed,
            });
            self.shared.send_msg(&push, 0, session_id);
        }

        Ok(())
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
