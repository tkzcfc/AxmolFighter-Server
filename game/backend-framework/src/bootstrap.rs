use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::delegate::BackendDelegate;
use crate::gateway_client::GatewayClient;
use crate::session::BackendSession;

/// 后端服务启动配置。
pub struct BackendConfig {
    pub service_id: u32,
    pub instance_id: u32,
    pub gateway_addr: String,
    pub reconnect_interval: Duration,
}

/// 启动后的运行时句柄。
pub struct BackendRuntime {
    pub session: Arc<BackendSession>,
    gw_task: JoinHandle<()>,
    server_dispatch_task: JoinHandle<()>,
    shutdown: CancellationToken,
}

impl BackendRuntime {
    /// 优雅关闭:先等所有 session 退出 + delegate.on_shutdown,再取消网关和分发任务。
    pub async fn shutdown(self) {
        self.session.shutdown().await;
        self.shutdown.cancel();
        let _ = self.gw_task.await;
        let _ = self.server_dispatch_task.await;
    }
}

pub fn spawn_backend<F>(config: BackendConfig, delegate_factory: F) -> BackendRuntime
where
    F: FnOnce(Arc<BackendSession>) -> Arc<dyn BackendDelegate>,
{
    let shutdown = CancellationToken::new();
    let session = BackendSession::new(config.service_id, config.instance_id);
    let delegate = delegate_factory(session.clone());
    session.set_delegate(delegate);

    // 服务间消息分发(对标 net_session::run)
    let server_dispatch_task = session.start_server_dispatcher(shutdown.clone());

    let gw_client = GatewayClient::new(
        config.gateway_addr,
        config.service_id,
        config.instance_id,
        config.reconnect_interval,
        shutdown.clone(),
    );

    let session_for_task = session.clone();
    let gw_task = tokio::spawn(async move {
        gw_client.run(session_for_task).await;
    });

    BackendRuntime {
        session,
        gw_task,
        server_dispatch_task,
        shutdown,
    }
}

/// 等待关闭信号(Ctrl+C 或 Unix 下的 SIGTERM)。
pub async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { info!("received Ctrl+C"); }
            _ = sigterm.recv() => { info!("received SIGTERM"); }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        info!("received Ctrl+C");
    }
    Ok(())
}
