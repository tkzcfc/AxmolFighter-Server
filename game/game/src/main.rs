#![allow(dead_code)]

mod codec;
mod config;
mod gateway_client;
mod handler;

use std::env;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::GameConfig;
use crate::gateway_client::GatewayClient;
use crate::handler::GameHandler;

async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = signal(SignalKind::terminate())?;

        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("received Ctrl+C");
            }
            _ = sigterm.recv() => {
                info!("received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c().await?;
        info!("received Ctrl+C");
    }

    Ok(())
}

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

    // 连接数据库
    let pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect(&config.database.url)
        .await?;
    info!("database connected");

    // 运行数据库迁移
    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("database migrations applied");

    let shutdown_token = CancellationToken::new();

    // 创建消息处理器
    let handler = GameHandler::new(pool);

    // 启动网关连接
    let gw_client = GatewayClient::new(
        config.gateway.addr.clone(),
        config.server.instance_id,
        Duration::from_secs(config.gateway.reconnect_interval),
        shutdown_token.clone(),
    );

    let gw_handler = handler.clone();
    let gw_task = tokio::spawn(async move {
        gw_client.run(gw_handler).await;
    });

    info!("game server started");

    // 等待关闭信号
    wait_for_shutdown_signal().await?;
    info!("shutting down...");

    shutdown_token.cancel();

    let _ = gw_task.await;

    info!("game server stopped");
    Ok(())
}
