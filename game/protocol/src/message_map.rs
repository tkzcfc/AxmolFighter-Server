use bytes::BufMut;
use prost::{DecodeError, Message};

#[derive(Clone)]
pub enum MessageType {
    None,
    GameCommonSuccessResp(super::game::CommonSuccessResp),
    GameCommonErrorResp(super::game::CommonErrorResp),
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
    GameBattleJoinReq(super::game::BattleJoinReq),
    GameBattleJoinResp(super::game::BattleJoinResp),
    GameDuelInviteReq(super::game::DuelInviteReq),
    GameDuelInviteResp(super::game::DuelInviteResp),
    GameDuelRespondReq(super::game::DuelRespondReq),
    GameDuelRespondResp(super::game::DuelRespondResp),
    GameEnterSceneReq(super::game::EnterSceneReq),
    GameEnterSceneResp(super::game::EnterSceneResp),
    GameAccountKickedPush(super::game::AccountKickedPush),
    GameBattleServerOfflinePush(super::game::BattleServerOfflinePush),
    GameTownServerOfflinePush(super::game::TownServerOfflinePush),
    GameDuelInvitePush(super::game::DuelInvitePush),
    GameDuelStartPush(super::game::DuelStartPush),
    GameDuelEndPush(super::game::DuelEndPush),
    BattleBattleInputPush(super::battle::BattleInputPush),
    BattleBattleSnapshotPush(super::battle::BattleSnapshotPush),
    TownTownPlayerStatePush(super::town::TownPlayerStatePush),
    TownScenePlayerEnterPush(super::town::ScenePlayerEnterPush),
    TownScenePlayerLeavePush(super::town::ScenePlayerLeavePush),
    TownScenePlayerStatePush(super::town::ScenePlayerStatePush),
    BattleInternalBattleCreateReq(super::battle_internal::BattleCreateReq),
    BattleInternalBattleCreateResp(super::battle_internal::BattleCreateResp),
    TownInternalTownEnterSceneReq(super::town_internal::TownEnterSceneReq),
    TownInternalTownEnterSceneResp(super::town_internal::TownEnterSceneResp),
    TownInternalTownLeaveSceneReq(super::town_internal::TownLeaveSceneReq),
    TownInternalTownLeaveSceneResp(super::town_internal::TownLeaveSceneResp),
    GatewayClientServerStatusReq(super::gateway_client::ServerStatusReq),
    GatewayClientServerStatusPush(super::gateway_client::ServerStatusPush),
    GatewayClientGatewayErrorResp(super::gateway_client::GatewayErrorResp),
    GatewayInternalServerRegReq(super::gateway_internal::ServerRegReq),
    GatewayInternalServerRegResp(super::gateway_internal::ServerRegResp),
    GatewayInternalBindServiceReq(super::gateway_internal::BindServiceReq),
    GatewayInternalBindServiceResp(super::gateway_internal::BindServiceResp),
    GatewayInternalUnbindServiceReq(super::gateway_internal::UnbindServiceReq),
    GatewayInternalUnbindServiceResp(super::gateway_internal::UnbindServiceResp),
    GatewayInternalKickSessionReq(super::gateway_internal::KickSessionReq),
    GatewayInternalKickSessionRsp(super::gateway_internal::KickSessionRsp),
    GatewayInternalSessionOnlinePush(super::gateway_internal::SessionOnlinePush),
    GatewayInternalSessionOfflinePush(super::gateway_internal::SessionOfflinePush),
    GatewayInternalServerOnlinePush(super::gateway_internal::ServerOnlinePush),
    GatewayInternalServerOfflinePush(super::gateway_internal::ServerOfflinePush),
    GatewayInternalForwardToServerReq(super::gateway_internal::ForwardToServerReq),
    GatewayInternalServiceLoadReportPush(super::gateway_internal::ServiceLoadReportPush),
    GatewayInternalServerPingReq(super::gateway_internal::ServerPingReq),
    GatewayInternalServerPongResp(super::gateway_internal::ServerPongResp),
}

impl MessageType {
    pub fn is_none(&self) -> bool {
        matches!(self, MessageType::None)
    }
}

