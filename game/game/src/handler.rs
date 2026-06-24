use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::codec::BackendFrame;
use crate::game_shared::GameShared;
use crate::gateway_client::GatewaySender;

#[async_trait]
pub trait MessageHandler: Send + Sync {
    fn on_gateway_connected(&self, tx: GatewaySender);

    fn on_gateway_disconnected(&self);

    async fn on_gateway_control_frame(&self, frame: BackendFrame);

    async fn on_business_frame(&self, frame: BackendFrame);
}

pub struct GameHandler {
    shared: Arc<GameShared>,
}

impl GameHandler {
    pub fn new(pool: PgPool) -> Self {
        Self {
            shared: GameShared::new(pool),
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
        self.shared.rpc.clear_pending();
        self.shared.stop_all_session_actors();
    }

    async fn on_gateway_control_frame(&self, frame: BackendFrame) {
        self.shared.dispatch_control_frame(frame).await;
    }

    async fn on_business_frame(&self, frame: BackendFrame) {
        self.shared.dispatch_client_frame(frame);
    }
}
