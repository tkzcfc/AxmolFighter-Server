use std::collections::HashMap;
use std::sync::atomic::Ordering;

use protocol::types::{CharacterInfo, DefaultSkin, EquipmentInfo, FashionInfo};

use super::GameShared;

const POS_CLOTHES: i32 = 2;
const POS_HAIR: i32 = 4;
const POS_SKIN: i32 = 7;

type CharacterRow = (i64, String, i32, i32, i32, i64, i64, i32, i32, i32);

impl GameShared {
    pub fn next_battle_id(&self) -> u32 {
        self.battle_id_seed.fetch_add(1, Ordering::Relaxed)
    }

    pub fn next_random_seed(&self) -> u64 {
        loop {
            let seed = self.random_seed_counter.fetch_add(1, Ordering::Relaxed);
            if seed != 0 {
                return seed;
            }
        }
    }

    pub async fn query_max_character_count(&self) -> i32 {
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

    pub fn character_to_proto(
        character_id: i64,
        name: String,
        class_id: i32,
        gender: i32,
        level: i32,
        exp: i64,
        gold: i64,
        hair_id: i32,
        clothes_id: i32,
        skin_id: i32,
        fashions: Vec<FashionInfo>,
        equipments: Vec<EquipmentInfo>,
    ) -> CharacterInfo {
        CharacterInfo {
            character_id,
            name,
            class_id,
            gender,
            level,
            exp,
            gold,
            default_skins: default_skins_from_ids(hair_id, clothes_id, skin_id),
            fashions,
            equipments,
        }
    }

    pub async fn load_character_lists(
        &self,
        character_id: i64,
    ) -> (Vec<FashionInfo>, Vec<EquipmentInfo>) {
        let mut map = self.load_lists_for_ids(&[character_id]).await;
        map.remove(&character_id).unwrap_or_default()
    }

    pub async fn load_character_by_id(&self, character_id: i64) -> Option<CharacterInfo> {
        let row = sqlx::query_as::<_, CharacterRow>(
            "SELECT id, name, class_id, gender, level, exp, gold, hair_fashion_id, clothes_fashion_id, skin_fashion_id \
             FROM characters WHERE id = $1",
        )
        .bind(character_id)
        .fetch_optional(&self.pool)
        .await
        .ok()??;

        let (fashions, equipments) = self.load_character_lists(character_id).await;
        Some(row_to_character(row, fashions, equipments))
    }

    pub async fn load_characters_for_player(
        &self,
        player_id: i64,
    ) -> Result<Vec<CharacterInfo>, sqlx::Error> {
        let rows = sqlx::query_as::<_, CharacterRow>(
            "SELECT id, name, class_id, gender, level, exp, gold, hair_fashion_id, clothes_fashion_id, skin_fashion_id \
             FROM characters WHERE player_id = $1 ORDER BY id ASC",
        )
        .bind(player_id)
        .fetch_all(&self.pool)
        .await?;

        let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
        let mut lists = self.load_lists_for_ids(&ids).await;
        Ok(rows
            .into_iter()
            .map(|row| {
                let (fashions, equipments) = lists.remove(&row.0).unwrap_or_default();
                row_to_character(row, fashions, equipments)
            })
            .collect())
    }

    async fn load_lists_for_ids(
        &self,
        ids: &[i64],
    ) -> HashMap<i64, (Vec<FashionInfo>, Vec<EquipmentInfo>)> {
        let mut map: HashMap<i64, (Vec<FashionInfo>, Vec<EquipmentInfo>)> = HashMap::new();
        for id in ids {
            map.insert(*id, (Vec::new(), Vec::new()));
        }
        if ids.is_empty() {
            return map;
        }

        let fashion_rows = sqlx::query_as::<_, (i64, i64, i32, i32, bool)>(
            "SELECT id, character_id, config_id, position, worn FROM character_fashions \
             WHERE character_id = ANY($1) ORDER BY id ASC",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        for (id, character_id, config_id, position, worn) in fashion_rows {
            if let Some((fashions, _)) = map.get_mut(&character_id) {
                fashions.push(FashionInfo {
                    id,
                    config_id,
                    position,
                    worn,
                });
            }
        }

        let equip_rows = sqlx::query_as::<_, (i64, i64, i64, i32, i32, i32)>(
            "SELECT id, owner_character_id, config_id, enhance_level, refine_level, slot \
             FROM equipments WHERE owner_character_id = ANY($1) ORDER BY id ASC",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        for (id, owner_id, config_id, enhance_level, refine_level, slot) in equip_rows {
            if let Some((_, equipments)) = map.get_mut(&owner_id) {
                equipments.push(EquipmentInfo {
                    id,
                    config_id,
                    enhance_level,
                    refine_level,
                    enchant_props: vec![],
                    slot,
                });
            }
        }

        map
    }
}

fn default_skins_from_ids(hair_id: i32, clothes_id: i32, skin_id: i32) -> Vec<DefaultSkin> {
    let mut skins = Vec::new();
    if hair_id > 0 {
        skins.push(DefaultSkin {
            position: POS_HAIR,
            res_fashion_id: hair_id,
        });
    }
    if clothes_id > 0 {
        skins.push(DefaultSkin {
            position: POS_CLOTHES,
            res_fashion_id: clothes_id,
        });
    }
    if skin_id > 0 {
        skins.push(DefaultSkin {
            position: POS_SKIN,
            res_fashion_id: skin_id,
        });
    }
    skins
}

fn row_to_character(
    row: CharacterRow,
    fashions: Vec<FashionInfo>,
    equipments: Vec<EquipmentInfo>,
) -> CharacterInfo {
    GameShared::character_to_proto(
        row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, fashions,
        equipments,
    )
}
