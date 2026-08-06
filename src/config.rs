use serde::Deserialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Configuration copied into a standalone release on first run when no
/// implicit `config.toml` is available.
pub const DEFAULT_CONFIG: &str = include_str!("../config.toml");

/// Idle and speaking avatar image paths/URLs for a single Discord user.
///
/// Each field may be:
/// - a remote URL: `https://cdn.example/idle.png`
/// - a local file under the assets dir: `alice-idle.png` or `assets/alice-idle.png`
///
/// Local paths are rewritten to absolute `http://host:port/assets/...` URLs at load time
/// so they still work after the Streamkit `<base href>` is injected.
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
    #[serde(default)]
    pub assets: AssetsConfig,
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

/// Local image directory served at `/assets/*`.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetsConfig {
    /// Folder on disk (relative to CWD or absolute). Default: `assets`.
    #[serde(default = "default_assets_dir")]
    pub dir: PathBuf,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            dir: default_assets_dir(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_assets_dir() -> PathBuf {
    PathBuf::from("assets")
}

impl Config {
    /// Create the bundled configuration without replacing an existing file.
    ///
    /// `create_new` also makes startup safe if two launches race to create the
    /// file: the second launch simply loads the file created by the first one.
    pub fn ensure_default(path: impl AsRef<Path>) -> anyhow::Result<bool> {
        let path = path.as_ref();
        let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "failed to create default config at {}: {error}",
                    path.display()
                ));
            }
        };

        file.write_all(DEFAULT_CONFIG.as_bytes()).map_err(|error| {
            anyhow::anyhow!(
                "failed to write default config at {}: {error}",
                path.display()
            )
        })?;
        Ok(true)
    }

    /// Load configuration from a TOML file and resolve local image paths.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config at {}: {e}", path.display()))?;
        let mut config: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config at {}: {e}", path.display()))?;
        config.resolve_local_assets();
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    /// Public origin used in generated CSS for local files.
    pub fn public_base_url(&self) -> String {
        format!("http://{}:{}", self.server.host, self.server.port)
    }

    /// Rewrite non-URL avatar paths to `http://host:port/assets/<file>`.
    fn resolve_local_assets(&mut self) {
        let base = self.public_base_url();
        for avatar in self.users.values_mut() {
            avatar.idle_url = resolve_asset_ref(&avatar.idle_url, &base);
            avatar.speaking_url = resolve_asset_ref(&avatar.speaking_url, &base);
        }
    }
}

/// Turn a config image reference into a browser-fetchable absolute URL.
///
/// Remote / data URLs are left unchanged. Everything else is treated as a file
/// under the served `/assets` mount.
pub fn resolve_asset_ref(raw: &str, public_base: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if is_absolute_url(trimmed) {
        return trimmed.to_string();
    }

    let relative = normalize_local_asset_path(trimmed);
    format!(
        "{}/assets/{}",
        public_base.trim_end_matches('/'),
        relative
    )
}

fn is_absolute_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
}

/// Strip `./`, leading `/`, and an optional `assets/` prefix so the path is
/// relative to the assets directory mount.
fn normalize_local_asset_path(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while p.starts_with("./") {
        p = p[2..].to_string();
    }
    p = p.trim_start_matches('/').to_string();
    if let Some(rest) = p.strip_prefix("assets/") {
        p = rest.to_string();
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_unchanged() {
        let base = "http://127.0.0.1:3000";
        assert_eq!(
            resolve_asset_ref("https://cdn.example/a.png", base),
            "https://cdn.example/a.png"
        );
    }

    #[test]
    fn bare_filename_becomes_assets_url() {
        let base = "http://127.0.0.1:3000";
        assert_eq!(
            resolve_asset_ref("alice-idle.png", base),
            "http://127.0.0.1:3000/assets/alice-idle.png"
        );
    }

    #[test]
    fn assets_prefix_stripped() {
        let base = "http://127.0.0.1:3000";
        assert_eq!(
            resolve_asset_ref("assets/team/bob-speak.webp", base),
            "http://127.0.0.1:3000/assets/team/bob-speak.webp"
        );
    }

    #[test]
    fn nested_local_path() {
        let base = "http://127.0.0.1:3000";
        assert_eq!(
            resolve_asset_ref("./chars/me/idle.png", base),
            "http://127.0.0.1:3000/assets/chars/me/idle.png"
        );
    }

    #[test]
    fn bundled_default_config_is_valid() {
        let config: Config = toml::from_str(DEFAULT_CONFIG).expect("default config must be valid");
        assert_eq!(config.server.port, 3000);
        assert!(!config.streamkit.url.is_empty());
    }
}
