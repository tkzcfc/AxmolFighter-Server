mod config;
mod delegate;
mod scene;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use backend_framework::bootstrap::{BackendConfig, spawn_backend, wait_for_shutdown_signal};
use backend_framework::delegate::BackendDelegate;
use backend_framework::service_id::SERVICE_ID_TOWN;
use tracing::info;

use crate::config::TownConfig;
use crate::delegate::TownDelegate;
use crate::scene::SyncConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "town.toml".to_string());
    let config = TownConfig::load_or_default(&config_path)?;

    let gateway_addr = config.gateway.addr.clone();
    let instance_id = config.server.instance_id;
    let reconnect_interval = Duration::from_secs(config.gateway.reconnect_interval);

    info!(
        "town server starting, gateway={}, service_id={}, instance_id={}",
        gateway_addr, SERVICE_ID_TOWN, instance_id
    );

    let sync = SyncConfig {
        max_speed: config.sync.max_speed,
        tolerance: config.sync.tolerance,
        burst_seconds: config.sync.burst_seconds,
        tick_ms: config.sync.tick_ms,
        aoi_radius: config.sync.aoi_radius,
        bounds: None,
    };

    let runtime = spawn_backend(
        BackendConfig {
            service_id: SERVICE_ID_TOWN,
            instance_id,
            gateway_addr,
            reconnect_interval,
        },
        move |session, shutdown| {
            Arc::new(TownDelegate::new(session, shutdown, instance_id, sync))
                as Arc<dyn BackendDelegate>
        },
    );

    info!("town server started");
    let shutdown_token = runtime.shutdown_token();
    tokio::select! {
        result = wait_for_shutdown_signal() => {
            result?;
        }
        _ = shutdown_token.cancelled() => {
            info!("town server shutdown requested");
        }
    }
    info!("shutting down...");
    runtime.shutdown().await;
    info!("town server stopped");
    Ok(())
}
