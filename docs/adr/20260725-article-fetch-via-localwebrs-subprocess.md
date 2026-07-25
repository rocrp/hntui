# Article fetch shells out to the localwebrs CLI, not the lib

Date: 2026-07-25 (UTC)

## Status

Accepted

## Context

hntui gains an Article concept (view the linked page's text; optionally feed it to the Summarizer). localwebrs is the obvious engine — it already does extraction, anti-captcha browser fallback, site plugins, and SQLite caching. It exposes a lib target, but with zero feature gates: chromiumoxide is always compiled, `wreq` drags in BoringSSL (C build), rusqlite is bundled, pdfium-render is a git dependency, and even clap/axum/tracing-subscriber ride along — ~500 lockfile packages. hntui ships prebuilt binaries including x86_64-unknown-linux-musl, where cross-compiling boring-sys is a known hazard.

## Decision

- hntui spawns `localwebrs visit <url> --json -c 86400` as a subprocess and parses the JSON (`to_dict()` shape: title/content/extra).
- Binary resolved via config `[article] bin` (default `"localwebrs"`, PATH lookup). No startup probe: a spawn ENOENT maps to a friendly install hint, and the feature degrades visibly (article overlay error state; summary falls back to comments-only with a banner).
- Child `current_dir` is set to hntui's cache dir so localwebrs's CWD-relative `cache/cache.sqlite` lands there.
- Visitor tier (`smart`) and cache TTL (86400) are hardcoded; no config knobs until a real need appears.
- Cancellation = kill the child (kill_on_drop) plus a 90s watchdog.

## Considered Options

- **Lib dependency (git dep, like smolllm)** — rejected: +~470 compiled packages, musl×BoringSSL CI risk, binary bloat, inherits a second CLI parser and logging init.
- **Feature-gate localwebrs first, then depend on a slim http+extract core** — rejected for now: a refactor round in another repo before any hntui value, and the slim core loses the browser fallback that is the point of localwebrs. Still open as a future project; the ArticleFetcher seam would absorb it without UX change.
- **localwebrs-server over HTTP** — rejected: requires a running service; wrong shape for a distributable TUI.

## Consequences

- The feature requires localwebrs installed; without it hntui works as before, minus articles.
- The JSON contract is informal — a `to_dict()` shape change in localwebrs breaks hntui's parse. Parse tolerantly (unknown fields ignored, missing `content` = extraction failure) and treat breakage as a fetch failure, never a crash.
- No fine-grained progress across the process boundary; the overlay shows elapsed time instead.
- Full SmartVisitor behavior (browser fallback, cookies `auto`, plugins) comes for free and stays in lockstep with the user's daily CLI.
