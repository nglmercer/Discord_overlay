use crate::web;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use reqwest::{Client, Url};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("invalid target URL: {0}")]
    InvalidUrl(String),
    #[error("only http(s) Streamkit URLs are allowed")]
    UnsupportedScheme,
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("request body too large")]
    BodyTooLarge,
}

/// Request headers that describe *our* hop and must not be forwarded upstream.
/// `accept-encoding` is dropped so the upstream answers uncompressed — we relay
/// bodies verbatim and have no decompressor.
const SKIP_REQUEST_HEADERS: [&str; 10] = [
    "host",
    "connection",
    "keep-alive",
    "proxy-connection",
    "accept-encoding",
    "content-length",
    "transfer-encoding",
    "upgrade",
    "te",
    "trailer",
];

/// Response headers that belong to the upstream hop, or that would stop the
/// browser from running our injected script / embedding the overlay in OBS.
const SKIP_RESPONSE_HEADERS: [&str; 10] = [
    "connection",
    "keep-alive",
    "content-length",
    "transfer-encoding",
    "content-encoding",
    "content-security-policy",
    "content-security-policy-report-only",
    "x-frame-options",
    "strict-transport-security",
    "alt-svc",
];

/// An upstream reply, ready to be handed back to the browser.
pub struct UpstreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl UpstreamResponse {
    pub fn is_html(&self) -> bool {
        self.headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.trim_start().starts_with("text/html"))
    }
}

/// Parse and validate an http(s) URL from user-controlled input.
pub fn parse_http_url(raw: &str) -> Result<Url, ProxyError> {
    let url = Url::parse(raw).map_err(|e| ProxyError::InvalidUrl(e.to_string()))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err(ProxyError::UnsupportedScheme),
    }
}

