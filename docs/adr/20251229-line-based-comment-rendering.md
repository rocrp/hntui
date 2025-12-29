# Line-based comment rendering for ratatui list gap

Date: 2025-12-29 (UTC)

## Status

Accepted

## Context

Ratatui `List` renders only whole items. If next item taller than remaining viewport, widget leaves blank space. In comment view, first entry shows gap until offset changes.

## Decision

Render comments as line stream instead of `List` items.

- Build `Vec<Line>` per comment (header + wrapped body).
- Track `comment_item_heights`, `comment_viewport_height`, `comment_line_offset`.
- Render visible line window `[line_offset, line_offset + viewport_height)` via `Paragraph`.
- Keep selection index in `ListState`; highlight selected comment by patching style onto its lines.
- `ensure_comment_line_offset` keeps selected comment visible; if comment height >= viewport, pin top of comment.
- Fail fast on height/list mismatch (panic) to avoid silent desync.
- Keep `List` only for loading/empty states.

## Consequences

- No blank space caused by partial-item limitation.
- Manual rendering path for comments; extra code to maintain line offset + heights.
- Slightly higher per-frame work; acceptable UX cost, cache later if needed.
