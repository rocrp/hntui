# hntui

Hacker News TUI (top stories + nested comments) using the official Firebase API.

## Screenshots

![Stories view](screenshots/hntui1.png)
![Comments view](screenshots/hntui2.png)

## Install

```bash
brew install rocrp/tap/hntui
```

Run: `hntui`

## Keys

Stories:
- `j/k` or `↓/↑`: move
- `gg` / `G`: top / bottom
- `Ctrl+d` / `Ctrl+u`: page down / up
- `Enter` / `Space` / `l` / `→`: open comments
- `o`: open source link in browser
- `O`: open comments page in browser
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
- `o`: open comments page in browser
- `O`: open source link in browser
- `r`: refresh
- `?`: help
- `q` / `Esc`: back
