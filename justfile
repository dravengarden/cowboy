# cowboy — build + quality tasks. Run `just` to list.

default:
    @just --list

toolchain-check: worktree-deps-check
    required="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "cowboy") | .rust_version')"; actual="$(rustc --version --verbose | awk '/^release:/ { print $2 }')"; test "$required" = "$actual" || { echo "rust-version $required does not match pinned rustc $actual" >&2; exit 1; }
    actual="$(deno --version | awk 'NR == 1 { print $2 }')"; test "$actual" = "$COWBOY_DENO_VERSION" || { echo "Deno $actual does not match pinned Deno $COWBOY_DENO_VERSION" >&2; exit 1; }

# Reject dependency views borrowed from another checkout. Deno's global cache
# may be shared, but node_modules and file: package links belong to this tree.
worktree-deps-check:
    deno check tools/check-worktree-dependencies.ts tools/check-worktree-dependencies_test.ts
    deno test --allow-read --allow-write tools/check-worktree-dependencies_test.ts
    deno run --allow-read tools/check-worktree-dependencies.ts

# Install a checkout-local frontend dependency view. DENO_DIR may be shared
# across worktrees; web/node_modules itself must never be shared or symlinked.
install:
    deno run --allow-read --allow-write=web tools/check-worktree-dependencies.ts --repair-borrowed-state
    cd web && deno install --frozen
    deno run --allow-read tools/check-worktree-dependencies.ts --require-installed

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

# Build the user-scoped Machine bootstrap commands on macOS or Linux.
build-machine-bootstrap:
    cargo build --release --locked --no-default-features --features machine-host --bin cowboy --bin cowboy-machine --bin cowboy-machine-install
    cargo build --release --locked --no-default-features --features code-adapter --bin cowboy-code-adapter

# Native macOS SwiftUI installer manager. Run these on macOS with Xcode's Swift
# toolchain; build-app packages the existing Machine bootstrap commands.
macos-installer-test:
    swift test --package-path apps/macos-installer

macos-installer-build:
    bash apps/macos-installer/scripts/build-app.sh --build-backend

macos-installer-verify APP="apps/macos-installer/dist/Cowboy Manager.app":
    bash apps/macos-installer/scripts/verify-app.sh "{{APP}}"

# Generic Cowboy Plugin lifecycle. Agent Provider and code-intelligence are
# payload kinds; neither owns a separate release or installation format.
component-package-check:
    for package in plugin-contract app-shell state-store state-sync state-sync-idb provider-ui provider-runtime code-intelligence; do npm pack --dry-run --json "./components/$package" >/dev/null; done
    cargo package --locked --allow-dirty --list -p cowboy-provider-sdk >/dev/null
    cargo package --locked --allow-dirty --list -p cowboy-plugin-sdk >/dev/null

plugin-check: component-package-check
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

# Prove that one Plugin can build outside the Cowboy checkout using only its
# own source directory, its component release pin, and the packaged SDK CLI.
plugin-isolation-check PLUGIN="codex":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    repo_root="$PWD"
    isolation_root="$(mktemp -d /tmp/cowboy-plugin-isolation-XXXXXX)"
    cleanup() { rm -r -- "$isolation_root"; }
    trap cleanup EXIT
    cargo build --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
    cd "$isolation_root"
    "$repo_root/target/debug/cowboy-plugin-pack" build "$repo_root/plugins/{{PLUGIN}}" "{{PLUGIN}}.cowboy-plugin"
    test -s "{{PLUGIN}}.cowboy-plugin"
    test -s "{{PLUGIN}}.release.json"

# Agent Plugin payload helper. Its output is the generic runtime manifest
# consumed by plugin-bind-runtime; it is not an independent release lifecycle.
agent-plugin-runtime-build PLUGIN BASE_URL:
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    test "$(jq -r .kind "plugins/{{PLUGIN}}/plugin.json")" = agent_provider
    deno run --allow-read --allow-write=dist --allow-net --allow-run components/provider-runtime/build.ts "plugins/{{PLUGIN}}" "{{BASE_URL}}"

plugin-set-artifact-url PLUGIN URL:
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- set-artifact-url "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" "{{URL}}"

# Turn the package digest produced by plugin-build into the immutable HTTPS
# publication URL required before a runtime matrix can be bound and signed.
plugin-set-published-artifact-url PLUGIN BASE_URL:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    release="dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json"
    test -f "$release"
    digest="$(jq -er '.package_digest | select(test("^sha256:[a-f0-9]{64}$")) | sub("^sha256:"; "")' "$release")"
    base="{{BASE_URL}}"
    just plugin-set-artifact-url "{{PLUGIN}}" "${base%/}/${digest}/{{PLUGIN}}.cowboy-plugin"

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

# Deployment preflight: every embedded Agent Provider version must have an
# exact signed publication receipt and immutable artifact set in the target
# Service Catalog before a Controller carrying those manifests is activated.
provider-release-coverage CATALOG:
    deno run --allow-read --allow-run=sha256sum tools/check-provider-release-coverage.ts "{{CATALOG}}"

# Cross-language package/linker conformance. This is also the Agent Plugin
# payload gate used by the generic Plugin release workflow.
provider-check: plugin-check
    deno check components/provider-runtime/build.ts components/provider-runtime/check.ts tools/check-provider-release-coverage.ts tools/check-provider-release-coverage_test.ts tools/plugin-publication-receipt.ts tools/publish-plugin-release.ts
    deno test --allow-read --allow-write --allow-run=sha256sum tools/check-provider-release-coverage_test.ts
    deno test --allow-read tools/provider-runtime-platforms_test.ts
    deno test --allow-read --allow-write .agents/skills/release-cowboy-plugin/scripts/audit-dependencies_test.ts
    deno test tools/plugin-publication-receipt_test.ts
    deno run --allow-read components/provider-runtime/check.ts
    cargo test --locked -p cowboy-provider-sdk --all-targets
    cargo test --locked -p cowboy-plugin-sdk --all-targets
    just plugin-build-all
    just plugin-isolation-check codex
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
    # The invoking Codex Plugin exports its own signed package path. Cowboy's
    # unit tests exercise embedded fixtures and must not inherit that runtime.
    env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --all-targets --all-features --locked
    cd plugins/zed/adapter && cargo test --all-targets --locked
    cd web && deno task test

check: toolchain-check provider-check fmt lint dependencies typecheck feature-check test build

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
