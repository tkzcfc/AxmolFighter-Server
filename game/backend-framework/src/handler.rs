use crate::codec::BackendFrame;
use crate::gateway_client::GatewaySender;

/// GatewayClient 与 BackendSession 之间的内部桥接(全部同步、非阻塞)。
///
/// 读循环调用这些方法时保持轻量 —— 它们只做路由:
/// - `on_gateway_control_frame` 内联处理轻量控制帧(会话上下线/ping/RPC 响应),
///   并把服务间请求/推送同步投递给 delegate 路由到目标 actor;
/// - `on_business_frame` 内联做 RPC 响应匹配 + 投递给 delegate 的 `on_client_message`。
///
/// 所有重活(decode 业务消息、DB、服务间回包)由业务侧 per-session actor 异步处理。
pub(crate) trait MessageHandler: Send + Sync {
    fn on_gateway_connected(&self, tx: GatewaySender);

    fn on_gateway_disconnected(&self);

    fn on_gateway_control_frame(&self, frame: BackendFrame);

    fn on_business_frame(&self, frame: BackendFrame);
}
