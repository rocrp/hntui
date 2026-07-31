set default-list

# Build the debug binary
build:
    cargo build --locked

# Run all local quality gates
check: fmt-check lint test

# Format Rust source
fmt:
    cargo fmt --all

# Check Rust formatting
fmt-check:
    cargo fmt --all -- --check

# Lint all targets
lint:
    cargo clippy --locked --all-targets -- -D warnings

# Cut and publish a release (commits, tags, and pushes)
release version:
    ./scripts/release.sh {{ quote(version) }}

# Rebuild screenshots and the demo GIF
screenshots:
    ./scripts/screenshots.sh

# Run the test suite
test:
    cargo test --locked
