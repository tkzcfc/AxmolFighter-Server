use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use base::net::WriterMessage;

use crate::protocol::ServiceType;

/// 已注册的后端服务连接
pub struct BackendConn {
    pub service_type: ServiceType,
    pub instance_id: u32,
    /// 网关为该后端分配的 session_id（internal listener 分配）
    pub session_id: u32,
    /// 向该后端发送数据的通道
    pub tx: mpsc::UnboundedSender<WriterMessage>,
}

/// 后端服务注册表
#[derive(Clone)]
pub struct ServerRegistry {
    /// 按 internal session_id 索引（方便断线时查找）
    conns: Arc<DashMap<u32, BackendConn>>,
}

impl ServerRegistry {
    pub fn new() -> Self {
        Self {
            conns: Arc::new(DashMap::new()),
        }
    }

    /// 注册后端服务
    pub fn register(
        &self,
        session_id: u32,
        service_type: ServiceType,
        instance_id: u32,
        tx: mpsc::UnboundedSender<WriterMessage>,
    ) {
        self.conns.insert(
            session_id,
            BackendConn {
                service_type,
                instance_id,
                session_id,
                tx,
            },
        );
    }

    /// 移除后端服务（按 internal session_id）
    pub fn remove(&self, session_id: u32) -> Option<(ServiceType, u32)> {
        self.conns
            .remove(&session_id)
            .map(|(_, conn)| (conn.service_type, conn.instance_id))
    }

    /// 查找指定类型的第一个后端（游戏服单实例场景）
    pub fn find_by_type(&self, service_type: ServiceType) -> Option<mpsc::UnboundedSender<WriterMessage>> {
        for entry in self.conns.iter() {
            if entry.value().service_type == service_type {
                return Some(entry.value().tx.clone());
            }
        }
        None
    }

    /// 查找指定类型+实例ID的后端
    pub fn find_by_instance(
        &self,
        service_type: ServiceType,
        instance_id: u32,
    ) -> Option<mpsc::UnboundedSender<WriterMessage>> {
        for entry in self.conns.iter() {
            if entry.value().service_type == service_type && entry.value().instance_id == instance_id
            {
                return Some(entry.value().tx.clone());
            }
        }
        None
    }

    /// 向所有已注册后端广播
    pub fn broadcast(&self, data: Bytes) {
        for entry in self.conns.iter() {
            let _ = entry.value().tx.send(WriterMessage::Send(data.clone(), true));
        }
    }

    /// 向除了指定 session_id 之外的所有后端广播
    pub fn broadcast_except(&self, exclude_session_id: u32, data: Bytes) {
        for entry in self.conns.iter() {
            if entry.value().session_id != exclude_session_id {
                let _ = entry.value().tx.send(WriterMessage::Send(data.clone(), true));
            }
        }
    }

    /// 获取已注册的所有服务列表 [(service_type_u8, instance_id), ...]
    pub fn list_all(&self) -> Vec<(u8, u32)> {
        self.conns
            .iter()
            .map(|entry| (entry.value().service_type as u8, entry.value().instance_id))
            .collect()
    }

    /// 检查指定服务类型是否有在线实例
    pub fn has_online(&self, service_type: ServiceType) -> bool {
        self.conns.iter().any(|e| e.value().service_type == service_type)
    }
}
