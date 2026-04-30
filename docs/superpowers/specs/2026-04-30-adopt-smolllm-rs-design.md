# Adopt smolllm-rs in hntui

**Date:** 2026-04-30
**Status:** Approved (design)
**Scope:** Replace hntui's hand-rolled OpenAI streaming client (`src/plugin/llm.rs`) with the `smolllm` crate from `../smolllm-rs`. Refactor smolllm-rs only if adoption blocks on it.

## Goals

- Delete `src/plugin/llm.rs` (~140 LOC of hand-rolled SSE).
- Use `smolllm::stream(...)` for chat completions in the summarize plugin.
- Adopt smolllm's `provider/model_name` config convention (breaking change to user TOML).
- Surface reasoning tokens (from thinking models) as dimmed live preview that swaps out once content arrives.
- Route smolllm's `log` macros into hntui's existing file logger.

## Non-goals

- Refactoring smolllm-rs internals. Discover-as-you-go: if integration hits a wall, surface it; otherwise leave the lib untouched.
- Image input, telemetry hooks, token-usage overlay display.
- Backward compatibility for existing `[summarize]` configs. CLAUDE.md says break old formats freely; an early-stage TUI does not need a migration path.
- Publishing smolllm to crates.io or fetching it via git. Path dependency only.

## Architecture

```
hntui/Cargo.toml
  + smolllm = { path = "../smolllm-rs" }
  + log = "0.4"

hntui/src/
  ├ main.rs                 # call logging::init_log_bridge()
  ├ logging.rs              # extend with log::Log adapter
  └ plugin/
      ├ llm.rs              # DELETE
      ├ summarize.rs        # rewrite stream loop
      ├ config.rs           # new schema
      └ mod.rs              # PluginEvent::SummarizeChunk gains reasoning field
```

Net effect: smolllm owns transport (SSE parsing, retries, model fallback, provider URL resolution). hntui owns prompting, UI, and config plumbing.

## Config schema (breaking change)

`~/.config/hntui/config.toml`:

```toml
[summarize]
model = "openai/gpt-4o-mini"   # required, smolllm provider/name format
                               # comma-separated for fallback: "openai/gpt-4o, gemini/flash"
api_key = "..."                # optional override
base_url = "..."               # optional override
max_comments = 200
system_prompt = "..."
```

Removed: `api_url`. URL is now built by smolllm from provider name plus optional `base_url` override.

API key resolution precedence (highest first):
1. `HNTUI_LLM_API_KEY` env var (kept as explicit hntui-scoped override)
2. config `api_key` field
3. smolllm-resolved `{PROVIDER}_API_KEY` env var (e.g. `OPENAI_API_KEY`)

`SummarizeConfig::resolve_api_key()` returns `Option<String>` for items 1+2. When `None`, the call site simply does not invoke `.api_key(...)` on the smolllm builder, letting smolllm fall through to its own env-var resolution.

## Data flow

`PluginEvent` change:

```rust
pub enum PluginEvent {
    SummarizeChunk { content: String, reasoning: String },
    SummarizeComplete,
    SummarizeError { message: String },
}
```

`summarize.rs` stream loop replaces the current `stream_chat_completion` task:

```rust
let mut builder = smolllm::stream(prompt)
    .model(&config.model)
    .system_prompt(&config.system_prompt)
    .http_client(http);
if let Some(key) = resolved_key.as_deref() {
    builder = builder.api_key(key);
}
if let Some(url) = config.base_url.as_deref() {
    builder = builder.base_url(url);
}

match builder.await {
    Ok(stream) => {
        tokio::pin!(stream);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(c) => { let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeChunk {
                    content: c.content, reasoning: c.reasoning,
                })); }
                Err(e) => { let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeError {
                    message: format!("{e:#}"),
                })); return; }
            }
        }
        let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeComplete));
    }
    Err(e) => {
        let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeError {
            message: format!("{e:#}"),
        }));
    }
}
```

## Reasoning UI

Plugin state additions:

```rust
pub struct SummarizePlugin {
    // ... existing fields ...
    pub reasoning_buffer: String,
    content_started: bool,
}
```

Event handler:

```rust
PluginEvent::SummarizeChunk { content, reasoning } => {
    if !reasoning.is_empty() && !self.content_started {
        self.reasoning_buffer.push_str(&reasoning);
    }
    if !content.is_empty() {
        self.content_started = true;
        self.summary_text.push_str(&content);
        if self.state == SummarizeState::Loading {
            self.state = SummarizeState::Streaming;
        }
    }
}
```

Render rule (overlay view):
- `summary_text` empty AND `reasoning_buffer` non-empty → render `reasoning_buffer` with `Modifier::DIM` under a "Thinking…" label.
- `summary_text` non-empty → render `summary_text` normally; reasoning hidden.
- `dismiss()` resets `reasoning_buffer` and `content_started`.

## Log bridge

`src/logging.rs` gains:

```rust
struct LogAdapter;

impl log::Log for LogAdapter {
    fn enabled(&self, m: &log::Metadata) -> bool { m.level() <= log::Level::Info }
    fn log(&self, record: &log::Record) {
        let line = format!("[{}] {}", record.target(), record.args());
        match record.level() {
            log::Level::Error => log_error(line),
            _ => log_info(line),
        }
    }
    fn flush(&self) {}
}

static LOG_ADAPTER: LogAdapter = LogAdapter;

pub fn init_log_bridge() {
    let _ = log::set_logger(&LOG_ADAPTER)
        .map(|_| log::set_max_level(log::LevelFilter::Info));
}
```

Called from `main.rs` once, after `logging::init(...)`. smolllm's retry warnings, model-fallback notices, and metrics line (`📊model …tok | 🚀TTFT | 🐎tok/s | ⌛duration`) then appear in the same file as hntui's own `INFO`/`ERROR` lines.

## Errors

`smolllm::Error` is mapped via `format!("{e:#}")` into the existing `SummarizeError { message }` event. smolllm internally retries 429/500/502/503/529 with exponential backoff (3 attempts) and falls back across comma-separated models in `model`, so hntui sees only the final outcome.

## Testing

- Unit test: `SummarizeConfig` parses new TOML correctly (`config.rs`).
- Build & typecheck: `cargo check`, `cargo build --release`.
- Manual smoke test:
  - Set `model = "openai/gpt-4o-mini"` and `HNTUI_LLM_API_KEY`. Trigger summarize on a story with comments. Confirm content streams.
  - Set `model = "openai/o1-mini"` (or another reasoning model). Confirm dimmed "Thinking…" preview appears, then is replaced by content.
  - Set `model = "openai/bogus"` with no API key. Confirm error overlay shows the smolllm error message.
  - Set `model = "openai/bad, openai/gpt-4o-mini"` (fallback). Confirm summarize succeeds via the second model and the log file shows the fallback notice.
- No live-API integration test in CI.

## Refactor candidates in smolllm-rs (tracked, not actioned)

Surfaced for future reference per discover-as-you-go decision:
- `src/client.rs` is 689 LOC mixing ask, stream, SSE parsing, retry, and request building. Splitting may help.
- Provider list inline in `src/provider.rs` (~60 entries) — verbose; could move to a JSON file like the Python sibling.
- Crate name `smolllm` versus directory `smolllm-rs` — minor mismatch.
- No top-level `tests/` directory; only inline `#[cfg(test)] mod tests`.

These are not part of this work. They will be revisited only if adoption uncovers a concrete blocker.
