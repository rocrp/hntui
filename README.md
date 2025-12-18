# hntui

Hacker News TUI (top stories + nested comments) using the official Firebase API.

## Run

```bash
cargo run --release
```

Options: `--count`, `--page-size`, `--cache-size`, `--concurrency`, `--no-file-cache`, `--file-cache-dir`, `--file-cache-ttl-secs`, `--base-url`

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
- `o`: open story in browser
- `r`: refresh
- `q` / `Esc`: quit

Comments:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `h` / `←`: collapse selected thread
- `l` / `→`: expand selected thread (lazy-load children)
- `c`: toggle collapse/expand
- `o`: open story in browser
- `r`: refresh
- `q` / `Esc`: back
