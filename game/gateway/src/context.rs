use std::sync::Arc;

use crate::bind_manager::BindManager;
use crate::config::GatewayConfig;
use crate::router::Router;
use crate::server_registry::ServerRegistry;
use crate::session::SessionManager;

/// 网关共享上下文，所有组件通过 Arc 共享
#[derive(Clone)]
pub struct GatewayContext {
    pub config: Arc<GatewayConfig>,
    pub sessions: SessionManager,
    pub registry: ServerRegistry,
    pub router: Router,
    pub binds: BindManager,
}

impl GatewayContext {
    pub fn new(config: GatewayConfig) -> Self {
        let router = Router::from_config(&config);
        Self {
            config: Arc::new(config),
            sessions: SessionManager::new(),
            registry: ServerRegistry::new(),
            router,
            binds: BindManager::new(),
        }
    }
}
