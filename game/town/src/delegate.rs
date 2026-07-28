use std::sync::Arc;

use async_trait::async_trait;
use backend_framework::bootstrap::ShutdownHandle;
use backend_framework::delegate::BackendDelegate;
use backend_framework::server_source::ServerSource;
use backend_framework::service_id::SERVICE_ID_GAME;
use backend_framework::session::BackendSession;
use backend_framework::session_delegate::SessionDelegate;
use protocol::message_map::MessageType;
use tracing::{debug, info, warn};

use crate::scene::TownShared;

pub struct TownDelegate {
    shutdown: ShutdownHandle,
    shared: Arc<TownShared>,
}

impl TownDelegate {
    pub fn new(session: Arc<BackendSession>, shutdown: ShutdownHandle, instance_id: u32) -> Self {
        Self {
            shutdown,
            shared: TownShared::new(session, instance_id),
        }
    }
}

struct TownSessionDelegate {
    session_id: u32,
    shared: Arc<TownShared>,
}

impl TownSessionDelegate {
    fn new(session_id: u32, shared: Arc<TownShared>) -> Self {
        Self { session_id, shared }
    }
}

#[async_trait]
impl SessionDelegate for TownSessionDelegate {
    async fn on_client_request(&self, msg: MessageType) -> anyhow::Result<MessageType> {
        warn!(
            "town has no client request handler for session={}",
            self.session_id
        );
        drop(msg);
        Err(anyhow::anyhow!("no handler"))
    }

    async fn on_client_push(&self, msg: MessageType) -> anyhow::Result<()> {
        match msg {
            MessageType::TownTownPlayerStatePush(push) => {
                self.shared.update_state(self.session_id, push);
            }
            other => {
                debug!("unhandled town client push session={}", self.session_id);
                drop(other);
            }
        }
        Ok(())
    }

    async fn on_stop(&self) {
        self.shared.leave_session(self.session_id);
    }
}

#[async_trait]
impl BackendDelegate for TownDelegate {
    fn on_connected(&self) {
        info!("town server connected to gateway");
    }

    fn on_disconnected(&self) {
        info!("town server disconnected from gateway, shutting down");
        self.shared.clear();
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
        session_id: u32,
        _session: Arc<BackendSession>,
    ) -> Box<dyn SessionDelegate> {
        Box::new(TownSessionDelegate::new(session_id, self.shared.clone()))
    }

    async fn on_server_request(
        &self,
        source: ServerSource,
        msg: MessageType,
    ) -> anyhow::Result<MessageType> {
        if source.service_id != SERVICE_ID_GAME {
            warn!(
                "rejected town server request from non-game source={}",
                source
            );
            return Err(anyhow::anyhow!("town request must come from game server"));
        }

        let resp = match msg {
            MessageType::TownInternalTownEnterSceneReq(req) => self.shared.enter_scene(req).into(),
            MessageType::TownInternalTownLeaveSceneReq(req) => self.shared.leave_scene(req).into(),
            other => {
                warn!("unhandled town server request source={}", source);
                drop(other);
                return Err(anyhow::anyhow!("no handler"));
            }
        };
        Ok(resp)
    }

    async fn on_shutdown(&self) {
        self.shared.clear();
    }
}
