use dashmap::DashMap;
use std::sync::Arc;

use crate::config::GatewayConfig;
use crate::protocol::ServiceType;

/// 路由条目
#[derive(Debug, Clone)]
struct RouteRule {
    pub min: u16,
    pub max: u16,
    pub service_type: ServiceType,
}

/// 路由器：根据 msg_id 决定转发目标
#[derive(Clone)]
pub struct Router {
    rules: Arc<Vec<RouteRule>>,
    /// 战斗服绑定表: session_id → battle_instance_id
    battle_bindings: Arc<DashMap<u32, u32>>,
}

/// 路由查询结果
#[derive(Debug, Clone)]
pub enum RouteTarget {
    /// 转发到游戏服（单实例，直接转）
    GameServer,
    /// 转发到战斗服（需要查绑定表）
    BattleServer(u32), // battle_instance_id
    /// 网关内部协议
    Internal,
    /// 未知路由（msg_id 不在任何范围内）
    Unknown,
    /// 战斗服未绑定
    BattleNotBound,
}

impl Router {
    /// 从配置构建路由表
    pub fn from_config(config: &GatewayConfig) -> Self {
        let mut rules = Vec::new();
        for (name, entry) in &config.route {
            if let Some(stype) = ServiceType::from_name(name) {
                rules.push(RouteRule {
                    min: entry.range[0],
                    max: entry.range[1],
                    service_type: stype,
                });
            } else {
                tracing::warn!("unknown service type in route config: {}", name);
            }
        }
        Self {
            rules: Arc::new(rules),
            battle_bindings: Arc::new(DashMap::new()),
        }
    }

    /// 根据 msg_id 和 session_id 查找路由目标
    pub fn resolve(&self, msg_id: u16, session_id: u32) -> RouteTarget {
        // 内部协议范围
        if crate::protocol::INTERNAL_MSG_RANGE.contains(&msg_id) {
            return RouteTarget::Internal;
        }

        // 查找路由规则
        for rule in self.rules.iter() {
            if msg_id >= rule.min && msg_id <= rule.max {
                return match rule.service_type {
                    ServiceType::Game => RouteTarget::GameServer,
                    ServiceType::Battle => {
                        // 查绑定表
                        if let Some(instance_id) = self.battle_bindings.get(&session_id) {
                            RouteTarget::BattleServer(*instance_id)
                        } else {
                            RouteTarget::BattleNotBound
                        }
                    }
                };
            }
        }

        RouteTarget::Unknown
    }

    /// 绑定 session 到 battle 实例
    pub fn bind_battle(&self, session_id: u32, battle_instance_id: u32) {
        self.battle_bindings.insert(session_id, battle_instance_id);
    }

    /// 解绑 session 的 battle
    pub fn unbind_battle(&self, session_id: u32) {
        self.battle_bindings.remove(&session_id);
    }

    /// 移除与指定 battle 实例相关的所有绑定（战斗服下线时清理）
    pub fn unbind_all_by_instance(&self, battle_instance_id: u32) {
        self.battle_bindings.retain(|_, v| *v != battle_instance_id);
    }

    /// 清理指定 session 的绑定（客户端断线时清理）
    pub fn cleanup_session(&self, session_id: u32) {
        self.battle_bindings.remove(&session_id);
    }
}
