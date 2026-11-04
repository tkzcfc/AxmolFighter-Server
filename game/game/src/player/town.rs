use std::time::Duration;

use backend_framework::rpc::RpcError;
use backend_framework::server_source::ServerSource;
use backend_framework::service_id::SERVICE_ID_TOWN;
use protocol::game::*;
use protocol::gateway_internal::{BindServiceReq, UnbindServiceReq};
use protocol::message_map::MessageType;
use protocol::town_internal::{TownEnterSceneReq, TownLeaveSceneReq, TownUpdateAppearanceReq};
use protocol::types::{DefaultSkin, TownAppearance, TownFashionItem};
use tracing::warn;

use crate::player::PlayerSessionDelegate;

const TOWN_ROLLBACK_TIMEOUT: Duration = Duration::from_secs(3);

impl PlayerSessionDelegate {
    /// 从当前选中角色构造城镇外观。
    pub(crate) fn build_town_appearance(&self) -> TownAppearance {
        let character = self.selected_character();
        let Some(character) = character else {
            return TownAppearance::default();
        };
        let fashions = character
            .fashions
            .iter()
            .filter(|f| f.worn)
            .map(|f| TownFashionItem {
                position: f.position,
                config_id: f.config_id,
            })
            .collect();
        let base_skins = character
            .default_skins
            .iter()
            .map(|s| DefaultSkin {
                position: s.position,
                res_fashion_id: s.res_fashion_id,
            })
            .collect();
        TownAppearance {
            class_id: character.class_id,
            name: character.name.clone(),
            level: character.level,
            fashions,
            pet_id: character.active_pet_id,
            base_skins,
            armament_id: character.armament_appearance_id,
        }
    }

    /// 通知 town 服：本 session 玩家外观已变化（换装 / 切换宠物 / 切换武装）。
    pub(crate) fn notify_town_appearance(&self) {
        let Some(player_id) = self.account_id() else {
            return;
        };
        let Some(town_instance_id) = self.shared.town_instance_for_session(self.session_id) else {
            return;
        };
        let msg = MessageType::TownInternalTownUpdateAppearanceReq(TownUpdateAppearanceReq {
            session_id: self.session_id,
            player_id,
            appearance: Some(self.build_town_appearance()),
        });
        self.shared
            .session
            .send_server_msg(ServerSource::new(SERVICE_ID_TOWN, town_instance_id as i32), &msg, 0)
            .ok();
    }

    pub(crate) async fn handle_enter_scene(&self, req: EnterSceneReq) -> EnterSceneResp {
        let Some(account_id) = self.account_id() else {
            return Self::enter_scene_error(401, "please login first");
        };

        let movement = req.movement;

        let target = self
            .shared
            .town_instance_for_session(self.session_id)
            .map(|instance_id| ServerSource::new(SERVICE_ID_TOWN, instance_id as i32))
            .unwrap_or_else(|| ServerSource::any_instance(SERVICE_ID_TOWN));

        let town_req = MessageType::TownInternalTownEnterSceneReq(TownEnterSceneReq {
            session_id: self.session_id,
            player_id: account_id,
            r#type: req.r#type,
            map_id: req.map_id,
            owner_uid: req.owner_uid,
            movement,
            appearance: Some(self.build_town_appearance()),
        });

        let town_resp = match self.shared.request_server(target, town_req).await {
            Ok(MessageType::TownInternalTownEnterSceneResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected town enter scene response type");
                return Self::enter_scene_error(-1, "invalid town response");
            }
            Err(RpcError::Gateway { code, message }) => {
                warn!(
                    "gateway rejected town enter scene request: code={} message={}",
                    code, message
                );
                return Self::enter_scene_error(
                    code as i32,
                    if message.is_empty() {
                        "town server unavailable"
                    } else {
                        &message
                    },
                );
            }
            Err(err) => {
                warn!("town enter scene request failed: {}", err);
                return Self::enter_scene_error(-1, "town server unavailable");
            }
        };

        if town_resp.code != 0 {
            return EnterSceneResp {
                code: town_resp.code,
                message: town_resp.message,
                scene: None,
                players: vec![],
            };
        }

        let bind_resp = match self
            .shared
            .request_gateway(MessageType::GatewayInternalBindServiceReq(BindServiceReq {
                session_id: self.session_id,
                service_id: SERVICE_ID_TOWN,
                target_instance_id: town_resp.town_instance_id as i32,
            }))
            .await
        {
            Ok(MessageType::GatewayInternalBindServiceResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected town bind response type");
                self.rollback_town_enter(town_resp.town_instance_id, account_id)
                    .await;
                return Self::enter_scene_error(-1, "invalid bind service response");
            }
            Err(RpcError::Gateway { code, message }) => {
                warn!(
                    "gateway rejected town bind request: code={} message={}",
                    code, message
                );
                self.rollback_town_enter(town_resp.town_instance_id, account_id)
                    .await;
                return Self::enter_scene_error(
                    code as i32,
                    if message.is_empty() {
                        "town server unavailable"
                    } else {
                        &message
                    },
                );
            }
            Err(err) => {
                warn!("bind town service failed: {}", err);
                self.rollback_town_enter(town_resp.town_instance_id, account_id)
                    .await;
                return Self::enter_scene_error(-1, "town server unavailable");
            }
        };

        if bind_resp.code != 0 {
            self.rollback_town_enter(town_resp.town_instance_id, account_id)
                .await;
            return Self::enter_scene_error(
                bind_resp.code as i32,
                if bind_resp.message.is_empty() {
                    "town server unavailable"
                } else {
                    &bind_resp.message
                },
            );
        }

        self.shared
            .bind_town_session(self.session_id, town_resp.town_instance_id);

        EnterSceneResp {
            code: 0,
            message: String::new(),
            scene: town_resp.scene,
            players: town_resp.players,
        }
    }

