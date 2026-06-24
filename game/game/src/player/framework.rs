use bytes::Bytes;
use protocol::message_map::{MessageType, decode_message};
use tracing::{debug, warn};

use crate::error_code::ErrorCode;

use crate::player::PlayerActor;

impl PlayerActor {
    pub(crate) async fn handle_client_frame(&mut self, msg_id: u16, serial: i32, payload: Bytes) {
        match decode_message(msg_id as u32, &payload) {
            Ok(msg) => {
                if serial == 0 {
                    self.handle_push(msg_id, msg).await;
                    return;
                }

                let resp = self.handle_client_request(msg_id, msg).await;
                self.shared.send_msg(&resp, -serial, self.session_id);
            }
            Err(e) => {
                warn!(
                    "failed to decode msg_id={} from session={}: {}",
                    msg_id, self.session_id, e
                );
            }
        }
    }

    async fn handle_client_request(&mut self, msg_id: u16, msg: MessageType) -> MessageType {
        match msg {
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
                warn!(
                    "no request handler for msg_id={}, session={}",
                    msg_id, self.session_id
                );
                drop(other);
                ErrorCode::InternalError.to_common_error_message()
            }
        }
    }

    async fn handle_push(&mut self, msg_id: u16, msg: MessageType) {
        debug!(
            "unhandled push message type, msg_id={}, session={}",
            msg_id, self.session_id
        );
        drop(msg);
    }
}