impl From<super::game::CommonSuccessResp> for MessageType {
    fn from(v: super::game::CommonSuccessResp) -> Self {
        MessageType::GameCommonSuccessResp(v)
    }
}
impl From<super::game::CommonErrorResp> for MessageType {
    fn from(v: super::game::CommonErrorResp) -> Self {
        MessageType::GameCommonErrorResp(v)
    }
}
impl From<super::game::LoginReq> for MessageType {
    fn from(v: super::game::LoginReq) -> Self {
        MessageType::GameLoginReq(v)
    }
}
impl From<super::game::LoginResp> for MessageType {
    fn from(v: super::game::LoginResp) -> Self {
        MessageType::GameLoginResp(v)
    }
}
impl From<super::game::RegisterReq> for MessageType {
    fn from(v: super::game::RegisterReq) -> Self {
        MessageType::GameRegisterReq(v)
    }
}
impl From<super::game::RegisterResp> for MessageType {
    fn from(v: super::game::RegisterResp) -> Self {
        MessageType::GameRegisterResp(v)
    }
}
impl From<super::game::PlayerInfo> for MessageType {
    fn from(v: super::game::PlayerInfo) -> Self {
        MessageType::GamePlayerInfo(v)
    }
}
impl From<super::game::CreateCharacterReq> for MessageType {
    fn from(v: super::game::CreateCharacterReq) -> Self {
        MessageType::GameCreateCharacterReq(v)
    }
}
impl From<super::game::CreateCharacterResp> for MessageType {
    fn from(v: super::game::CreateCharacterResp) -> Self {
        MessageType::GameCreateCharacterResp(v)
    }
}
impl From<super::game::FetchCharacterListReq> for MessageType {
    fn from(v: super::game::FetchCharacterListReq) -> Self {
        MessageType::GameFetchCharacterListReq(v)
    }
}
impl From<super::game::FetchCharacterListResp> for MessageType {
    fn from(v: super::game::FetchCharacterListResp) -> Self {
        MessageType::GameFetchCharacterListResp(v)
    }
}
impl From<super::game::SelectCharacterReq> for MessageType {
    fn from(v: super::game::SelectCharacterReq) -> Self {
        MessageType::GameSelectCharacterReq(v)
    }
}
impl From<super::game::SelectCharacterResp> for MessageType {
    fn from(v: super::game::SelectCharacterResp) -> Self {
        MessageType::GameSelectCharacterResp(v)
    }
}
impl From<super::game::BattleJoinReq> for MessageType {
    fn from(v: super::game::BattleJoinReq) -> Self {
        MessageType::GameBattleJoinReq(v)
    }
}
impl From<super::game::BattleJoinResp> for MessageType {
    fn from(v: super::game::BattleJoinResp) -> Self {
        MessageType::GameBattleJoinResp(v)
    }
}
impl From<super::game::DuelInviteReq> for MessageType {
    fn from(v: super::game::DuelInviteReq) -> Self {
        MessageType::GameDuelInviteReq(v)
    }
}
impl From<super::game::DuelInviteResp> for MessageType {
    fn from(v: super::game::DuelInviteResp) -> Self {
        MessageType::GameDuelInviteResp(v)
    }
}
impl From<super::game::DuelRespondReq> for MessageType {
    fn from(v: super::game::DuelRespondReq) -> Self {
        MessageType::GameDuelRespondReq(v)
    }
}
impl From<super::game::DuelRespondResp> for MessageType {
    fn from(v: super::game::DuelRespondResp) -> Self {
        MessageType::GameDuelRespondResp(v)
    }
}
impl From<super::game::EnterSceneReq> for MessageType {
    fn from(v: super::game::EnterSceneReq) -> Self {
        MessageType::GameEnterSceneReq(v)
    }
}
impl From<super::game::EnterSceneResp> for MessageType {
    fn from(v: super::game::EnterSceneResp) -> Self {
        MessageType::GameEnterSceneResp(v)
    }
}
impl From<super::game::AccountKickedPush> for MessageType {
    fn from(v: super::game::AccountKickedPush) -> Self {
        MessageType::GameAccountKickedPush(v)
    }
}
impl From<super::game::BattleServerOfflinePush> for MessageType {
    fn from(v: super::game::BattleServerOfflinePush) -> Self {
        MessageType::GameBattleServerOfflinePush(v)
    }
}
impl From<super::game::TownServerOfflinePush> for MessageType {
    fn from(v: super::game::TownServerOfflinePush) -> Self {
        MessageType::GameTownServerOfflinePush(v)
    }
}
impl From<super::game::DuelInvitePush> for MessageType {
    fn from(v: super::game::DuelInvitePush) -> Self {
        MessageType::GameDuelInvitePush(v)
    }
}
impl From<super::game::DuelStartPush> for MessageType {
    fn from(v: super::game::DuelStartPush) -> Self {
        MessageType::GameDuelStartPush(v)
    }
}
impl From<super::game::DuelEndPush> for MessageType {
    fn from(v: super::game::DuelEndPush) -> Self {
        MessageType::GameDuelEndPush(v)
    }
}
impl From<super::battle::BattleInputPush> for MessageType {
    fn from(v: super::battle::BattleInputPush) -> Self {
        MessageType::BattleBattleInputPush(v)
    }
}
impl From<super::battle::BattleSnapshotPush> for MessageType {
    fn from(v: super::battle::BattleSnapshotPush) -> Self {
        MessageType::BattleBattleSnapshotPush(v)
    }
}
impl From<super::town::TownPlayerStatePush> for MessageType {
    fn from(v: super::town::TownPlayerStatePush) -> Self {
        MessageType::TownTownPlayerStatePush(v)
    }
}
impl From<super::town::ScenePlayerEnterPush> for MessageType {
    fn from(v: super::town::ScenePlayerEnterPush) -> Self {
        MessageType::TownScenePlayerEnterPush(v)
    }
}
impl From<super::town::ScenePlayerLeavePush> for MessageType {
    fn from(v: super::town::ScenePlayerLeavePush) -> Self {
        MessageType::TownScenePlayerLeavePush(v)
    }
}
impl From<super::town::ScenePlayerStatePush> for MessageType {
    fn from(v: super::town::ScenePlayerStatePush) -> Self {
        MessageType::TownScenePlayerStatePush(v)
    }
}
impl From<super::battle_internal::BattleCreateReq> for MessageType {
    fn from(v: super::battle_internal::BattleCreateReq) -> Self {
        MessageType::BattleInternalBattleCreateReq(v)
    }
}
impl From<super::battle_internal::BattleCreateResp> for MessageType {
    fn from(v: super::battle_internal::BattleCreateResp) -> Self {
        MessageType::BattleInternalBattleCreateResp(v)
    }
}
impl From<super::town_internal::TownEnterSceneReq> for MessageType {
    fn from(v: super::town_internal::TownEnterSceneReq) -> Self {
        MessageType::TownInternalTownEnterSceneReq(v)
    }
}
impl From<super::town_internal::TownEnterSceneResp> for MessageType {
    fn from(v: super::town_internal::TownEnterSceneResp) -> Self {
        MessageType::TownInternalTownEnterSceneResp(v)
    }
}
impl From<super::town_internal::TownLeaveSceneReq> for MessageType {
    fn from(v: super::town_internal::TownLeaveSceneReq) -> Self {
        MessageType::TownInternalTownLeaveSceneReq(v)
    }
}
impl From<super::town_internal::TownLeaveSceneResp> for MessageType {
    fn from(v: super::town_internal::TownLeaveSceneResp) -> Self {
        MessageType::TownInternalTownLeaveSceneResp(v)
    }
}
impl From<super::gateway_client::ServerStatusReq> for MessageType {
    fn from(v: super::gateway_client::ServerStatusReq) -> Self {
        MessageType::GatewayClientServerStatusReq(v)
    }
}
impl From<super::gateway_client::ServerStatusPush> for MessageType {
    fn from(v: super::gateway_client::ServerStatusPush) -> Self {
        MessageType::GatewayClientServerStatusPush(v)
    }
}
impl From<super::gateway_client::GatewayErrorResp> for MessageType {
    fn from(v: super::gateway_client::GatewayErrorResp) -> Self {
        MessageType::GatewayClientGatewayErrorResp(v)
    }
}
impl From<super::gateway_internal::ServerRegReq> for MessageType {
    fn from(v: super::gateway_internal::ServerRegReq) -> Self {
        MessageType::GatewayInternalServerRegReq(v)
    }
}
impl From<super::gateway_internal::ServerRegResp> for MessageType {
    fn from(v: super::gateway_internal::ServerRegResp) -> Self {
        MessageType::GatewayInternalServerRegResp(v)
    }
}
impl From<super::gateway_internal::BindServiceReq> for MessageType {
    fn from(v: super::gateway_internal::BindServiceReq) -> Self {
        MessageType::GatewayInternalBindServiceReq(v)
    }
}
impl From<super::gateway_internal::BindServiceResp> for MessageType {
    fn from(v: super::gateway_internal::BindServiceResp) -> Self {
        MessageType::GatewayInternalBindServiceResp(v)
    }
}
impl From<super::gateway_internal::UnbindServiceReq> for MessageType {
    fn from(v: super::gateway_internal::UnbindServiceReq) -> Self {
        MessageType::GatewayInternalUnbindServiceReq(v)
    }
}
impl From<super::gateway_internal::UnbindServiceResp> for MessageType {
    fn from(v: super::gateway_internal::UnbindServiceResp) -> Self {
        MessageType::GatewayInternalUnbindServiceResp(v)
    }
}
impl From<super::gateway_internal::KickSessionReq> for MessageType {
    fn from(v: super::gateway_internal::KickSessionReq) -> Self {
        MessageType::GatewayInternalKickSessionReq(v)
    }
}
impl From<super::gateway_internal::KickSessionRsp> for MessageType {
    fn from(v: super::gateway_internal::KickSessionRsp) -> Self {
        MessageType::GatewayInternalKickSessionRsp(v)
    }
}
impl From<super::gateway_internal::SessionOnlinePush> for MessageType {
    fn from(v: super::gateway_internal::SessionOnlinePush) -> Self {
        MessageType::GatewayInternalSessionOnlinePush(v)
    }
}
impl From<super::gateway_internal::SessionOfflinePush> for MessageType {
    fn from(v: super::gateway_internal::SessionOfflinePush) -> Self {
        MessageType::GatewayInternalSessionOfflinePush(v)
    }
}
impl From<super::gateway_internal::ServerOnlinePush> for MessageType {
    fn from(v: super::gateway_internal::ServerOnlinePush) -> Self {
        MessageType::GatewayInternalServerOnlinePush(v)
    }
}
impl From<super::gateway_internal::ServerOfflinePush> for MessageType {
    fn from(v: super::gateway_internal::ServerOfflinePush) -> Self {
        MessageType::GatewayInternalServerOfflinePush(v)
    }
}
impl From<super::gateway_internal::ForwardToServerReq> for MessageType {
    fn from(v: super::gateway_internal::ForwardToServerReq) -> Self {
        MessageType::GatewayInternalForwardToServerReq(v)
    }
}
impl From<super::gateway_internal::ServiceLoadReportPush> for MessageType {
    fn from(v: super::gateway_internal::ServiceLoadReportPush) -> Self {
        MessageType::GatewayInternalServiceLoadReportPush(v)
    }
}
impl From<super::gateway_internal::ServerPingReq> for MessageType {
    fn from(v: super::gateway_internal::ServerPingReq) -> Self {
        MessageType::GatewayInternalServerPingReq(v)
    }
}
impl From<super::gateway_internal::ServerPongResp> for MessageType {
    fn from(v: super::gateway_internal::ServerPongResp) -> Self {
        MessageType::GatewayInternalServerPongResp(v)
    }
}

