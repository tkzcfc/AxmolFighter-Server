use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use base::net::WriterMessage;

#[derive(Clone, Debug)]
pub struct ServiceLoad {
    // 0 最空闲，100 视为满载。
    pub load_score: u32,
    // false 时不参与自动选择。
    pub accepting_bindings: bool,
    pub message: String,
}

impl Default for ServiceLoad {
    fn default() -> Self {
        Self {
            load_score: 100,
            accepting_bindings: false,
            message: String::new(),
        }
    }
}

pub struct BackendConn {
    pub service_id: u8,
    pub instance_id: u32,
    pub session_id: u32,
    pub tx: mpsc::UnboundedSender<WriterMessage>,
    pub load: ServiceLoad,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterResult {
    Registered,
    AlreadyRegistered,
    SessionAlreadyRegistered,
    InstanceAlreadyRegistered,
}

#[derive(Clone)]
pub struct BindCandidate {
    pub instance_id: u32,
    pub tx: mpsc::UnboundedSender<WriterMessage>,
}

#[derive(Clone)]
pub struct ServerRegistry {
    conns: Arc<DashMap<u32, BackendConn>>,
    instances: Arc<DashMap<(u8, u32), u32>>,
}

impl ServerRegistry {
    pub fn new() -> Self {
        Self {
            conns: Arc::new(DashMap::new()),
            instances: Arc::new(DashMap::new()),
        }
    }

    pub fn register(
        &self,
        session_id: u32,
        service_id: u8,
        instance_id: u32,
        tx: mpsc::UnboundedSender<WriterMessage>,
    ) -> RegisterResult {
        if let Some(conn) = self.conns.get(&session_id) {
            if conn.service_id == service_id && conn.instance_id == instance_id {
                return RegisterResult::AlreadyRegistered;
            }
            return RegisterResult::SessionAlreadyRegistered;
        }

        let instance_key = (service_id, instance_id);
        if let Some(registered_session_id) = self.instances.get(&instance_key)
            && *registered_session_id != session_id
        {
            return RegisterResult::InstanceAlreadyRegistered;
        }

        self.instances.insert(instance_key, session_id);
        self.conns.insert(
            session_id,
            BackendConn {
                service_id,
                instance_id,
                session_id,
                tx,
                load: ServiceLoad::default(),
                latency_ms: None,
            },
        );
        RegisterResult::Registered
    }

