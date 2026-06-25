use std::sync::Arc;

use async_trait::async_trait;
use protocol::message_map::MessageType;

use crate::server_source::ServerSource;
use crate::session::BackendSession;
use crate::session_delegate::SessionDelegate;

// ═══════════════════════════════════════════════════════════════
// BackendDelegate — 全局业务代理(服务间消息)
//
// 同步方法在读循环内联调用;异步方法由框架的独立任务调用。
// per-session(客户端)逻辑通过 create_session_delegate 工厂创建
// SessionDelegate,由框架管理其完整生命周期。
// ═══════════════════════════════════════════════════════════════

#[async_trait]
pub trait BackendDelegate: Send + Sync {
    /// 网关连接建立(同步;需异步初始化的业务自行 spawn)
    fn on_connected(&self) {}
    /// 网关连接断开(同步,重连前完成)
    fn on_disconnected(&self) {}

    /// 工厂:每个客户端 session online 时创建 per-session delegate。
    /// 框架管理其生命周期(on_start → 循环 on_client_* → on_stop)。
    fn create_session_delegate(
        &self,
        session_id: u32,
        session: Arc<BackendSession>,
    ) -> Box<dyn SessionDelegate>;

    /// 收到其他服务的 RPC 请求(msg 已由框架解码)。
    /// 返回 Ok(resp) 框架自动 send_server_msg 回包;Err 框架自动回 CommonErrorResp。
    async fn on_server_request(
        &self,
        _source: ServerSource,
        _msg: MessageType,
    ) -> anyhow::Result<MessageType> {
        Err(anyhow::anyhow!("no server request handler"))
    }

    /// 收到其他服务的单向推送(msg 已由框架解码)。返回 Err 框架只打日志。
    async fn on_server_push(&self, _source: ServerSource, _msg: MessageType) -> anyhow::Result<()> {
        Ok(())
    }

    /// 全服关闭入口(异步,框架已停止所有 session 和 server_dispatcher 后调用)。
    async fn on_shutdown(&self) {}
}

/// 供框架构造通用错误回包。`message` 为具体错误描述,格式化为 "internal error: {message}"。
pub(crate) fn common_error_response(message: impl std::fmt::Display) -> MessageType {
    MessageType::GameCommonErrorResp(protocol::game::CommonErrorResp {
        code: -1,
        message: format!("internal error: {message}"),
    })
}
