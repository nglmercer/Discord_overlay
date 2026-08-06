use crate::config::Config;
use crate::css::generate_overlay_css;
use axum::body::Bytes;
use reqwest::{Client, Response, Url};
use thiserror::Error;

const USER_AGENT: &str = "DiscordOverlayProxy/0.1 (+https://github.com/local/discord_overlay)";

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("invalid target URL: {0}")]
    InvalidUrl(String),
    #[error("only http(s) Streamkit URLs are allowed")]
    UnsupportedScheme,
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("upstream returned {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("response body is not valid UTF-8")]
    InvalidUtf8,
}

/// Parse and validate an http(s) URL from user-controlled input.
pub fn parse_http_url(raw: &str) -> Result<Url, ProxyError> {
    let url = Url::parse(raw).map_err(|e| ProxyError::InvalidUrl(e.to_string()))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err(ProxyError::UnsupportedScheme),
    }
}

/// GET `url`, returning the response only when the upstream status is a success.
async fn get_ok(client: &Client, url: Url, accept: Option<&str>) -> Result<Response, ProxyError> {
    let mut request = client.get(url).header("User-Agent", USER_AGENT);
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }

    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_default();
    Err(ProxyError::Upstream {
        status: status.as_u16(),
        body: body.chars().take(200).collect(),
    })
}

/// Fetch the Streamkit overlay HTML and inject custom CSS into `<head>`.
pub async fn fetch_and_inject(
    client: &Client,
    target: &str,
    config: &Config,
    version: u64,
) -> Result<String, ProxyError> {
    let target_url = parse_http_url(target)?;
    let response = get_ok(
        client,
        target_url.clone(),
        Some("text/html,application/xhtml+xml"),
    )
    .await?;

    let html = response.text().await.map_err(|_| ProxyError::InvalidUtf8)?;
    Ok(inject_head(&html, &head_injection(config, version, &target_url)))
}

/// Everything we splice into the Streamkit document head: a `<base href>` so
/// relative assets still resolve upstream, the generated avatar CSS, and the
/// watcher script that reloads the page when `config.toml` changes.
fn head_injection(config: &Config, version: u64, target_url: &Url) -> String {
    let mut origin = target_url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);

    // Prefer origin + directory of the overlay page for relative resolution.
    let base_href = target_url
        .join("./")
        .map(|u| u.to_string())
        .unwrap_or_else(|_| origin.to_string());

    let css = generate_overlay_css(&config.users);
    let script = HOT_RELOAD_SCRIPT
        .replace(
            "__ENDPOINT__",
            &format!(
                "{}/reload-events",
                config.public_base_url().trim_end_matches('/')
            ),
        )
        .replace("__VERSION__", &version.to_string());

    format!(
        r#"<base href="{base_href}">
<style id="discord-overlay-custom" type="text/css">
{css}
</style>
{script}
"#
    )
}

/// Long-polls the proxy for config changes and reloads the overlay on any edit.
const HOT_RELOAD_SCRIPT: &str = r#"<script id="discord-overlay-hotreload">
(() => {
  const endpoint = "__ENDPOINT__";
  const version = __VERSION__;
  const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
  (async () => {
    for (;;) {
      try {
        const res = await fetch(endpoint + "?since=" + version, { cache: "no-store" });
        const next = Number(await res.text());
        if (Number.isFinite(next) && next !== version) {
          location.reload();
          return;
        }
      } catch (err) {
        // Proxy restarting or unreachable — back off and keep trying.
        await sleep(2000);
      }
    }
  })();
})();
</script>"#;

/// Splice `injection` into the document head, tolerating malformed documents.
fn inject_head(html: &str, injection: &str) -> String {
    // Insert before </head> when present; otherwise prepend a minimal head.
    if let Some(idx) = html.to_ascii_lowercase().find("</head>") {
        let mut out = String::with_capacity(html.len() + injection.len());
        out.push_str(&html[..idx]);
        out.push_str(injection);
        out.push_str(&html[idx..]);
        out
    } else if let Some(idx) = html.to_ascii_lowercase().find("<head>") {
        let insert_at = idx + "<head>".len();
        let mut out = String::with_capacity(html.len() + injection.len());
        out.push_str(&html[..insert_at]);
        out.push('\n');
        out.push_str(injection);
        out.push_str(&html[insert_at..]);
        out
    } else {
        format!(
            r#"<!DOCTYPE html>
<html>
<head>
{injection}
</head>
<body>
{html}
</body>
</html>"#
        )
    }
}

/// Proxy a static asset from the Streamkit origin (or an absolute URL).
///
/// Returns the body plus the upstream `Content-Type`.
pub async fn fetch_asset(
    client: &Client,
    asset_url: &str,
) -> Result<(Bytes, String), ProxyError> {
    let response = get_ok(client, parse_http_url(asset_url)?, None).await?;

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = response.bytes().await?;
    Ok((bytes, content_type))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{AssetsConfig, ServerConfig, StreamkitConfig};

    fn test_config() -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3000,
            },
            streamkit: StreamkitConfig {
                url: "https://streamkit.discord.com/overlay/voice".to_string(),
            },
            assets: AssetsConfig::default(),
            users: Default::default(),
        }
    }

    #[test]
    fn injects_before_closing_head() {
        let html = "<!DOCTYPE html><html><head><title>t</title></head><body>ok</body></html>";
        let out = inject_head(html, "<style>body{color:red}</style>");
        assert!(out.contains("body{color:red}"));
        let lower = out.to_ascii_lowercase();
        assert!(lower.find("color:red").unwrap() < lower.find("</head>").unwrap());
    }

    #[test]
    fn injection_carries_base_css_and_hot_reload() {
        let url = Url::parse("https://streamkit.discord.com/overlay/voice?x=1").unwrap();
        let out = head_injection(&test_config(), 7, &url);
        assert!(out.contains(r#"id="discord-overlay-custom""#));
        assert!(out.contains(r#"<base href="https://streamkit.discord.com/overlay/""#));
        assert!(out.contains("http://127.0.0.1:3000/reload-events"));
        assert!(out.contains("const version = 7;"));
        assert!(!out.contains("__ENDPOINT__"));
    }
}
