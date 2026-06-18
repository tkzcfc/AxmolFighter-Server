use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub gateway: GatewaySection,
    #[serde(default)]
    pub route: Vec<RouteEntry>,
}

#[derive(Debug, Deserialize)]
pub struct GatewaySection {
    pub client_listen: String,
    pub internal_listen: String,
}

#[derive(Debug, Deserialize)]
pub struct RouteEntry {
    pub service_id: u8,
    pub range: [u16; 2],
    #[serde(default)]
    pub require_binding: bool,
}

impl GatewayConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        for entry in &self.route {
            if entry.range[0] > entry.range[1] {
                anyhow::bail!(
                    "route service_id={}: range [{}, {}] is invalid (min > max)",
                    entry.service_id,
                    entry.range[0],
                    entry.range[1]
                );
            }
        }
        Ok(())
    }
}
