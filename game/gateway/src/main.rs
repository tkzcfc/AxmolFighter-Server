#![allow(dead_code)]

mod client_listener;
mod codec;
mod config;
mod context;
mod internal_listener;
mod protocol;
mod router;
mod server_registry;
mod session;

use std::env;
use std::sync::Arc;

use tokio::signal;
use tokio::sync::Notify;
use tracing::info;

use base::net::tcp_server;

use crate::client_listener::ClientDelegate;
use crate::config::GatewayConfig;
use crate::context::GatewayContext;
use crate::internal_listener::InternalDelegate;

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
        .unwrap_or_else(|| "gateway.toml".to_string());

    let config = GatewayConfig::load(&config_path)?;
    info!("config loaded from {}", config_path);
    info!(
        "client_listen={}, internal_listen={}",
        config.gateway.client_listen, config.gateway.internal_listen
    );

    let ctx = GatewayContext::new(config);

    // 关闭通知
    let shutdown_notify = Arc::new(Notify::new());

    // 启动客户端监听
    let client_ctx = ctx.clone();
    let client_addr = ctx.config.gateway.client_listen.clone();
    let client_shutdown = shutdown_notify.clone();
    let client_server = tokio::spawn(async move {
        let builder = tcp_server::Builder::new(Box::new(move || {
            Box::new(ClientDelegate::new(client_ctx.clone()))
        }));
        if let Err(e) = builder.build(&client_addr, client_shutdown.notified()).await {
            tracing::error!("client listener error: {}", e);
        }
    });

    // 启动内部监听（后端注册）
    let internal_ctx = ctx.clone();
    let internal_addr = ctx.config.gateway.internal_listen.clone();
    let internal_shutdown = shutdown_notify.clone();
    let internal_server = tokio::spawn(async move {
        let builder = tcp_server::Builder::new(Box::new(move || {
            Box::new(InternalDelegate::new(internal_ctx.clone()))
        }));
        if let Err(e) = builder
            .build(&internal_addr, internal_shutdown.notified())
            .await
        {
            tracing::error!("internal listener error: {}", e);
        }
    });

    info!("gateway started");

    // 等待关闭信号
    signal::ctrl_c().await?;
    info!("shutting down...");

    // 通知所有 listener 停止接受新连接
    shutdown_notify.notify_waiters();

    // 等待服务器任务结束
    let _ = tokio::join!(client_server, internal_server);

    info!("gateway stopped");
    Ok(())
}