pub fn get_message_id(message: &MessageType) -> Option<u32> {
    match message {
        MessageType::GameCommonSuccessResp(_) => Some(900u32),
        MessageType::GameCommonErrorResp(_) => Some(901u32),
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
        MessageType::GameBattleJoinReq(_) => Some(1300u32),
        MessageType::GameBattleJoinResp(_) => Some(1301u32),
        MessageType::GameDuelInviteReq(_) => Some(1304u32),
        MessageType::GameDuelInviteResp(_) => Some(1305u32),
        MessageType::GameDuelRespondReq(_) => Some(1306u32),
        MessageType::GameDuelRespondResp(_) => Some(1307u32),
        MessageType::GameEnterSceneReq(_) => Some(1302u32),
        MessageType::GameEnterSceneResp(_) => Some(1303u32),
        MessageType::GameAccountKickedPush(_) => Some(1400u32),
        MessageType::GameBattleServerOfflinePush(_) => Some(1401u32),
        MessageType::GameTownServerOfflinePush(_) => Some(1402u32),
        MessageType::GameDuelInvitePush(_) => Some(1410u32),
        MessageType::GameDuelStartPush(_) => Some(1411u32),
        MessageType::GameDuelEndPush(_) => Some(1412u32),
        MessageType::BattleBattleInputPush(_) => Some(20012u32),
        MessageType::BattleBattleSnapshotPush(_) => Some(20013u32),
        MessageType::TownTownPlayerStatePush(_) => Some(30000u32),
        MessageType::TownScenePlayerEnterPush(_) => Some(30001u32),
        MessageType::TownScenePlayerLeavePush(_) => Some(30002u32),
        MessageType::TownScenePlayerStatePush(_) => Some(30003u32),
        MessageType::BattleInternalBattleCreateReq(_) => Some(60000u32),
        MessageType::BattleInternalBattleCreateResp(_) => Some(60001u32),
        MessageType::TownInternalTownEnterSceneReq(_) => Some(61000u32),
        MessageType::TownInternalTownEnterSceneResp(_) => Some(61001u32),
        MessageType::TownInternalTownLeaveSceneReq(_) => Some(61002u32),
        MessageType::TownInternalTownLeaveSceneResp(_) => Some(61003u32),
        MessageType::GatewayClientServerStatusReq(_) => Some(1u32),
        MessageType::GatewayClientServerStatusPush(_) => Some(2u32),
        MessageType::GatewayClientGatewayErrorResp(_) => Some(3u32),
        MessageType::GatewayInternalServerRegReq(_) => Some(100u32),
        MessageType::GatewayInternalServerRegResp(_) => Some(101u32),
        MessageType::GatewayInternalBindServiceReq(_) => Some(102u32),
        MessageType::GatewayInternalBindServiceResp(_) => Some(103u32),
        MessageType::GatewayInternalUnbindServiceReq(_) => Some(104u32),
        MessageType::GatewayInternalUnbindServiceResp(_) => Some(105u32),
        MessageType::GatewayInternalKickSessionReq(_) => Some(106u32),
        MessageType::GatewayInternalKickSessionRsp(_) => Some(107u32),
        MessageType::GatewayInternalSessionOnlinePush(_) => Some(108u32),
        MessageType::GatewayInternalSessionOfflinePush(_) => Some(109u32),
        MessageType::GatewayInternalServerOnlinePush(_) => Some(110u32),
        MessageType::GatewayInternalServerOfflinePush(_) => Some(111u32),
        MessageType::GatewayInternalForwardToServerReq(_) => Some(112u32),
        MessageType::GatewayInternalServiceLoadReportPush(_) => Some(113u32),
        MessageType::GatewayInternalServerPingReq(_) => Some(114u32),
        MessageType::GatewayInternalServerPongResp(_) => Some(115u32),
        _ => None,
    }
}

