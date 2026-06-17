use bytes::BufMut;
use prost::{DecodeError, Message};

#[derive(Clone)]
pub enum MessageType {
    None,
    GameLoginReq(super::game::LoginReq),
    GameLoginResp(super::game::LoginResp),
    GameRegisterReq(super::game::RegisterReq),
    GameRegisterResp(super::game::RegisterResp),
    GamePlayerInfo(super::game::PlayerInfo),
    GameCreateCharacterReq(super::game::CreateCharacterReq),
    GameCreateCharacterResp(super::game::CreateCharacterResp),
    GameFetchCharacterListReq(super::game::FetchCharacterListReq),
    GameFetchCharacterListResp(super::game::FetchCharacterListResp),
    GameSelectCharacterReq(super::game::SelectCharacterReq),
    GameSelectCharacterResp(super::game::SelectCharacterResp),
    GamePlayerState(super::game::PlayerState),
}

impl MessageType {
    pub fn is_none(&self) -> bool {
        matches!(self, MessageType::None)
    }
}

pub fn get_message_id(message: &MessageType) -> Option<u32> {
    match message {
        MessageType::GameLoginReq(_) => Some(1000u32),
        MessageType::GameLoginResp(_) => Some(1001u32),
        MessageType::GameRegisterReq(_) => Some(1002u32),
        MessageType::GameRegisterResp(_) => Some(1003u32),
        MessageType::GamePlayerInfo(_) => Some(1100u32),
        MessageType::GameCreateCharacterReq(_) => Some(1200u32),
        MessageType::GameCreateCharacterResp(_) => Some(1201u32),
        MessageType::GameFetchCharacterListReq(_) => Some(1202u32),
        MessageType::GameFetchCharacterListResp(_) => Some(1203u32),
        MessageType::GameSelectCharacterReq(_) => Some(1204u32),
        MessageType::GameSelectCharacterResp(_) => Some(1205u32),
        MessageType::GamePlayerState(_) => Some(20000u32),
        _ => None,
    }
}

pub fn decode_message(message_id: u32, bytes: &[u8]) -> Result<MessageType, DecodeError> {
    match message_id {
        1000u32 => match super::game::LoginReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameLoginReq(message)),
            Err(err) => Err(err),
        },
        1001u32 => match super::game::LoginResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameLoginResp(message)),
            Err(err) => Err(err),
        },
        1002u32 => match super::game::RegisterReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameRegisterReq(message)),
            Err(err) => Err(err),
        },
        1003u32 => match super::game::RegisterResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameRegisterResp(message)),
            Err(err) => Err(err),
        },
        1100u32 => match super::game::PlayerInfo::decode(bytes) {
            Ok(message) => Ok(MessageType::GamePlayerInfo(message)),
            Err(err) => Err(err),
        },
        1200u32 => match super::game::CreateCharacterReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameCreateCharacterReq(message)),
            Err(err) => Err(err),
        },
        1201u32 => match super::game::CreateCharacterResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameCreateCharacterResp(message)),
            Err(err) => Err(err),
        },
        1202u32 => match super::game::FetchCharacterListReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameFetchCharacterListReq(message)),
            Err(err) => Err(err),
        },
        1203u32 => match super::game::FetchCharacterListResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameFetchCharacterListResp(message)),
            Err(err) => Err(err),
        },
        1204u32 => match super::game::SelectCharacterReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameSelectCharacterReq(message)),
            Err(err) => Err(err),
        },
        1205u32 => match super::game::SelectCharacterResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameSelectCharacterResp(message)),
            Err(err) => Err(err),
        },
        20000u32 => match super::game::PlayerState::decode(bytes) {
            Ok(message) => Ok(MessageType::GamePlayerState(message)),
            Err(err) => Err(err),
        },
        _ => Err(DecodeError::new("unknown message id")),
    }
}

