# Adopt smolllm-rs in hntui — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hntui's hand-rolled OpenAI streaming client with the `smolllm` crate at `../smolllm-rs`, switch the user TOML config to smolllm's `provider/model` convention, surface reasoning tokens as a dimmed "Thinking…" preview in the summary overlay, and route smolllm's `log` macros into hntui's existing file logger.

**Architecture:** smolllm owns transport (SSE parsing, retries, model fallback, provider URL resolution); hntui owns prompting, UI, and config plumbing. The summarize plugin issues `smolllm::stream(...)`, consumes a `Stream<Item = Result<StreamChunk, Error>>`, and forwards `(content, reasoning)` deltas through the existing `PluginEvent` channel.

**Tech Stack:** Rust 2021, ratatui 0.29, tokio 1, smolllm (path dep), log 0.4, toml 0.8.

**Spec:** [docs/superpowers/specs/2026-04-30-adopt-smolllm-rs-design.md](../specs/2026-04-30-adopt-smolllm-rs-design.md)

**Test model for manual smoke tests:** `gemini/gemini-flash-lite-latest` (set `HNTUI_LLM_API_KEY` or `GEMINI_API_KEY`).

**Conventions:**
- Conventional commits.
- Never run `cargo build` or other long commands without checking they compile incrementally with `cargo check` first.
- Use existing CLAUDE.md rule: break old config formats freely.

---

## Files map

**Create:**
- (none — all changes extend or replace existing files)

**Modify:**
- `Cargo.toml` — add `smolllm` (path dep) and `log` deps.
- `src/logging.rs` — add `log::Log` adapter and `init_log_bridge()`.
- `src/main.rs` — call `logging::init_log_bridge()` after `logging::init`.
- `src/plugin/config.rs` — replace `api_url` with `base_url`; both `api_key` and `base_url` become `Option<String>`.
- `src/plugin/mod.rs` — `PluginEvent::SummarizeChunk { content, reasoning }`.
- `src/plugin/summarize.rs` — use `smolllm::stream(...)`; track `reasoning_buffer` + `content_started`.
- `src/ui/plugin_overlay.rs` — render dimmed reasoning when content empty.
- `src/app.rs` — `SettingsPopup` fields (`api_url` → `base_url`, both optional); `save_settings` writes the new schema.
- `src/ui/settings.rs` — relabel "API URL" → "Base URL".
- `plugin-config.toml` — example config rewritten for new schema.
- `README.md` — update the AI summarization docs section.

**Delete:**
- `src/plugin/llm.rs` (~140 LOC of hand-rolled SSE).

---

## Task 1: Add smolllm and log dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Edit Cargo.toml**

Add to the `[dependencies]` section (preserve existing entries; the two new lines may go anywhere alphabetically):

```toml
smolllm = { path = "../smolllm-rs" }
log = "0.4"
```

- [ ] **Step 2: Verify it resolves**

Run: `cargo check`
Expected: compiles. New deps appear in `Cargo.lock`. The crate-name vs directory mismatch is fine — Cargo uses the `name` from `../smolllm-rs/Cargo.toml`, which is `smolllm`.

If `cargo check` fails because hntui is using fields that no longer exist (e.g. once tasks 4-7 land), that's expected — only worry here is that the deps themselves resolve. Check the first few lines of output for resolution issues, not later compile errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(deps): add smolllm path dep and log crate"
```

---

## Task 2: Add log::Log adapter to logging.rs

**Files:**
- Modify: `src/logging.rs`

- [ ] **Step 1: Add the adapter type and init function**

Append to `src/logging.rs` (after the existing `open_log_file_at` function):

```rust
struct LogAdapter;

impl log::Log for LogAdapter {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("[{}] {}", record.target(), record.args());
        match record.level() {
            log::Level::Error => log_error(line),
            _ => log_info(line),
        }
    }

    fn flush(&self) {}
}

static LOG_ADAPTER: LogAdapter = LogAdapter;