    pub fn remove(&self, session_id: u32) -> Option<(u8, u32)> {
        self.conns.remove(&session_id).map(|(_, conn)| {
            self.instances.remove(&(conn.service_id, conn.instance_id));
            (conn.service_id, conn.instance_id)
        })
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

    pub fn select_lowest_load_instance<F>(
        &self,
        service_id: u8,
        binding_count: F,
    ) -> Option<BindCandidate>
    where
        F: Fn(u8, u32) -> usize,
    {
        let mut candidates: Vec<_> = self
            .conns
            .iter()
            .filter(|entry| entry.value().service_id == service_id)
            .filter(|entry| {
                let load = &entry.value().load;
                load.accepting_bindings && load.load_score < 100
            })
            .map(|entry| {
                let conn = entry.value();
                (
                    conn.load.load_score,
                    binding_count(conn.service_id, conn.instance_id),
                    conn.instance_id,
                    conn.tx.clone(),
                )
            })
            .collect();

        // 同分时优先绑定少、实例号小的服务。
        candidates.sort_by_key(|(load_score, binding_count, instance_id, _)| {
            (*load_score, *binding_count, *instance_id)
        });

        candidates
            .into_iter()
            .map(|(_, _, instance_id, tx)| BindCandidate { instance_id, tx })
            .next()
    }

    pub fn update_load(
        &self,
        session_id: u32,
        service_id: u8,
        instance_id: u32,
        load_score: u32,
        accepting_bindings: bool,
        message: String,
    ) -> bool {
        let Some(mut conn) = self.conns.get_mut(&session_id) else {
            return false;
        };

        if conn.service_id != service_id || conn.instance_id != instance_id {
            return false;
        }

        conn.load = ServiceLoad {
            load_score: load_score.min(100),
            accepting_bindings,
            message,
        };
        true
    }

    pub fn update_latency(&self, session_id: u32, latency_ms: u32) -> bool {
        let Some(mut conn) = self.conns.get_mut(&session_id) else {
            return false;
        };

        conn.latency_ms = Some(latency_ms);
        true
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

    pub fn list_all_except(&self, exclude_session_id: u32) -> Vec<(u8, u32)> {
        self.conns
            .iter()
            .filter(|entry| entry.value().session_id != exclude_session_id)
            .map(|entry| (entry.value().service_id, entry.value().instance_id))
            .collect()
    }

    pub fn service_statuses(&self) -> Vec<(u8, u32)> {
        self.list_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(registry: &ServerRegistry, session_id: u32, instance_id: u32) {
        let (tx, _rx) = mpsc::unbounded_channel();
        assert_eq!(
            registry.register(session_id, 1, instance_id, tx),
            RegisterResult::Registered
        );
    }

    fn register_with_result(
        registry: &ServerRegistry,
        session_id: u32,
        service_id: u8,
        instance_id: u32,
    ) -> RegisterResult {
        let (tx, _rx) = mpsc::unbounded_channel();
        registry.register(session_id, service_id, instance_id, tx)
    }

    #[test]
    fn select_prefers_lower_load_score() {
        let registry = ServerRegistry::new();
        register(&registry, 101, 1);
        register(&registry, 102, 2);
        registry.update_load(101, 1, 1, 80, true, String::new());
        registry.update_load(102, 1, 2, 20, true, String::new());

        let candidate = registry.select_lowest_load_instance(1, |_, _| 0);

        assert_eq!(candidate.map(|candidate| candidate.instance_id), Some(2));
    }

    #[test]
    fn select_filters_instances_that_do_not_accept_bindings() {
        let registry = ServerRegistry::new();
        register(&registry, 101, 1);
        register(&registry, 102, 2);
        registry.update_load(101, 1, 1, 1, false, String::new());
        registry.update_load(102, 1, 2, 90, true, String::new());

        let candidate = registry.select_lowest_load_instance(1, |_, _| 0);

        assert_eq!(candidate.map(|candidate| candidate.instance_id), Some(2));
    }

    #[test]
    fn select_filters_full_load_instances() {
        let registry = ServerRegistry::new();
        register(&registry, 101, 1);
        register(&registry, 102, 2);
        registry.update_load(101, 1, 1, 100, true, String::new());
        registry.update_load(102, 1, 2, 99, true, String::new());

        let candidate = registry.select_lowest_load_instance(1, |_, _| 0);

        assert_eq!(candidate.map(|candidate| candidate.instance_id), Some(2));
    }

    #[test]
    fn select_uses_default_full_load_for_instances_without_load_update() {
        let registry = ServerRegistry::new();
        register(&registry, 101, 1);
        register(&registry, 102, 2);
        registry.update_load(102, 1, 2, 50, true, String::new());

        let candidate = registry.select_lowest_load_instance(1, |_, _| 0);

        assert_eq!(candidate.map(|candidate| candidate.instance_id), Some(2));
    }

    #[test]
    fn register_is_idempotent_for_same_session_and_instance() {
        let registry = ServerRegistry::new();
        assert_eq!(
            register_with_result(&registry, 101, 1, 1),
            RegisterResult::Registered
        );
        assert_eq!(
            register_with_result(&registry, 101, 1, 1),
            RegisterResult::AlreadyRegistered
        );

        assert_eq!(registry.list_all(), vec![(1, 1)]);
    }

    #[test]
    fn register_rejects_same_session_with_different_instance() {
        let registry = ServerRegistry::new();
        assert_eq!(
            register_with_result(&registry, 101, 1, 1),
            RegisterResult::Registered
        );
        assert_eq!(
            register_with_result(&registry, 101, 1, 2),
            RegisterResult::SessionAlreadyRegistered
        );

        assert_eq!(registry.find_by_instance(1, 1).is_some(), true);
        assert_eq!(registry.find_by_instance(1, 2).is_none(), true);
    }

    #[test]
    fn register_rejects_same_instance_from_different_session() {
        let registry = ServerRegistry::new();
        assert_eq!(
            register_with_result(&registry, 101, 1, 1),
            RegisterResult::Registered
        );
        assert_eq!(
            register_with_result(&registry, 102, 1, 1),
            RegisterResult::InstanceAlreadyRegistered
        );

        assert_eq!(registry.list_all(), vec![(1, 1)]);
    }

    #[test]
    fn register_allows_instance_after_previous_session_is_removed() {
        let registry = ServerRegistry::new();
        assert_eq!(
            register_with_result(&registry, 101, 1, 1),
            RegisterResult::Registered
        );
        assert_eq!(registry.remove(101), Some((1, 1)));
        assert_eq!(
            register_with_result(&registry, 102, 1, 1),
            RegisterResult::Registered
        );

        assert_eq!(registry.list_all(), vec![(1, 1)]);
    }

    #[test]
    fn update_latency_updates_registered_session() {
        let registry = ServerRegistry::new();
        register(&registry, 101, 1);

        assert!(registry.update_latency(101, 42));
        assert_eq!(registry.conns.get(&101).unwrap().latency_ms, Some(42));
    }

    #[test]
    fn update_latency_rejects_missing_session() {
        let registry = ServerRegistry::new();

        assert!(!registry.update_latency(404, 42));
    }
}
