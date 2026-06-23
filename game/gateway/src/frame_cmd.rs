// 普通业务消息，msg_id 对应业务协议。
pub const CMD_BUSINESS: u8 = 0;
// 网关发给客户端的错误响应。
pub const CMD_GATEWAY_ERROR: u8 = 1;
// 网关控制消息，由 msg_id 区分具体协议。
pub const CMD_GATEWAY_CONTROL: u8 = 2;