pub fn decode_message(message_id: u32, bytes: &[u8]) -> Result<MessageType, DecodeError> {
    match message_id {
        900u32 => match super::game::CommonSuccessResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameCommonSuccessResp(message)),
            Err(err) => Err(err),
        },
        901u32 => match super::game::CommonErrorResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameCommonErrorResp(message)),
            Err(err) => Err(err),
        },
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
        1300u32 => match super::game::BattleJoinReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameBattleJoinReq(message)),
            Err(err) => Err(err),
        },
        1301u32 => match super::game::BattleJoinResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameBattleJoinResp(message)),
            Err(err) => Err(err),
        },
        1304u32 => match super::game::DuelInviteReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelInviteReq(message)),
            Err(err) => Err(err),
        },
        1305u32 => match super::game::DuelInviteResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelInviteResp(message)),
            Err(err) => Err(err),
        },
        1306u32 => match super::game::DuelRespondReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelRespondReq(message)),
            Err(err) => Err(err),
        },
        1307u32 => match super::game::DuelRespondResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelRespondResp(message)),
            Err(err) => Err(err),
        },
        1302u32 => match super::game::EnterSceneReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GameEnterSceneReq(message)),
            Err(err) => Err(err),
        },
        1303u32 => match super::game::EnterSceneResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GameEnterSceneResp(message)),
            Err(err) => Err(err),
        },
        1400u32 => match super::game::AccountKickedPush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameAccountKickedPush(message)),
            Err(err) => Err(err),
        },
        1401u32 => match super::game::BattleServerOfflinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameBattleServerOfflinePush(message)),
            Err(err) => Err(err),
        },
        1402u32 => match super::game::TownServerOfflinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameTownServerOfflinePush(message)),
            Err(err) => Err(err),
        },
        1410u32 => match super::game::DuelInvitePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelInvitePush(message)),
            Err(err) => Err(err),
        },
        1411u32 => match super::game::DuelStartPush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelStartPush(message)),
            Err(err) => Err(err),
        },
        1412u32 => match super::game::DuelEndPush::decode(bytes) {
            Ok(message) => Ok(MessageType::GameDuelEndPush(message)),
            Err(err) => Err(err),
        },
        20012u32 => match super::battle::BattleInputPush::decode(bytes) {
            Ok(message) => Ok(MessageType::BattleBattleInputPush(message)),
            Err(err) => Err(err),
        },
        20013u32 => match super::battle::BattleSnapshotPush::decode(bytes) {
            Ok(message) => Ok(MessageType::BattleBattleSnapshotPush(message)),
            Err(err) => Err(err),
        },
        30000u32 => match super::town::TownPlayerStatePush::decode(bytes) {
            Ok(message) => Ok(MessageType::TownTownPlayerStatePush(message)),
            Err(err) => Err(err),
        },
        30001u32 => match super::town::ScenePlayerEnterPush::decode(bytes) {
            Ok(message) => Ok(MessageType::TownScenePlayerEnterPush(message)),
            Err(err) => Err(err),
        },
        30002u32 => match super::town::ScenePlayerLeavePush::decode(bytes) {
            Ok(message) => Ok(MessageType::TownScenePlayerLeavePush(message)),
            Err(err) => Err(err),
        },
        30003u32 => match super::town::ScenePlayerStatePush::decode(bytes) {
            Ok(message) => Ok(MessageType::TownScenePlayerStatePush(message)),
            Err(err) => Err(err),
        },
        60000u32 => match super::battle_internal::BattleCreateReq::decode(bytes) {
            Ok(message) => Ok(MessageType::BattleInternalBattleCreateReq(message)),
            Err(err) => Err(err),
        },
        60001u32 => match super::battle_internal::BattleCreateResp::decode(bytes) {
            Ok(message) => Ok(MessageType::BattleInternalBattleCreateResp(message)),
            Err(err) => Err(err),
        },
        61000u32 => match super::town_internal::TownEnterSceneReq::decode(bytes) {
            Ok(message) => Ok(MessageType::TownInternalTownEnterSceneReq(message)),
            Err(err) => Err(err),
        },
        61001u32 => match super::town_internal::TownEnterSceneResp::decode(bytes) {
            Ok(message) => Ok(MessageType::TownInternalTownEnterSceneResp(message)),
            Err(err) => Err(err),
        },
        61002u32 => match super::town_internal::TownLeaveSceneReq::decode(bytes) {
            Ok(message) => Ok(MessageType::TownInternalTownLeaveSceneReq(message)),
            Err(err) => Err(err),
        },
        61003u32 => match super::town_internal::TownLeaveSceneResp::decode(bytes) {
            Ok(message) => Ok(MessageType::TownInternalTownLeaveSceneResp(message)),
            Err(err) => Err(err),
        },
        1u32 => match super::gateway_client::ServerStatusReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayClientServerStatusReq(message)),
            Err(err) => Err(err),
        },
        2u32 => match super::gateway_client::ServerStatusPush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayClientServerStatusPush(message)),
            Err(err) => Err(err),
        },
        3u32 => match super::gateway_client::GatewayErrorResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayClientGatewayErrorResp(message)),
            Err(err) => Err(err),
        },
        100u32 => match super::gateway_internal::ServerRegReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerRegReq(message)),
            Err(err) => Err(err),
        },
        101u32 => match super::gateway_internal::ServerRegResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerRegResp(message)),
            Err(err) => Err(err),
        },
        102u32 => match super::gateway_internal::BindServiceReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalBindServiceReq(message)),
            Err(err) => Err(err),
        },
        103u32 => match super::gateway_internal::BindServiceResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalBindServiceResp(message)),
            Err(err) => Err(err),
        },
        104u32 => match super::gateway_internal::UnbindServiceReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalUnbindServiceReq(message)),
            Err(err) => Err(err),
        },
        105u32 => match super::gateway_internal::UnbindServiceResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalUnbindServiceResp(message)),
            Err(err) => Err(err),
        },
        106u32 => match super::gateway_internal::KickSessionReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalKickSessionReq(message)),
            Err(err) => Err(err),
        },
        107u32 => match super::gateway_internal::KickSessionRsp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalKickSessionRsp(message)),
            Err(err) => Err(err),
        },
        108u32 => match super::gateway_internal::SessionOnlinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalSessionOnlinePush(message)),
            Err(err) => Err(err),
        },
        109u32 => match super::gateway_internal::SessionOfflinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalSessionOfflinePush(message)),
            Err(err) => Err(err),
        },
        110u32 => match super::gateway_internal::ServerOnlinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerOnlinePush(message)),
            Err(err) => Err(err),
        },
        111u32 => match super::gateway_internal::ServerOfflinePush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerOfflinePush(message)),
            Err(err) => Err(err),
        },
        112u32 => match super::gateway_internal::ForwardToServerReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalForwardToServerReq(message)),
            Err(err) => Err(err),
        },
        113u32 => match super::gateway_internal::ServiceLoadReportPush::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServiceLoadReportPush(message)),
            Err(err) => Err(err),
        },
        114u32 => match super::gateway_internal::ServerPingReq::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerPingReq(message)),
            Err(err) => Err(err),
        },
        115u32 => match super::gateway_internal::ServerPongResp::decode(bytes) {
            Ok(message) => Ok(MessageType::GatewayInternalServerPongResp(message)),
            Err(err) => Err(err),
        },
        _ => Err(DecodeError::new("unknown message id")),
    }
}

