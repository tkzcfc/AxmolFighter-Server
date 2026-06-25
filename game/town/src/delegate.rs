use std::sync::Arc;

use async_trait::async_trait;
use protocol::message_map::MessageType;
use tracing::info;

use backend_framework::delegate::BackendDelegate;
use backend_framework::session::BackendSession;
use backend_framework::session_delegate::SessionDelegate;

/// 城镇服业务代理（空壳，后续添加城镇逻辑）
pub struct TownDelegate;

struct TownSessionDelegate;

#[async_trait]
impl SessionDelegate for TownSessionDelegate {
    async fn on_client_request(&self, msg: MessageType) -> anyhow::Result<MessageType> {
        info!("town client request");
        drop(msg);
        Err(anyhow::anyhow!("no handler"))
    }

    async fn on_client_push(&self, msg: MessageType) -> anyhow::Result<()> {
        info!("town client push");
        drop(msg);
        Ok(())
    }
}

#[async_trait]
impl BackendDelegate for TownDelegate {
    fn on_connected(&self) {
        info!("town server connected to gateway");
    }

    fn on_disconnected(&self) {
        info!("town server disconnected from gateway");
    }

    fn create_session_delegate(
        &self,
        _session_id: u32,
        _session: Arc<BackendSession>,
    ) -> Box<dyn SessionDelegate> {
        Box::new(TownSessionDelegate)
    }
}
