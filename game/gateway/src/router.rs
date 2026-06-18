use dashmap::DashMap;
use std::sync::Arc;

use crate::config::GatewayConfig;

#[derive(Debug, Clone)]
struct RouteRule {
    pub min: u16,
    pub max: u16,
    pub service_id: u8,
    pub require_binding: bool,
}

#[derive(Clone)]
pub struct Router {
    rules: Arc<Vec<RouteRule>>,
    bindings: Arc<DashMap<(u32, u8), u32>>,
}

#[derive(Debug, Clone)]
pub enum RouteTarget {
    Service(u8),
    BoundService { service_id: u8, instance_id: u32 },
    ServiceNotBound(u8),
    Unknown,
}

impl Router {
    pub fn from_config(config: &GatewayConfig) -> Self {
        let rules = config
            .route
            .iter()
            .map(|entry| RouteRule {
                min: entry.range[0],
                max: entry.range[1],
                service_id: entry.service_id,
                require_binding: entry.require_binding,
            })
            .collect();

        Self {
            rules: Arc::new(rules),
            bindings: Arc::new(DashMap::new()),
        }
    }

    pub fn resolve(&self, msg_id: u16, session_id: u32) -> RouteTarget {
        let Some(rule) = self
            .rules
            .iter()
            .find(|rule| msg_id >= rule.min && msg_id <= rule.max)
        else {
            return RouteTarget::Unknown;
        };

        if !rule.require_binding {
            return RouteTarget::Service(rule.service_id);
        }

        if let Some(instance_id) = self.bindings.get(&(session_id, rule.service_id)) {
            RouteTarget::BoundService {
                service_id: rule.service_id,
                instance_id: *instance_id,
            }
        } else {
            RouteTarget::ServiceNotBound(rule.service_id)
        }
    }

    pub fn bind_service(&self, session_id: u32, service_id: u8, instance_id: u32) {
        self.bindings.insert((session_id, service_id), instance_id);
    }

    pub fn unbind_service(&self, session_id: u32, service_id: u8) {
        self.bindings.remove(&(session_id, service_id));
    }

    pub fn unbind_all_by_instance(&self, service_id: u8, instance_id: u32) {
        self.bindings
            .retain(|(sid, bound_service_id), bound_instance_id| {
                let _ = sid;
                !(*bound_service_id == service_id && *bound_instance_id == instance_id)
            });
    }

    pub fn cleanup_session(&self, session_id: u32) {
        self.bindings.retain(|(sid, _), _| *sid != session_id);
    }

    pub fn binding_count(&self, service_id: u8, instance_id: u32) -> usize {
        self.bindings
            .iter()
            .filter(|entry| entry.key().1 == service_id && *entry.value() == instance_id)
            .count()
    }
}
