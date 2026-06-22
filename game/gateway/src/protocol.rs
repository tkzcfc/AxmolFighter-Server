// 普通业务消息，msg_id 是业务 PB 协议号，payload 是业务 PB 数据。
pub const CMD_BUSINESS: u8 = 0;
// 网关返回给客户端的错误响应，msg_id 是 GatewayErrorResp 的 PB 协议号。
pub const CMD_GATEWAY_ERROR: u8 = 1;
// 网关内部控制消息，msg_id 是 gateway.proto 中的 PB 协议号。
pub const CMD_GATEWAY_CONTROL: u8 = 2;
