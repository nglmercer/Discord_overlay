use crate::config::Config;
use crate::css::generate_overlay_css;
use crate::proxy::{self, ProxyError};
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub client: Client,
}

pub fn app_router(state: AppState) -> Router {
    let assets_dir: PathBuf = state.config.assets.dir.clone();
    if !assets_dir.exists() {
        tracing::warn!(
            dir = %assets_dir.display(),
            "assets directory does not exist yet — create it and drop PNG/WebP/GIF files there"
        );
    } else {
        tracing::info!(dir = %assets_dir.display(), "serving local images at /assets/*");
    }

    // Serve local avatar files: GET /assets/alice-idle.png → assets/alice-idle.png
    let static_files = ServeDir::new(assets_dir).append_index_html_on_directories(false);

    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/overlay", get(overlay))
        .route("/css", get(preview_css))
        .route("/proxy", get(asset_proxy))
        .nest_service("/assets", static_files)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let assets = state.config.assets.dir.display();
    let base = state.config.public_base_url();
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Discord Overlay Proxy</title>
  <style>
    body {{ font-family: system-ui, sans-serif; max-width: 42rem; margin: 2rem auto; padding: 0 1rem; }}
    code {{ background: #f4f4f4; padding: 0.1em 0.35em; border-radius: 4px; }}
    a {{ color: #5865f2; }}
    pre {{ background: #f4f4f4; padding: 0.75rem 1rem; border-radius: 8px; overflow-x: auto; }}
  </style>
</head>
<body>
  <h1>Discord Overlay Proxy</h1>
  <p>Transparent Streamkit overlay with custom avatars for TikTok Live Studio.</p>
  <ul>
    <li><a href="/overlay"><code>/overlay</code></a> — proxied overlay</li>
    <li><code>/overlay?target=URL</code> — override Streamkit URL</li>
    <li><a href="/css"><code>/css</code></a> — preview generated CSS</li>
    <li><code>/assets/…</code> — local images from <code>{assets}/</code></li>
    <li><a href="/health"><code>/health</code></a> — liveness</li>
  </ul>
  <h2>Local images</h2>
  <p>Drop files in <code>{assets}/</code>, then reference them in <code>config.toml</code>:</p>
  <pre>[users.YOUR_DISCORD_ID]
idle_url = "alice-idle.png"
speaking_url = "alice-speaking.png"</pre>
  <p>They are served as <code>{base}/assets/…</code> (absolute URLs, so Streamkit’s base tag cannot break them).</p>
</body>
</html>"#
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[derive(Debug, Deserialize)]
pub struct OverlayQuery {
    /// Optional override for the Streamkit URL from config.
    pub target: Option<String>,
}

async fn overlay(
    State(state): State<AppState>,
    Query(query): Query<OverlayQuery>,
) -> Result<Response, AppError> {
    let target = query
        .target
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(state.config.streamkit.url.as_str());

    tracing::info!(%target, "fetching streamkit overlay");

    let html = proxy::fetch_and_inject(&state.client, target, &state.config.users).await?;

    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // Always re-fetch in TikTok Live Studio / OBS browser sources.
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        html,
    )
        .into_response())
}

async fn preview_css(State(state): State<AppState>) -> impl IntoResponse {
    let css = generate_overlay_css(&state.config.users);
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        css,
    )
}

#[derive(Debug, Deserialize)]
pub struct ProxyQuery {
    /// Absolute URL of the asset to fetch.
    pub url: String,
}

/// Optional asset proxy for cases where relative rewriting is not enough
/// (e.g. mixed-content or CORS edge cases).
/// Usage: `/proxy?url=https://streamkit.discord.com/...`
async fn asset_proxy(
    State(state): State<AppState>,
    Query(query): Query<ProxyQuery>,
) -> Result<Response, AppError> {
    if !is_allowed_proxy_url(&query.url) {
        return Err(AppError::forbidden(
            "proxy only allows discord streamkit / cdn hosts".into(),
        ));
    }

    let (bytes, content_type) = proxy::fetch_asset(&state.client, &query.url).await?;

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=300".to_string()),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// Domains the `/proxy` endpoint is allowed to fetch from.
const ALLOWED_PROXY_HOSTS: [&str; 3] = ["discord.com", "discordapp.com", "discord.gg"];

fn is_allowed_proxy_url(raw: &str) -> bool {
    let Ok(url) = proxy::parse_http_url(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    ALLOWED_PROXY_HOSTS.iter().any(|allowed| {
        // Exact match, or a real subdomain — not a look-alike like `evildiscord.com`.
        host == *allowed || host.strip_suffix(allowed).is_some_and(|p| p.ends_with('.'))
    })
}

/// Map internal errors to HTTP responses.
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn forbidden(message: String) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }
}

impl From<ProxyError> for AppError {
    fn from(err: ProxyError) -> Self {
        let status = match &err {
            ProxyError::InvalidUrl(_) | ProxyError::UnsupportedScheme => StatusCode::BAD_REQUEST,
            // Anything that went wrong upstream is reported as a gateway failure.
            _ => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::warn!(status = %self.status, error = %self.message, "request failed");
        (
            self.status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            self.message,
        )
            .into_response()
    }
}
