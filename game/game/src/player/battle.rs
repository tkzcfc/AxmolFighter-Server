use std::time::Duration;

use protocol::game::*;
use protocol::gateway::BindServiceReq;
use protocol::message_map::MessageType;
use tracing::warn;

use crate::game_shared::rpc::RpcError;
use crate::player::PlayerActor;

const SERVICE_ID_BATTLE: u32 = 1;
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(10);

impl PlayerActor {
    pub(super) async fn handle_battle_join(&mut self, req: BattleJoinReq) -> BattleJoinResp {
        let Some(account_id) = self.account_id() else {
            return BattleJoinResp {
                code: 401,
                message: "please login first".to_string(),
                battle_id: 0,
                server_frame: 0,
                world_dump: vec![],
            };
        };

        let battle_id = self.shared.next_battle_id();
        let create_req = MessageType::GameBattleCreateReq(BattleCreateReq {
            battle_id,
            map_id: if req.map_id <= 0 { 1 } else { req.map_id },
            players: vec![BattlePlayerSpec {
                session_id: self.session_id,
                player_id: account_id,
            }],
            requester_service_id: 0,
            requester_instance_id: 0,
        });

        let create_resp = match self
            .shared
            .request_server(
                SERVICE_ID_BATTLE,
                -1,
                self.session_id,
                create_req,
                DEFAULT_RPC_TIMEOUT,
            )
            .await
        {
            Ok(MessageType::GameBattleCreateResp(resp)) => resp,
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
            };
        }

        let bind_resp = match self
            .shared
            .request_gateway(
                MessageType::GatewayBindServiceReq(BindServiceReq {
                    session_id: self.session_id,
                    service_id: SERVICE_ID_BATTLE,
                    target_instance_id: create_resp.battle_instance_id as i32,
                }),
                self.session_id,
                DEFAULT_RPC_TIMEOUT,
            )
            .await
        {
            Ok(MessageType::GatewayBindServiceResp(resp)) => resp,
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
            };
        }

        BattleJoinResp {
            code: 0,
            message: String::new(),
            battle_id,
            server_frame: create_resp.server_frame,
            world_dump: create_resp.world_dump,
        }
    }

    fn battle_join_error(code: i32, message: &str) -> BattleJoinResp {
        BattleJoinResp {
            code,
            message: message.to_string(),
            battle_id: 0,
            server_frame: 0,
            world_dump: vec![],
        }
    }
}
