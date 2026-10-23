use protocol::game::{
    ArmamentSyncReq, ArmamentSyncResp, FashionWearReq, FashionWearResp, PartnerSyncReq,
    PartnerSyncResp, PetSyncReq, PetSyncResp, SkillSlotsSyncReq, SkillSlotsSyncResp,
};
use protocol::types::FashionInfo;
use tracing::warn;

use super::PlayerSessionDelegate;

impl PlayerSessionDelegate {
    fn require_selected_character_id(&self) -> Result<i64, (i32, String)> {
        if self.account_id().is_none() {
            return Err((401, "请先登录".to_string()));
        }
        match self.selected_character() {
            Some(c) => Ok(c.character_id),
            None => Err((2, "请先选择角色".to_string())),
        }
    }

    pub(crate) async fn handle_fashion_wear(&self, req: FashionWearReq) -> FashionWearResp {
        let character_id = match self.require_selected_character_id() {
            Ok(id) => id,
            Err((code, message)) => {
                return FashionWearResp { code, message };
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM character_fashions WHERE character_id = $1")
            .bind(character_id)
            .execute(&self.shared.pool)
            .await
        {
            warn!("fashion wear delete failed: {}", e);
            return FashionWearResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        for f in &req.fashions {
            if f.config_id <= 0 {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO character_fashions (character_id, config_id, position, worn) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(character_id)
            .bind(f.config_id)
            .bind(f.position)
            .bind(f.worn)
            .execute(&self.shared.pool)
            .await
            {
                warn!("fashion wear insert failed: {}", e);
                return FashionWearResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        }

        self.refresh_selected_character_fashions(character_id, &req.fashions)
            .await;
        FashionWearResp {
            code: 0,
            message: String::new(),
        }
    }

    async fn refresh_selected_character_fashions(
        &self,
        character_id: i64,
        fashions: &[FashionInfo],
    ) {
        let mut guard = self.selected_character.lock().unwrap();
        if let Some(c) = guard.as_mut() {
            if c.character_id == character_id {
                c.fashions = fashions.to_vec();
            }
        }
        if let Some(c) = guard.clone() {
            self.shared
                .set_selected_character_for_session(self.session_id, c);
        }
    }

    pub(crate) async fn handle_armament_sync(&self, req: ArmamentSyncReq) -> ArmamentSyncResp {
        let character_id = match self.require_selected_character_id() {
            Ok(id) => id,
            Err((code, message)) => {
                return ArmamentSyncResp { code, message };
            }
        };

        let mut tx = match self.shared.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                warn!("armament sync begin failed: {}", e);
                return ArmamentSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM character_armaments WHERE character_id = $1")
            .bind(character_id)
            .execute(&mut *tx)
            .await
        {
            warn!("armament sync delete failed: {}", e);
            return ArmamentSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        for a in &req.armaments {
            if a.config_id <= 0 {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO character_armaments \
                 (character_id, config_id, level, star, grade, skin_id, skill_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(character_id)
            .bind(a.config_id)
            .bind(a.level.max(1))
            .bind(a.star.max(0))
            .bind(a.grade.max(0))
            .bind(a.skin_id.max(0))
            .bind(a.skill_id.max(0))
            .execute(&mut *tx)
            .await
            {
                warn!("armament sync insert failed: {}", e);
                return ArmamentSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        }

        if let Err(e) = sqlx::query(
            "UPDATE characters SET armament_appearance_id = $2, updated_at = NOW() WHERE id = $1",
        )
        .bind(character_id)
        .bind(req.appearance_id.max(0))
        .execute(&mut *tx)
        .await
        {
            warn!("armament appearance update failed: {}", e);
            return ArmamentSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        if let Err(e) = tx.commit().await {
            warn!("armament sync commit failed: {}", e);
            return ArmamentSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        {
            let mut guard = self.selected_character.lock().unwrap();
            if let Some(c) = guard.as_mut() {
                c.armaments = req.armaments.clone();
                c.armament_appearance_id = req.appearance_id.max(0);
            }
            if let Some(c) = guard.clone() {
                self.shared
                    .set_selected_character_for_session(self.session_id, c);
            }
        }

        ArmamentSyncResp {
            code: 0,
            message: String::new(),
        }
    }

    pub(crate) async fn handle_skill_slots_sync(
        &self,
        req: SkillSlotsSyncReq,
    ) -> SkillSlotsSyncResp {
        let character_id = match self.require_selected_character_id() {
            Ok(id) => id,
            Err((code, message)) => {
                return SkillSlotsSyncResp { code, message };
            }
        };

        let mut tx = match self.shared.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                warn!("skill slots sync begin failed: {}", e);
                return SkillSlotsSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM character_skill_slots WHERE character_id = $1")
            .bind(character_id)
            .execute(&mut *tx)
            .await
        {
            warn!("skill slots delete failed: {}", e);
            return SkillSlotsSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        for s in &req.slots {
            if let Err(e) = sqlx::query(
                "INSERT INTO character_skill_slots (character_id, slot, leaf_id) VALUES ($1, $2, $3)",
            )
            .bind(character_id)
            .bind(s.slot)
            .bind(s.leaf_id.max(0))
            .execute(&mut *tx)
            .await
            {
                warn!("skill slots insert failed: {}", e);
                return SkillSlotsSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        }

        if let Err(e) = tx.commit().await {
            warn!("skill slots commit failed: {}", e);
            return SkillSlotsSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        {
            let mut guard = self.selected_character.lock().unwrap();
            if let Some(c) = guard.as_mut() {
                c.skill_slots = req.slots.clone();
            }
            if let Some(c) = guard.clone() {
                self.shared
                    .set_selected_character_for_session(self.session_id, c);
            }
        }

        SkillSlotsSyncResp {
            code: 0,
            message: String::new(),
        }
    }

    pub(crate) async fn handle_pet_sync(&self, req: PetSyncReq) -> PetSyncResp {
        let character_id = match self.require_selected_character_id() {
            Ok(id) => id,
            Err((code, message)) => {
                return PetSyncResp { code, message };
            }
        };

        let mut tx = match self.shared.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                warn!("pet sync begin failed: {}", e);
                return PetSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM character_pets WHERE character_id = $1")
            .bind(character_id)
            .execute(&mut *tx)
            .await
        {
            warn!("pet sync delete failed: {}", e);
            return PetSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        for p in &req.pets {
            if p.config_id <= 0 {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO character_pets (character_id, config_id, level, star) VALUES ($1, $2, $3, $4)",
            )
            .bind(character_id)
            .bind(p.config_id)
            .bind(p.level.max(1))
            .bind(p.star.max(0))
            .execute(&mut *tx)
            .await
            {
                warn!("pet sync insert failed: {}", e);
                return PetSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        }

        if let Err(e) = sqlx::query(
            "UPDATE characters SET active_pet_id = $2, updated_at = NOW() WHERE id = $1",
        )
        .bind(character_id)
        .bind(req.active_pet_id.max(0))
        .execute(&mut *tx)
        .await
        {
            warn!("active pet update failed: {}", e);
            return PetSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        if let Err(e) = tx.commit().await {
            warn!("pet sync commit failed: {}", e);
            return PetSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        {
            let mut guard = self.selected_character.lock().unwrap();
            if let Some(c) = guard.as_mut() {
                c.pets = req.pets.clone();
                c.active_pet_id = req.active_pet_id.max(0);
            }
            if let Some(c) = guard.clone() {
                self.shared
                    .set_selected_character_for_session(self.session_id, c);
            }
        }

        PetSyncResp {
            code: 0,
            message: String::new(),
        }
    }

    pub(crate) async fn handle_partner_sync(&self, req: PartnerSyncReq) -> PartnerSyncResp {
        let character_id = match self.require_selected_character_id() {
            Ok(id) => id,
            Err((code, message)) => {
                return PartnerSyncResp { code, message };
            }
        };

        let mut tx = match self.shared.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                warn!("partner sync begin failed: {}", e);
                return PartnerSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM character_partners WHERE character_id = $1")
            .bind(character_id)
            .execute(&mut *tx)
            .await
        {
            warn!("partner sync delete failed: {}", e);
            return PartnerSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        for p in &req.partners {
            if p.config_id <= 0 {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO character_partners (character_id, config_id, level, star) VALUES ($1, $2, $3, $4)",
            )
            .bind(character_id)
            .bind(p.config_id)
            .bind(p.level.max(1))
            .bind(p.star.max(0))
            .execute(&mut *tx)
            .await
            {
                warn!("partner sync insert failed: {}", e);
                return PartnerSyncResp {
                    code: -1,
                    message: "服务器内部错误".to_string(),
                };
            }
        }

        if let Err(e) = sqlx::query(
            "UPDATE characters SET active_partner_id = $2, updated_at = NOW() WHERE id = $1",
        )
        .bind(character_id)
        .bind(req.active_partner_id.max(0))
        .execute(&mut *tx)
        .await
        {
            warn!("active partner update failed: {}", e);
            return PartnerSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        if let Err(e) = tx.commit().await {
            warn!("partner sync commit failed: {}", e);
            return PartnerSyncResp {
                code: -1,
                message: "服务器内部错误".to_string(),
            };
        }

        {
            let mut guard = self.selected_character.lock().unwrap();
            if let Some(c) = guard.as_mut() {
                c.partners = req.partners.clone();
                c.active_partner_id = req.active_partner_id.max(0);
            }
            if let Some(c) = guard.clone() {
                self.shared
                    .set_selected_character_for_session(self.session_id, c);
            }
        }

        PartnerSyncResp {
            code: 0,
            message: String::new(),
        }
    }
}