pub fn encode_message(message: &MessageType) -> Option<(u32, Vec<u8>)> {
    match message {
        MessageType::GameCommonSuccessResp(msg) => Some((900u32, msg.encode_to_vec())),
        MessageType::GameCommonErrorResp(msg) => Some((901u32, msg.encode_to_vec())),
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
        MessageType::GameBattleJoinReq(msg) => Some((1300u32, msg.encode_to_vec())),
        MessageType::GameBattleJoinResp(msg) => Some((1301u32, msg.encode_to_vec())),
        MessageType::GameDuelInviteReq(msg) => Some((1304u32, msg.encode_to_vec())),
        MessageType::GameDuelInviteResp(msg) => Some((1305u32, msg.encode_to_vec())),
        MessageType::GameDuelRespondReq(msg) => Some((1306u32, msg.encode_to_vec())),
        MessageType::GameDuelRespondResp(msg) => Some((1307u32, msg.encode_to_vec())),
        MessageType::GameEnterSceneReq(msg) => Some((1302u32, msg.encode_to_vec())),
        MessageType::GameEnterSceneResp(msg) => Some((1303u32, msg.encode_to_vec())),
        MessageType::GameAccountKickedPush(msg) => Some((1400u32, msg.encode_to_vec())),
        MessageType::GameBattleServerOfflinePush(msg) => Some((1401u32, msg.encode_to_vec())),
        MessageType::GameTownServerOfflinePush(msg) => Some((1402u32, msg.encode_to_vec())),
        MessageType::GameDuelInvitePush(msg) => Some((1410u32, msg.encode_to_vec())),
        MessageType::GameDuelStartPush(msg) => Some((1411u32, msg.encode_to_vec())),
        MessageType::GameDuelEndPush(msg) => Some((1412u32, msg.encode_to_vec())),
        MessageType::BattleBattleInputPush(msg) => Some((20012u32, msg.encode_to_vec())),
        MessageType::BattleBattleSnapshotPush(msg) => Some((20013u32, msg.encode_to_vec())),
        MessageType::TownTownPlayerStatePush(msg) => Some((30000u32, msg.encode_to_vec())),
        MessageType::TownScenePlayerEnterPush(msg) => Some((30001u32, msg.encode_to_vec())),
        MessageType::TownScenePlayerLeavePush(msg) => Some((30002u32, msg.encode_to_vec())),
        MessageType::TownScenePlayerStatePush(msg) => Some((30003u32, msg.encode_to_vec())),
        MessageType::BattleInternalBattleCreateReq(msg) => Some((60000u32, msg.encode_to_vec())),
        MessageType::BattleInternalBattleCreateResp(msg) => Some((60001u32, msg.encode_to_vec())),
        MessageType::TownInternalTownEnterSceneReq(msg) => Some((61000u32, msg.encode_to_vec())),
        MessageType::TownInternalTownEnterSceneResp(msg) => Some((61001u32, msg.encode_to_vec())),
        MessageType::TownInternalTownLeaveSceneReq(msg) => Some((61002u32, msg.encode_to_vec())),
        MessageType::TownInternalTownLeaveSceneResp(msg) => Some((61003u32, msg.encode_to_vec())),
        MessageType::GatewayClientServerStatusReq(msg) => Some((1u32, msg.encode_to_vec())),
        MessageType::GatewayClientServerStatusPush(msg) => Some((2u32, msg.encode_to_vec())),
        MessageType::GatewayClientGatewayErrorResp(msg) => Some((3u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerRegReq(msg) => Some((100u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerRegResp(msg) => Some((101u32, msg.encode_to_vec())),
        MessageType::GatewayInternalBindServiceReq(msg) => Some((102u32, msg.encode_to_vec())),
        MessageType::GatewayInternalBindServiceResp(msg) => Some((103u32, msg.encode_to_vec())),
        MessageType::GatewayInternalUnbindServiceReq(msg) => Some((104u32, msg.encode_to_vec())),
        MessageType::GatewayInternalUnbindServiceResp(msg) => Some((105u32, msg.encode_to_vec())),
        MessageType::GatewayInternalKickSessionReq(msg) => Some((106u32, msg.encode_to_vec())),
        MessageType::GatewayInternalKickSessionRsp(msg) => Some((107u32, msg.encode_to_vec())),
        MessageType::GatewayInternalSessionOnlinePush(msg) => Some((108u32, msg.encode_to_vec())),
        MessageType::GatewayInternalSessionOfflinePush(msg) => Some((109u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerOnlinePush(msg) => Some((110u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerOfflinePush(msg) => Some((111u32, msg.encode_to_vec())),
        MessageType::GatewayInternalForwardToServerReq(msg) => Some((112u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServiceLoadReportPush(msg) => Some((113u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerPingReq(msg) => Some((114u32, msg.encode_to_vec())),
        MessageType::GatewayInternalServerPongResp(msg) => Some((115u32, msg.encode_to_vec())),
        _ => None,
    }
}

pub fn get_message_size(message: &MessageType) -> usize {
    match message {
        MessageType::GameCommonSuccessResp(msg) => msg.encoded_len(),
        MessageType::GameCommonErrorResp(msg) => msg.encoded_len(),
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
        MessageType::GameBattleJoinReq(msg) => msg.encoded_len(),
        MessageType::GameBattleJoinResp(msg) => msg.encoded_len(),
        MessageType::GameDuelInviteReq(msg) => msg.encoded_len(),
        MessageType::GameDuelInviteResp(msg) => msg.encoded_len(),
        MessageType::GameDuelRespondReq(msg) => msg.encoded_len(),
        MessageType::GameDuelRespondResp(msg) => msg.encoded_len(),
        MessageType::GameEnterSceneReq(msg) => msg.encoded_len(),
        MessageType::GameEnterSceneResp(msg) => msg.encoded_len(),
        MessageType::GameAccountKickedPush(msg) => msg.encoded_len(),
        MessageType::GameBattleServerOfflinePush(msg) => msg.encoded_len(),
        MessageType::GameTownServerOfflinePush(msg) => msg.encoded_len(),
        MessageType::GameDuelInvitePush(msg) => msg.encoded_len(),
        MessageType::GameDuelStartPush(msg) => msg.encoded_len(),
        MessageType::GameDuelEndPush(msg) => msg.encoded_len(),
        MessageType::BattleBattleInputPush(msg) => msg.encoded_len(),
        MessageType::BattleBattleSnapshotPush(msg) => msg.encoded_len(),
        MessageType::TownTownPlayerStatePush(msg) => msg.encoded_len(),
        MessageType::TownScenePlayerEnterPush(msg) => msg.encoded_len(),
        MessageType::TownScenePlayerLeavePush(msg) => msg.encoded_len(),
        MessageType::TownScenePlayerStatePush(msg) => msg.encoded_len(),
        MessageType::BattleInternalBattleCreateReq(msg) => msg.encoded_len(),
        MessageType::BattleInternalBattleCreateResp(msg) => msg.encoded_len(),
        MessageType::TownInternalTownEnterSceneReq(msg) => msg.encoded_len(),
        MessageType::TownInternalTownEnterSceneResp(msg) => msg.encoded_len(),
        MessageType::TownInternalTownLeaveSceneReq(msg) => msg.encoded_len(),
        MessageType::TownInternalTownLeaveSceneResp(msg) => msg.encoded_len(),
        MessageType::GatewayClientServerStatusReq(msg) => msg.encoded_len(),
        MessageType::GatewayClientServerStatusPush(msg) => msg.encoded_len(),
        MessageType::GatewayClientGatewayErrorResp(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerRegReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerRegResp(msg) => msg.encoded_len(),
        MessageType::GatewayInternalBindServiceReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalBindServiceResp(msg) => msg.encoded_len(),
        MessageType::GatewayInternalUnbindServiceReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalUnbindServiceResp(msg) => msg.encoded_len(),
        MessageType::GatewayInternalKickSessionReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalKickSessionRsp(msg) => msg.encoded_len(),
        MessageType::GatewayInternalSessionOnlinePush(msg) => msg.encoded_len(),
        MessageType::GatewayInternalSessionOfflinePush(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerOnlinePush(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerOfflinePush(msg) => msg.encoded_len(),
        MessageType::GatewayInternalForwardToServerReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServiceLoadReportPush(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerPingReq(msg) => msg.encoded_len(),
        MessageType::GatewayInternalServerPongResp(msg) => msg.encoded_len(),
        _ => 0,
    }
}

pub fn encode_raw_message(message: &MessageType, buf: &mut impl BufMut) {
    match message {
        MessageType::GameCommonSuccessResp(msg) => msg.encode_raw(buf),
        MessageType::GameCommonErrorResp(msg) => msg.encode_raw(buf),
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
        MessageType::GameBattleJoinReq(msg) => msg.encode_raw(buf),
        MessageType::GameBattleJoinResp(msg) => msg.encode_raw(buf),
        MessageType::GameDuelInviteReq(msg) => msg.encode_raw(buf),
        MessageType::GameDuelInviteResp(msg) => msg.encode_raw(buf),
        MessageType::GameDuelRespondReq(msg) => msg.encode_raw(buf),
        MessageType::GameDuelRespondResp(msg) => msg.encode_raw(buf),
        MessageType::GameEnterSceneReq(msg) => msg.encode_raw(buf),
        MessageType::GameEnterSceneResp(msg) => msg.encode_raw(buf),
        MessageType::GameAccountKickedPush(msg) => msg.encode_raw(buf),
        MessageType::GameBattleServerOfflinePush(msg) => msg.encode_raw(buf),
        MessageType::GameTownServerOfflinePush(msg) => msg.encode_raw(buf),
        MessageType::GameDuelInvitePush(msg) => msg.encode_raw(buf),
        MessageType::GameDuelStartPush(msg) => msg.encode_raw(buf),
        MessageType::GameDuelEndPush(msg) => msg.encode_raw(buf),
        MessageType::BattleBattleInputPush(msg) => msg.encode_raw(buf),
        MessageType::BattleBattleSnapshotPush(msg) => msg.encode_raw(buf),
        MessageType::TownTownPlayerStatePush(msg) => msg.encode_raw(buf),
        MessageType::TownScenePlayerEnterPush(msg) => msg.encode_raw(buf),
        MessageType::TownScenePlayerLeavePush(msg) => msg.encode_raw(buf),
        MessageType::TownScenePlayerStatePush(msg) => msg.encode_raw(buf),
        MessageType::BattleInternalBattleCreateReq(msg) => msg.encode_raw(buf),
        MessageType::BattleInternalBattleCreateResp(msg) => msg.encode_raw(buf),
        MessageType::TownInternalTownEnterSceneReq(msg) => msg.encode_raw(buf),
        MessageType::TownInternalTownEnterSceneResp(msg) => msg.encode_raw(buf),
        MessageType::TownInternalTownLeaveSceneReq(msg) => msg.encode_raw(buf),
        MessageType::TownInternalTownLeaveSceneResp(msg) => msg.encode_raw(buf),
        MessageType::GatewayClientServerStatusReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayClientServerStatusPush(msg) => msg.encode_raw(buf),
        MessageType::GatewayClientGatewayErrorResp(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerRegReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerRegResp(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalBindServiceReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalBindServiceResp(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalUnbindServiceReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalUnbindServiceResp(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalKickSessionReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalKickSessionRsp(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalSessionOnlinePush(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalSessionOfflinePush(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerOnlinePush(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerOfflinePush(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalForwardToServerReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServiceLoadReportPush(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerPingReq(msg) => msg.encode_raw(buf),
        MessageType::GatewayInternalServerPongResp(msg) => msg.encode_raw(buf),
        _ => {}
    }
}

#[cfg(feature = "serde-serialize")]
pub fn serialize_to_json(message: &MessageType) -> serde_json::Result<String> {
    match message {
        MessageType::GameCommonSuccessResp(msg) => serde_json::to_string(&msg),
        MessageType::GameCommonErrorResp(msg) => serde_json::to_string(&msg),
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
        MessageType::GameBattleJoinReq(msg) => serde_json::to_string(&msg),
        MessageType::GameBattleJoinResp(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelInviteReq(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelInviteResp(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelRespondReq(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelRespondResp(msg) => serde_json::to_string(&msg),
        MessageType::GameEnterSceneReq(msg) => serde_json::to_string(&msg),
        MessageType::GameEnterSceneResp(msg) => serde_json::to_string(&msg),
        MessageType::GameAccountKickedPush(msg) => serde_json::to_string(&msg),
        MessageType::GameBattleServerOfflinePush(msg) => serde_json::to_string(&msg),
        MessageType::GameTownServerOfflinePush(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelInvitePush(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelStartPush(msg) => serde_json::to_string(&msg),
        MessageType::GameDuelEndPush(msg) => serde_json::to_string(&msg),
        MessageType::BattleBattleInputPush(msg) => serde_json::to_string(&msg),
        MessageType::BattleBattleSnapshotPush(msg) => serde_json::to_string(&msg),
        MessageType::TownTownPlayerStatePush(msg) => serde_json::to_string(&msg),
        MessageType::TownScenePlayerEnterPush(msg) => serde_json::to_string(&msg),
        MessageType::TownScenePlayerLeavePush(msg) => serde_json::to_string(&msg),
        MessageType::TownScenePlayerStatePush(msg) => serde_json::to_string(&msg),
        MessageType::BattleInternalBattleCreateReq(msg) => serde_json::to_string(&msg),
        MessageType::BattleInternalBattleCreateResp(msg) => serde_json::to_string(&msg),
        MessageType::TownInternalTownEnterSceneReq(msg) => serde_json::to_string(&msg),
        MessageType::TownInternalTownEnterSceneResp(msg) => serde_json::to_string(&msg),
        MessageType::TownInternalTownLeaveSceneReq(msg) => serde_json::to_string(&msg),
        MessageType::TownInternalTownLeaveSceneResp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayClientServerStatusReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayClientServerStatusPush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayClientGatewayErrorResp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerRegReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerRegResp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalBindServiceReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalBindServiceResp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalUnbindServiceReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalUnbindServiceResp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalKickSessionReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalKickSessionRsp(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalSessionOnlinePush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalSessionOfflinePush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerOnlinePush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerOfflinePush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalForwardToServerReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServiceLoadReportPush(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerPingReq(msg) => serde_json::to_string(&msg),
        MessageType::GatewayInternalServerPongResp(msg) => serde_json::to_string(&msg),
        _ => Ok("null".into()),
    }
}
