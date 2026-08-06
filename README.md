# Discord Overlay Proxy

Fast local Rust server that proxies the Discord Streamkit voice overlay, injects custom CSS (per-user idle/speaking avatars), and serves a transparent page ready for **TikTok Live Studio** (or OBS).

```
TikTok Live Studio  →  Rust (Axum)  →  Discord Streamkit
   (localhost)         inject CSS       original WS/DOM
```

## Requirements

- Rust 1.85+ (edition 2024)
- A Discord Streamkit voice overlay URL

## Quick start

```bash
# 1. Edit config with your Streamkit URL + user avatar maps
$EDITOR config.toml

# 2. Run
cargo run

# 3. In TikTok Live Studio browser source, load:
#    http://127.0.0.1:3000/overlay
```

Optional config path:

```bash
cargo run -- ./my-config.toml
```

## Configuration

`config.toml` holds:

| Section    | Purpose |
|-----------|---------|
| `[server]` | Bind host/port (default `127.0.0.1:3000`) |
| `[streamkit]` | Default Streamkit overlay URL |
| `[users.<discord_id>]` | `idle_url` + `speaking_url` per user |

Enable **Developer Mode** in Discord → right-click a user → **Copy User ID**.

### Streamkit URL binding

1. **Config file (default)** — set `[streamkit].url`
2. **Query override** — `http://127.0.0.1:3000/overlay?target=https://streamkit.discord.com/overlay/voice?...`

## Routes

| Path | Description |
|------|-------------|
| `GET /` | Small index page |
| `GET /overlay` | Fetch Streamkit HTML, inject CSS, serve |
| `GET /css` | Preview generated stylesheet |
| `GET /proxy?url=...` | Optional asset proxy (Discord hosts only) |
| `GET /health` | Liveness |

## How CSS injection works

1. Server fetches the Streamkit HTML with `reqwest`
2. Injects `<base href="...">` so relative scripts/styles resolve on Streamkit (WebSockets keep working)
3. Injects a `<style>` block that:
   - Forces a transparent background
   - Hides default avatars
   - Shows custom images for mapped `data-userid` values
   - Swaps speaking art + bounce animation when Discord adds speaking classes

## Project layout

```
src/
  main.rs      # bootstrap + logging
  config.rs    # TOML config + UserMap
  css.rs       # CSS generator
  proxy.rs     # fetch HTML / assets + inject
  routes.rs    # Axum routes
config.toml    # local settings (edit me)
```

## Notes for TikTok Live Studio

- Use **transparent** background in the browser source if the host supports it
- Prefer `127.0.0.1` over `localhost` if the embed sandbox is picky
- Avatar image URLs must be reachable from the browser that loads the overlay (public HTTPS or local static files)

## License

MIT (or your choice) — local tool, no affiliation with Discord or TikTok.
