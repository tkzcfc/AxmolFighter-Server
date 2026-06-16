#![allow(dead_code)]

mod codec;
mod config;
mod gateway_client;
mod handler;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use tokio::signal;
use tokio::sync::Notify;
use tracing::info;

use crate::config::GameConfig;
use crate::gateway_client::GatewayClient;
use crate::handler::GameHandler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 加载配置
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "game.toml".to_string());

    let config = GameConfig::load(&config_path)?;
    info!("config loaded from {}", config_path);
    info!(
        "instance_id={}, gateway={}",
        config.server.instance_id, config.gateway.addr
    );

    let shutdown = Arc::new(Notify::new());

    // 创建消息处理器
    let handler = GameHandler::new();

    // 启动网关连接
    let gw_client = GatewayClient::new(
        config.gateway.addr.clone(),
        config.server.instance_id,
        Duration::from_secs(config.gateway.reconnect_interval),
        shutdown.clone(),
    );

    let gw_handler = handler.clone();
    let gw_task = tokio::spawn(async move {
        gw_client.run(gw_handler).await;
    });

    info!("game server started");

    // 等待关闭信号
    signal::ctrl_c().await?;
    info!("shutting down...");

    shutdown.notify_waiters();

    let _ = gw_task.await;

    info!("game server stopped");
    Ok(())
}
