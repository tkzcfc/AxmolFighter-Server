use std::sync::atomic::Ordering;
use std::time::Duration;
use std::{error, fmt};

use async_trait::async_trait;
use bytes::Bytes;
use protocol::game::{
    BattleJoinResp, CreateCharacterResp, FetchCharacterListResp, LoginResp, RegisterResp,
    SelectCharacterResp,
};
use protocol::gateway::{
    BindServiceResp, ForwardToServerReq, GatewayErrorResp, KickSessionReq, UnbindServiceResp,
};
use protocol::message_map::{MessageType, decode_message};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::gateway_client::GatewaySender;
use crate::handler::{GameHandler, GameShared, MessageHandler};
use crate::player::{PlayerActor, PlayerCommand, PlayerSendError};
use crate::wire::{CMD_BUSINESS, CMD_GATEWAY_CONTROL};

pub(super) enum PendingResponse {
    Message(MessageType),
    GatewayError { code: u32, message: String },
}

#[derive(Debug)]
pub(crate) enum RpcError {
    Send(String),
    Cancelled,
    Timeout,
    Gateway { code: u32, message: String },
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(message) => write!(f, "request send failed: {message}"),
            Self::Cancelled => write!(f, "request cancelled"),
            Self::Timeout => write!(f, "request timeout"),
            Self::Gateway { code, message } => {
                write!(f, "gateway error code={code} message={message}")
            }
        }
    }
}

impl error::Error for RpcError {}

impl GameShared {
    /// 发送 protobuf 消息给客户端（自动编码）
    pub fn send_msg(&self, msg: &MessageType, serial: i32, session_id: u32) {
        if let Err(err) = self.try_send_frame_msg(CMD_BUSINESS, msg, serial, session_id) {
            warn!("failed to send business message: {}", err);
        }
    }

