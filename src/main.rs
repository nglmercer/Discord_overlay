mod config;
mod css;
mod proxy;
mod reload;
mod routes;

use config::Config;
use reload::ConfigWatcher;
use routes::{app_router, AppState};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let config = Config::load(&config_path)?;
    tracing::info!(
        path = %config_path.display(),
        users = config.users.len(),
        assets = %config.assets.dir.display(),
        "loaded configuration"
    );

    // Ensure the local image folder exists so drops are easy.
    if !config.assets.dir.exists() {
        std::fs::create_dir_all(&config.assets.dir)?;
        tracing::info!(dir = %config.assets.dir.display(), "created assets directory");
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let addr = config.bind_addr();
    let watcher = ConfigWatcher::new(config_path.clone(), config);
    tokio::spawn(watcher.clone().watch());

    let state = AppState { watcher, client };

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "discord overlay proxy listening");
    tracing::info!("overlay → http://{addr}/overlay");
    tracing::info!("local images → http://{addr}/assets/<file>  (from ./assets/)");
    tracing::info!(
        path = %config_path.display(),
        "hot reload active — saving the config refreshes open overlays"
    );

    axum::serve(listener, app_router(state)).await?;
    Ok(())
}
