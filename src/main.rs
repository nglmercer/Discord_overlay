mod config;
mod css;
mod proxy;
mod routes;

use config::Config;
use routes::{app_router, AppState};
use std::path::PathBuf;
use std::sync::Arc;
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
        "loaded configuration"
    );

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let state = AppState {
        config: Arc::new(config.clone()),
        client,
    };

    let addr = config.bind_addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "discord overlay proxy listening");
    tracing::info!("open http://{addr}/overlay in TikTok Live Studio");

    axum::serve(listener, app_router(state)).await?;
    Ok(())
}
