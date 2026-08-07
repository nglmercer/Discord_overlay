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
| `[overlay]` | `show_status` — diagnostic message while the overlay is not live |
| `[users.<discord_id>]` | `order`, `idle_url` + `speaking_url` per user |

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
order = 1
idle_url = "alice-idle.png"
speaking_url = "alice-speaking.png"
```

`order` controls the rendered voice-state position in the overlay, left to
right. Lower values appear first; omitted values use `0`, and users that all
share the same value keep Streamkit's own alphabetical-by-nickname sort.

Both images of every configured user are preloaded while the overlay boots, so
the first time someone speaks the swap is instant instead of showing one empty
frame.

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
| `GET /overlay?debug=1` | Same, with the status message on for that page only |
| `GET /css` | Preview generated stylesheet |
| `GET /web/*.css` / `GET /web/*.js` | Same-origin page assets used by the index and overlay |
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

## Why is my overlay blank?

A transparent overlay that has not connected yet looks exactly like one that is
broken. Turn the status message on and it will tell you which:

```bash
# one page only, nothing to edit
http://127.0.0.1:3000/overlay?debug=1
```

or set `show_status = true` under `[overlay]` in `config.toml` (hot-reloads
like everything else).

| Message | Meaning |
|---------|---------|
| *Discord isn't running* | The RPC ports 6463-6472 refused every connection — start the desktop app |
| *Connected — waiting for someone in the voice channel* | Discord is bridged fine; the channel is just empty |
| *Can't reach Streamkit* | Streamkit's own script never started — check the URL in `config.toml` |

It stays **off by default** because a browser source is captured from the moment
it starts: a message left enabled would appear on stream after any reconnect.
Even when enabled it waits 1.5s before showing anything (a healthy start shows
nothing at all) and disappears for good once the overlay goes live — someone
leaving the channel mid-stream never brings it back.

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
2. Injects the parser-blocking RPC bridge script before Streamkit's bundle:
   - `/web/rpc-bridge.js` rewrites Discord's local websocket connections to `/rpc/<port>/`
3. Injects `/css` as a stylesheet link, last in `<head>` so it wins over Streamkit's own CSS:
   - Forces a transparent background
   - Hides default avatars
   - Shows custom images for mapped `data-userid` values
   - Swaps speaking art + bounce animation when Discord adds speaking classes
4. Injects `/web/hot-reload.js`, which watches `/reload-events` and reloads the overlay when config changes

The HTML, CSS, and JavaScript source files live under `web/`. They are embedded at
compile time, so standalone binaries do not need a separate `web/` folder at runtime.
Editing those files requires rebuilding the binary; editing `config.toml` still hot-reloads.

## Project layout

```
src/
  main.rs      # bootstrap + logging
  config.rs    # TOML config + UserMap
  css.rs       # renders web/overlay.css + web/user.css for the current users
  proxy.rs     # fetch HTML / assets + inject
  reload.rs    # config.toml watcher + hot-reload signal
  rpc.rs       # websocket bridge to the local Discord client
  routes.rs    # Axum routes
  web.rs       # compile-time web templates/assets and small render helpers
web/
  index.html   # landing page template
  index.css    # landing page stylesheet
  overlay.css  # shared overlay stylesheet
  user.css     # per-user overlay CSS template
  preload.css  # avatar cache warmer template
  status.css   # diagnostic status pill (opt-in)
  status.js    # decides what the status pill says
  overlay-head.html
  status-head.html
  rpc-bridge-head.html
  rpc-bridge.js
  hot-reload.js
config.toml    # local settings (edit me)
```

## Notes for TikTok Live Studio

- Use **transparent** background in the browser source if the host supports it
- Prefer `127.0.0.1` over `localhost` if the embed sandbox is picky
- Avatar image URLs must be reachable from the browser that loads the overlay (public HTTPS or local static files)

## License

MIT (or your choice) — local tool, no affiliation with Discord or TikTok.
