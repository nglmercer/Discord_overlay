//! Compile-time web templates and browser assets.
//!
//! Keeping these files outside Rust source makes the HTML, CSS, and JavaScript
//! easy to edit while `include_str!` keeps standalone binaries self-contained.

pub const INDEX_HTML: &str = include_str!("../web/index.html");
pub const INDEX_CSS: &str = include_str!("../web/index.css");
pub const OVERLAY_CSS: &str = include_str!("../web/overlay.css");
pub const USER_CSS_TEMPLATE: &str = include_str!("../web/user.css");
pub const OVERLAY_HEAD: &str = include_str!("../web/overlay-head.html");
pub const RPC_BRIDGE_HEAD: &str = include_str!("../web/rpc-bridge-head.html");
pub const RPC_BRIDGE_JS: &str = include_str!("../web/rpc-bridge.js");
pub const HOT_RELOAD_JS: &str = include_str!("../web/hot-reload.js");

pub fn render_index(assets_dir: &str, public_base_url: &str) -> String {
    let assets_dir = escape_html(assets_dir);
    let public_base_url = escape_html(public_base_url);
    render_template(
        INDEX_HTML,
        &[
            ("ASSETS_DIR", &assets_dir),
            ("PUBLIC_BASE_URL", &public_base_url),
        ],
    )
}

pub fn render_overlay_head(version: u64) -> String {
    let version = version.to_string();
    render_template(OVERLAY_HEAD, &[("VERSION", &version)])
}

fn render_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (name, value) in values {
        rendered = rendered.replace(&format!("{{{{{name}}}}}"), value);
    }
    rendered
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_index_values_as_html() {
        let html = render_index("assets & avatars", "http://127.0.0.1:3000");
        assert!(html.contains("assets &amp; avatars"));
        assert!(html.contains("http://127.0.0.1:3000/assets/…"));
        assert!(!html.contains("{{ASSETS_DIR}}"));
    }

    #[test]
    fn renders_external_overlay_assets_with_version() {
        let head = render_overlay_head(7);
        assert!(head.contains(r#"href="/css?version=7""#));
        assert!(head.contains(r#"src="/web/hot-reload.js?version=7""#));
    }

    /// The bridge is injected separately, before Streamkit's bundle; repeating
    /// it in the late head block loaded the script — and re-wrapped
    /// `window.WebSocket` — a second time.
    #[test]
    fn rpc_bridge_is_only_injected_once() {
        assert!(RPC_BRIDGE_HEAD.contains(r#"src="/web/rpc-bridge.js""#));
        assert!(!render_overlay_head(7).contains("rpc-bridge.js"));
    }
}
