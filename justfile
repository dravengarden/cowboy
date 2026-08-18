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
# COWBOY_DATABASE_URL. Login is still required (no anonymous in-memory mode).
dev *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export COWBOY_DATABASE_URL="${COWBOY_DATABASE_URL:-sqlite:///${PWD}/.cowboy-dev.sqlite}"
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

# Build one independently installable Provider artifact. The Provider id is
# resolved only beneath providers/; shell metacharacters and path traversal are
# rejected before any output path is created.
provider-build PROVIDER:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    test -f "providers/{{PROVIDER}}/provider.json"
    mkdir -p "dist/providers/{{PROVIDER}}"
    cargo run --locked -p cowboy-provider-sdk --bin cowboy-provider-pack -- build "providers/{{PROVIDER}}/provider.json" "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider" "cowboy-provider://{{PROVIDER}}"

provider-build-all:
    just provider-build claude-code
    just provider-build codex
    just provider-build gemini
    just provider-build grok
    just provider-build claude-deepseek
    just provider-build codex-deepseek

provider-set-artifact-url PROVIDER URL:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    cargo run --locked -p cowboy-provider-sdk --bin cowboy-provider-pack -- set-artifact-url "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider" "dist/providers/{{PROVIDER}}/{{PROVIDER}}.release.json" "{{URL}}"

provider-runtime-build PROVIDER BASE_URL="https://cowboy.stormbird.xyz/provider-artifacts":
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    deno run --allow-read --allow-write=dist --allow-net --allow-run --allow-env=COLUMBUS_ROOT tools/build-provider-runtime.ts "{{PROVIDER}}" "{{BASE_URL}}"

provider-release-build PROVIDER BASE_URL="https://cowboy.stormbird.xyz/provider-artifacts":
    just provider-build "{{PROVIDER}}"
    just provider-runtime-build "{{PROVIDER}}" "{{BASE_URL}}"
    package_digest=$(jq -r .package_digest "dist/providers/{{PROVIDER}}/{{PROVIDER}}.release.json"); package_hex=${package_digest#sha256:}; just provider-set-artifact-url "{{PROVIDER}}" "{{BASE_URL}}/$package_hex/{{PROVIDER}}.cowboy-provider"
    just provider-bind-runtime "{{PROVIDER}}" "dist/providers/{{PROVIDER}}/runtime/runtime-artifacts.json"

provider-publish PROVIDER CATALOG PUBLIC_KEY:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    just provider-verify "{{PROVIDER}}" "{{PUBLIC_KEY}}"
    deno run --allow-read --allow-write="{{CATALOG}}" --allow-run=sha256sum tools/publish-provider-release.ts "{{PROVIDER}}" "{{CATALOG}}" "{{PUBLIC_KEY}}"

# Bind the independently built package to one immutable runtime component set
# per declared OS/architecture. The resulting composite digest is the Catalog,
# Machine-generation, and session identity.
provider-bind-runtime PROVIDER RUNTIME_ARTIFACTS:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    test -f "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider"
    test -f "{{RUNTIME_ARTIFACTS}}"
    cargo run --locked -p cowboy-provider-sdk --bin cowboy-provider-pack -- bind-runtime "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider" "dist/providers/{{PROVIDER}}/{{PROVIDER}}.release.json" "{{RUNTIME_ARTIFACTS}}"

provider-sign PROVIDER PRIVATE_KEY:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    cargo run --locked -p cowboy-provider-sdk --bin cowboy-provider-pack -- sign "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider" "dist/providers/{{PROVIDER}}/{{PROVIDER}}.release.json" "{{PRIVATE_KEY}}"

provider-verify PROVIDER PUBLIC_KEY:
    case "{{PROVIDER}}" in (*[!a-z0-9-]*|"") echo "invalid Provider id" >&2; exit 2;; esac
    cargo run --locked -p cowboy-provider-sdk --bin cowboy-provider-pack -- verify "dist/providers/{{PROVIDER}}/{{PROVIDER}}.cowboy-provider" "dist/providers/{{PROVIDER}}/{{PROVIDER}}.release.json" "{{PUBLIC_KEY}}"

# Cross-language package/linker conformance. This is also the Provider release
# skill's deterministic pre-publish gate.
provider-check:
    deno check tools/build-provider-runtime.ts tools/check-provider-runtime-lock.ts tools/provider-publication-receipt.ts tools/publish-provider-release.ts
    deno test --allow-read tools/provider-runtime-platforms_test.ts
    deno test --allow-read --allow-write .agents/skills/release-cowboy-provider/scripts/audit-dependencies_test.ts
    deno test tools/provider-publication-receipt_test.ts
    deno run --allow-read tools/check-provider-runtime-lock.ts
    cargo test --locked -p cowboy-provider-sdk --all-targets
    just provider-build-all
    cd web && deno task typecheck
    deno run --allow-read packages/provider-ui-sdk/validate-packages.ts dist/providers/*/*.cowboy-provider
    cd web && deno test --allow-read src/providerSdk.test.ts

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
