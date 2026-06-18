use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use base::net::WriterMessage;

pub struct BackendConn {
    pub service_id: u8,
    pub instance_id: u32,
    pub session_id: u32,
    pub tx: mpsc::UnboundedSender<WriterMessage>,
}

#[derive(Clone)]
pub struct ServerRegistry {
    conns: Arc<DashMap<u32, BackendConn>>,
}

impl ServerRegistry {
    pub fn new() -> Self {
        Self {
            conns: Arc::new(DashMap::new()),
        }
    }

    pub fn register(
        &self,
        session_id: u32,
        service_id: u8,
        instance_id: u32,
        tx: mpsc::UnboundedSender<WriterMessage>,
    ) {
        self.conns.insert(
            session_id,
            BackendConn {
                service_id,
                instance_id,
                session_id,
                tx,
            },
        );
    }

    pub fn remove(&self, session_id: u32) -> Option<(u8, u32)> {
        self.conns
            .remove(&session_id)
            .map(|(_, conn)| (conn.service_id, conn.instance_id))
    }

    pub fn find_by_service(&self, service_id: u8) -> Option<mpsc::UnboundedSender<WriterMessage>> {
        self.conns
            .iter()
            .find(|entry| entry.value().service_id == service_id)
            .map(|entry| entry.value().tx.clone())
    }

    pub fn find_by_instance(
        &self,
        service_id: u8,
        instance_id: u32,
    ) -> Option<mpsc::UnboundedSender<WriterMessage>> {
        self.conns
            .iter()
            .find(|entry| {
                entry.value().service_id == service_id && entry.value().instance_id == instance_id
            })
            .map(|entry| entry.value().tx.clone())
    }

    pub fn least_loaded_instance<F>(
        &self,
        service_id: u8,
        binding_count: F,
    ) -> Option<(u32, mpsc::UnboundedSender<WriterMessage>)>
    where
        F: Fn(u8, u32) -> usize,
    {
        self.conns
            .iter()
            .filter(|entry| entry.value().service_id == service_id)
            .map(|entry| {
                let conn = entry.value();
                (
                    binding_count(conn.service_id, conn.instance_id),
                    conn.instance_id,
                    conn.tx.clone(),
                )
            })
            .min_by_key(|(count, instance_id, _)| (*count, *instance_id))
            .map(|(_, instance_id, tx)| (instance_id, tx))
    }

    pub fn broadcast(&self, data: Bytes) {
        for entry in self.conns.iter() {
            let _ = entry
                .value()
                .tx
                .send(WriterMessage::Send(data.clone(), true));
        }
    }

    pub fn broadcast_except(&self, exclude_session_id: u32, data: Bytes) {
        for entry in self.conns.iter() {
            if entry.value().session_id != exclude_session_id {
                let _ = entry
                    .value()
                    .tx
                    .send(WriterMessage::Send(data.clone(), true));
            }
        }
    }

    pub fn list_all(&self) -> Vec<(u8, u32)> {
        self.conns
            .iter()
            .map(|entry| (entry.value().service_id, entry.value().instance_id))
            .collect()
    }

    pub fn service_statuses(&self) -> Vec<(u8, u32)> {
        self.list_all()
    }
}
