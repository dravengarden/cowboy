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

# Build the bridge and expose it at ./bin/cowboy for Zed's dev agent entry
# (`cowboy-dev: …` → command: <repo>/bin/cowboy). Explicit on purpose: re-run
# after changing bridge code, then restart the Zed thread. One fixed path, no
# fallback — if bin/cowboy is stale it's because you didn't re-run this, not
# because a wrapper silently picked an old artifact. The deployed `cowboy` on
# PATH (nix) backs the prod entries and needs none of this.
bridge-dev:
    cargo build --release
    mkdir -p bin && ln -sf ../target/release/cowboy bin/cowboy

# Quality gates.
fmt:
    cargo fmt

lint:
    cargo clippy --all-targets -- -D warnings
    cd web && deno task lint

typecheck:
    cd web && deno task typecheck

check: fmt lint typecheck
    cargo build

# Show sccache cache stats.
cache-stats:
    sccache --show-stats
