#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSource {
    pub service_id: u32,
    pub instance_id: u32,
}

impl ServerSource {
    pub const fn new(service_id: u32, instance_id: u32) -> Self {
        Self {
            service_id,
            instance_id,
        }
    }
}
