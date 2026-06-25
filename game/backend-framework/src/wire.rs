// cmd 只描述帧类别；具体网关控制消息类型由 msg_id 对应的 PB 协议号决定。
pub const CMD_BUSINESS: u8 = 0;
pub const CMD_GATEWAY_CONTROL: u8 = 2;
