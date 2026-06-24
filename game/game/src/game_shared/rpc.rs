use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use protocol::gateway::ForwardToServerReq;
use protocol::message_map::MessageType;
use tokio::sync::oneshot;

use crate::wire::{CMD_BUSINESS, CMD_GATEWAY_CONTROL};

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

pub struct RpcManager {
    pending_requests: DashMap<i32, oneshot::Sender<PendingResponse>>,
    control_serial_seed: AtomicI32,
}

impl RpcManager {
    pub fn new() -> Self {
        Self {
            pending_requests: DashMap::new(),
            control_serial_seed: AtomicI32::new(1),
        }
    }

    fn next_request_id(&self) -> i32 {
        let request_id = self.control_serial_seed.fetch_add(1, Ordering::Relaxed);
        if request_id <= 0 {
            self.control_serial_seed.store(1, Ordering::Relaxed);
            1
        } else {
            request_id
        }
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

    /// 向网关发送 RPC 请求并等待回复。
    pub async fn request_gateway<F>(
        &self,
        try_send: &F,
        msg: MessageType,
        session_id: u32,
        timeout: Duration,
    ) -> Result<MessageType, RpcError>
    where
        F: Fn(u8, &MessageType, i32, u32) -> anyhow::Result<()> + Sync,
    {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id, tx);

        if let Err(err) = try_send(CMD_GATEWAY_CONTROL, &msg, -request_id, session_id) {
            self.pending_requests.remove(&request_id);
            return Err(RpcError::Send(err.to_string()));
        }

        self.wait_pending_response(request_id, rx, timeout).await
    }

    /// 向其他服务发送 RPC 请求并等待回复。
    pub async fn request_server<F>(
        &self,
        try_send: &F,
        target_service_id: u32,
        target_instance_id: i32,
        session_id: u32,
        msg: MessageType,
        timeout: Duration,
    ) -> Result<MessageType, RpcError>
    where
        F: Fn(u8, &MessageType, i32, u32) -> anyhow::Result<()> + Sync,
    {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id, tx);

        let send_result = (|| -> anyhow::Result<()> {
            let (msg_id, payload) = protocol::message_map::encode_message(&msg)
                .ok_or_else(|| anyhow::anyhow!("failed to encode inner message"))?;
            let frame = crate::codec::encode_frame(
                CMD_BUSINESS,
                msg_id as u16,
                -request_id,
                session_id,
                &payload,
            );
            let forward = MessageType::GatewayForwardToServerReq(ForwardToServerReq {
                target_service_id,
                target_instance_id,
                payload: frame.to_vec(),
                source_service_id: 0,
                source_instance_id: 0,
            });
            try_send(CMD_GATEWAY_CONTROL, &forward, -request_id, session_id)
        })();

        if let Err(err) = send_result {
            self.pending_requests.remove(&request_id);
            return Err(RpcError::Send(err.to_string()));
        }

        self.wait_pending_response(request_id, rx, timeout).await
    }

    /// 尝试根据 serial 匹配并完成一个 pending 请求。
    /// 返回 `true` 表示成功匹配并投递了回复。
    pub fn resolve_pending_response(&self, serial: i32, msg_id: u16, payload: Bytes) -> bool {
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

    /// 清空所有未完成的请求（网关断连时调用）。
    pub fn clear_pending(&self) {
        self.pending_requests.clear();
    }
}
