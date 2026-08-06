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
| `[assets]` | Local image folder (default `assets/`) |
| `[users.<discord_id>]` | `idle_url` + `speaking_url` per user |

Enable **Developer Mode** in Discord → right-click a user → **Copy User ID**.

### Local images

1. Put files in `./assets/` (created automatically on first run):

```text
assets/
  alice-idle.png
  alice-speaking.png
```

2. Reference them by **filename** (not `file://`):

```toml
[users.123456789012345678]
idle_url = "alice-idle.png"
speaking_url = "alice-speaking.png"
```

The server rewrites those to absolute URLs:

```text
http://127.0.0.1:3000/assets/alice-idle.png
```

That matters because the overlay injects Streamkit’s `<base href>` — relative paths would otherwise resolve on Discord’s domain, not yours.

You can still use full remote URLs (`https://…`) if the image is hosted elsewhere.

### Streamkit URL binding

1. **Config file (default)** — set `[streamkit].url`
2. **Query override** — `http://127.0.0.1:3000/overlay?target=https://streamkit.discord.com/overlay/voice?...`

## Routes

| Path | Description |
|------|-------------|
| `GET /` | Small index page |
| `GET /overlay` | Fetch Streamkit HTML, inject CSS, serve |
| `GET /css` | Preview generated stylesheet |
| `GET /assets/*` | **Local images** from `./assets/` |
| `GET /proxy?url=...` | Optional asset proxy (Discord hosts only) |
| `GET /health` | Liveness |
| `GET /reload-events?since=N` | Hot-reload long poll (used by the injected script) |

## Hot reload

`config.toml` is watched while the server runs. Save the file and every open
`/overlay` — including OBS and TikTok Live Studio browser sources — reloads
itself within about a second. No restart, no clicking "refresh" in the source.

- The overlay page carries a small script that long-polls `/reload-events`; the
  server answers as soon as the config version changes.
- A config with a syntax error is logged and **ignored**: the previously loaded
  configuration stays live, so the overlay never goes blank on a typo.
- Restarting the proxy also refreshes connected overlays.
- `assets.dir` is the one setting that still requires a restart; the static
  mount is created at startup.

## How CSS injection works

1. Server fetches the Streamkit HTML with `reqwest`
2. Injects `<base href="...">` so relative scripts/styles resolve on Streamkit (WebSockets keep working)
3. Injects a `<style>` block that:
   - Forces a transparent background
   - Hides default avatars
   - Shows custom images for mapped `data-userid` values
   - Swaps speaking art + bounce animation when Discord adds speaking classes
4. Injects the hot-reload watcher script

## Project layout

```
src/
  main.rs      # bootstrap + logging
  config.rs    # TOML config + UserMap
  css.rs       # CSS generator
  proxy.rs     # fetch HTML / assets + inject
  reload.rs    # config.toml watcher + hot-reload signal
  routes.rs    # Axum routes
config.toml    # local settings (edit me)
```

## Notes for TikTok Live Studio

- Use **transparent** background in the browser source if the host supports it
- Prefer `127.0.0.1` over `localhost` if the embed sandbox is picky
- Avatar image URLs must be reachable from the browser that loads the overlay (public HTTPS or local static files)

## License

MIT (or your choice) — local tool, no affiliation with Discord or TikTok.
