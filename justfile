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
    cargo run --locked -- serve {{ARGS}}

# Frontend dev server (Vite), proxying /ws + /healthz to a running daemon.
dev-web:
    cd web && deno task dev

# Build the independently deployed frontend bundle.
build-web:
    cd web && deno task build

# Rebuild the project-owned browser bridge around mvdan/sh's core parser.
shellfmt-wasm:
    cd web/shellfmt-wasm && go test ./...
    cd web/shellfmt-wasm && GOOS=js GOARCH=wasm go build -trimpath -ldflags="-s -w" -o ../public/shellfmt.wasm .
    chmod 0644 web/public/shellfmt.wasm
    rm -f web/public/wasm_exec.js
    cp "$(go env GOROOT)/lib/wasm/wasm_exec.js" web/public/wasm_exec.js
    chmod u+w web/public/wasm_exec.js

# Build both release artifacts for local use.
build: build-web
    cargo build --release --locked

# Quality gates.
fmt:
    cargo fmt --check

fmt-write:
    cargo fmt

lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cd web && deno task lint

dependencies:
    cargo deny check
    cargo machete --with-metadata

typecheck:
    cd web && deno task typecheck

test:
    cargo test --all-targets --all-features --locked
    cd web && deno task test

check: fmt lint dependencies typecheck test
    cargo build --all-features --locked

test-fast:
    cargo nextest run --all-features --locked

# Opt in only for clean rebuilds that demonstrate useful cache reuse.
build-cached:
    RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build --all-targets --all-features --locked

# Show sccache cache stats.
cache-stats:
    sccache --show-stats
