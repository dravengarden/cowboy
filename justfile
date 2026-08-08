# cowboy — build + quality tasks. Run `just` to list.

default:
    @just --list

toolchain-check:
    required="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "cowboy") | .rust_version')"; actual="$(rustc --version --verbose | awk '/^release:/ { print $2 }')"; test "$required" = "$actual" || { echo "rust-version $required does not match pinned rustc $actual" >&2; exit 1; }

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
    cd zed-adapter && cargo build --release --locked

# Quality gates.
fmt:
    cargo fmt --check
    cd zed-adapter && cargo fmt --check

fmt-write:
    cargo fmt
    cd zed-adapter && cargo fmt

lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cd zed-adapter && cargo clippy --all-targets --locked -- -D warnings
    cd web && deno task lint

dependencies:
    cargo deny check
    cargo machete --with-metadata
    cd zed-adapter && cargo deny check
    cd zed-adapter && cargo machete --with-metadata

typecheck:
    cd web && deno task typecheck

# Keep independently packaged feature slices honest. An all-features build can
# hide accidental dependencies on modules that are absent from these releases.
feature-check:
    cargo check --locked --no-default-features --features machine-host --bin cowboy-machine --bin cowboy-machine-install
    cargo check --locked --no-default-features --features code-adapter --bin cowboy-code-adapter

test:
    cargo test --all-targets --all-features --locked
    cd zed-adapter && cargo test --all-targets --locked
    cd web && deno task test

check: toolchain-check fmt lint dependencies typecheck feature-check test build

# Run the complete quality gate without growing workspace incremental caches.
# sccache stays opt-in until cross-worktree Rust cache hits are proven locally.
check-compact:
    CARGO_INCREMENTAL=0 just check

test-fast:
    cargo nextest run --all-features --locked

# Opt into the shared compiler cache for clean or batch builds. Interactive
# Cargo commands intentionally keep rustc incremental state for the edit loop.
build-cached:
    RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build --all-targets --all-features --locked
    cd zed-adapter && RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build --all-targets --locked

# Show sccache cache stats.
cache-stats:
    sccache --show-stats

# Whole-target cleanup is reserved for an inactive checkout. The independent
# Zed adapter workspace is always inspected and cleaned explicitly.
cache-usage:
    @if test -d target; then du -sh target; else echo "target: absent"; fi
    @if test -d zed-adapter/target; then du -sh zed-adapter/target; else echo "zed-adapter/target: absent"; fi

cache-clean-dry:
    cargo clean --dry-run
    cargo clean --dry-run --manifest-path zed-adapter/Cargo.toml

cache-clean:
    cargo clean
    cargo clean --manifest-path zed-adapter/Cargo.toml