    /// 处理客户端离开城镇场景请求（进入本地副本等离线战斗时调用）。
    pub(crate) async fn handle_leave_scene(&self, _req: LeaveSceneReq) -> LeaveSceneResp {
        if self.leave_town_scene().await {
            LeaveSceneResp {
                code: 0,
                message: String::new(),
            }
        } else {
            LeaveSceneResp {
                code: -1,
                message: "failed to leave town scene".to_string(),
            }
        }
    }

    pub(crate) async fn leave_town_scene(&self) -> bool {
        let Some(account_id) = self.account_id() else {
            return false;
        };

        let Some(town_instance_id) = self.shared.town_instance_for_session(self.session_id) else {
            return true;
        };

        let leave_req = MessageType::TownInternalTownLeaveSceneReq(TownLeaveSceneReq {
            session_id: self.session_id,
            player_id: account_id,
        });

        let town_resp = match self
            .shared
            .request_server(
                ServerSource::new(SERVICE_ID_TOWN, town_instance_id as i32),
                leave_req,
            )
            .await
        {
            Ok(MessageType::TownInternalTownLeaveSceneResp(resp)) => resp,
            Ok(_) => {
                warn!("unexpected town leave scene response type");
                return false;
            }
            Err(err) => {
                warn!("town leave scene request failed: {}", err);
                return false;
            }
        };

        if town_resp.code != 0 {
            warn!(
                "town leave scene failed: code={} message={}",
                town_resp.code, town_resp.message
            );
            return false;
        }

        let unbind_result = self
            .shared
            .request_gateway(MessageType::GatewayInternalUnbindServiceReq(
                UnbindServiceReq {
                    session_id: self.session_id,
                    service_id: SERVICE_ID_TOWN,
                },
            ))
            .await;

        self.shared.clear_town_session(self.session_id);

        match unbind_result {
            Ok(MessageType::GatewayInternalUnbindServiceResp(resp)) if resp.code == 0 => true,
            Ok(MessageType::GatewayInternalUnbindServiceResp(resp)) => {
                warn!(
                    "unbind town service failed: code={} message={}",
                    resp.code, resp.message
                );
                false
            }
            Ok(_) => {
                warn!("unexpected unbind town service response type");
                false
            }
            Err(err) => {
                warn!("unbind town service failed: {}", err);
                false
            }
        }
    }

    async fn rollback_town_enter(&self, town_instance_id: u32, player_id: i64) {
        let msg = MessageType::TownInternalTownLeaveSceneReq(TownLeaveSceneReq {
            session_id: self.session_id,
            player_id,
        });
        let result = self
            .shared
            .request_server_timeout(
                ServerSource::new(SERVICE_ID_TOWN, town_instance_id as i32),
                msg,
                TOWN_ROLLBACK_TIMEOUT,
            )
            .await;
        if let Err(err) = result {
            warn!("rollback town enter failed: {}", err);
        }
    }

    fn enter_scene_error(code: i32, message: &str) -> EnterSceneResp {
        EnterSceneResp {
            code,
            message: message.to_string(),
            scene: None,
            players: vec![],
        }
    }

}

#[cfg(test)]
mod tests {
    use backend_framework::service_id::SERVICE_ID_GAME;
    use backend_framework::session::BackendSession;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::game_shared::GameShared;

    fn test_delegate() -> PlayerSessionDelegate {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1/test")
            .unwrap();
        let session = BackendSession::new(SERVICE_ID_GAME, 1);
        let shared = GameShared::new(pool, session);
        PlayerSessionDelegate::new(42, shared)
    }

    #[tokio::test]
    async fn enter_scene_requires_login() {
        let delegate = test_delegate();
        let resp = delegate.handle_enter_scene(EnterSceneReq::default()).await;

        assert_eq!(resp.code, 401);
        assert!(resp.players.is_empty());
    }

    #[tokio::test]
    async fn leave_town_scene_without_town_binding_is_success() {
        let delegate = test_delegate();
        *delegate.account_id.lock().unwrap() = Some(100);

        let left = delegate.leave_town_scene().await;

        assert!(left);
    }
}
