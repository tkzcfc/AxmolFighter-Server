use serde::Deserialize;
use std::path::Path;

/// 游戏服配置
#[derive(Debug, Deserialize)]
pub struct GameConfig {
    pub server: ServerSection,
    pub gateway: GatewaySection,
}

/// 服务器自身配置
#[derive(Debug, Deserialize)]
pub struct ServerSection {
    /// 实例ID（用于向网关注册）
    pub instance_id: u32,
}

/// 网关连接配置
#[derive(Debug, Deserialize)]
pub struct GatewaySection {
    /// 网关 internal_listen 地址
    pub addr: String,
    /// 断线重连间隔（秒）
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval: u64,
}

fn default_reconnect_interval() -> u64 {
    3
}

impl GameConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}
