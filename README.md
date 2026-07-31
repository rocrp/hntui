# hntui

Hacker News TUI — top stories + nested comments.

## Demo

![demo](screenshots/demo.gif)

## Screenshots

![Stories view](screenshots/stories.png)
![Comments view](screenshots/comments.png)

## Install

```bash
# macOS / Linux Homebrew
brew install rocrp/tap/hntui

# Linux (no Homebrew)
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/scripts/install.sh | bash
```

## Keys

**Stories**

| Key | Action |
|-----|--------|
| `j/k`, `↓/↑` | Move |
| `gg` / `G` | Top / bottom |
| `Ctrl+d/u` | Page down / up |
| `Enter`, `Space`, `l`, `→` | Open comments |
| `o` / `O` | Open source / HN link |
| `f` | Filter feed |
| `/` | Search |
| `s` | Summarize (requires LLM key) |
| `v` | View article (requires localwebrs) |
| `r` | Refresh |
| `,` | Settings |
| `?` | Help |
| `q`, `Esc` | Quit |

**Comments**

| Key | Action |
|-----|--------|
| `j/k`, `↓/↑` | Move |
| `gg` / `G` | Top / bottom |
| `Ctrl+d/u` | Page down / up |
| `h/l`, `←/→` | Collapse / expand thread |
| `Enter`, `c` | Toggle collapse |
| `o` / `O` | Open HN / source link |
| `y` | Copy selected comment |
| `s` | Summarize (requires LLM key) |
| `v` | View article (requires localwebrs) |
| `r` | Refresh |
| `,` | Settings |
| `q`, `Esc` | Back |

**Article** (`v`)

| Key | Action |
|-----|--------|
| `j/k`, `↓/↑` | Scroll |
| `gg` / `G` | Top / bottom |
| `Ctrl+d/u`, `PgDn/PgUp` | Page down / up |
| `c` | Copy article to clipboard |
| `o` | Open the original (browser) |
| `q`, `Esc` | Close (cancels a running fetch) |

**Touch / Mouse** (Termux, etc.)

| Gesture | Action |
|---------|--------|
| Tap item | Select it |
| Tap selected item | Open comments / toggle collapse |
| Scroll up/down | Move selection |
| Tap title bar (comments) | Go back |
| Tap outside popup | Dismiss |

## Configuration

The UI uses a fixed Catppuccin Frappé theme.

### Config search order

1. Current working directory
2. `hntui` binary directory
3. `~/.config/hntui/` (recommended)

Explicit config path: `hntui --config PATH`

### AI summarization (`config.toml`)

Press `s` on any story to summarize its discussion. Requires an LLM API key.

```bash
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/config.toml \
  -o ~/.config/hntui/config.toml
```

```bash
export HNTUI_LLM_API_KEY="your-key-here"
```

Or set `api_key` in `config.toml`. Default uses Gemini (`gemini/gemini-flash-lite-latest`). For OpenAI:

```toml
[summarize]
model = "openai/gpt-4o-mini"
```

The `model` field uses smolllm's `provider/model_name` format. Comma-separate
for fallback (`"openai/gpt-4o, gemini/gemini-flash-lite-latest"`). See
[smolllm-rs](https://github.com/rocrp/smolllm-rs) for the full provider list.
Optional `base_url` overrides the provider's default endpoint. With `base_url`
set, a bare model name without the `provider/` prefix is also accepted (e.g.
`model = "qwen3"`); the key must then come from `HNTUI_LLM_API_KEY` or
`api_key`, and the one `base_url` applies to every leg of a fallback list.

#### Base URL grammar

For standard and custom OpenAI-compatible routes, `base_url` resolves as follows:

| Behavior | Input `base_url` | Resolved endpoint |
|----------|------------------|-------------------|
| Trailing `#`: use the URL verbatim as the full endpoint; remove the marker | `https://gateway.example/openai/chat/completions#` | `https://gateway.example/openai/chat/completions` |
| Trailing `/`: append `chat/completions` without injecting a version | `https://gateway.example/openai/` | `https://gateway.example/openai/chat/completions` |
| Final version segment (`/v1`, `/v3`, …): append `/chat/completions` | `https://gateway.example/v3` | `https://gateway.example/v3/chat/completions` |
| Otherwise: append `/v1/chat/completions` | `https://gateway.example/openai` | `https://gateway.example/openai/v1/chat/completions` |

Built-in Anthropic and Gemini providers retain their provider-specific path
injection. While editing Model or Base URL, check the live ResolvedEndpoint
preview for the exact POST URL, then select `[ Test connection ]` to verify the
draft configuration.

`hntui` auto-loads `~/.env.smolllm` if it exists (process env always wins).
Pass `--env-file <path>` to load a different file explicitly.

### Articles (`config.toml`)

Press `v` on any story to read the linked page as text — or the post's own body
for an Ask HN. Article text also grounds the summary by default.

Both need [localwebrs](https://github.com/rocrp/localwebrs) on your `PATH`:

```bash
cargo install --git https://github.com/rocrp/localwebrs
```

```toml
[summarize]
include_article = true    # feed the article to the summarizer (default)
max_article_chars = 20000 # head-truncated at this many characters

[article]
bin = "localwebrs"        # override if it is not on PATH
```

Without localwebrs, `hntui` works as before: `v` reports the missing binary, and
`s` summarizes the comments alone with a banner saying the article was skipped.
Self-posts need no subprocess at all.

## Development

Requires [just](https://github.com/casey/just) 1.52 or newer. Run `just` to list
the available project actions. Common commands:

```bash
just check
just build
just screenshots
```

Cut and publish a release through the canonical release script:

```bash
just release 0.5.2
```

This requires a clean working tree and creates the release commit and tag before
atomically pushing both.
