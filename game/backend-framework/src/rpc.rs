use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use protocol::message_map::MessageType;
use tokio::sync::oneshot;

pub enum PendingResponse {
    Message(MessageType),
    GatewayError { code: u32, message: String },
    DecodeError { msg_id: u16, message: String },
}

#[derive(Debug)]
pub enum RpcError {
    Send(String),
    Cancelled,
    Timeout,
    Gateway { code: u32, message: String },
    Decode { msg_id: u16, message: String },
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(message) => write!(f, "request send failed: {message}"),
            Self::Cancelled => write!(f, "request cancelled"),
            Self::Timeout => write!(f, "request timeout"),
            Self::Gateway { code, message } => {
                write!(f, "gateway error code={code} message={message}")
            }
            Self::Decode { msg_id, message } => {
                write!(
                    f,
                    "decode response failed msg_id={msg_id} message={message}"
                )
            }
        }
    }
}

impl std::error::Error for RpcError {}

/// RPC pending 表管理。仅负责分配 request_id、登记/取消/完成 pending 请求,
/// 不负责发送(发送由 `BackendSession` 直接调用 `try_send_frame_msg` 完成),
/// 因此本结构不依赖任何发送句柄,也不再需要闭包。
pub struct RpcManager {
    pending_requests: DashMap<i32, oneshot::Sender<PendingResponse>>,
    serial_seed: AtomicI32,
}

impl Default for RpcManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcManager {
    pub fn new() -> Self {
        Self {
            pending_requests: DashMap::new(),
            serial_seed: AtomicI32::new(1),
        }
    }

    /// 分配一个唯一的正数 request_id(1..=i32::MAX)。
    /// CAS 循环保证并发下不重复;达到 i32::MAX 后回绕到 1
    /// (需 20 亿次 RPC 且与回绕后 id 碰撞的 pending 请求同时存在才会出问题,实际不可能)。
    pub fn next_request_id(&self) -> i32 {
        loop {
            let current = self.serial_seed.load(Ordering::Relaxed);
            let next = if current == i32::MAX { 1 } else { current + 1 };
            if self
                .serial_seed
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    pub fn insert_pending(&self, request_id: i32, tx: oneshot::Sender<PendingResponse>) {
        self.pending_requests.insert(request_id, tx);
    }

    pub fn remove_pending(&self, request_id: i32) {
        self.pending_requests.remove(&request_id);
    }

    async fn wait_pending_response(
        &self,
        request_id: i32,
        rx: oneshot::Receiver<PendingResponse>,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(PendingResponse::Message(msg))) => Ok(msg),
            Ok(Ok(PendingResponse::GatewayError { code, message })) => {
                Err(RpcError::Gateway { code, message })
            }
            Ok(Ok(PendingResponse::DecodeError { msg_id, message })) => {
                Err(RpcError::Decode { msg_id, message })
            }
            Ok(Err(_)) => Err(RpcError::Cancelled),
            Err(_) => {
                self.pending_requests.remove(&request_id);
                Err(RpcError::Timeout)
            }
        }
    }

    /// 由 `BackendSession` 的请求方法调用:登记 pending 后等待回复。
    pub async fn wait_response(
        &self,
        request_id: i32,
        rx: oneshot::Receiver<PendingResponse>,
        timeout: Duration,
    ) -> Result<MessageType, RpcError> {
        self.wait_pending_response(request_id, rx, timeout).await
    }

    /// 尝试根据 serial 匹配并完成一个 pending 请求。
    /// 返回 `true` 表示成功匹配并投递了回复。
    pub fn resolve_pending_response(
        &self,
        serial: i32,
        msg_id: u16,
        payload: bytes::Bytes,
    ) -> bool {
        if serial <= 0 {
            return false;
        }

        if let Some((_, tx)) = self.pending_requests.remove(&serial) {
            let response = match protocol::message_map::decode_message(msg_id as u32, &payload) {
                Ok(MessageType::GatewayGatewayErrorResp(resp)) => PendingResponse::GatewayError {
                    code: resp.code,
                    message: resp.message,
                },
                Ok(msg) => PendingResponse::Message(msg),
                Err(err) => PendingResponse::DecodeError {
                    msg_id,
                    message: err.to_string(),
                },
            };
            let _ = tx.send(response);
            true
        } else {
            false
        }
    }

    /// 清空所有未完成的请求(网关断连时调用)。
    pub fn clear_pending(&self) {
        self.pending_requests.clear();
    }
}
