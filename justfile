# cowboy — build + quality tasks. Run `just` to list.

default:
    @just --list

# Install frontend deps.
install:
    cd web && bun install

# Run the daemon in the foreground (dev). Pair with `just dev-web` for HMR.
dev *ARGS:
    cargo run -- serve {{ARGS}}

# Frontend dev server (Vite), proxying /ws + /healthz to a running daemon.
dev-web:
    cd web && bun run dev

# Build the frontend bundle (embedded into the binary via rust-embed).
build-web:
    cd web && bun run build

# Build the release binary (embeds the current web/dist).
build: build-web
    cargo build --release

# Quality gates.
fmt:
    cargo fmt

lint:
    cargo clippy --all-targets -- -D warnings
    cd web && bun run lint

typecheck:
    cd web && bun run typecheck

check: fmt lint typecheck
    cargo build

# Show sccache cache stats.
cache-stats:
    sccache --show-stats
