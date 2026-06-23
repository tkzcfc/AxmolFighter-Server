use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use protocol::game::*;
use protocol::gateway::{BindServiceReq, BindServiceResp, ForwardToServerReq};
use protocol::message_map::{MessageType, decode_message};

use crate::gateway_client::GatewaySender;

const CMD_BUSINESS: u8 = 0;
const CMD_GATEWAY_CONTROL: u8 = 2;
const SERVICE_ID_BATTLE: u32 = 1;

#[derive(Clone)]
struct PendingBattleJoin {
    session_id: u32,
    client_serial: i32,
    battle_id: u32,
    map_id: i32,
    player_id: i64,
    battle_instance_id: u32,
    server_frame: u32,
    world_dump: Vec<u8>,
}

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
    async fn on_bind_service_resp(&self, serial: i32, resp: BindServiceResp);

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
    pending_battle_joins: DashMap<i32, PendingBattleJoin>,
    control_serial_seed: AtomicI32,
    battle_id_seed: AtomicU32,
}

impl GameHandler {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            gateway_tx: Mutex::new(None),
            pool,
            session_accounts: DashMap::new(),
            pending_battle_joins: DashMap::new(),
            control_serial_seed: AtomicI32::new(1),
            battle_id_seed: AtomicU32::new(1),
        })
    }

    /// 发送 protobuf 消息给客户端（自动编码）
    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        self.send_frame_msg(CMD_BUSINESS, msg, serial, session_id);
    }

    fn send_control_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        self.send_frame_msg(CMD_GATEWAY_CONTROL, msg, serial, session_id);
    }

    fn send_frame_msg(&self, cmd: u8, msg: &MessageType, serial: i32, session_id: u32) {
        let tx = self.gateway_tx.lock().unwrap();
        if let Some(sender) = tx.as_ref() {
            if let Some((msg_id, payload)) = protocol::message_map::encode_message(msg) {
                let data =
                    crate::codec::encode_frame(cmd, msg_id as u16, serial, session_id, &payload);
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

    fn next_control_request_id(&self) -> i32 {
        self.control_serial_seed.fetch_add(1, Ordering::Relaxed)
    }

    fn next_battle_id(&self) -> u32 {
        self.battle_id_seed.fetch_add(1, Ordering::Relaxed)
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
                    characters.push(Self::db_character_to_proto(
                        id, name, class_id, gender, level, exp, gold,
                    ));
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
                if let Some(db_err) = e.as_database_error()
                    && db_err.is_unique_violation()
                {
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
                character: Some(Self::db_character_to_proto(
                    id, name, class_id, gender, level, exp, gold,
                )),
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
                character: Some(Self::db_character_to_proto(
                    id, name, class_id, gender, level, exp, gold,
                )),
                inventory: Some(InventoryInfo { items, equipments }),
            }),
            -serial,
            session_id,
        );
    }

    async fn handle_battle_join(&self, serial: i32, session_id: u32, req: BattleJoinReq) {
        let Some(account_id) = self.get_account_id(session_id) else {
            self.send_msg(
                &MessageType::GameBattleJoinResp(BattleJoinResp {
                    code: 401,
                    message: "please login first".to_string(),
                    battle_id: 0,
                    server_frame: 0,
                    world_dump: vec![],
                }),
                -serial,
                session_id,
            );
            return;
        };

        let request_id = self.next_control_request_id();
        self.pending_battle_joins.insert(
            request_id,
            PendingBattleJoin {
                session_id,
                client_serial: serial,
                battle_id: self.next_battle_id(),
                map_id: if req.map_id <= 0 { 1 } else { req.map_id },
                player_id: account_id,
                battle_instance_id: 0,
                server_frame: 0,
                world_dump: vec![],
            },
        );

        let create_req = MessageType::GameBattleCreateReq(BattleCreateReq {
            battle_id: self
                .pending_battle_joins
                .get(&request_id)
                .map(|pending| pending.battle_id)
                .unwrap_or(0),
            map_id: self
                .pending_battle_joins
                .get(&request_id)
                .map(|pending| pending.map_id)
                .unwrap_or(1),
            players: vec![BattlePlayerSpec {
                session_id,
                player_id: account_id,
            }],
            requester_service_id: 0,
            requester_instance_id: 0,
        });
        let Some((msg_id, payload)) = protocol::message_map::encode_message(&create_req) else {
            warn!("failed to encode BattleCreateReq");
            return;
        };

        let battle_frame = crate::codec::encode_frame(
            CMD_BUSINESS,
            msg_id as u16,
            -request_id,
            session_id,
            &payload,
        );
        self.send_control_msg(
            &MessageType::GatewayForwardToServerReq(ForwardToServerReq {
                target_service_id: SERVICE_ID_BATTLE,
                target_instance_id: -1,
                payload: battle_frame.to_vec(),
                source_service_id: 0,
                source_instance_id: 0,
            }),
            0,
            session_id,
        );
    }

    async fn handle_battle_create_resp(
        &self,
        serial: i32,
        _session_id: u32,
        resp: BattleCreateResp,
    ) {
        let Some(mut pending) = self.pending_battle_joins.get(&serial).map(|v| v.clone()) else {
            debug!("ignored stale BattleCreateResp serial={}", serial);
            return;
        };

        if resp.code != 0 {
            self.pending_battle_joins.remove(&serial);
            self.send_msg(
                &MessageType::GameBattleJoinResp(BattleJoinResp {
                    code: resp.code,
                    message: resp.message,
                    battle_id: 0,
                    server_frame: 0,
                    world_dump: vec![],
                }),
                -pending.client_serial,
                pending.session_id,
            );
            return;
        }

        pending.battle_instance_id = resp.battle_instance_id;
        pending.server_frame = resp.server_frame;
        pending.world_dump = resp.world_dump;
        self.pending_battle_joins.insert(serial, pending.clone());

        self.send_control_msg(
            &MessageType::GatewayBindServiceReq(BindServiceReq {
                session_id: pending.session_id,
                service_id: SERVICE_ID_BATTLE,
                target_instance_id: pending.battle_instance_id as i32,
            }),
            -serial,
            pending.session_id,
        );
    }

    async fn handle_bind_service_resp(&self, serial: i32, resp: BindServiceResp) {
        let Some((_, pending)) = self.pending_battle_joins.remove(&serial) else {
            debug!("ignored stale BindServiceResp serial={}", serial);
            return;
        };

        if resp.code != 0 {
            self.send_msg(
                &MessageType::GameBattleJoinResp(BattleJoinResp {
                    code: resp.code as i32,
                    message: if resp.message.is_empty() {
                        "battle server unavailable".to_string()
                    } else {
                        resp.message
                    },
                    battle_id: 0,
                    server_frame: 0,
                    world_dump: vec![],
                }),
                -pending.client_serial,
                pending.session_id,
            );
            return;
        }

        self.send_msg(
            &MessageType::GameBattleJoinResp(BattleJoinResp {
                code: 0,
                message: String::new(),
                battle_id: pending.battle_id,
                server_frame: pending.server_frame,
                world_dump: pending.world_dump,
            }),
            -pending.client_serial,
            pending.session_id,
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
        self.pending_battle_joins
            .retain(|_, pending| pending.session_id != session_id);
        debug!("player session {} offline", session_id);
    }

    async fn on_bind_service_resp(&self, serial: i32, resp: BindServiceResp) {
        self.handle_bind_service_resp(serial, resp).await;
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
                MessageType::GameBattleJoinReq(req) => {
                    self.handle_battle_join(serial, session_id, req).await
                }
                MessageType::GameBattleCreateResp(resp) => {
                    self.handle_battle_create_resp(serial, session_id, resp)
                        .await
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
