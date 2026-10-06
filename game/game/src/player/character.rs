use protocol::game::*;
use protocol::types::*;
use tracing::warn;

use super::fashion_defaults;
use super::PlayerSessionDelegate;
use crate::game_shared::GameShared;

impl PlayerSessionDelegate {
    pub(crate) async fn handle_fetch_character_list(
        &self,
        _req: FetchCharacterListReq,
    ) -> FetchCharacterListResp {
        let Some(account_id) = self.account_id() else {
            return FetchCharacterListResp {
                code: 401,
                message: "请先登录".to_string(),
                characters: vec![],
            };
        };

        match self.shared.load_characters_for_player(account_id).await {
            Ok(characters) => FetchCharacterListResp {
                code: 0,
                message: String::new(),
                characters,
            },
            Err(e) => {
                warn!("fetch character list failed: {}", e);
                FetchCharacterListResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    characters: vec![],
                }
            }
        }
    }

    pub(crate) async fn handle_create_character(
        &self,
        req: CreateCharacterReq,
    ) -> CreateCharacterResp {
        let Some(account_id) = self.account_id() else {
            return CreateCharacterResp {
                code: 401,
                message: "请先登录".to_string(),
                character: None,
            };
        };

        let Some(appearance) =
            fashion_defaults::resolve_create_appearance(req.class_id, req.hair_id, req.clothes_id)
        else {
            return CreateCharacterResp {
                code: 3,
                message: "头饰或服饰无效".to_string(),
                character: None,
            };
        };

        let max_count = self.shared.query_max_character_count().await as i64;
        let current_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM characters WHERE player_id = $1")
                .bind(account_id)
                .fetch_one(&self.shared.pool)
                .await
                .unwrap_or(0);

        if current_count >= max_count {
            return CreateCharacterResp {
                code: 2,
                message: "角色数量已达上限".to_string(),
                character: None,
            };
        }

        let insert_result = sqlx::query_as::<_, (i64, String, i32, i32, i32, i64, i64, i32, i32, i32)>(
            "INSERT INTO characters (player_id, name, class_id, gender, hair_fashion_id, clothes_fashion_id, skin_fashion_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, name, class_id, gender, level, exp, gold, hair_fashion_id, clothes_fashion_id, skin_fashion_id",
        )
        .bind(account_id)
        .bind(&req.name)
        .bind(req.class_id)
        .bind(req.gender)
        .bind(appearance.hair_id)
        .bind(appearance.clothes_id)
        .bind(appearance.skin_id)
        .fetch_one(&self.shared.pool)
        .await;

        let (id, name, class_id, gender, level, exp, gold, hair_id, clothes_id, skin_id) =
            match insert_result {
                Ok(v) => v,
                Err(e) => {
                    if let Some(db_err) = e.as_database_error()
                        && db_err.is_unique_violation()
                    {
                        return CreateCharacterResp {
                            code: 1,
                            message: "角色名已存在".to_string(),
                            character: None,
                        };
                    }
                    warn!("create character failed: {}", e);
                    return CreateCharacterResp {
                        code: -1,
                        message: "服务器内部错误".to_string(),
                        character: None,
                    };
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

        let (fashions, equipments) = self.shared.load_character_lists(id).await;
        CreateCharacterResp {
            code: 0,
            message: String::new(),
            character: Some(GameShared::character_to_proto(
                id, name, class_id, gender, level, exp, gold, hair_id, clothes_id, skin_id,
                fashions, equipments,
            )),
        }
    }

    pub(crate) async fn handle_select_character(
        &self,
        req: SelectCharacterReq,
    ) -> SelectCharacterResp {
        let Some(account_id) = self.account_id() else {
            return SelectCharacterResp {
                code: 401,
                message: "请先登录".to_string(),
                character: None,
                inventory: None,
            };
        };

        let owned = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM characters WHERE id = $1 AND player_id = $2",
        )
        .bind(req.character_id)
        .bind(account_id)
        .fetch_optional(&self.shared.pool)
        .await;

        let character_id = match owned {
            Ok(Some(id)) => id,
            Ok(None) => {
                return SelectCharacterResp {
                    code: 2,
                    message: "角色不存在或不属于当前账号".to_string(),
                    character: None,
                    inventory: None,
                };
            }
            Err(e) => {
                warn!("select character query failed: {}", e);
                return SelectCharacterResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                    character: None,
                    inventory: None,
                };
            }
        };

        let Some(character) = self.shared.load_character_by_id(character_id).await else {
            return SelectCharacterResp {
                code: 2,
                message: "角色不存在或不属于当前账号".to_string(),
                character: None,
                inventory: None,
            };
        };

        let item_rows = sqlx::query_as::<_, (i64, i64, i32)>(
            "SELECT id, config_id, count FROM inventory_items WHERE character_id = $1 ORDER BY id ASC",
        )
        .bind(character_id)
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

        let inventory = InventoryInfo {
            items,
            equipments: character.equipments.clone(),
            fashions: character.fashions.clone(),
        };

        *self.selected_character.lock().unwrap() = Some(character.clone());
        self.shared
            .set_selected_character_for_session(self.session_id, character.clone());

        SelectCharacterResp {
            code: 0,
            message: String::new(),
            character: Some(character),
            inventory: Some(inventory),
        }
    }
}
