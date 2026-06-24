mod account;
mod battle;
mod character;
mod framework;

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::game_shared::GameShared;

const PLAYER_MAILBOX_SIZE: usize = 256;

#[derive(Clone)]
pub(crate) struct PlayerRef {
    tx: mpsc::Sender<PlayerCommand>,
    abort_handle: AbortHandle,
}

pub(crate) enum PlayerCommand {
    ClientMessage {
        msg_id: u16,
        serial: i32,
        payload: Bytes,
    },
    Stop,
}

pub(crate) enum PlayerSendError {
    Full(PlayerCommand),
    Closed(PlayerCommand),
}

pub(crate) struct PlayerActor {
    pub(super) session_id: u32,
    pub(super) account_id: Option<i64>,
    pub(super) shared: Arc<GameShared>,
    rx: mpsc::Receiver<PlayerCommand>,
}

impl PlayerRef {
    fn new(tx: mpsc::Sender<PlayerCommand>, abort_handle: AbortHandle) -> Self {
        Self { tx, abort_handle }
    }

    pub(crate) fn try_send(&self, cmd: PlayerCommand) -> Result<(), PlayerSendError> {
        match self.tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(cmd)) => Err(PlayerSendError::Full(cmd)),
            Err(mpsc::error::TrySendError::Closed(cmd)) => Err(PlayerSendError::Closed(cmd)),
        }
    }

    pub(crate) fn stop(&self) {
        let _ = self.tx.try_send(PlayerCommand::Stop);
        self.abort_handle.abort();
    }
}

impl PlayerActor {
    pub(crate) fn spawn(shared: Arc<GameShared>, session_id: u32) -> PlayerRef {
        let (tx, rx) = mpsc::channel(PLAYER_MAILBOX_SIZE);
        let actor = Self {
            session_id,
            account_id: None,
            shared,
            rx,
        };
        let handle = tokio::spawn(async move {
            actor.run().await;
        });
        PlayerRef::new(tx, handle.abort_handle())
    }

    async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                PlayerCommand::ClientMessage {
                    msg_id,
                    serial,
                    payload,
                } => self.handle_client_frame(msg_id, serial, payload).await,
                PlayerCommand::Stop => break,
            }
        }
    }

    pub(super) fn account_id(&self) -> Option<i64> {
        self.account_id
    }
}
