use protocol::game::CommonErrorResp;
use protocol::message_map::MessageType;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Ok = 0,
    InternalError,
    ServerBusy,
    DecodeMessageFailed,
}

impl ErrorCode {
    pub const fn code(self) -> i32 {
        self as i32
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Ok => "",
            Self::InternalError => "服务器内部错误",
            Self::ServerBusy => "服务器繁忙",
            Self::DecodeMessageFailed => "消息解码失败",
        }
    }

    pub fn to_common_error_message(self) -> MessageType {
        MessageType::GameCommonErrorResp(CommonErrorResp {
            code: self.code(),
            message: self.message().to_string(),
        })
    }
}