    pub(super) fn try_send_frame_msg(
        &self,
        cmd: u8,
        msg: &MessageType,
        serial: i32,
        session_id: u32,
    ) -> anyhow::Result<()> {
        let tx = self.gateway_tx.lock().unwrap();
        let sender = tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("gateway not connected"))?;
        let (msg_id, payload) = protocol::message_map::encode_message(msg)
            .ok_or_else(|| anyhow::anyhow!("failed to encode message"))?;
        let data = crate::codec::encode_frame(cmd, msg_id as u16, serial, session_id, &payload);
        sender
            .send(data)
            .map_err(|_| anyhow::anyhow!("gateway channel closed"))?;
        Ok(())
    }

    fn next_request_id(&self) -> i32 {
        let request_id = self.control_serial_seed.fetch_add(1, Ordering::Relaxed);
        if request_id <= 0 {
            self.control_serial_seed.store(1, Ordering::Relaxed);
            1
        } else {
            request_id
        }
    }

    async fn wait_pending_response(
        &self,
        request_id: i32,
        rx: oneshot::Receiver<PendingResponse>,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(PendingResponse::Message(msg))) => Ok(msg),
            Ok(Ok(PendingResponse::GatewayError { code, message })) => {
                Err(RpcError::Gateway { code, message })
            }
            Ok(Err(_)) => Err(RpcError::Cancelled),
            Err(_) => {
                self.pending_requests.remove(&request_id);
                Err(RpcError::Timeout)
            }
        }
    }

    pub(crate) async fn request_gateway(
        &self,
        msg: MessageType,
        session_id: u32,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id, tx);

        let send_result =
            self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &msg, -request_id, session_id);
        if let Err(err) = send_result {
            self.pending_requests.remove(&request_id);
            return Err(RpcError::Send(err.to_string()));
        }

        self.wait_pending_response(request_id, rx, timeout).await
    }

    pub(crate) async fn request_server(
        &self,
        target_service_id: u32,
        target_instance_id: i32,
        session_id: u32,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id, tx);

        let send_result = (|| -> anyhow::Result<()> {
            let (msg_id, payload) = protocol::message_map::encode_message(&msg)
                .ok_or_else(|| anyhow::anyhow!("failed to encode inner message"))?;
            let frame = crate::codec::encode_frame(
                CMD_BUSINESS,
                msg_id as u16,
                -request_id,
                session_id,
                &payload,
            );
            let forward = MessageType::GatewayForwardToServerReq(ForwardToServerReq {
                target_service_id,
                target_instance_id,
                payload: frame.to_vec(),
                source_service_id: 0,
                source_instance_id: 0,
            });
            self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &forward, -request_id, session_id)
        })();

        if let Err(err) = send_result {
            self.pending_requests.remove(&request_id);
            return Err(RpcError::Send(err.to_string()));
        }

        self.wait_pending_response(request_id, rx, timeout).await
    }

    pub(super) fn resolve_pending_response(&self, serial: i32, msg: MessageType) -> bool {
        if serial <= 0 {
            return false;
        }

        if let Some((_, tx)) = self.pending_requests.remove(&serial) {
            let _ = tx.send(PendingResponse::Message(msg));
            true
        } else {
            false
        }
    }

    pub(super) fn handle_gateway_error_resp(&self, serial: i32, resp: GatewayErrorResp) {
        if serial > 0 {
            if let Some((_, tx)) = self.pending_requests.remove(&serial) {
                let _ = tx.send(PendingResponse::GatewayError {
                    code: resp.code,
                    message: resp.message,
                });
                return;
            }
        }

        debug!("ignored unmatched GatewayErrorResp serial={}", serial);
    }

    pub(super) fn start_session_actor(self: &std::sync::Arc<Self>, session_id: u32) {
        if let Some((_, old_ref)) = self.players.remove(&session_id) {
            old_ref.stop();
        }

        let player_ref = PlayerActor::spawn(self.clone(), session_id);
        self.players.insert(session_id, player_ref);
        debug!("player session {} online", session_id);
    }

    pub(super) fn stop_session_actor(&self, session_id: u32) {
        if let Some((_, player_ref)) = self.players.remove(&session_id) {
            player_ref.stop();
        }
        self.unbind_session_accounts(session_id);
        debug!("player session {} offline", session_id);
    }

    pub(super) fn stop_all_session_actors(&self) {
        let session_ids: Vec<u32> = self.players.iter().map(|entry| *entry.key()).collect();
        for session_id in session_ids {
            if let Some((_, player_ref)) = self.players.remove(&session_id) {
                player_ref.stop();
            }
        }
        self.account_sessions.lock().unwrap().clear();
    }

    pub(crate) fn bind_account(&self, session_id: u32, account_id: i64) {
        let old_session_id = {
            let mut account_sessions = self.account_sessions.lock().unwrap();
            account_sessions.insert(account_id, session_id)
        };

        let Some(old_session_id) = old_session_id else {
            return;
        };
        if old_session_id == session_id {
            return;
        }

        warn!(
            "account {} login from session {}, kicking old session {}",
            account_id, session_id, old_session_id
        );
        self.kick_session(old_session_id);
        self.stop_session_actor(old_session_id);
    }

    pub(crate) fn next_battle_id(&self) -> u32 {
        self.battle_id_seed.fetch_add(1, Ordering::Relaxed)
    }

    fn unbind_session_accounts(&self, session_id: u32) {
        let mut account_sessions = self.account_sessions.lock().unwrap();
        account_sessions.retain(|_, bound_session_id| *bound_session_id != session_id);
    }

    fn kick_session(&self, session_id: u32) {
        let msg = MessageType::GatewayKickSessionReq(KickSessionReq { session_id });
        if let Err(err) = self.try_send_frame_msg(CMD_GATEWAY_CONTROL, &msg, 0, session_id) {
            warn!("failed to kick old session {}: {}", session_id, err);
        }
    }

    fn dispatch_client_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes) {
        let Some(player_ref) = self
            .players
            .get(&session_id)
            .map(|entry| entry.value().clone())
        else {
            debug!(
                "dropped message for missing player actor session={} msg_id={}",
                session_id, msg_id
            );
            return;
        };

        let cmd = PlayerCommand::ClientMessage {
            msg_id,
            serial,
            payload,
        };
        if let Err(err) = player_ref.try_send(cmd) {
            match err {
                PlayerSendError::Full(PlayerCommand::ClientMessage {
                    msg_id,
                    serial,
                    payload,
                }) => {
                    warn!(
                        "player mailbox full session={} msg_id={}",
                        session_id, msg_id
                    );
                    if serial < 0
                        && let Some(resp) = Self::busy_response(msg_id, &payload)
                    {
                        self.send_msg(&resp, -serial, session_id);
                    }
                }
                PlayerSendError::Closed(_) => {
                    warn!(
                        "player mailbox closed session={} msg_id={}",
                        session_id, msg_id
                    );
                    self.stop_session_actor(session_id);
                }
                PlayerSendError::Full(PlayerCommand::Stop) => {}
            }
        }
    }

    fn busy_response(msg_id: u16, payload: &[u8]) -> Option<MessageType> {
        match decode_message(msg_id as u32, payload).ok()? {
            MessageType::GameLoginReq(_) => Some(MessageType::GameLoginResp(LoginResp {
                code: -1,
                message: "服务器繁忙".to_string(),
                player_id: 0,
                nickname: String::new(),
                server_config: None,
                account_info: None,
            })),
            MessageType::GameRegisterReq(_) => Some(MessageType::GameRegisterResp(RegisterResp {
                code: -1,
                message: "服务器繁忙".to_string(),
                player_id: 0,
            })),
            MessageType::GameFetchCharacterListReq(_) => Some(
                MessageType::GameFetchCharacterListResp(FetchCharacterListResp {
                    code: -1,
                    message: "服务器繁忙".to_string(),
                    characters: vec![],
                }),
            ),
            MessageType::GameCreateCharacterReq(_) => {
                Some(MessageType::GameCreateCharacterResp(CreateCharacterResp {
                    code: -1,
                    message: "服务器繁忙".to_string(),
                    character: None,
                }))
            }
            MessageType::GameSelectCharacterReq(_) => {
                Some(MessageType::GameSelectCharacterResp(SelectCharacterResp {
                    code: -1,
                    message: "服务器繁忙".to_string(),
                    character: None,
                    inventory: None,
                }))
            }
            MessageType::GameBattleJoinReq(_) => {
                Some(MessageType::GameBattleJoinResp(BattleJoinResp {
                    code: -1,
                    message: "服务器繁忙".to_string(),
                    battle_id: 0,
                    server_frame: 0,
                    world_dump: vec![],
                }))
            }
            _ => None,
        }
    }
}

