use serde::Deserialize;
use std::path::Path;

/// 城镇服配置。
#[derive(Debug, Deserialize)]
pub struct TownConfig {
    /// 自身配置。
    #[serde(default)]
    pub server: ServerSection,
    /// 网关连接配置。
    #[serde(default)]
    pub gateway: GatewaySection,
    /// 城镇同步配置。
    #[serde(default)]
    pub sync: SyncSection,
}

/// 服务器自身配置。
#[derive(Debug, Deserialize)]
pub struct ServerSection {
    /// 实例 ID（用于向网关注册）。
    #[serde(default = "default_instance_id")]
    pub instance_id: u32,
}

fn default_instance_id() -> u32 {
    1
}

/// 网关连接配置。
#[derive(Debug, Deserialize)]
pub struct GatewaySection {
    /// 网关 internal_listen 地址。
    #[serde(default = "default_gateway_addr")]
    pub addr: String,
    /// 断线重连间隔（秒）。
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval: u64,
}

fn default_gateway_addr() -> String {
    "127.0.0.1:17100".to_string()
}

fn default_reconnect_interval() -> u64 {
    3
}

/// 城镇同步（移动校验 / 广播 / AOI）配置。
#[derive(Debug, Deserialize)]
pub struct SyncSection {
    /// 允许的最大移动速度（像素/秒）。
    #[serde(default = "default_max_speed")]
    pub max_speed: f32,
    /// 位移校验容差（相对 max_speed 的倍数）。
    #[serde(default = "default_tolerance")]
    pub tolerance: f32,
    /// 位移预算（token bucket）的突发时长（秒）：预算上限 = max_speed × tolerance × burst_seconds。
    #[serde(default = "default_burst_seconds")]
    pub burst_seconds: f32,
    /// 广播 tick 间隔（毫秒），默认 10 Hz。
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    /// AOI 半径（像素）。0 表示关闭 AOI，整个场景可见。
    #[serde(default = "default_aoi_radius")]
    pub aoi_radius: f32,
}

fn default_max_speed() -> f32 {
    600.0
}

fn default_tolerance() -> f32 {
    1.5
}

fn default_burst_seconds() -> f32 {
    0.5
}

fn default_tick_ms() -> u64 {
    100
}

fn default_aoi_radius() -> f32 {
    0.0
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            instance_id: default_instance_id(),
        }
    }
}

impl Default for GatewaySection {
    fn default() -> Self {
        Self {
            addr: default_gateway_addr(),
            reconnect_interval: default_reconnect_interval(),
        }
    }
}

impl Default for SyncSection {
    fn default() -> Self {
        Self {
            max_speed: default_max_speed(),
            tolerance: default_tolerance(),
            burst_seconds: default_burst_seconds(),
            tick_ms: default_tick_ms(),
            aoi_radius: default_aoi_radius(),
        }
    }
}

impl TownConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// 加载配置；文件缺失时退回默认值。
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        if path.as_ref().exists() {
            Self::load(path)
        } else {
            Ok(Self {
                server: ServerSection::default(),
                gateway: GatewaySection::default(),
                sync: SyncSection::default(),
            })
        }
    }
}