pub fn encode_message(message: &MessageType) -> Option<(u32, Vec<u8>)> {
    match message {
        MessageType::GameLoginReq(msg) => Some((1000u32, msg.encode_to_vec())),
        MessageType::GameLoginResp(msg) => Some((1001u32, msg.encode_to_vec())),
        MessageType::GameRegisterReq(msg) => Some((1002u32, msg.encode_to_vec())),
        MessageType::GameRegisterResp(msg) => Some((1003u32, msg.encode_to_vec())),
        MessageType::GamePlayerInfo(msg) => Some((1100u32, msg.encode_to_vec())),
        MessageType::GameCreateCharacterReq(msg) => Some((1200u32, msg.encode_to_vec())),
        MessageType::GameCreateCharacterResp(msg) => Some((1201u32, msg.encode_to_vec())),
        MessageType::GameFetchCharacterListReq(msg) => Some((1202u32, msg.encode_to_vec())),
        MessageType::GameFetchCharacterListResp(msg) => Some((1203u32, msg.encode_to_vec())),
        MessageType::GameSelectCharacterReq(msg) => Some((1204u32, msg.encode_to_vec())),
        MessageType::GameSelectCharacterResp(msg) => Some((1205u32, msg.encode_to_vec())),
        MessageType::GamePlayerState(msg) => Some((20000u32, msg.encode_to_vec())),
        _ => None,
    }
}

pub fn get_message_size(message: &MessageType) -> usize {
    match message {
        MessageType::GameLoginReq(msg) => msg.encoded_len(),
        MessageType::GameLoginResp(msg) => msg.encoded_len(),
        MessageType::GameRegisterReq(msg) => msg.encoded_len(),
        MessageType::GameRegisterResp(msg) => msg.encoded_len(),
        MessageType::GamePlayerInfo(msg) => msg.encoded_len(),
        MessageType::GameCreateCharacterReq(msg) => msg.encoded_len(),
        MessageType::GameCreateCharacterResp(msg) => msg.encoded_len(),
        MessageType::GameFetchCharacterListReq(msg) => msg.encoded_len(),
        MessageType::GameFetchCharacterListResp(msg) => msg.encoded_len(),
        MessageType::GameSelectCharacterReq(msg) => msg.encoded_len(),
        MessageType::GameSelectCharacterResp(msg) => msg.encoded_len(),
        MessageType::GamePlayerState(msg) => msg.encoded_len(),
        _ => 0,
    }
}

pub fn encode_raw_message(message: &MessageType, buf: &mut impl BufMut) {
    match message {
        MessageType::GameLoginReq(msg) => msg.encode_raw(buf),
        MessageType::GameLoginResp(msg) => msg.encode_raw(buf),
        MessageType::GameRegisterReq(msg) => msg.encode_raw(buf),
        MessageType::GameRegisterResp(msg) => msg.encode_raw(buf),
        MessageType::GamePlayerInfo(msg) => msg.encode_raw(buf),
        MessageType::GameCreateCharacterReq(msg) => msg.encode_raw(buf),
        MessageType::GameCreateCharacterResp(msg) => msg.encode_raw(buf),
        MessageType::GameFetchCharacterListReq(msg) => msg.encode_raw(buf),
        MessageType::GameFetchCharacterListResp(msg) => msg.encode_raw(buf),
        MessageType::GameSelectCharacterReq(msg) => msg.encode_raw(buf),
        MessageType::GameSelectCharacterResp(msg) => msg.encode_raw(buf),
        MessageType::GamePlayerState(msg) => msg.encode_raw(buf),
        _ => {}
    }
}

#[cfg(feature = "serde-serialize")]
pub fn serialize_to_json(message: &MessageType) -> serde_json::Result<String> {
    match message {
        MessageType::GameLoginReq(msg) => serde_json::to_string(&msg),
        MessageType::GameLoginResp(msg) => serde_json::to_string(&msg),
        MessageType::GameRegisterReq(msg) => serde_json::to_string(&msg),
        MessageType::GameRegisterResp(msg) => serde_json::to_string(&msg),
        MessageType::GamePlayerInfo(msg) => serde_json::to_string(&msg),
        MessageType::GameCreateCharacterReq(msg) => serde_json::to_string(&msg),
        MessageType::GameCreateCharacterResp(msg) => serde_json::to_string(&msg),
        MessageType::GameFetchCharacterListReq(msg) => serde_json::to_string(&msg),
        MessageType::GameFetchCharacterListResp(msg) => serde_json::to_string(&msg),
        MessageType::GameSelectCharacterReq(msg) => serde_json::to_string(&msg),
        MessageType::GameSelectCharacterResp(msg) => serde_json::to_string(&msg),
        MessageType::GamePlayerState(msg) => serde_json::to_string(&msg),
        _ => Ok("null".into()),
    }
}
