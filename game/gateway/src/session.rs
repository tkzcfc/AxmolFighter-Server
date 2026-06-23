use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use base::net::WriterMessage;

/// 客户端会话信息
pub struct ClientSession {
    /// 向该客户端发送数据的通道
    pub tx: mpsc::UnboundedSender<WriterMessage>,
}

/// 会话管理器
#[derive(Clone)]
pub struct SessionManager {
    /// session_id → ClientSession
    sessions: Arc<DashMap<u32, ClientSession>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }

    /// 注册客户端会话
    pub fn add(&self, session_id: u32, tx: mpsc::UnboundedSender<WriterMessage>) {
        self.sessions.insert(session_id, ClientSession { tx });
    }

    /// 移除客户端会话
    pub fn remove(&self, session_id: u32) {
        self.sessions.remove(&session_id);
    }

    /// 向指定客户端发送数据
    pub fn send_to_client(&self, session_id: u32, data: Bytes) -> bool {
        if let Some(session) = self.sessions.get(&session_id) {
            session.tx.send(WriterMessage::Send(data, true)).is_ok()
        } else {
            false
        }
    }

    /// 向所有客户端广播
    pub fn broadcast_to_clients(&self, data: Bytes) {
        for entry in self.sessions.iter() {
            let _ = entry
                .value()
                .tx
                .send(WriterMessage::Send(data.clone(), true));
        }
    }

    /// 踢出客户端（关闭连接）
    pub fn kick(&self, session_id: u32) {
        if let Some((_, session)) = self.sessions.remove(&session_id) {
            let _ = session.tx.send(WriterMessage::Close);
        }
    }

    /// 获取所有在线 session_id 列表
    pub fn online_sessions(&self) -> Vec<u32> {
        self.sessions.iter().map(|entry| *entry.key()).collect()
    }
}
