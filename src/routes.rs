use crate::css::generate_overlay_css;
use crate::proxy::{self, ProxyError};
use crate::reload::ConfigWatcher;
use axum::body::{Body, Bytes};
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

/// How long a `/reload-events` request parks before answering with the
/// unchanged version. Short enough to survive proxies and browser timeouts.
const RELOAD_POLL_TIMEOUT: Duration = Duration::from_secs(25);

/// Cap on request bodies relayed upstream (Cloudflare challenge posts are tiny).
const MAX_UPSTREAM_BODY: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub watcher: ConfigWatcher,
    pub client: Client,
}

pub fn app_router(state: AppState) -> Router {
    // The assets mount is fixed at startup; changing `assets.dir` in
    // config.toml needs a restart, everything else hot-reloads.
    let assets_dir: PathBuf = state.watcher.config().assets.dir.clone();
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
        .route("/reload-events", get(reload_events))
        .nest_service("/assets", static_files)
        // Everything else is Streamkit's own app (scripts, images, Cloudflare
        // challenge endpoints), relayed so the browser only ever sees one origin.
        .fallback(upstream)
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
    let config = state.watcher.config();
    let assets = config.assets.dir.display();
    let base = config.public_base_url();
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
  <h2>Hot reload</h2>
  <p>Save <code>config.toml</code> and every open <code>/overlay</code> (OBS / TikTok Live Studio
  browser sources included) refreshes itself within a second. A config with a syntax error is
  ignored and the previous one stays live, so the overlay never goes blank.</p>
  <p>Changing <code>assets.dir</code> is the one setting that still needs a restart.</p>
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

/// Entry point: send the browser to the *same* path Streamkit uses, on our
/// origin. Keeping the path identical matters — the Streamkit SPA routes on
/// `location.pathname` to pick the voice/chat/status overlay.
async fn overlay(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OverlayQuery>,
) -> Result<Response, AppError> {
    let config = state.watcher.config();
    let target = query
        .target
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(config.streamkit.url.as_str());
    let target_url = proxy::parse_http_url(target)?;

    let local_path = path_and_query(target_url.path(), target_url.query());
    if local_path != "/overlay" {
        tracing::info!(%target, %local_path, "redirecting to the proxied overlay path");
        return Ok(Redirect::temporary(&local_path).into_response());
    }

    // The upstream overlay really does live at `/overlay`: serve it in place
    // rather than redirecting to ourselves forever.
    relay(&state, target_url, Method::GET, &headers, Bytes::new()).await
}

/// Reverse-proxy any path we do not serve ourselves to the Streamkit origin,
/// injecting the overlay CSS into HTML replies.
async fn upstream(State(state): State<AppState>, request: Request) -> Result<Response, AppError> {
    let config = state.watcher.config();
    let base = proxy::parse_http_url(&config.streamkit.url)?;

    let (parts, body) = request.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |pq| pq.as_str());
    let target = base
        .join(path)
        .map_err(|e| ProxyError::InvalidUrl(e.to_string()))?;

    let body = axum::body::to_bytes(body, MAX_UPSTREAM_BODY)
        .await
        .map_err(|_| ProxyError::BodyTooLarge)?;

    relay(&state, target, parts.method, &parts.headers, body).await
}

/// Shared plumbing: forward upstream, inject into HTML, hand back the reply.
async fn relay(
    state: &AppState,
    target: reqwest::Url,
    method: Method,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut upstream =
        proxy::forward(&state.client, target, method, headers, body).await?;
    proxy::inject_overlay(
        &mut upstream,
        &state.watcher.config(),
        state.watcher.version(),
    );

    // Always re-fetch in TikTok Live Studio / OBS browser sources.
    if upstream.is_html() {
        upstream.headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store, no-cache, must-revalidate"),
        );
    }

    Ok((upstream.status, upstream.headers, Body::from(upstream.body)).into_response())
}

fn path_and_query(path: &str, query: Option<&str>) -> String {
    match query {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    }
}

async fn preview_css(State(state): State<AppState>) -> impl IntoResponse {
    let css = generate_overlay_css(&state.watcher.config().users);
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        css,
    )
}

#[derive(Debug, Deserialize)]
pub struct ReloadQuery {
    /// Config version the overlay page was rendered with.
    #[serde(default)]
    pub since: u64,
}

/// Long-poll endpoint driving hot reload: parks until `config.toml` changes,
/// then answers with the new version so the overlay can refresh itself.
async fn reload_events(
    State(state): State<AppState>,
    Query(query): Query<ReloadQuery>,
) -> impl IntoResponse {
    let version = state
        .watcher
        .wait_for_change(query.since, RELOAD_POLL_TIMEOUT)
        .await;
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        version.to_string(),
    )
}

#[derive(Debug, Deserialize)]
pub struct ProxyQuery {
    /// Absolute URL of the asset to fetch.
    pub url: String,
}

/// Escape hatch for fetching an absolute Discord URL that the same-origin
/// fallback cannot reach (a CDN host, say).
/// Usage: `/proxy?url=https://cdn.discordapp.com/...`
async fn asset_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ProxyQuery>,
) -> Result<Response, AppError> {
    if !is_allowed_proxy_url(&query.url) {
        return Err(AppError::forbidden(
            "proxy only allows discord streamkit / cdn hosts".into(),
        ));
    }

    let target = proxy::parse_http_url(&query.url)?;
    relay(&state, target, Method::GET, &headers, Bytes::new()).await
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
