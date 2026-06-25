mod account;
mod battle;
mod character;

use std::sync::{Arc, Mutex};

use protocol::message_map::MessageType;
use tracing::{debug, warn};

use backend_framework::session_delegate::SessionDelegate;

use crate::game_shared::GameShared;

pub(crate) struct PlayerSessionDelegate {
    pub(crate) session_id: u32,
    pub(crate) account_id: Mutex<Option<i64>>,
    pub(crate) shared: Arc<GameShared>,
}

impl PlayerSessionDelegate {
    pub(crate) fn new(session_id: u32, shared: Arc<GameShared>) -> Self {
        Self {
            session_id,
            account_id: Mutex::new(None),
            shared,
        }
    }

    pub(crate) fn account_id(&self) -> Option<i64> {
        *self.account_id.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl SessionDelegate for PlayerSessionDelegate {
    async fn on_client_request(&self, msg: MessageType) -> anyhow::Result<MessageType> {
        let resp = match msg {
            MessageType::GameLoginReq(req) => self.handle_login(req).await.into(),
            MessageType::GameRegisterReq(req) => self.handle_register(req).await.into(),
            MessageType::GameFetchCharacterListReq(req) => {
                self.handle_fetch_character_list(req).await.into()
            }
            MessageType::GameCreateCharacterReq(req) => {
                self.handle_create_character(req).await.into()
            }
            MessageType::GameSelectCharacterReq(req) => {
                self.handle_select_character(req).await.into()
            }
            MessageType::GameBattleJoinReq(req) => self.handle_battle_join(req).await.into(),
            other => {
                warn!("no request handler for session={}", self.session_id);
                drop(other);
                return Err(anyhow::anyhow!("unhandled message type"));
            }
        };
        Ok(resp)
    }

    async fn on_client_push(&self, msg: MessageType) -> anyhow::Result<()> {
        debug!("unhandled push type session={}", self.session_id);
        drop(msg);
        Ok(())
    }
}
