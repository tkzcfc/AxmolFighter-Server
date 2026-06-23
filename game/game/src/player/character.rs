use protocol::game::*;
use protocol::message_map::MessageType;
use tracing::warn;

use crate::handler::GameShared;
use crate::player::PlayerActor;

impl PlayerActor {
    pub(super) async fn handle_fetch_character_list(
        &mut self,
        _req: FetchCharacterListReq,
    ) -> MessageType {
        let Some(account_id) = self.account_id() else {
            return MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                code: 401,
                message: "请先登录".to_string(),
                characters: vec![],
            });
        };

        let result = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "SELECT id, name, class_id, gender, level, exp, gold FROM characters WHERE player_id = $1 ORDER BY id ASC",
        )
        .bind(account_id)
        .fetch_all(&self.shared.pool)
        .await;

        match result {
            Ok(rows) => {
                let mut characters = Vec::with_capacity(rows.len());
                for (id, name, class_id, gender, level, exp, gold) in rows {
                    characters.push(GameShared::db_character_to_proto(
                        id, name, class_id, gender, level, exp, gold,
                    ));
                }

                MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                    code: 0,
                    message: String::new(),
                    characters,
                })
            }
            Err(e) => {
                warn!("fetch character list failed: {}", e);
                MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    characters: vec![],
                })
            }
        }
    }

    pub(super) async fn handle_create_character(&mut self, req: CreateCharacterReq) -> MessageType {
        let Some(account_id) = self.account_id() else {
            return MessageType::GameCreateCharacterResp(CreateCharacterResp {
                code: 401,
                message: "请先登录".to_string(),
                character: None,
            });
        };

        let max_count = self.shared.query_max_character_count().await as i64;
        let current_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM characters WHERE player_id = $1")
                .bind(account_id)
                .fetch_one(&self.shared.pool)
                .await
                .unwrap_or(0);

        if current_count >= max_count {
            return MessageType::GameCreateCharacterResp(CreateCharacterResp {
                code: 2,
                message: "角色数量已达上限".to_string(),
                character: None,
            });
        }

        let insert_result = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "INSERT INTO characters (player_id, name, class_id, gender) VALUES ($1, $2, $3, $4) RETURNING id, name, class_id, gender, level, exp, gold",
        )
        .bind(account_id)
        .bind(&req.name)
        .bind(req.class_id)
        .bind(req.gender)
        .fetch_one(&self.shared.pool)
        .await;

        let (id, name, class_id, gender, level, exp, gold) = match insert_result {
            Ok(v) => v,
            Err(e) => {
                if let Some(db_err) = e.as_database_error()
                    && db_err.is_unique_violation()
                {
                    return MessageType::GameCreateCharacterResp(CreateCharacterResp {
                        code: 1,
                        message: "角色名已存在".to_string(),
                        character: None,
                    });
                }
                warn!("create character failed: {}", e);
                return MessageType::GameCreateCharacterResp(CreateCharacterResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    character: None,
                });
            }
        };

        for slot in 0..6 {
            let _ = sqlx::query(
                "INSERT INTO equipments (owner_character_id, config_id, enhance_level, refine_level, enchant_props_json, slot, in_bag) VALUES ($1, 0, 0, 0, '[]', $2, false)",
            )
            .bind(id)
            .bind(slot)
            .execute(&self.shared.pool)
            .await;
        }

        MessageType::GameCreateCharacterResp(CreateCharacterResp {
            code: 0,
            message: String::new(),
            character: Some(GameShared::db_character_to_proto(
                id, name, class_id, gender, level, exp, gold,
            )),
        })
    }

    pub(super) async fn handle_select_character(&mut self, req: SelectCharacterReq) -> MessageType {
        let Some(account_id) = self.account_id() else {
            return MessageType::GameSelectCharacterResp(SelectCharacterResp {
                code: 401,
                message: "请先登录".to_string(),
                character: None,
                inventory: None,
            });
        };

        let character_row = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64)>(
            "SELECT id, name, class_id, gender, level, exp, gold FROM characters WHERE id = $1 AND player_id = $2",
        )
        .bind(req.character_id)
        .bind(account_id)
        .fetch_optional(&self.shared.pool)
        .await;

        let Some((id, name, class_id, gender, level, exp, gold)) = (match character_row {
            Ok(v) => v,
            Err(e) => {
                warn!("select character query failed: {}", e);
                return MessageType::GameSelectCharacterResp(SelectCharacterResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    character: None,
                    inventory: None,
                });
            }
        }) else {
            return MessageType::GameSelectCharacterResp(SelectCharacterResp {
                code: 2,
                message: "角色不存在或不属于当前账号".to_string(),
                character: None,
                inventory: None,
            });
        };

        let equip_rows = sqlx::query_as::<_, (i64, i64, i32, i32, i32, bool)>(
            "SELECT id, config_id, enhance_level, refine_level, slot, in_bag FROM equipments WHERE owner_character_id = $1 ORDER BY id ASC",
        )
        .bind(id)
        .fetch_all(&self.shared.pool)
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
        .fetch_all(&self.shared.pool)
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

        MessageType::GameSelectCharacterResp(SelectCharacterResp {
            code: 0,
            message: String::new(),
            character: Some(GameShared::db_character_to_proto(
                id, name, class_id, gender, level, exp, gold,
            )),
            inventory: Some(InventoryInfo { items, equipments }),
        })
    }
}
