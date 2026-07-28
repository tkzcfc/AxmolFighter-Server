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
            };
        };

        let battle_id = self.shared.next_battle_id();
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
        }
    }
}
