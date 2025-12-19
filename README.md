# hntui

Hacker News TUI (top stories + nested comments) using the official Firebase API.

## Screenshots

![Stories view](screenshots/hntui1.png)
![Comments view](screenshots/hntui2.png)

## Run

```bash
cargo run --release
```

Options: `--count`, `--page-size`, `--cache-size`, `--concurrency`, `--no-file-cache`, `--file-cache-dir`, `--file-cache-ttl-secs`, `--base-url`, `--ui-config`

UI config: `ui-config.toml` (TOML, comments supported)

## Cache

- Disk cache: HN items + story list state (restore instantly, refresh in background)
- TTL (items only): `--file-cache-ttl-secs` (default 3600)
- Disable: `--no-file-cache`

## Keys

Stories:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `Enter` / `Space` / `l` / `→`: open comments
- `o`: open source link in browser
- `r`: refresh
- `?`: help
- `q` / `Esc`: quit

Comments:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `h` / `←`: collapse selected thread
- `l` / `→`: expand selected thread (lazy-load children)
- `Enter` / `c`: toggle collapse/expand
- `o`: open source link in browser
- `r`: refresh
- `?`: help
- `q` / `Esc`: back
