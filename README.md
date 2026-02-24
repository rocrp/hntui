# hntui

Hacker News TUI (top stories + nested comments) using the official Firebase API.

## Screenshots

![Stories view](screenshots/hntui1.png)
![Comments view](screenshots/hntui2.png)

## Install

Homebrew (macOS + Linux Homebrew):

```bash
brew install rocrp/tap/hntui
```

Linux (no Homebrew):

```bash
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/scripts/install.sh | bash
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

## Configuration

### Config file locations

hntui searches for config files in this order (first match wins):

1. Current working directory (`./ui-config.toml`, `./plugin-config.toml`)
2. Directory of the `hntui` binary
3. `~/.config/hntui/` (recommended for persistent user config)

To install configs to the standard location:

```bash
mkdir -p ~/.config/hntui
```

You can also pass paths explicitly:

```
hntui --ui-config /path/to/ui-config.toml --plugin-config /path/to/plugin-config.toml
```

### UI customization (`ui-config.toml`)

Controls colors, layout, and score/comment heat scales. Copy the default config as a starting point:

```bash
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/ui-config.toml \
  -o ~/.config/hntui/ui-config.toml
```

Key options:

| Section | Key | Description |
|---|---|---|
| `[layout]` | `comment_max_lines` | Max lines per comment (`-1` = no limit) |
| `[layout]` | `comment_default_visible_levels` | Depth shown on open (`1` = top-level only) |
| `[palette]` | color keys | Hex colors for text, backgrounds, accents |
| `[palette]` | `rainbow` | 10-color list for comment depth + score accents |
| `[score_scale]` | `steps` | Background color per score threshold |
| `[comment_scale]` | `steps` | Background color per comment-count threshold |

### AI summarization (`plugin-config.toml`)

Press `s` on any story to summarize its discussion thread. Requires an LLM API key.

```bash
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/plugin-config.toml \
  -o ~/.config/hntui/plugin-config.toml
```

Set your API key via environment variable (preferred):

```bash
export HNTUI_LLM_API_KEY="your-key-here"
```

Or set `api_key` directly in `plugin-config.toml`.

The default config uses Gemini (`gemini-flash-lite-latest`) via its OpenAI-compatible endpoint. To use OpenAI or any other OpenAI-compatible API, change `api_url` and `model`:

```toml
[summarize]
api_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
```
