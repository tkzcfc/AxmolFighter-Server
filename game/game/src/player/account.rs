use protocol::game::*;
use protocol::message_map::MessageType;
use tracing::{info, warn};

use crate::player::PlayerActor;

impl PlayerActor {
    pub(super) async fn handle_login(&mut self, req: LoginReq) -> MessageType {
        info!(
            "login request from session={}: account={}",
            self.session_id, req.account
        );

        let login_result = sqlx::query_as::<_, (i64, String, String)>(
            "SELECT id, password, nickname FROM accounts WHERE account = $1",
        )
        .bind(&req.account)
        .fetch_optional(&self.shared.pool)
        .await;

        let resp = match login_result {
            Ok(Some((player_id, stored_password, nickname))) => {
                if stored_password == req.password {
                    self.account_id = Some(player_id);
                    self.shared.bind_account(self.session_id, player_id);

                    let max_character_count = self.shared.query_max_character_count().await;
                    LoginResp {
                        code: 0,
                        message: String::new(),
                        player_id,
                        nickname: nickname.clone(),
                        server_config: Some(ServerConfig {
                            max_character_count,
                        }),
                        account_info: Some(AccountInfo {
                            player_id,
                            account: req.account,
                            nickname,
                        }),
                    }
                } else {
                    LoginResp {
                        code: 1,
                        message: "密码错误".to_string(),
                        player_id: 0,
                        nickname: String::new(),
                        server_config: None,
                        account_info: None,
                    }
                }
            }
            Ok(None) => LoginResp {
                code: 2,
                message: "账号不存在".to_string(),
                player_id: 0,
                nickname: String::new(),
                server_config: None,
                account_info: None,
            },
            Err(e) => {
                warn!("database error during login: {}", e);
                LoginResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    player_id: 0,
                    nickname: String::new(),
                    server_config: None,
                    account_info: None,
                }
            }
        };

        MessageType::GameLoginResp(resp)
    }

    pub(super) async fn handle_register(&mut self, req: RegisterReq) -> MessageType {
        info!(
            "register request from session={}: account={}",
            self.session_id, req.account
        );

        let resp = match sqlx::query_scalar::<_, i64>(
            "INSERT INTO accounts (account, password, nickname) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&req.account)
        .bind(&req.password)
        .bind(&req.nickname)
        .fetch_one(&self.shared.pool)
        .await
        {
            Ok(player_id) => RegisterResp {
                code: 0,
                message: String::new(),
                player_id,
            },
            Err(e) => {
                if let Some(db_err) = e.as_database_error() {
                    if db_err.is_unique_violation() {
                        RegisterResp {
                            code: 1,
                            message: "账号已存在".to_string(),
                            player_id: 0,
                        }
                    } else {
                        warn!("database error during register: {}", e);
                        RegisterResp {
                            code: -1,
                            message: "服务器内部错误".to_string(),
                            player_id: 0,
                        }
                    }
                } else {
                    warn!("database error during register: {}", e);
                    RegisterResp {
                        code: -1,
                        message: "服务器内部错误".to_string(),
                        player_id: 0,
                    }
                }
            }
        };

        MessageType::GameRegisterResp(resp)
    }
}
