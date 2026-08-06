# Local avatar images

Drop idle / speaking images here. They are served at:

```
http://127.0.0.1:3000/assets/<filename>
```

## Example

```
assets/
  alice-idle.png
  alice-speaking.png
  team/
    bob-idle.webp
    bob-speaking.webp
```

In `config.toml`:

```toml
[users.123456789012345678]
order = 1
idle_url = "alice-idle.png"
speaking_url = "alice-speaking.png"

[users.987654321098765432]
order = 2
idle_url = "team/bob-idle.webp"
speaking_url = "team/bob-speaking.webp"
```

Lower `order` values render first. If omitted, the value defaults to `0`.

Supported formats: PNG, WebP, GIF, JPG, SVG — anything the browser can show in CSS `content: url(...)`.

**Tip:** Prefer transparent PNGs/WebPs sized for your overlay (e.g. 256×256 or 512×512).
