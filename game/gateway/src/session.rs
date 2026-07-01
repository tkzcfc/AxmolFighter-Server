use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use base::net::WriterMessage;

/// 客户端会话信息。
pub struct ClientSession {
    /// 发往客户端的写通道。
    pub tx: mpsc::UnboundedSender<WriterMessage>,
    /// 客户端是否已经通过 ServerStatusReq 完成网关认证。
    pub authenticated: bool,
}

/// 客户端会话管理器。
#[derive(Clone)]
pub struct SessionManager {
    /// session_id 到客户端会话。
    sessions: Arc<DashMap<u32, ClientSession>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }

    /// 登记客户端连接。
    pub fn add(&self, session_id: u32, tx: mpsc::UnboundedSender<WriterMessage>) {
        self.sessions.insert(
            session_id,
            ClientSession {
                tx,
                authenticated: false,
            },
        );
    }

    /// 移除客户端连接，并返回它之前是否已认证。
    pub fn remove(&self, session_id: u32) -> bool {
        self.sessions
            .remove(&session_id)
            .map(|(_, session)| session.authenticated)
            .unwrap_or(false)
    }

    /// 标记客户端已认证，返回这次是否为首次认证。
    pub fn authenticate(&self, session_id: u32) -> bool {
        let Some(mut session) = self.sessions.get_mut(&session_id) else {
            return false;
        };

        if session.authenticated {
            return false;
        }

        session.authenticated = true;
        true
    }

    /// 查询客户端是否已认证。
    pub fn is_authenticated(&self, session_id: u32) -> bool {
        self.sessions
            .get(&session_id)
            .map(|session| session.authenticated)
            .unwrap_or(false)
    }

    /// 向指定客户端发送数据。
    pub fn send_to_client(&self, session_id: u32, data: Bytes) -> bool {
        if let Some(session) = self.sessions.get(&session_id) {
            session.tx.send(WriterMessage::Send(data, true)).is_ok()
        } else {
            false
        }
    }

    /// 踢掉客户端，关闭连接。
    pub fn kick(&self, session_id: u32) {
        if let Some(session) = self.sessions.get(&session_id) {
            let _ = session.tx.send(WriterMessage::Close);
        }
    }

    /// 获取所有已认证客户端 session_id。
    pub fn authenticated_sessions(&self) -> Vec<u32> {
        self.sessions
            .iter()
            .filter(|entry| entry.value().authenticated)
            .map(|entry| *entry.key())
            .collect()
    }
}
