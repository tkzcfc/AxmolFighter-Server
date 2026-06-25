mod delegate;

use std::sync::Arc;
use std::time::Duration;

use backend_framework::bootstrap::{BackendConfig, spawn_backend, wait_for_shutdown_signal};
use backend_framework::delegate::BackendDelegate;
use backend_framework::service_id::SERVICE_ID_TOWN;
use tracing::info;

use crate::delegate::TownDelegate;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let gateway_addr = "127.0.0.1:7100".to_string();
    let instance_id = 1u32;
    let reconnect_interval = Duration::from_secs(3);

    info!(
        "town server starting, gateway={}, service_id={}, instance_id={}",
        gateway_addr, SERVICE_ID_TOWN, instance_id
    );

    let runtime = spawn_backend(
        BackendConfig {
            service_id: SERVICE_ID_TOWN,
            instance_id,
            gateway_addr,
            reconnect_interval,
        },
        move |_| Arc::new(TownDelegate) as Arc<dyn BackendDelegate>,
    );

    info!("town server started");
    wait_for_shutdown_signal().await?;
    info!("shutting down...");
    runtime.shutdown().await;
    info!("town server stopped");
    Ok(())
}
