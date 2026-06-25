use std::fmt;

/// 服务间消息的来源/目标标识。
///
/// `instance_id` 与协议 `ForwardToServerReq.target_instance_id` 一致使用 `i32`:
/// - 作为**来源**时,值来自协议的 `uint32 source_instance_id`,恒 >= 0;
/// - 作为**目标**时,`-1` 表示「任意实例」,由网关按负载择一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSource {
    pub service_id: u32,
    pub instance_id: i32,
}

impl fmt::Display for ServerSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.service_id, self.instance_id)
    }
}

impl ServerSource {
    pub const fn new(service_id: u32, instance_id: i32) -> Self {
        Self {
            service_id,
            instance_id,
        }
    }

    /// 任意实例的目标(由网关按负载择一)。
    pub const fn any_instance(service_id: u32) -> Self {
        Self {
            service_id,
            instance_id: -1,
        }
    }
}
