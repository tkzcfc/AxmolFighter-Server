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
        MessageType::GamePlayerState(msg) => serde_json::to_string(&msg),
        _ => Ok("null".into()),
    }
}
