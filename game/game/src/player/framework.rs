use bytes::Bytes;
use protocol::message_map::{MessageType, decode_message};
use tracing::{debug, warn};

use crate::player::PlayerActor;

impl PlayerActor {
    pub(crate) async fn handle_client_frame(&mut self, msg_id: u16, serial: i32, payload: Bytes) {
        match decode_message(msg_id as u32, &payload) {
            Ok(msg) => {
                if serial == 0 {
                    self.handle_push(msg_id, msg).await;
                    return;
                }

                if let Some(resp) = self.handle_client_request(msg_id, msg).await {
                    self.shared.send_msg(&resp, -serial, self.session_id);
                }
            }
            Err(e) => {
                warn!(
                    "failed to decode msg_id={} from session={}: {}",
                    msg_id, self.session_id, e
                );
            }
        }
    }

    async fn handle_client_request(
        &mut self,
        msg_id: u16,
        msg: MessageType,
    ) -> Option<MessageType> {
        match msg {
            MessageType::GameLoginReq(req) => Some(self.handle_login(req).await),
            MessageType::GameRegisterReq(req) => Some(self.handle_register(req).await),
            MessageType::GameFetchCharacterListReq(req) => {
                Some(self.handle_fetch_character_list(req).await)
            }
            MessageType::GameCreateCharacterReq(req) => {
                Some(self.handle_create_character(req).await)
            }
            MessageType::GameSelectCharacterReq(req) => {
                Some(self.handle_select_character(req).await)
            }
            MessageType::GameBattleJoinReq(req) => Some(self.handle_battle_join(req).await),
            other => {
                warn!(
                    "no request handler for msg_id={}, session={}",
                    msg_id, self.session_id
                );
                drop(other);
                None
            }
        }
    }

    async fn handle_push(&mut self, msg_id: u16, msg: MessageType) {
        match msg {
            other => {
                debug!(
                    "unhandled push message type, msg_id={}, session={}",
                    msg_id, self.session_id
                );
                drop(other);
            }
        }
    }
}