#[async_trait]
impl MessageHandler for GameHandler {
    fn on_gateway_connected(&self, tx: GatewaySender) {
        let mut guard = self.shared.gateway_tx.lock().unwrap();
        *guard = Some(tx);
    }

    fn on_gateway_disconnected(&self) {
        let mut guard = self.shared.gateway_tx.lock().unwrap();
        *guard = None;
        self.shared.pending_requests.clear();
        self.shared.stop_all_session_actors();
    }

    async fn on_session_online(&self, session_id: u32) {
        self.shared.start_session_actor(session_id);
    }

    async fn on_session_offline(&self, session_id: u32) {
        self.shared.stop_session_actor(session_id);
    }

    async fn on_bind_service_resp(&self, serial: i32, resp: BindServiceResp) {
        if self
            .shared
            .resolve_pending_response(serial, MessageType::GatewayBindServiceResp(resp))
        {
            return;
        }

        if serial > 0 {
            debug!("ignored unmatched BindServiceResp serial={}", serial);
        } else {
            debug!(
                "ignored BindServiceResp with non-response serial={}",
                serial
            );
        }
    }

    async fn on_unbind_service_resp(&self, serial: i32, resp: UnbindServiceResp) {
        if self
            .shared
            .resolve_pending_response(serial, MessageType::GatewayUnbindServiceResp(resp))
        {
            return;
        }

        if serial > 0 {
            debug!("ignored unmatched UnbindServiceResp serial={}", serial);
        } else {
            debug!(
                "ignored UnbindServiceResp with non-response serial={}",
                serial
            );
        }
    }

