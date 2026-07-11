# cowboy — build + quality tasks. Run `just` to list.

default:
    @just --list

# Install frontend deps. `deno install` reads package.json + deno.json and
# populates web/node_modules (nodeModulesDir = "auto") so Vite resolves
# `vite`, `tsc`, etc. from there as it always did.
install:
    cd web && deno install

# Run the daemon in the foreground (dev). Pair with `just dev-web` for HMR.
dev *ARGS:
    cargo run -- serve {{ARGS}}

# Frontend dev server (Vite), proxying /ws + /healthz to a running daemon.
dev-web:
    cd web && deno task dev

# Build the frontend bundle (embedded into the binary via rust-embed).
build-web:
    cd web && deno task build

# Build the release binary (embeds the current web/dist).
build: build-web
    cargo build --release

# Quality gates.
fmt:
    cargo fmt --check

fmt-write:
    cargo fmt

lint:
    cargo clippy --all-targets -- -D warnings -A clippy::pedantic
    cd web && deno task lint

typecheck:
    cd web && deno task typecheck

test:
    cargo test --all-targets
    cd web && deno task test

check: fmt lint typecheck test
    cargo build

# Show sccache cache stats.
cache-stats:
    sccache --show-stats
