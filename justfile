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
# Default SQLite so /admin can create a product user; override with
# COWBOY_DATABASE_URL. Product login stays off unless explicitly enabled.
dev *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export COWBOY_DATABASE_URL="${COWBOY_DATABASE_URL:-sqlite:///${PWD}/.cowboy-dev.sqlite}"
    cargo run --locked --bin cowboy -- serve {{ARGS}}

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
    cd plugins/zed/adapter && cargo build --release --locked

# Build the three user-scoped Machine bootstrap commands on macOS or Linux.
build-machine-bootstrap:
    cargo build --release --locked --no-default-features --features machine-host --bin cowboy --bin cowboy-machine --bin cowboy-machine-install

# Generic Cowboy Plugin lifecycle. Agent Provider and code-intelligence are
# payload kinds; neither owns a separate release or installation format.
plugin-check:
    deno check tools/check-plugin-components.ts
    deno test --allow-read tools/check-plugin-components_test.ts
    deno run --allow-read tools/check-plugin-components.ts

plugin-build PLUGIN:
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    deno run --allow-read tools/check-plugin-components.ts
    test -f "plugins/{{PLUGIN}}/plugin.json"
    mkdir -p "dist/plugins/{{PLUGIN}}"
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- build "plugins/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "cowboy-plugin://{{PLUGIN}}"

plugin-build-all:
    for plugin in claude-code claude-deepseek codex codex-deepseek gemini grok zed; do just plugin-build "$plugin"; done

# Agent Plugin payload helper. Its output is the generic runtime manifest
# consumed by plugin-bind-runtime; it is not an independent release lifecycle.
agent-plugin-runtime-build PLUGIN BASE_URL:
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    test "$(jq -r .kind "plugins/{{PLUGIN}}/plugin.json")" = agent_provider
    deno run --allow-read --allow-write=dist --allow-net --allow-run tools/build-provider-runtime.ts "{{PLUGIN}}" "{{BASE_URL}}"

plugin-set-artifact-url PLUGIN URL:
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- set-artifact-url "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" "{{URL}}"

plugin-bind-runtime PLUGIN RUNTIME_ARTIFACTS:
    test -f "{{RUNTIME_ARTIFACTS}}"
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- bind-runtime "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" "{{RUNTIME_ARTIFACTS}}"

plugin-sign PLUGIN PRIVATE_KEY:
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- sign "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" "{{PRIVATE_KEY}}"

plugin-verify PLUGIN PUBLIC_KEY:
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- verify "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" "{{PUBLIC_KEY}}"

plugin-publish PLUGIN CATALOG PUBLIC_KEY:
    just plugin-verify "{{PLUGIN}}" "{{PUBLIC_KEY}}"
    deno run --allow-read --allow-write="{{CATALOG}}" --allow-run=sha256sum tools/publish-plugin-release.ts "{{PLUGIN}}" "{{CATALOG}}" "{{PUBLIC_KEY}}"

# Cross-language package/linker conformance. This is also the Agent Plugin
# payload gate used by the generic Plugin release workflow.
provider-check: plugin-check
    deno check tools/build-provider-runtime.ts tools/check-provider-runtime-lock.ts tools/plugin-publication-receipt.ts tools/publish-plugin-release.ts
    deno test --allow-read tools/provider-runtime-platforms_test.ts
    deno test --allow-read --allow-write .agents/skills/release-cowboy-plugin/scripts/audit-dependencies_test.ts
    deno test tools/plugin-publication-receipt_test.ts
    deno run --allow-read tools/check-provider-runtime-lock.ts
    cargo test --locked -p cowboy-provider-sdk --all-targets
    cargo test --locked -p cowboy-plugin-sdk --all-targets
    just plugin-build-all
    cd web && deno task typecheck
    deno run --allow-read components/provider-ui/validate-packages.ts dist/plugins/*/*.cowboy-plugin
    cd web && deno test --allow-read src/providerSdk.test.ts

# Quality gates.
fmt:
    cargo fmt --check
    cd plugins/zed/adapter && cargo fmt --check

fmt-write:
    cargo fmt
    cd plugins/zed/adapter && cargo fmt

lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cd plugins/zed/adapter && cargo clippy --all-targets --locked -- -D warnings
    cd web && deno task lint

dependencies:
    cargo deny check
    cargo machete --with-metadata
    cd plugins/zed/adapter && cargo deny check
    cd plugins/zed/adapter && cargo machete --with-metadata

typecheck:
    cd web && deno task typecheck

# Keep independently packaged feature slices honest. An all-features build can
# hide accidental dependencies on modules that are absent from these releases.
feature-check:
    cargo check --locked --no-default-features --features machine-host --bin cowboy-machine --bin cowboy-machine-install
    cargo check --locked --no-default-features --features code-adapter --bin cowboy-code-adapter

test:
    cargo test --all-targets --all-features --locked
    cd plugins/zed/adapter && cargo test --all-targets --locked
    cd web && deno task test

check: toolchain-check plugin-check fmt lint dependencies typecheck feature-check test build

# Run the complete quality gate without growing workspace incremental caches.
# sccache stays opt-in until cross-worktree Rust cache hits are proven locally.
check-compact:
    CARGO_INCREMENTAL=0 just check

test-fast:
    cargo nextest run --all-features --locked

# Compact-control-plane memory fixture against a local SQLite Durable Object.
do-memory-mock:
    deno run --allow-read tools/do-memory-mock/mock.ts
    bash tools/do-memory-mock/run.sh deploy --dry-run --outdir /tmp/cowboy-do-memory-mock

# Generate a real Hub compact fixture and POST it into local wrangler SQLite.
do-memory-mock-seed:
    bash tools/do-memory-mock/seed-from-hub.sh

# Dozens of sessions, filled tails, 1000 tools, fat payloads, SQLite archive.
do-memory-mock-extreme:
    bash tools/do-memory-mock/extreme-from-hub.sh

# Opt into the shared compiler cache for clean or batch builds. Interactive
# Cargo commands intentionally keep rustc incremental state for the edit loop.
build-cached:
    RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build --all-targets --all-features --locked
    cd plugins/zed/adapter && RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build --all-targets --locked

# Show sccache cache stats.
cache-stats:
    sccache --show-stats

# Whole-target cleanup is reserved for an inactive checkout. The independent
# Zed adapter workspace is always inspected and cleaned explicitly.
cache-usage:
    @if test -d target; then du -sh target; else echo "target: absent"; fi
    @if test -d plugins/zed/adapter/target; then du -sh plugins/zed/adapter/target; else echo "plugins/zed/adapter/target: absent"; fi

cache-clean-dry:
    cargo clean --dry-run
    cargo clean --dry-run --manifest-path plugins/zed/adapter/Cargo.toml

cache-clean:
    cargo clean
    cargo clean --manifest-path plugins/zed/adapter/Cargo.toml
