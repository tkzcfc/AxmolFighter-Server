use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use protocol::game::*;
use protocol::message_map::{MessageType, decode_message};

use crate::gateway_client::GatewaySender;

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
    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes);
}

/// 默认的游戏消息处理器
pub struct GameHandler {
    /// 网关发送通道
    gateway_tx: Mutex<Option<GatewaySender>>,
    /// 数据库连接池
    pool: PgPool,
    /// 会话映射（session_id -> account_id）
    session_accounts: DashMap<u32, i64>,
}

impl GameHandler {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            gateway_tx: Mutex::new(None),
            pool,
            session_accounts: DashMap::new(),
        })
    }

    /// 发送 protobuf 消息给客户端（自动编码）
    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        let tx = self.gateway_tx.lock().unwrap();
        if let Some(sender) = tx.as_ref() {
            if let Some((msg_id, payload)) = protocol::message_map::encode_message(msg) {
                let data =
                    crate::codec::encode_frame(0, msg_id as u16, serial, session_id, &payload);
                if sender.send(data).is_err() {
                    warn!("failed to send to gateway (channel closed)");
                }
            }
        } else {
            debug!(
                "gateway not connected, dropping response to session={}",
                session_id
            );
        }
    }

    fn get_account_id(&self, session_id: u32) -> Option<i64> {
        self.session_accounts.get(&session_id).map(|v| *v)
    }

    async fn query_max_character_count(&self) -> i32 {
        match sqlx::query_scalar::<_, i64>(
            "SELECT value_int FROM server_settings WHERE key = 'max_character_count'",
        )
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(v)) => v as i32,
            _ => 10,
        }
    }

    fn db_character_to_proto(
        &self,
        character_id: i64,
        name: String,
        class_id: i32,
        gender: i32,
        level: i32,
        exp: i64,
        gold: i64,
    ) -> CharacterInfo {
        CharacterInfo {
            character_id,
            name,
            class_id,
            gender,
            level,
            exp,
            gold,
        }
    }

    async fn handle_login(&self, serial: i32, session_id: u32, req: LoginReq) {
        info!(
            "login request from session={}: account={}",
            session_id, req.account
        );

        let login_result = sqlx::query_as::<_, (i64, String, String)>(
            "SELECT id, password, nickname FROM accounts WHERE account = $1",
        )
        .bind(&req.account)
        .fetch_optional(&self.pool)
        .await;

        let resp = match login_result {
            Ok(Some((player_id, stored_password, nickname))) => {
                if stored_password == req.password {
                    self.session_accounts.insert(session_id, player_id);

                    let max_character_count = self.query_max_character_count().await;
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

        self.send_msg(&MessageType::GameLoginResp(resp), -serial, session_id);
    }

    async fn handle_register(&self, serial: i32, session_id: u32, req: RegisterReq) {
        info!(
            "register request from session={}: account={}",
            session_id, req.account
        );

        let resp = match sqlx::query_scalar::<_, i64>(
            "INSERT INTO accounts (account, password, nickname) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&req.account)
        .bind(&req.password)
        .bind(&req.nickname)
        .fetch_one(&self.pool)
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

        self.send_msg(&MessageType::GameRegisterResp(resp), -serial, session_id);
    }

    async fn handle_fetch_character_list(
        &self,
        serial: i32,
        session_id: u32,
        _req: FetchCharacterListReq,
    ) {
        let Some(account_id) = self.get_account_id(session_id) else {
            self.send_msg(
                &MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                    code: 401,
                    message: "请先登录".to_string(),
                    characters: vec![],
                }),
                -serial,
                session_id,
            );
            return;
        };

        let result = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "SELECT id, name, class_id, gender, level, exp, gold FROM characters WHERE player_id = $1 ORDER BY id ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await;

        match result {
            Ok(rows) => {
                let mut characters = Vec::with_capacity(rows.len());
                for (id, name, class_id, gender, level, exp, gold) in rows {
                    characters.push(
                        self.db_character_to_proto(id, name, class_id, gender, level, exp, gold),
                    );
                }

                self.send_msg(
                    &MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                        code: 0,
                        message: String::new(),
                        characters,
                    }),
                    -serial,
                    session_id,
                );
            }
            Err(e) => {
                warn!("fetch character list failed: {}", e);
                self.send_msg(
                    &MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                        code: -1,
                        message: "服务器内部错误".to_string(),
                        characters: vec![],
                    }),
                    -serial,
                    session_id,
                );
            }
        }
    }

    async fn handle_create_character(&self, serial: i32, session_id: u32, req: CreateCharacterReq) {
        let Some(account_id) = self.get_account_id(session_id) else {
            self.send_msg(
                &MessageType::GameCreateCharacterResp(CreateCharacterResp {
                    code: 401,
                    message: "请先登录".to_string(),
                    character: None,
                }),
                -serial,
                session_id,
            );
            return;
        };

        let max_count = self.query_max_character_count().await as i64;
        let current_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM characters WHERE player_id = $1")
                .bind(account_id)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        if current_count >= max_count {
            self.send_msg(
                &MessageType::GameCreateCharacterResp(CreateCharacterResp {
                    code: 2,
                    message: "角色数量已达上限".to_string(),
                    character: None,
                }),
                -serial,
                session_id,
            );
            return;
        }

        let insert_result = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "INSERT INTO characters (player_id, name, class_id, gender) VALUES ($1, $2, $3, $4) RETURNING id, name, class_id, gender, level, exp, gold",
        )
        .bind(account_id)
        .bind(&req.name)
        .bind(req.class_id)
        .bind(req.gender)
        .fetch_one(&self.pool)
        .await;

        let (id, name, class_id, gender, level, exp, gold) = match insert_result {
            Ok(v) => v,
            Err(e) => {
                if let Some(db_err) = e.as_database_error() {
                    if db_err.is_unique_violation() {
                        self.send_msg(
                            &MessageType::GameCreateCharacterResp(CreateCharacterResp {
                                code: 1,
                                message: "角色名已存在".to_string(),
                                character: None,
                            }),
                            -serial,
                            session_id,
                        );
                        return;
                    }
                }
                warn!("create character failed: {}", e);
                self.send_msg(
                    &MessageType::GameCreateCharacterResp(CreateCharacterResp {
                        code: -1,
                        message: "服务器内部错误".to_string(),
                        character: None,
                    }),
                    -serial,
                    session_id,
                );
                return;
            }
        };

        for slot in 0..6 {
            let _ = sqlx::query(
                "INSERT INTO equipments (owner_character_id, config_id, enhance_level, refine_level, enchant_props_json, slot, in_bag) VALUES ($1, 0, 0, 0, '[]', $2, false)",
            )
            .bind(id)
            .bind(slot)
            .execute(&self.pool)
            .await;
        }

        self.send_msg(
            &MessageType::GameCreateCharacterResp(CreateCharacterResp {
                code: 0,
                message: String::new(),
                character: Some(
                    self.db_character_to_proto(id, name, class_id, gender, level, exp, gold),
                ),
            }),
            -serial,
            session_id,
        );
    }

    async fn handle_select_character(&self, serial: i32, session_id: u32, req: SelectCharacterReq) {
        let Some(account_id) = self.get_account_id(session_id) else {
            self.send_msg(
                &MessageType::GameSelectCharacterResp(SelectCharacterResp {
                    code: 401,
                    message: "请先登录".to_string(),
                    character: None,
                    inventory: None,
                }),
                -serial,
                session_id,
            );
            return;
        };

        let character_row = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "SELECT id, name, class_id, gender, level, exp, gold FROM characters WHERE id = $1 AND player_id = $2",
        )
        .bind(req.character_id)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await;

        let Some((id, name, class_id, gender, level, exp, gold)) = (match character_row {
            Ok(v) => v,
            Err(e) => {
                warn!("select character query failed: {}", e);
                self.send_msg(
                    &MessageType::GameSelectCharacterResp(SelectCharacterResp {
                        code: -1,
                        message: "服务器内部错误".to_string(),
                        character: None,
                        inventory: None,
                    }),
                    -serial,
                    session_id,
                );
                return;
            }
        }) else {
            self.send_msg(
                &MessageType::GameSelectCharacterResp(SelectCharacterResp {
                    code: 2,
                    message: "角色不存在或不属于当前账号".to_string(),
                    character: None,
                    inventory: None,
                }),
                -serial,
                session_id,
            );
            return;
        };

        let equip_rows = sqlx::query_as::<_, (i64, i64, i32, i32, i32, bool)>(
            "SELECT id, config_id, enhance_level, refine_level, slot, in_bag FROM equipments WHERE owner_character_id = $1 ORDER BY id ASC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let mut equipments = Vec::new();
        for (eid, config_id, enhance_level, refine_level, slot, in_bag) in equip_rows {
            if in_bag {
                equipments.push(EquipmentInfo {
                    id: eid,
                    config_id,
                    enhance_level,
                    refine_level,
                    enchant_props: vec![],
                    slot,
                });
            }
        }

        let item_rows = sqlx::query_as::<_, (i64, i64, i32)>(
            "SELECT id, config_id, count FROM inventory_items WHERE character_id = $1 ORDER BY id ASC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let mut items = Vec::new();
        for (iid, config_id, count) in item_rows {
            items.push(ItemInfo {
                id: iid,
                config_id,
                count,
            });
        }

        self.send_msg(
            &MessageType::GameSelectCharacterResp(SelectCharacterResp {
                code: 0,
                message: String::new(),
                character: Some(
                    self.db_character_to_proto(id, name, class_id, gender, level, exp, gold),
                ),
                inventory: Some(InventoryInfo { items, equipments }),
            }),
            -serial,
            session_id,
        );
    }
}

