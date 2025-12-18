# hntui

Hacker News TUI (top stories + nested comments) using the official Firebase API.

## Run

```bash
cargo run --release
```

Options: `--count`, `--page-size`, `--cache-size`, `--concurrency`, `--no-file-cache`, `--file-cache-dir`, `--file-cache-ttl-secs`, `--base-url`

## Keys

Stories:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `Enter`: open comments
- `o`: open story in browser
- `r`: refresh
- `q` / `Esc`: quit

Comments:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `h/l` or `←/→` or `c`: collapse/expand selected thread
- `o`: open story in browser
- `r`: refresh
- `q` / `Esc`: back
