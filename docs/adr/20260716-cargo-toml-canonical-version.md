# Cargo.toml is the canonical version; CI rejects mismatched tags

Date: 2026-07-16 (UTC)

## Status

Accepted

## Context

Version lived in four places with no wiring: Cargo.toml (`0.1.7`), git tags (`v0.4.2`), Homebrew formula (tag-derived), binary (none — no `--version` flag at all). Releases are tag-driven and never touched Cargo.toml → three minor versions of drift. Released binary cannot report what it is.

## Decision

- Cargo.toml `version` is the single source of truth.
- Binary reports it via clap `#[command(version)]` (`CARGO_PKG_VERSION`); custom help template shows `hntui {version}` at top of `--help`.
- Version string is plain (`hntui 0.4.3`) — no git sha / build date. Every release is a signed, tagged build; version ↔ commit resolves via the tag.
- Release = `scripts/release.sh <version>`: set Cargo.toml version, sync Cargo.lock, commit, tag `v<version>`, push. One command; drift impossible by construction.
- Release workflow fails fast when tag ≠ Cargo.toml version (backstop for hand-cut tags).

## Considered Options

Tag-canonical (inject `GITHUB_REF_NAME` via build.rs / env at build time) — rejected: extra build plumbing, local/source builds get fake or empty version, against cargo ecosystem grain (`cargo-release` etc. assume manifest-canonical).

## Consequences

- Dev builds between releases print the last-released version, indistinguishable from the release binary. Accepted: only the maintainer builds from source.
- Hand-cut tag that skips `release.sh` and mismatches → CI fails; delete tag, re-tag. Intentional.
- Cargo.lock pins hntui's own version and CI builds `--locked`; any manual bump must sync the lockfile (release.sh does this).