    async fn on_gateway_error_resp(&self, serial: i32, resp: GatewayErrorResp) {
        self.shared.handle_gateway_error_resp(serial, resp);
    }

    async fn on_message(&self, msg_id: u16, serial: i32, session_id: u32, payload: Bytes) {
        if serial > 0 {
            match decode_message(msg_id as u32, &payload) {
                Ok(msg) => {
                    if !self.shared.resolve_pending_response(serial, msg) {
                        debug!("ignored unmatched response serial={}", serial);
                    }
                }
                Err(e) => {
                    warn!(
                        "failed to decode response msg_id={} from session={}: {}",
                        msg_id, session_id, e
                    );
                }
            }
            return;
        }

        self.shared
            .dispatch_client_message(msg_id, serial, session_id, payload);
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use protocol::message_map::{MessageType, decode_message};
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::mpsc;

    use super::*;
    use crate::codec::try_extract_frame;

    fn test_handler() -> std::sync::Arc<GameHandler> {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost/test")
            .expect("test pool");
        GameHandler::new(pool)
    }

    #[tokio::test]
    async fn session_online_creates_actor_and_offline_removes_it() {
        let handler = test_handler();

        handler.on_session_online(1001).await;
        assert!(handler.shared.players.contains_key(&1001));

        handler.shared.bind_account(1001, 5001);
        assert_eq!(
            handler
                .shared
                .account_sessions
                .lock()
                .unwrap()
                .get(&5001)
                .copied(),
            Some(1001)
        );

        handler.on_session_offline(1001).await;
        assert!(!handler.shared.players.contains_key(&1001));
        assert!(handler.shared.account_sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_login_replaces_account_binding_and_kicks_old_session() {
        let handler = test_handler();
        let (tx, mut rx) = mpsc::unbounded_channel();
        handler.on_gateway_connected(tx);
        handler.on_session_online(1001).await;
        handler.on_session_online(1002).await;

        handler.shared.bind_account(1001, 5001);
        handler.shared.bind_account(1002, 5001);

        assert!(!handler.shared.players.contains_key(&1001));
        assert!(handler.shared.players.contains_key(&1002));
        assert_eq!(
            handler
                .shared
                .account_sessions
                .lock()
                .unwrap()
                .get(&5001)
                .copied(),
            Some(1002)
        );

        let data = rx.recv().await.expect("kick frame");
        let mut buf = BytesMut::from(data.as_ref());
        let frame = try_extract_frame(&mut buf)
            .expect("valid frame")
            .expect("complete frame");
        assert_eq!(frame.session_id, 1001);
        assert_eq!(frame.serial, 0);
        assert!(matches!(
            decode_message(frame.msg_id as u32, &frame.payload).expect("kick message"),
            MessageType::GatewayKickSessionReq(req) if req.session_id == 1001
        ));
    }

    #[tokio::test]
    async fn gateway_disconnected_treats_all_sessions_as_offline() {
        let handler = test_handler();
        let (tx, _rx) = mpsc::unbounded_channel();
        handler.on_gateway_connected(tx);
        handler.on_session_online(1001).await;
        handler.on_session_online(1002).await;
        handler.shared.bind_account(1001, 5001);
        handler.shared.bind_account(1002, 5002);

        handler.on_gateway_disconnected();

        assert!(handler.shared.players.is_empty());
        assert!(handler.shared.account_sessions.lock().unwrap().is_empty());
        assert!(handler.shared.pending_requests.is_empty());
        assert!(handler.shared.gateway_tx.lock().unwrap().is_none());
    }
}
