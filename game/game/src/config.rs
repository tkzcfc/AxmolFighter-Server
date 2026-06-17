use serde::Deserialize;
use std::path::Path;

/// 游戏服配置
#[derive(Debug, Deserialize)]
pub struct GameConfig {
    pub server: ServerSection,
    pub gateway: GatewaySection,
    pub database: DatabaseSection,
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

/// 数据库配置
#[derive(Debug, Deserialize)]
pub struct DatabaseSection {
    /// PostgreSQL 连接地址
    pub url: String,
    /// 最大连接数
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

impl GameConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}
