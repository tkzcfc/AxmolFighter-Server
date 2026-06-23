use protocol::game::CharacterInfo;

use crate::handler::GameShared;

impl GameShared {
    pub(crate) async fn query_max_character_count(&self) -> i32 {
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

    pub(crate) fn db_character_to_proto(
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
}
