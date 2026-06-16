use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// 网关配置
#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub gateway: GatewaySection,
    /// 路由表：key 为 service_type 名称（如 "game", "battle"）
    #[serde(default)]
    pub route: HashMap<String, RouteEntry>,
}

/// 网关监听地址配置
#[derive(Debug, Deserialize)]
pub struct GatewaySection {
    /// 客户端连接端口
    pub client_listen: String,
    /// 后端服务注册端口
    pub internal_listen: String,
}

/// 路由条目：定义 msg_id 范围对应的服务类型
#[derive(Debug, Deserialize)]
pub struct RouteEntry {
    /// [min, max] 闭区间
    pub range: [u16; 2],
}

impl GatewayConfig {
    /// 从 TOML 文件加载配置
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// 校验配置合法性
    fn validate(&self) -> anyhow::Result<()> {
        for (name, entry) in &self.route {
            if entry.range[0] > entry.range[1] {
                anyhow::bail!(
                    "route '{}': range [{}, {}] is invalid (min > max)",
                    name,
                    entry.range[0],
                    entry.range[1]
                );
            }
        }
        Ok(())
    }
}
