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

## Standalone builds

Build artifacts are written to `dist/` with the files they need beside each
executable:

```text
dist/
  discord_overlay-linux-x86_64
  config.toml
  README.md
  assets/
```

Run a standalone build from its folder, edit the copied `config.toml`, and then
start the executable:

```bash
cd dist
./discord_overlay-linux-x86_64
```

If the implicit `config.toml` is missing, the executable creates it from the
bundled default beside the executable without overwriting an existing file.
An existing `config.toml` in the working directory still takes precedence. This
makes a binary copied by itself to another folder usable on its first run. An
explicitly provided path is never created automatically:

```bash
./discord_overlay-linux-x86_64 ./my-config.toml
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
| `GET /overlay` | Entry point — redirects to the proxied Streamkit path |
| `GET /css` | Preview generated stylesheet |
| `GET /assets/*` | **Local images** from `./assets/` |
| `GET /proxy?url=...` | Optional asset proxy (Discord hosts only) |
| `GET /health` | Liveness |
| `GET /reload-events?since=N` | Hot-reload long poll (used by the injected script) |
| `GET /rpc/{port}` | WebSocket bridge to the local Discord client (6463-6472) |
| `ANY /*` | Reverse-proxied to the Streamkit origin |

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

## How it works

The proxy is a **same-origin reverse proxy**, not just an HTML rewriter. Everything
the browser loads — the Streamkit HTML, `/static/js/*`, images, Cloudflare's
challenge endpoints — is relayed through `127.0.0.1:3000`, so from the browser's
point of view there is only one origin.

That matters because the alternative (serving the HTML locally and pointing a
`<base href>` at `streamkit.discord.com`) puts the document and its resources on
different origins, which breaks CORS on Cloudflare's challenge requests and makes
the browser reject Discord's `__dcfduid` / `__sdcfduid` cookies as cross-site.

- `GET /overlay` redirects to the *same path Streamkit uses* (`/overlay/voice/...`)
  on this server — the SPA routes on `location.pathname`, so the path must match.
- Any path this server does not handle is relayed upstream, with the request
  method and body intact (Cloudflare challenges POST).
- Upstream `Set-Cookie` headers are rewritten for a local plain-HTTP origin:
  `Domain=` and `Secure` are dropped and `SameSite=None` becomes `Lax`.
- `Content-Security-Policy` and `X-Frame-Options` are stripped from upstream
  replies so the injected script runs and OBS can embed the page.

### The Discord RPC websocket

The overlay does not get its voice data from Streamkit — its bundle opens
`ws://127.0.0.1:<port>` straight to the **Discord desktop app** (ports
6463-6472). That socket checks the handshake's `Origin` against Discord's own
allowlist, and a browser will always stamp it with the page's real origin, so a
page served from `127.0.0.1:3000` is closed with `4001 Invalid Origin`.

The fix is a bridge: an injected script patches `window.WebSocket` — before the
bundle runs — to point those URLs at `/rpc/<port>/` on this server, and the
server dials Discord itself with `Origin: https://streamkit.discord.com`.

The bridge connects to Discord *before* accepting the browser's upgrade, so the
overlay's port scan still sees a plain connection failure on ports where no
Discord is listening. **Discord must be running** for the overlay to show
anyone.

### CSS injection

1. Server relays the Streamkit HTML with `reqwest`
2. Injects a `<style>` block, last in `<head>` so it wins over Streamkit's own CSS:
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
  rpc.rs       # websocket bridge to the local Discord client
  routes.rs    # Axum routes
config.toml    # local settings (edit me)
```

## Notes for TikTok Live Studio

- Use **transparent** background in the browser source if the host supports it
- Prefer `127.0.0.1` over `localhost` if the embed sandbox is picky
- Avatar image URLs must be reachable from the browser that loads the overlay (public HTTPS or local static files)

## License

MIT (or your choice) — local tool, no affiliation with Discord or TikTok.
