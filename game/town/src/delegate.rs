use std::sync::Arc;

use async_trait::async_trait;
use protocol::message_map::MessageType;
use tracing::info;

use backend_framework::bootstrap::ShutdownHandle;
use backend_framework::delegate::BackendDelegate;
use backend_framework::service_id::SERVICE_ID_GAME;
use backend_framework::session::BackendSession;
use backend_framework::session_delegate::SessionDelegate;

/// 城镇服业务代理（空壳，后续添加城镇逻辑）
pub struct TownDelegate {
    shutdown: ShutdownHandle,
}

impl TownDelegate {
    pub fn new(shutdown: ShutdownHandle) -> Self {
        Self { shutdown }
    }
}

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
        info!("town server disconnected from gateway, shutting down");
        self.shutdown.request_shutdown();
    }

    fn on_server_offline(&self, service_id: u32, instance_id: u32) {
        info!(
            "backend server offline service_id={} instance_id={}",
            service_id, instance_id
        );
        if service_id == SERVICE_ID_GAME {
            info!("game server offline, shutting down town server");
            self.shutdown.request_shutdown();
        }
    }

    fn create_session_delegate(
        &self,
        _session_id: u32,
        _session: Arc<BackendSession>,
    ) -> Box<dyn SessionDelegate> {
        Box::new(TownSessionDelegate)
    }
}