#[async_trait]
impl MessageHandler for GameHandler {
    fn on_gateway_connected(&self, tx: GatewaySender) {
        let mut guard = self.gateway_tx.lock().unwrap();
        *guard = Some(tx);
    }

    fn on_gateway_disconnected(&self) {
        let mut guard = self.gateway_tx.lock().unwrap();
        *guard = None;
    }

    async fn on_session_online(&self, session_id: u32) {
        debug!("player session {} online", session_id);
    }

    async fn on_session_offline(&self, session_id: u32) {
        self.session_accounts.remove(&session_id);
        debug!("player session {} offline", session_id);
    }

    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes) {
        match decode_message(msg_id as u32, &payload) {
            Ok(msg) => match msg {
                MessageType::GameLoginReq(req) => self.handle_login(serial, session_id, req).await,
                MessageType::GameRegisterReq(req) => {
                    self.handle_register(serial, session_id, req).await
                }
                MessageType::GameFetchCharacterListReq(req) => {
                    self.handle_fetch_character_list(serial, session_id, req)
                        .await
                }
                MessageType::GameCreateCharacterReq(req) => {
                    self.handle_create_character(serial, session_id, req).await
                }
                MessageType::GameSelectCharacterReq(req) => {
                    self.handle_select_character(serial, session_id, req).await
                }
                other => {
                    debug!("unhandled message type, msg_id={}", msg_id);
                    if serial < 0 {
                        warn!("no handler for msg_id={}, session={}", msg_id, session_id);
                    }
                    drop(other);
                }
            },
            Err(e) => {
                warn!(
                    "failed to decode msg_id={} from session={}: {}",
                    msg_id, session_id, e
                );
            }
        }
    }
}
