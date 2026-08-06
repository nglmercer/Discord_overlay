use crate::css::generate_overlay_css;
use crate::config::UserMap;
use reqwest::Client;
use thiserror::Error;
use url::Url;

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

/// Fetch the Streamkit overlay HTML and inject custom CSS into `<head>`.
pub async fn fetch_and_inject(
    client: &Client,
    target: &str,
    users: &UserMap,
) -> Result<String, ProxyError> {
    let target_url = Url::parse(target).map_err(|e| ProxyError::InvalidUrl(e.to_string()))?;
    if target_url.scheme() != "https" && target_url.scheme() != "http" {
        return Err(ProxyError::UnsupportedScheme);
    }

    let response = client
        .get(target_url.clone())
        .header(
            "User-Agent",
            "DiscordOverlayProxy/0.1 (+https://github.com/local/discord_overlay)",
        )
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProxyError::Upstream {
            status: status.as_u16(),
            body: body.chars().take(200).collect(),
        });
    }

    let html = response.text().await.map_err(|_| ProxyError::InvalidUtf8)?;
    let css = generate_overlay_css(users);
    Ok(inject_css_and_base(&html, &css, &target_url))
}

/// Inject a `<base href>` (so relative assets resolve to Streamkit) and our
/// custom `<style>` block into the document head.
fn inject_css_and_base(html: &str, css: &str, target_url: &Url) -> String {
    let mut base = target_url.clone();
    base.set_path("/");
    base.set_query(None);
    base.set_fragment(None);

    // Prefer origin + directory of the overlay page for relative resolution.
    let base_href = target_url
        .join("./")
        .map(|u| u.to_string())
        .unwrap_or_else(|_| base.to_string());

    let injection = format!(
        r#"<base href="{base_href}">
<style id="discord-overlay-custom" type="text/css">
{css}
</style>
"#
    );

    // Insert before </head> when present; otherwise prepend a minimal head.
    if let Some(idx) = html.to_ascii_lowercase().find("</head>") {
        let mut out = String::with_capacity(html.len() + injection.len());
        out.push_str(&html[..idx]);
        out.push_str(&injection);
        out.push_str(&html[idx..]);
        out
    } else if let Some(idx) = html.to_ascii_lowercase().find("<head>") {
        let insert_at = idx + "<head>".len();
        let mut out = String::with_capacity(html.len() + injection.len());
        out.push_str(&html[..insert_at]);
        out.push('\n');
        out.push_str(&injection);
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
pub async fn fetch_asset(
    client: &Client,
    asset_url: &str,
) -> Result<(reqwest::header::HeaderMap, bytes::Bytes, String), ProxyError> {
    let url = Url::parse(asset_url).map_err(|e| ProxyError::InvalidUrl(e.to_string()))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(ProxyError::UnsupportedScheme);
    }

    let response = client
        .get(url)
        .header(
            "User-Agent",
            "DiscordOverlayProxy/0.1 (+https://github.com/local/discord_overlay)",
        )
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProxyError::Upstream {
            status: status.as_u16(),
            body: body.chars().take(200).collect(),
        });
    }

    let headers = response.headers().clone();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = response.bytes().await?;
    Ok((headers, bytes, content_type))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_style_before_closing_head() {
        let html = "<!DOCTYPE html><html><head><title>t</title></head><body>ok</body></html>";
        let url = Url::parse("https://streamkit.discord.com/overlay/voice?x=1").unwrap();
        let out = inject_css_and_base(html, "body{color:red}", &url);
        assert!(out.contains(r#"id="discord-overlay-custom""#));
        assert!(out.contains("body{color:red}"));
        assert!(out.contains("<base href="));
        let lower = out.to_ascii_lowercase();
        let style_pos = lower.find("discord-overlay-custom").unwrap();
        let head_close = lower.find("</head>").unwrap();
        assert!(style_pos < head_close);
    }
}