/// Install the global `log` crate sink so library logs (e.g. smolllm's retry
/// warnings and metrics) flow through the same file as hntui's own logs.
pub fn init_log_bridge() {
    if log::set_logger(&LOG_ADAPTER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles. No `unused` warnings on the new items (they're re-exported via `pub fn`).

- [ ] **Step 3: Commit**

```bash
git add src/logging.rs
git commit -m "feat(logging): add log::Log adapter routing to file logger"
```

---

## Task 3: Wire log bridge into main.rs

**Files:**
- Modify: `src/main.rs:166-172` (the `main` fn body)

- [ ] **Step 1: Add the call**

Find this block in `main`:

```rust
    logging::init(cli.log_file.clone()).context("init logging")?;
```

Insert the bridge call on the next line:

```rust
    logging::init(cli.log_file.clone()).context("init logging")?;
    logging::init_log_bridge();
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(logging): install log bridge at startup"
```

---

## Task 4: Replace SummarizeConfig schema (TDD)

**Files:**
- Modify: `src/plugin/config.rs`

- [ ] **Step 1: Write the failing parse test**

Append to the bottom of `src/plugin/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let src = r#"
            [summarize]
            model = "gemini/gemini-flash-lite-latest"
        "#;
        let cfg: PluginConfig = toml::from_str(src).expect("parse");
        let s = cfg.summarize.expect("summarize present");
        assert_eq!(s.model, "gemini/gemini-flash-lite-latest");
        assert!(s.api_key.is_none());
        assert!(s.base_url.is_none());
        assert_eq!(s.max_comments, 200);
        assert!(s.system_prompt.contains("Summarize"));
    }

    #[test]
    fn parses_full_config() {
        let src = r#"
            [summarize]
            model = "openai/gpt-4o-mini"
            api_key = "sk-test"
            base_url = "https://example.com"
            max_comments = 50
            system_prompt = "be terse"
        "#;
        let cfg: PluginConfig = toml::from_str(src).expect("parse");
        let s = cfg.summarize.expect("summarize present");
        assert_eq!(s.api_key.as_deref(), Some("sk-test"));
        assert_eq!(s.base_url.as_deref(), Some("https://example.com"));
        assert_eq!(s.max_comments, 50);
        assert_eq!(s.system_prompt, "be terse");
    }

    #[test]
    fn resolve_api_key_prefers_env_var() {
        // Use a unique env var key per test to avoid clashing with other tests.
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::set_var(var, "from-env");
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: Some("from-config".into()),
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert_eq!(cfg.resolve_api_key().as_deref(), Some("from-env"));
        match prev {
            Some(v) => std::env::set_var(var, v),
            None => std::env::remove_var(var),
        }
    }

    #[test]
    fn resolve_api_key_falls_back_to_config() {
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::remove_var(var);
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: Some("from-config".into()),
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert_eq!(cfg.resolve_api_key().as_deref(), Some("from-config"));
        if let Some(v) = prev {
            std::env::set_var(var, v);
        }
    }

    #[test]
    fn resolve_api_key_returns_none_when_unset() {
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::remove_var(var);
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: None,
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert!(cfg.resolve_api_key().is_none());
        if let Some(v) = prev {
            std::env::set_var(var, v);
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hntui plugin::config -- --test-threads=1`
Expected: FAIL with errors like `no field "api_key" of type Option<String> on struct SummarizeConfig` (current schema has `api_key: String`).

The `--test-threads=1` is needed because the env-var tests mutate process-global state.

- [ ] **Step 3: Update `SummarizeConfig` to the new schema**

Replace the existing `SummarizeConfig` struct and `resolve_api_key` impl in `src/plugin/config.rs` with:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummarizeConfig {
    /// smolllm `provider/model` string (or comma-separated for fallback).
    pub model: String,

    /// Optional API key override. If `None`, smolllm resolves from
    /// `{PROVIDER}_API_KEY` env var.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Optional base URL override. If `None`, smolllm uses the provider's
    /// default (or `{PROVIDER}_BASE_URL` env var).
    #[serde(default)]
    pub base_url: Option<String>,

    #[serde(default = "default_max_comments")]
    pub max_comments: usize,

    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

impl SummarizeConfig {
    /// Resolve API key: env var `HNTUI_LLM_API_KEY` > config field > `None`
    /// (in which case smolllm will try `{PROVIDER}_API_KEY` itself).
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Ok(key) = std::env::var("HNTUI_LLM_API_KEY") {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
        self.api_key.as_ref().and_then(|k| {
            let trimmed = k.trim();
            (!trimmed.is_empty()).then(|| k.clone())
        })
    }
}
```

The old fields `api_url` and `api_key: String` are gone.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p hntui plugin::config -- --test-threads=1`
Expected: all 5 tests pass. (Other parts of the crate will fail to compile because callers still reference `api_url`. That's fine — tasks 5 and 9 fix them.)

If `cargo test` won't run because the rest of the crate doesn't compile, run only the unit module:
`cargo test -p hntui --lib plugin::config -- --test-threads=1` (won't help if hntui is a binary crate). In that case, accept that the test step is verified once Task 9 lands and re-run it as part of Task 10.

- [ ] **Step 5: Commit**

```bash
git add src/plugin/config.rs
git commit -m "feat(plugin/config)!: switch to smolllm provider/model schema

- model is now smolllm 'provider/name' (or comma-separated fallback)
- api_key and base_url become optional overrides
- api_url field removed
- HNTUI_LLM_API_KEY env still wins over config api_key

BREAKING CHANGE: existing [summarize] configs with api_url no longer parse"
```

---

## Task 5: Update SettingsPopup struct and labels

**Files:**
- Modify: `src/app.rs:85-159` (SettingsPopup definition and helpers), `src/app.rs:2127-2160` (save_settings)
- Modify: `src/ui/settings.rs` (no field-name change needed since it reads through `field_values`)

- [ ] **Step 1: Replace `api_url` with `base_url` in `SettingsPopup`**

In `src/app.rs`, change the `SettingsPopup` struct:

```rust
pub struct SettingsPopup {
    pub cursor: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub edit_cursor: usize,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub max_comments: String,
    pub system_prompt: String,
    pub saved_at: Option<Instant>,
}
```

(Field order changed: `model`, `api_key`, `base_url` — model first because it's now the primary required field.)

Update `from_config`:

```rust
    pub fn from_config(config: &Option<SummarizeConfig>) -> Self {
        match config {
            Some(c) => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                edit_cursor: 0,
                model: c.model.clone(),
                api_key: c.api_key.clone().unwrap_or_default(),
                base_url: c.base_url.clone().unwrap_or_default(),
                max_comments: c.max_comments.to_string(),
                system_prompt: c.system_prompt.clone(),
                saved_at: None,
            },
            None => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                edit_cursor: 0,
                model: String::new(),
                api_key: String::new(),
                base_url: String::new(),
                max_comments: "200".to_string(),
                system_prompt: String::new(),
                saved_at: None,
            },
        }
    }
```

Update labels and value/mut helpers:

```rust
    pub fn field_labels(&self) -> [&str; Self::FIELD_COUNT] {
        [
            "Model",
            "API Key",
            "Base URL",
            "Max Comments",
            "System Prompt",
        ]
    }

    pub fn field_values(&self) -> [&str; Self::FIELD_COUNT] {
        [
            &self.model,
            &self.api_key,
            &self.base_url,
            &self.max_comments,
            &self.system_prompt,
        ]
    }

    fn field_mut(&mut self, idx: usize) -> &mut String {
        match idx {
            0 => &mut self.model,
            1 => &mut self.api_key,
            2 => &mut self.base_url,
            3 => &mut self.max_comments,
            4 => &mut self.system_prompt,
            _ => unreachable!(),
        }
    }
```

The masking logic in `src/ui/settings.rs:35-44` keys off `i == 2` to mask the API key. After the field reorder, the API key is now `i == 1`. Update the condition:

In `src/ui/settings.rs` find this line:

```rust
        } else if i == 2 && !values[i].is_empty() {
```

Replace with:

```rust
        } else if i == 1 && !values[i].is_empty() {
```

- [ ] **Step 2: Update `save_settings` in `src/app.rs`**

Replace the `save_settings` body's `SummarizeConfig` construction:

```rust
        let api_key = if popup.api_key.trim().is_empty() {
            None
        } else {
            Some(popup.api_key.clone())
        };
        let base_url = if popup.base_url.trim().is_empty() {
            None
        } else {
            Some(popup.base_url.clone())
        };

        let config = SummarizeConfig {
            model: popup.model.clone(),
            api_key,
            base_url,
            max_comments,
            system_prompt,
        };
```

- [ ] **Step 3: Verify it compiles (config-level only)**

Run: `cargo check`
Expected: hntui still won't fully compile because `summarize.rs` references `config.api_url` and `PluginEvent::SummarizeChunk` doesn't yet carry `reasoning`. Just confirm the new `SettingsPopup` errors are gone.

- [ ] **Step 4: Commit**

```bash
git add src/app.rs src/ui/settings.rs
git commit -m "feat(settings)!: replace API URL field with Base URL

Settings popup now edits model/api_key/base_url to match new
SummarizeConfig schema. API key masking shifted to index 1."
```

---

## Task 6: Extend PluginEvent::SummarizeChunk with reasoning

**Files:**
- Modify: `src/plugin/mod.rs`

- [ ] **Step 1: Replace the enum variant**

In `src/plugin/mod.rs`:

```rust
#[derive(Debug)]
pub enum PluginEvent {
    SummarizeChunk { content: String, reasoning: String },
    SummarizeComplete,
    SummarizeError { message: String },
}
```

- [ ] **Step 2: Verify the change is consistent**

Run: `cargo check`
Expected: compile errors point to `plugin/llm.rs` and `plugin/summarize.rs` constructing/matching `SummarizeChunk { delta }`. These will be fixed in Task 7 and 8.

- [ ] **Step 3: Commit**

```bash
git add src/plugin/mod.rs
git commit -m "feat(plugin)!: SummarizeChunk now carries content + reasoning"
```

---

## Task 7: Rewrite summarize.rs to use smolllm

**Files:**
- Modify: `src/plugin/summarize.rs`

- [ ] **Step 1: Update imports and add reasoning state**

At the top of `src/plugin/summarize.rs`:

```rust
use crate::app::AppEvent;
use crate::plugin::config::SummarizeConfig;
use crate::plugin::{PluginContext, PluginEvent};
use crate::ui::comment_view::hn_html_to_plain;
use anyhow::anyhow;
use futures::StreamExt;
use std::time::Instant;
```

Drop the `use crate::plugin::llm::{stream_chat_completion, ChatMessage};` line.

In the `SummarizePlugin` struct (after `pub content_height: usize,`), add:

```rust
    pub reasoning_buffer: String,
    content_started: bool,
```

In `SummarizePlugin::new`, initialize them:

```rust
            reasoning_buffer: String::new(),
            content_started: false,
```

In `dismiss(&mut self)`, reset them alongside the existing fields:

```rust
        self.reasoning_buffer.clear();
        self.content_started = false;
```

- [ ] **Step 2: Replace the `start` method's spawn block**

Find this section in `start()`:

```rust
        let prompt = build_prompt(story, ctx.comment_list, config.max_comments);
        let messages = vec![
            ChatMessage {
                role: "system",
                content: config.system_prompt.clone(),
            },
            ChatMessage {
                role: "user",
                content: prompt,
            },
        ];

        let http = self.http.clone();
        let api_url = config.api_url.clone();
        let model = config.model.clone();
        let tx = ctx.tx.clone();

        tokio::spawn(async move {
            stream_chat_completion(&http, &api_url, &api_key, &model, messages, tx).await;
        });
```

Replace it with:

```rust
        let prompt = build_prompt(story, ctx.comment_list, config.max_comments);
        let system_prompt = config.system_prompt.clone();
        let model = config.model.clone();
        let base_url = config.base_url.clone();
        let http = self.http.clone();
        let tx = ctx.tx.clone();

        tokio::spawn(async move {
            run_stream(http, model, system_prompt, prompt, api_key, base_url, tx).await;
        });
```

Where `api_key: Option<String>` already comes from `config.resolve_api_key()` — but the current `start` calls `let Some(api_key) = config.resolve_api_key() else { ... }`. Loosen that branch: `resolve_api_key()` may legitimately be `None` now (smolllm will fall through to `{PROVIDER}_API_KEY`). So replace:

```rust
        let Some(api_key) = config.resolve_api_key() else {
            self.state = SummarizeState::Error;
            self.error = Some(
                "API key not set. Press , for settings or set HNTUI_LLM_API_KEY env var"
                    .to_string(),
            );
            return;
        };
```

with:

```rust
        let api_key = config.resolve_api_key();
```

(smolllm reports a clear `MissingApiKey` error if neither config nor env var is set.)

- [ ] **Step 3: Add `run_stream` helper**

Append to the file (before `fn build_prompt`):

```rust
async fn run_stream(
    http: reqwest::Client,
    model: String,
    system_prompt: String,
    user_prompt: String,
    api_key: Option<String>,
    base_url: Option<String>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    let result = stream_inner(
        http,
        &model,
        &system_prompt,
        user_prompt,
        api_key.as_deref(),
        base_url.as_deref(),
        &tx,
    )
    .await;
    match result {
        Ok(()) => {
            let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeComplete));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeError {
                message: format!("{e:#}"),
            }));
        }
    }
}

async fn stream_inner(
    http: reqwest::Client,
    model: &str,
    system_prompt: &str,
    user_prompt: String,
    api_key: Option<&str>,
    base_url: Option<&str>,
    tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let mut builder = smolllm::stream(user_prompt)
        .model(model)
        .system_prompt(system_prompt)
        .http_client(http);
    if let Some(key) = api_key {
        builder = builder.api_key(key);
    }
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }

    let mut stream = builder.await.map_err(|e| anyhow!("{e}"))?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("{e}"))?;
        if chunk.is_empty() {
            continue;
        }
        let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeChunk {
            content: chunk.content,
            reasoning: chunk.reasoning,
        }));
    }
    Ok(())
}
```

- [ ] **Step 4: Update the event handler**

Replace the existing `handle_event` body's `SummarizeChunk` arm:

```rust
            PluginEvent::SummarizeChunk { content, reasoning } => {
                if !reasoning.is_empty() && !self.content_started {
                    self.reasoning_buffer.push_str(&reasoning);
                    if self.state == SummarizeState::Loading {
                        self.state = SummarizeState::Streaming;
                    }
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

(Note: matching `SummarizeChunk { content, reasoning }` instead of `SummarizeChunk { delta }`.)

- [ ] **Step 5: Verify it compiles**

Run: `cargo check`
Expected: still fails with one error in `src/plugin/llm.rs` (still importing the old enum shape) and possibly `src/plugin/mod.rs` which still declares `pub mod llm;`. Task 8 deletes the file.

- [ ] **Step 6: Commit**

```bash
git add src/plugin/summarize.rs
git commit -m "feat(plugin/summarize): use smolllm::stream and surface reasoning"
```

---

## Task 8: Delete plugin/llm.rs

**Files:**
- Delete: `src/plugin/llm.rs`
- Modify: `src/plugin/mod.rs`

- [ ] **Step 1: Remove the `pub mod llm;` declaration**

In `src/plugin/mod.rs`, delete the line:

```rust
pub mod llm;
```

- [ ] **Step 2: Delete the file**

Run: `git rm src/plugin/llm.rs`

- [ ] **Step 3: Verify build is clean**

Run: `cargo check`
Expected: hntui compiles cleanly. No warnings about unused imports.

If there are still references, search for them:
`rg "plugin::llm|stream_chat_completion|ChatMessage" src/`
and remove any stragglers.

- [ ] **Step 4: Run tests**

Run: `cargo test -p hntui -- --test-threads=1`
Expected: all tests pass (config tests from Task 4 plus any pre-existing tests).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(plugin): drop hand-rolled llm.rs in favor of smolllm"
```

---

## Task 9: Render dimmed reasoning in the overlay

**Files:**
- Modify: `src/ui/plugin_overlay.rs`

- [ ] **Step 1: Update the lines builder**

Find this block in `pub fn render`:

```rust
    let lines: Vec<Line> = match state {
        SummarizeState::Loading => {
            vec![Line::from(Span::styled(
                format!("Waiting for LLM response {spinner}"),
                theme::HINT,
            ))]
        }
        SummarizeState::Streaming | SummarizeState::Done => {
            let mut l = markdown::render_markdown(&plugin.summary_text);
            if state == SummarizeState::Streaming {
                l.push(Line::from(Span::styled(format!("{spinner}"), theme::HINT)));
            }
            l
        }
        SummarizeState::Error => {
            let msg = plugin.error.as_deref().unwrap_or("Unknown error");
            vec![Line::from(Span::styled(msg.to_string(), theme::ERROR))]
        }
        SummarizeState::Idle => vec![],
    };
```

Replace it with:

```rust
    let lines: Vec<Line> = match state {
        SummarizeState::Loading => {
            if plugin.reasoning_buffer.is_empty() {
                vec![Line::from(Span::styled(
                    format!("Waiting for LLM response {spinner}"),
                    theme::HINT,
                ))]
            } else {
                reasoning_lines(&plugin.reasoning_buffer, spinner, true)
            }
        }
        SummarizeState::Streaming => {
            if plugin.summary_text.is_empty() {
                reasoning_lines(&plugin.reasoning_buffer, spinner, true)
            } else {
                let mut l = markdown::render_markdown(&plugin.summary_text);
                l.push(Line::from(Span::styled(format!("{spinner}"), theme::HINT)));
                l
            }
        }
        SummarizeState::Done => markdown::render_markdown(&plugin.summary_text),
        SummarizeState::Error => {
            let msg = plugin.error.as_deref().unwrap_or("Unknown error");
            vec![Line::from(Span::styled(msg.to_string(), theme::ERROR))]
        }
        SummarizeState::Idle => vec![],
    };
```

- [ ] **Step 2: Add the helper**

At the bottom of `src/ui/plugin_overlay.rs` add:

```rust
fn reasoning_lines(buffer: &str, spinner: char, streaming: bool) -> Vec<Line<'static>> {
    use ratatui::style::{Modifier, Style};

    let dim_style = Style::default()
        .fg(theme::OVERLAY0)
        .add_modifier(Modifier::DIM | Modifier::ITALIC);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let label = if streaming {
        format!("Thinking {spinner}")
    } else {
        "Thinking".to_string()
    };
    lines.push(Line::from(Span::styled(label, theme::HINT)));
    lines.push(Line::raw(""));

    for raw in buffer.lines() {
        lines.push(Line::from(Span::styled(raw.to_string(), dim_style)));
    }
    lines
}
```

`theme::OVERLAY0` is already declared at `src/ui/theme.rs` — change its visibility from `pub(crate)` to remain `pub(crate)` (already accessible from within crate). No theme change needed.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/ui/plugin_overlay.rs
git commit -m "feat(ui): show dimmed 'Thinking' preview from reasoning tokens"
```

---

## Task 10: Update example plugin-config.toml and README

**Files:**
- Modify: `plugin-config.toml`
- Modify: `README.md`

- [ ] **Step 1: Rewrite `plugin-config.toml`**

Replace the entire file contents with:

```toml
[summarize]
model = "gemini/gemini-flash-lite-latest"

# API key: set HNTUI_LLM_API_KEY env var (preferred) or uncomment below.
# Smolllm also accepts {PROVIDER}_API_KEY (e.g. GEMINI_API_KEY).
# api_key = ""

# Optional base URL override (defaults to provider's standard URL).
# Useful for proxies or self-hosted endpoints.
# base_url = ""

max_comments = 200
system_prompt = "Summarize this Hacker News discussion concisely. Highlight key arguments, disagreements, and consensus points."
```

- [ ] **Step 2: Update README's AI summarization section**

Find lines 105-125 (the "AI summarization (`plugin-config.toml`)" section):

```markdown
### AI summarization (`plugin-config.toml`)

Press `s` on any story to summarize its discussion. Requires an LLM API key.

...

Or set `api_key` in `plugin-config.toml`. Default uses Gemini (`gemini-flash-lite-latest`). For OpenAI or compatible APIs:

```toml
[summarize]
api_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
```
```

Replace with:

```markdown
### AI summarization (`plugin-config.toml`)

Press `s` on any story to summarize its discussion. Requires an LLM API key.

```bash
curl -fsSL https://raw.githubusercontent.com/rocrp/hntui/main/plugin-config.toml \
  -o ~/.config/hntui/plugin-config.toml
```

```bash
export HNTUI_LLM_API_KEY="your-key-here"
```

Or set `api_key` in `plugin-config.toml`. Default uses Gemini (`gemini/gemini-flash-lite-latest`). For OpenAI:

```toml
[summarize]
model = "openai/gpt-4o-mini"
```

The `model` field uses smolllm's `provider/model_name` format. Comma-separate
for fallback (`"openai/gpt-4o, gemini/gemini-flash-lite-latest"`). See
[smolllm-rs](https://github.com/rocrp/smolllm-rs) for the full provider list.
Optional `base_url` overrides the provider's default endpoint.
```

- [ ] **Step 3: Verify**

`git diff README.md plugin-config.toml` shows only the intended changes.

- [ ] **Step 4: Commit**

```bash
git add plugin-config.toml README.md
git commit -m "docs: update example config and README for new schema"
```

---

## Task 11: Build and verify

**Files:** none (verification only)

- [ ] **Step 1: Type-check release build**

Run: `cargo check --release`
Expected: clean.

- [ ] **Step 2: Run the full test suite**

Run: `cargo test -p hntui -- --test-threads=1`
Expected: all tests pass. The env-var tests in `plugin::config::tests` mutate process state; `--test-threads=1` keeps them deterministic.

- [ ] **Step 3: Run clippy if configured**

Run: `cargo clippy --all-targets -- -D warnings` (skip if it errors on pre-existing issues unrelated to this work)
Expected: no new warnings introduced by these changes.

- [ ] **Step 4: Build release**

Run: `cargo build --release`
Expected: success.

- [ ] **Step 5: Manual smoke tests** (require an API key)

Set the test key:
```bash
export HNTUI_LLM_API_KEY=...   # Gemini API key
```

Write `~/.config/hntui/config.toml`:
```toml
[summarize]
model = "gemini/gemini-flash-lite-latest"
max_comments = 50
system_prompt = "Summarize concisely."
```

Run hntui, point it at a discussion-rich story, press `s`. Verify:
- (a) Streaming content appears within ~3s.
- (b) `tail -f $HNTUI_LOG_FILE` shows smolllm's `[smolllm::client] Sending stream request: …` and final `📊…tok | 🚀… | 🐎…tok/s | ⌛…` lines.
- (c) Press `c` after completion. Confirm "Copied!" flash and check the clipboard frontmatter still includes the `model:` line as before.
- (d) Set `model = "gemini/bogus-model-name"` and re-run. Confirm error overlay shows the smolllm error string (not a panic).
- (e) Set `model = "gemini/bogus, gemini/gemini-flash-lite-latest"` and re-run. Confirm summarize succeeds via the second model and the log shows the fallback message.
- (f) Set `model = "gemini/gemini-2.5-flash"` (or any reasoning model accessible with the same key) and re-run. Confirm dimmed "Thinking" preview appears, then is replaced by content. If no reasoning model is reachable, document this and skip — non-blocking.

- [ ] **Step 6: If any smoke test fails, fix and re-commit**

Use targeted fix commits (`fix(plugin): …`). Do NOT amend earlier commits.

---

## Self-Review Notes

(Performed during plan authoring; record kept for transparency.)

- **Spec coverage:** Every spec section has a task — Goals → Tasks 1-9; Config schema → Task 4; Data flow → Task 7; Reasoning UI → Tasks 7+9; Log bridge → Tasks 2-3; Errors → handled in Task 7's `format!("{e:#}")`; Testing → Task 11.
- **Placeholder scan:** No "TBD"/"TODO"/"add error handling". All steps include actual code.
- **Type consistency:** `SummarizeChunk { content, reasoning }` is defined in Task 6 (mod.rs), produced in Task 7 (summarize.rs), consumed in Task 7 (handle_event) and Task 9 (overlay). `SummarizeConfig` fields (`model`, `api_key: Option<String>`, `base_url: Option<String>`, `max_comments`, `system_prompt`) match across Tasks 4, 5, and 7. `init_log_bridge` is defined in Task 2, called in Task 3.
- **Field-mut index alignment:** `SettingsPopup` field order (`model`, `api_key`, `base_url`) is consistent across `from_config`, `field_labels`, `field_values`, `field_mut`, and the masking condition in `ui/settings.rs` (now `i == 1`).
