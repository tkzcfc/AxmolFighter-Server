use async_trait::async_trait;
use protocol::message_map::MessageType;

/// per-session 代理(框架管理生命周期,对标 base/net 的 SessionDelegate)。
///
/// 框架为每个客户端 session 创建一个实现了此 trait 的实例,
/// 在一个独立的 `tokio::spawn` 任务中按顺序调用:
///   on_start() → loop { on_client_request / on_client_push } → on_stop()
#[async_trait]
pub trait SessionDelegate: Send + Sync {
    /// session 创建后框架立即调用(异步,可做初始化)。
    async fn on_start(&self) {}

    /// 收到客户端 RPC 请求(已解码)。返回 Ok(resp) 框架自动 send_msg 回包;
    /// 返回 Err 框架自动回 CommonErrorResp。
    async fn on_client_request(&self, msg: MessageType) -> anyhow::Result<MessageType>;

    /// 收到客户端推送(已解码)。返回 Err 框架只打日志,不回包。
    async fn on_client_push(&self, msg: MessageType) -> anyhow::Result<()> {
        drop(msg);
        Ok(())
    }

    /// session 退出前框架调用(异步,可做清理)。
    async fn on_stop(&self) {}
}
