use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Idle and speaking avatar image URLs for a single Discord user.
#[derive(Debug, Clone, Deserialize)]
pub struct AvatarOverride {
    pub idle_url: String,
    pub speaking_url: String,
}

/// Discord user ID → avatar override mapping.
pub type UserMap = HashMap<String, AvatarOverride>;

/// Top-level application configuration loaded from `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub streamkit: StreamkitConfig,
    /// Keys are Discord snowflake user IDs as strings.
    #[serde(default)]
    pub users: UserMap,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address, e.g. `127.0.0.1`.
    #[serde(default = "default_host")]
    pub host: String,
    /// Listen port, e.g. `3000`.
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamkitConfig {
    /// Default Discord Streamkit overlay URL.
    /// Can be overridden per-request with `?target=...`.
    pub url: String,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

impl Config {
    /// Load configuration from a TOML file on disk.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config at {}: {e}", path.display()))?;
        let config: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config at {}: {e}", path.display()))?;
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}
