# cowboy — build + quality tasks. Run `just` to list.

default:
    @just --list

# Run the daemon in the foreground (dev).
dev *ARGS:
    cargo run -- serve {{ARGS}}

# Build the backend.
build:
    cargo build --release

# Quality gates.
fmt:
    cargo fmt

lint:
    cargo clippy --all-targets -- -D warnings

check: fmt lint
    cargo build

# Show sccache cache stats.
cache-stats:
    sccache --show-stats
