#![allow(dead_code)]

mod config;
mod error_code;
mod game_shared;
mod player;

use std::env;
use std::time::Duration;

use backend_framework::bootstrap::{BackendConfig, spawn_backend, wait_for_shutdown_signal};
use backend_framework::service_id::SERVICE_ID_GAME;
use sqlx::postgres::PgPoolOptions;
use tracing::info;

use crate::config::GameConfig;
use crate::game_shared::GameShared;

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
    info!("connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database.url)
        .await?;
    info!("database connected");

    // 运行数据库迁移
    info!("applying database migrations...");
    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("database migrations applied");

    // 启动后端服务:spawn_backend 内部创建 BackendSession,并通过闭包把它交给 GameShared
    let runtime = spawn_backend(
        BackendConfig {
            service_id: SERVICE_ID_GAME,
            instance_id: config.server.instance_id,
            gateway_addr: config.gateway.addr.clone(),
            reconnect_interval: Duration::from_secs(config.gateway.reconnect_interval),
        },
        move |session| GameShared::new(pool, session),
    );

    info!("game server started");

    // 等待关闭信号
    wait_for_shutdown_signal().await?;
    info!("shutting down...");

    runtime.shutdown().await;

    info!("game server stopped");
    Ok(())
}