/// Relay a request upstream and return the reply untouched (status included).
///
/// Keeping the browser on a single origin is the whole point: Streamkit's
/// scripts, Cloudflare challenges and cookies all behave as same-origin
/// traffic, so no CORS or `SameSite` rule can reject them.
pub async fn forward(
    client: &Client,
    target: Url,
    method: Method,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<UpstreamResponse, ProxyError> {
    let origin = origin_of(&target);
    let mut outgoing = HeaderMap::new();
    for (name, value) in headers {
        if SKIP_REQUEST_HEADERS.contains(&name.as_str()) {
            continue;
        }
        // Present ourselves as Streamkit's own page, not as `127.0.0.1`.
        let value = match name.as_str() {
            "origin" => HeaderValue::from_str(origin.trim_end_matches('/')).ok(),
            "referer" => rebase_referer(value, &target),
            _ => Some(value.clone()),
        };
        if let Some(value) = value {
            outgoing.insert(name.clone(), value);
        }
    }

    let mut request = client.request(method, target).headers(outgoing);
    if !body.is_empty() {
        request = request.body(body);
    }
    let response = request.send().await?;

    let status = response.status();
    let mut out_headers = HeaderMap::new();
    for (name, value) in response.headers() {
        if SKIP_RESPONSE_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let value = if name == header::SET_COOKIE {
            match localize_cookie(value) {
                Some(value) => value,
                None => continue,
            }
        } else {
            value.clone()
        };
        out_headers.append(name.clone(), value);
    }

    Ok(UpstreamResponse {
        status,
        headers: out_headers,
        body: response.bytes().await?,
    })
}

/// Point a `Referer` at the upstream copy of the page we are proxying.
fn rebase_referer(value: &HeaderValue, target: &Url) -> Option<HeaderValue> {
    let referer = value.to_str().ok()?;
    let path = Url::parse(referer).ok()?;
    let mut rebased = target.clone();
    rebased.set_path(path.path());
    rebased.set_query(path.query());
    HeaderValue::from_str(rebased.as_str()).ok()
}

/// Make an upstream cookie storable on `http://127.0.0.1`.
///
/// `Domain=.discord.com` would be rejected outright, and `Secure`,
/// `SameSite=None` and `Partitioned` cannot survive a plain-HTTP local origin —
/// `Partitioned` in particular is dropped by the browser without `Secure`,
/// which is what rejected Cloudflare's `cf_clearance`.
fn localize_cookie(value: &HeaderValue) -> Option<HeaderValue> {
    let cookie = value.to_str().ok()?;
    let rewritten = cookie
        .split(';')
        .map(str::trim)
        .filter(|part| {
            let lower = part.to_ascii_lowercase();
            !lower.starts_with("domain=") && lower != "secure" && lower != "partitioned"
        })
        .map(|part| {
            if part.eq_ignore_ascii_case("samesite=none") {
                "SameSite=Lax"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    HeaderValue::from_str(&rewritten).ok()
}

fn origin_of(url: &Url) -> String {
    let mut origin = url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    origin.to_string()
}

/// Splice our CSS and hot-reload script into an HTML reply.
///
/// Anything that is not successfully-served HTML (scripts, images, Cloudflare
/// challenges, redirects) passes through untouched.
pub fn inject_overlay(response: &mut UpstreamResponse, version: u64) {
    if !response.status.is_success() || !response.is_html() {
        return;
    }
    let Ok(html) = std::str::from_utf8(&response.body) else {
        return;
    };

    // The RPC bridge must patch `window.WebSocket` before Streamkit's bundle
    // runs, so it goes first; the CSS goes last to win the cascade.
    let injected = inject_head(
        html,
        web::RPC_BRIDGE_HEAD,
        &web::render_overlay_head(version),
    );
    response.body = Bytes::from(injected);
}

/// Everything we splice into the Streamkit document head is a file under
/// `web/`: the generated avatar CSS is loaded from `/css`, and the watcher
/// script is loaded from `/web/hot-reload.js`.
///
/// Note there is deliberately no `<base href>`: the whole Streamkit app is
/// reverse-proxied through this server, so relative URLs must keep resolving
/// against our own origin.
/// Splice `early` in right after `<head>` and `late` in just before `</head>`,
/// tolerating documents that are missing either tag.
fn inject_head(html: &str, early: &str, late: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let head_start = head_open_end(&lower);
    let head_end = lower.find("</head>");

    let mut out = String::with_capacity(html.len() + early.len() + late.len());
    match (head_start, head_end) {
        (Some(start), Some(end)) if start <= end => {
            out.push_str(&html[..start]);
            out.push_str(early);
            out.push_str(&html[start..end]);
            out.push_str(late);
            out.push_str(&html[end..]);
        }
        // Only one anchor to work with: keep the injections together there.
        (Some(at), _) | (None, Some(at)) => {
            out.push_str(&html[..at]);
            out.push_str(early);
            out.push_str(late);
            out.push_str(&html[at..]);
        }
        (None, None) => {
            return format!(
                "<!DOCTYPE html>\n<html>\n<head>\n{early}{late}</head>\n<body>\n{html}\n</body>\n</html>"
            );
        }
    }
    out
}

/// Byte offset just past the opening `<head …>` tag, if there is one.
fn head_open_end(lower: &str) -> Option<usize> {
    let start = lower.find("<head")?;
    // Reject `<header>` and friends.
    let next = lower[start + "<head".len()..].chars().next()?;
    if next != '>' && !next.is_whitespace() {
        return None;
    }
    lower[start..].find('>').map(|end| start + end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_lands_after_head_open_and_late_before_head_close() {
        // Mirrors Streamkit's real document: the bundle is already in <head>.
        let html = concat!(
            "<!doctype html><html><head><title>t</title>",
            r#"<script defer src="/static/js/main.js"></script>"#,
            "</head><body>ok</body></html>"
        );
        let out = inject_head(html, "<!--EARLY-->", "<!--LATE-->");
        let early = out.find("<!--EARLY-->").unwrap();
        let late = out.find("<!--LATE-->").unwrap();

        assert!(
            early < out.find("<title>").unwrap(),
            "early must precede the bundle"
        );
        assert!(early < out.find("main.js").unwrap());
        assert!(late > out.find("main.js").unwrap());
        assert!(late < out.find("</head>").unwrap());
    }

    #[test]
    fn injects_after_head_open_when_unclosed() {
        let html = "<html><head><title>t</title><body>ok";
        let out = inject_head(html, "<!--EARLY-->", "<!--LATE-->");
        assert!(out.find("<!--EARLY-->").unwrap() < out.find("<title>").unwrap());
        assert!(out.find("<!--LATE-->").unwrap() < out.find("<title>").unwrap());
    }

    #[test]
    fn builds_a_document_when_there_is_no_head() {
        let out = inject_head("just text", "<!--EARLY-->", "<!--LATE-->");
        assert!(out.contains("<!--EARLY--><!--LATE-->"));
        assert!(out.contains("just text"));
    }

    #[test]
    fn header_tag_is_not_mistaken_for_head() {
        assert_eq!(head_open_end("<header><h1>hi</h1>"), None);
        assert_eq!(head_open_end("<head lang=\"en\">"), Some(16));
    }

    #[test]
    fn injection_loads_external_web_assets_but_no_base_tag() {
        let out = crate::web::render_overlay_head(7);
        assert!(out.contains(r#"id="discord-overlay-custom""#));
        assert!(out.contains(r#"href="/css?version=7""#));
        assert!(out.contains(r#"src="/web/hot-reload.js?version=7""#));
        assert!(!out.contains("const version = 7;"));
        assert!(!out.contains("__VERSION__"));
        assert!(!out.contains("<style"));
        // A <base> pointing upstream is what broke same-origin loading before.
        assert!(!out.contains("<base"));
    }

    #[test]
    fn cookies_are_rewritten_for_a_local_plain_http_origin() {
        let cookie = HeaderValue::from_static(
            "__dcfduid=abc; Domain=.discord.com; Path=/; Secure; SameSite=None; HttpOnly",
        );
        let out = localize_cookie(&cookie).unwrap();
        assert_eq!(
            out.to_str().unwrap(),
            "__dcfduid=abc; Path=/; SameSite=Lax; HttpOnly"
        );
    }
}
