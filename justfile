# cowboy — build + quality tasks. Run `just` to list.

# The one all-feature Rust test graph. `test`, `composition-check` and
# `test-postgres` share it, so each gate run compiles it once; a different
# package or feature selection unifies dependency features differently and
# recompiles the workspace.
# The graph links its binaries into an artifact directory of its own. Every
# Cargo run re-links the binaries it uplifts, fresh ones included, so in
# target/debug it would swap the Machine execution host the script suites run
# (`claude-remote-prepare`) for the graph's all-feature build, even mid-run.
# Directories are Cargo flags, never CARGO_* variables: sccache hashes those
# into every crate's key, so a checkout path there makes every key unique to
# the checkout and no other worktree ever hits.
rust_test_args := "-p cowboy -p cowboy-store -p cowboy-server-auth -p cowboy-server-accounts -p cowboy-server-code -p cowboy-server-code-buffers -p cowboy-server-plugins -p cowboy-server-runtime -p cowboy-server-agent-api -p cowboy-server-telemetry -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-provider-sdk -p cowboy-plugin-sdk --all-targets --all-features --locked --target-dir " + justfile_directory() + "/target/test-graph"
# How every recipe runs that graph. The invoking Codex Plugin exports its own
# signed package path; Cowboy's unit tests exercise embedded fixtures and must
# not inherit that runtime. The graph always keeps rustc incremental state,
# even inside `check-compact`: after a one-file edit it rebuilt in 25s against
# 114s without (2026-11-21 on Hawk), for about 9G of incremental cache. Each
# invocation must agree, since the setting is part of Cargo's fingerprint.
rust_test_env := "env -u COWBOY_PROVIDER_PACKAGE_PATH -u CARGO_INCREMENTAL"
# The wire, standalone Machine and code-adapter test slices select features of
# their own, so they share no workspace artifacts with the main graph. They
# compile under a Cargo profile of their own (`gate-slices`, identical to dev):
# Cargo locks each profile's build and artifact directories, so only a separate
# profile lets `check` build them beside the main graph instead of after it,
# and it keeps their state in the build directory the host's retention manages.
# They keep incremental state like the main graph: the Antigravity and Claude
# runtime suites wait for this profile's execution host, so its rebuild after an
# edit is on the gate's critical path. Their workspace crates could not hit
# sccache across worktrees anyway (each key carries the checkout's
# CARGO_MANIFEST_DIR).
rust_slice_env := "env -u COWBOY_PROVIDER_PACKAGE_PATH -u CARGO_INCREMENTAL"
rust_slice_profile := "gate-slices"
rust_wire_args := "--locked -p cowboy-wire --features full,machine-host --all-targets"
rust_machine_args := "--locked --no-default-features --features machine-host -p cowboy -p cowboy-store -p cowboy-machine --lib"
rust_code_adapter_args := "--locked --no-default-features --features code-adapter --lib"

default:
    @just --list

toolchain-check: worktree-deps-check
    required="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "cowboy") | .rust_version')"; actual="$(rustc --version --verbose | awk '/^release:/ { print $2 }')"; test "$required" = "$actual" || { echo "rust-version $required does not match pinned rustc $actual" >&2; exit 1; }
    actual="$(bun --version)"; test "$actual" = "$COWBOY_BUN_VERSION" || { echo "Bun $actual does not match pinned Bun $COWBOY_BUN_VERSION" >&2; exit 1; }
    actual="$(node --version)"; test "$actual" = "v$COWBOY_NODE_VERSION" || { echo "Node $actual does not match pinned Node $COWBOY_NODE_VERSION" >&2; exit 1; }

# Reject dependency views borrowed from another checkout. Bun's global cache
# may be shared, but node_modules and workspace links belong to this tree.
worktree-deps-check:
    bun run typecheck
    bun test ./tools/check-worktree-dependencies_test.ts
    bun tools/check-worktree-dependencies.ts

# Install a checkout-local dependency view. Bun's install cache may be shared
# across worktrees; node_modules itself must never be shared or symlinked.
install:
    bun tools/check-worktree-dependencies.ts --repair-borrowed-state
    bun install --frozen-lockfile
    bun tools/check-worktree-dependencies.ts --require-installed

# Run the daemon in the foreground (dev). Pair with `just dev-web` for HMR.
# Default SQLite so /admin can create a product user; override with
# COWBOY_DATABASE_URL. Product login stays off unless explicitly enabled.
dev *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export COWBOY_DATABASE_URL="${COWBOY_DATABASE_URL:-sqlite:///${PWD}/.cowboy-dev.sqlite}"
    cargo run --locked -p cowboy-server --bin cowboy-controller -- serve {{ARGS}}

# Frontend dev server (Vite), proxying /ws + /healthz to a running daemon.
dev-web:
    cd web && bun run dev

# Build the independently deployed frontend bundle.
build-web:
    cd web && bun run build

# Optional real IndexedDB gate. Build .#cowboy-idb-test-browser and pass its
# absolute /bin/firefox; never use an authenticated/system browser profile.
# The Desktop keyboard/UI gate. Required before shipping Desktop keyboard,
# leader, label, modal or Draft-page changes: the keyboard suites in pinned
# Firefox and Chromium (light and dark), then trusted-input acceptance with
# screenshots in a running Chrome when CDP_ENDPOINT answers (hawk
# chrome-debug by default). An unreachable endpoint is reported, never
# treated as a pass for that step.
desktop-browser-gate CDP_ENDPOINT="http://127.0.0.1:9222" OUT="/tmp/cowboy-desktop-acceptance":
    #!/usr/bin/env bash
    set -euo pipefail
    firefox="$(nix build --no-link --print-out-paths .#cowboy-idb-test-browser)/bin/firefox"
    chromium="$(nix build --no-link --print-out-paths .#cowboy-chromium-test-browser)/bin/chromium"
    for browser in "$firefox" "$chromium"; do
      for suite in desktop-composer draft-documents session-fold sheet-keyboard; do
        for theme in light dark; do
          echo "== $(basename "$browser") $suite $theme"
          unshare --user --map-current-user --keep-caps --net bash -euc \
            'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" "$2" "$3"' \
            conformance "$browser" "$suite" "$theme" </dev/null | grep -E '"ok"|"browser"'
        done
      done
    done
    if bun -e "await fetch('{{CDP_ENDPOINT}}/json/version')" >/dev/null 2>&1; then
      just desktop-keyboard-acceptance "{{CDP_ENDPOINT}}" "{{OUT}}"
      echo "screenshots: {{OUT}}"
    else
      echo "SKIPPED trusted-input acceptance: no Chrome DevTools at {{CDP_ENDPOINT}}" >&2
      exit 3
    fi

# Run a browser fixture in an already running Chrome over a loopback DevTools
# endpoint (hawk chrome-debug :9222, or the macbook-air bridge :9223) inside a
# disposable browser context; see tools/cdp-fixture.ts.
cdp-browser-conformance ENDPOINT SUITE THEME="light":
    bun tools/cdp-browser-conformance.ts "{{ENDPOINT}}" "{{SUITE}}" "{{THEME}}"

# Trusted-input Desktop keyboard acceptance with screenshots (leader, labels,
# dialog leader, Vim/IME ownership, the Draft page) in a running Chrome, then
# the shortcut audit: every visible control must show a keyboard slot.
desktop-keyboard-acceptance ENDPOINT OUT:
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}"
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" draft
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" approval
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" tabs
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" run-config
    bun tools/cdp-keyboard-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" surfaces
    bun tools/cdp-review-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" light
    bun tools/cdp-review-acceptance.ts "{{ENDPOINT}}" "{{OUT}}" dark
    bun tools/cdp-shortcut-audit.ts "{{ENDPOINT}}" "{{OUT}}"

# Desktop performance profile under a production-sized synthetic workload
# (250 sessions, 8 busy, live stream, status churn, switches, typing) in a
# running Chrome's disposable context. Prints frame/long-task measurements and
# the hottest functions per scenario and writes .cpuprofile files to OUT.
# Never profile production in a signed-in profile. PERF_SCENARIOS and
# PERF_THEME=dark narrow a run; DATA optionally replaces the transcripts.
desktop-perf-profile ENDPOINT="http://127.0.0.1:9222" OUT="/tmp/cowboy-desktop-perf" *DATA:
    bun tools/desktop-perf-profile.ts "{{ENDPOINT}}" "{{OUT}}" {{DATA}}

idb-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1"' conformance "{{BROWSER}}"

# Real browser signatures, HTTPS, WSS and durable device identity. No real account.
device-browser-conformance BROWSER TEST_BINARY CERTUTIL:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/browser-device-conformance.py "$1" "$2" "$3"' conformance "{{BROWSER}}" "{{TEST_BINARY}}" "{{CERTUTIL}}"

# Same native engine with independent Worker owners and abrupt termination.
idb-outbox-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" idb-outbox' conformance "{{BROWSER}}"

# Real core lifecycle history UI, synthetic evidence only; never a live account.
plugin-lifecycle-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" plugin-lifecycle' conformance "{{BROWSER}}"

# Mobile-width About recovery, real IndexedDB with 376 synthetic older records.
settings-recovery-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" settings-recovery' conformance "{{BROWSER}}"

# Disposable authenticated HTTP + WebSocket handshakes against exact releases.
product-sync-controller-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_PRODUCT_SYNC_MATRIX="$1" COWBOY_TEST_PRODUCT_SYNC_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::probe::product_sync::immutable_product_sync -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Same isolated browser runner, real React development StrictMode + MUI.
provider-ui-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" provider-ui' conformance "{{BROWSER}}"

# Core Service sign-in and Machine confirmation owners, with fake HTTP effects.
provider-management-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" provider-management' conformance "{{BROWSER}}"

# Real context selection, first-send admission UI and cross-device edit conflicts.
session-context-browser-conformance BROWSER THEME="light":
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" session-context "$2"' conformance "{{BROWSER}}" "{{THEME}}"

# Core buffer owner: real development StrictMode, no Review/native activation.
code-buffer-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" code-buffers' conformance "{{BROWSER}}"

# Actual product identity/dataset lifetimes with synthetic HTTP, no login/Plugin.
code-buffer-context-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" code-buffer-context' conformance "{{BROWSER}}"

# Core Settings cleanup projection, original owners and synthetic HTTP only.
code-buffer-cleanup-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" code-buffer-cleanup' conformance "{{BROWSER}}"

# Original synchronization confirmation/uncertainty UI; no production account.
code-buffer-sync-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" code-buffer-sync' conformance "{{BROWSER}}"

# Actual Review owner hook / content and Outline consumers, isolated fixture HTTP.
review-code-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-code' conformance "{{BROWSER}}"

# Actual Review comparison choice and line selection, isolated fixture HTTP.
review-compare-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-compare' conformance "{{BROWSER}}"

# Actual Review DocumentView refresh/scroll anchoring, isolated fixture HTTP.
review-document-refresh-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-document-refresh' conformance "{{BROWSER}}"

# Actual diff projection/owner/status/CodeMirror integration; no real account.
review-diff-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-diff' conformance "{{BROWSER}}"

# Actual Review destination + independent target view + passive navigation recovery.
review-destination-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-destination' conformance "{{BROWSER}}"

# Actual Git review recovery after an unavailable Machine; no real account.
review-recovery-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" review-recovery' conformance "{{BROWSER}}"

workspace-extensions-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" workspace-extensions' conformance "{{BROWSER}}"

idb-conformance-check:
    bun x --no-install biome format tools/idb-browser-conformance.ts tools/idb-browser-bundle.mjs tools/cdp-fixture.ts tools/cdp-browser-conformance.ts tools/cdp-keyboard-acceptance.ts tools/ios-simulator-conformance.ts tools/dom-apps-conformance.ts tools/mcp-apps-conformance.ts
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi

# Retain an actual product-store bundle before and after a latency change.
send-latency-bundle OUTPUT:
    node tools/send-latency-bundle.mjs "{{OUTPUT}}"

# Controlled discovery/handshake/echo delays, actual Firefox IndexedDB and WS.
send-latency-browser BROWSER BEFORE AFTER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/send-latency-browser.ts "$1" "$2" "$3"' latency "{{BROWSER}}" "{{BEFORE}}" "{{AFTER}}"

# Admission failures must retain authored data and permanently fence replacements.
send-admission-browser BROWSER BUNDLE:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/send-latency-browser.ts "$1" "$2" --safety' admission "{{BROWSER}}" "{{BUNDLE}}"

# Build the public Cowboy product website from the first-party Plugin manifests.
build-site:
    bun site/build.ts --out site-dist

# Keep the published ecosystem synchronized with the real Plugin Catalog inputs.
site-check:
    bun x --no-install biome format site/build.ts site/build_test.ts site/site.js
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    bun test ./site/build_test.ts
    just build-site

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
    cargo build --release --locked -p cowboy-server
    cd plugins/zed/adapter && cargo build --release --locked

# Build the user-scoped Machine bootstrap commands on macOS or Linux.
build-machine-bootstrap:
    cargo build --release --locked --no-default-features --features machine-host -p cowboy --bin cowboy -p cowboy-machine --bin cowboy-machine --bin cowboy-machine-install
    cargo build --release --locked --no-default-features --features code-adapter --bin cowboy-code-adapter
    cargo build --release --locked --bin cowboy-acp-worker

# Native macOS SwiftUI installer manager. Run these on macOS with Xcode's Swift
# toolchain; build-app packages the existing Machine bootstrap commands.
macos-installer-test:
    swift test --package-path apps/macos-installer

native-plugin-conformance RUNTIME="auto":
    bash tools/native-plugin-conformance.sh "{{RUNTIME}}"

# Source/dependency and keyboard checks also run on Linux in the pinned shell.
native-shell-check:
    bun x --no-install biome format tools/check-native-shell.ts tools/check-native-shell_test.ts tools/native-shell-probe.js tools/native-shell-probe_test.ts
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    bun test ./tools/check-native-shell_test.ts ./tools/native-shell-probe_test.ts ./tools/native-shell-loader_test.ts
    python3 -m unittest discover -s tools -p 'native_shell_smoke_test.py'
    python3 -m unittest discover -s tools -p 'native_swift_rs_compat_test.py'
    bun tools/check-native-shell.ts
    bash -n tools/build-native-shell.sh tools/sign-android-apk.sh tools/cowboysim.sh tools/cowboysim-remote.sh
    bash tools/check-keyboard-geometry.sh
    cargo metadata --locked --no-deps --format-version 1 --manifest-path apps/native-shell/tauri/Cargo.toml >/dev/null

# Build only, from clean Git source on a Mac. Never installs or launches an app.
native-shell-build PLATFORM="ios-sim" *ARGS:
    bash tools/build-native-shell.sh {{PLATFORM}} {{ARGS}}

# Sign a disposable copy and launch it only in an exclusively-created Simulator.
# Sign an unsigned Android release APK from its build receipt (native-android shell).
native-shell-sign-android RECEIPT *ARGS:
    bash tools/sign-android-apk.sh "{{RECEIPT}}" {{ARGS}}

native-shell-smoke RECEIPT *ARGS:
    python3 tools/native_shell_smoke.py "{{RECEIPT}}" {{ARGS}}

macos-installer-build:
    bash apps/macos-installer/scripts/build-app.sh --build-backend

macos-installer-verify APP="apps/macos-installer/dist/Cowboy Manager.app":
    bash apps/macos-installer/scripts/verify-app.sh "{{APP}}"

# Generic Cowboy Plugin lifecycle. Agent Provider and code-intelligence are
# payload kinds; neither owns a separate release or installation format.
component-package-check:
    for package in plugin-contract plugin-api app-shell state-store state-sync state-sync-idb provider-authoring provider-ui provider-runtime memory-client code-intelligence; do npm pack --dry-run --json "./components/$package" >/dev/null; done
    cargo package --locked --allow-dirty --list -p cowboy-provider-sdk >/dev/null
    cargo package --locked --allow-dirty --list -p cowboy-plugin-sdk >/dev/null

plugin-check: component-package-check
    bun x --no-install biome format plugins/claude-code/collector/credits.js plugins/claude-code/collector/resets.js plugins/claude-code/resets.test.js
    bun x --no-install biome format plugins/codex/collector/index.js plugins/grok/collector/index.js plugins/claude-deepseek/collector/index.js plugins/claude-deepseek/collector/pricing.js plugins/collector-sidecars.test.js plugins/claude-deepseek/pricing.test.js plugins/claude-code/collector/index.js plugins/claude-code/collector/usage.js plugins/claude-code/collector/process.js plugins/codex/collector/process.js plugins/grok/collector/process.js plugins/claude-code/usage.test.js
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    bun test ./components/plugin-api/*.test.ts ./components/state-store/*.test.ts ./components/app-shell/*.test.ts
    bun test ./tools/check-plugin-components_test.ts ./tools/plugin-component-closure_test.ts
    bun test ./tools/plugin-source-digest_test.ts
    cd plugins && bun test ./collector-sidecars.test.js ./claude-deepseek/pricing.test.js ./claude-code/usage.test.js ./claude-code/resets.test.js
    bun tools/check-plugin-components.ts
    bun x --no-install biome format tools/worker-registry-input.ts tools/worker-registry-input_test.ts
    bun test ./tools/worker-registry-input_test.ts
    bun tools/worker-registry-input.ts

plugin-build PLUGIN:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then
      bun tools/check-plugin-components.ts
    fi
    test -f "plugins/{{PLUGIN}}/plugin.json"
    mkdir -p "dist/plugins/{{PLUGIN}}"
    if [ -n "${COWBOY_PLUGIN_PACK_BIN:-}" ] && [ -x "$COWBOY_PLUGIN_PACK_BIN" ]; then
      "$COWBOY_PLUGIN_PACK_BIN" build \
        "plugins/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    else
      cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- build \
        "plugins/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    fi

example-auth-bundle PLUGIN:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    test -f "examples/authentication/{{PLUGIN}}/plugin.json"
    mkdir -p "dist/plugins/{{PLUGIN}}"
    if [ -n "${COWBOY_PLUGIN_PACK_BIN:-}" ] && [ -x "$COWBOY_PLUGIN_PACK_BIN" ]; then
      "$COWBOY_PLUGIN_PACK_BIN" build \
        "examples/authentication/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    else
      cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- build \
        "examples/authentication/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    fi

# Every bootstrap login host must have an independently buildable signed
# Plugin source. Discovery deliberately has no list of authentication IDs.
# Pack an editor plugin directory (manifest.json + main.js) into one
# installable .cowboy-plugin file; see docs/editor-plugins.md.
editor-plugin-pack DIR OUT:
    bun tools/editor-plugin-pack.ts "{{DIR}}" "{{OUT}}"

example-auth-build-all:
    #!/usr/bin/env bash
    set -euo pipefail
    pack="${COWBOY_PLUGIN_PACK_BIN:-}"
    if [ -z "$pack" ]; then
      cargo build --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
      pack="$PWD/target/debug/cowboy-plugin-pack"
    fi
    pids=()
    for host in examples/authentication/*/host.json; do
      source_dir="${host%/host.json}"
      test -f "$source_dir/plugin.json"
      COWBOY_PLUGIN_PACK_BIN="$pack" just example-auth-bundle "${source_dir##*/}" &
      pids+=("$!")
    done
    status=0
    for pid in "${pids[@]}"; do wait "$pid" || status=$?; done
    exit "$status"

# Data-only backend packages use the exact same pack/sign/publish lifecycle.
example-telemetry-bundle PLUGIN:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    test -f "examples/telemetry/{{PLUGIN}}/plugin.json"
    mkdir -p "dist/plugins/{{PLUGIN}}"
    if [ -n "${COWBOY_PLUGIN_PACK_BIN:-}" ] && [ -x "$COWBOY_PLUGIN_PACK_BIN" ]; then
      "$COWBOY_PLUGIN_PACK_BIN" build \
        "examples/telemetry/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    else
      cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- build \
        "examples/telemetry/{{PLUGIN}}" "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
        "cowboy-plugin://{{PLUGIN}}"
    fi

plugin-build-all:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then
      bun tools/check-plugin-components.ts
    fi
    pack="${COWBOY_PLUGIN_PACK_BIN:-}"
    if [ -z "$pack" ]; then
      cargo build --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
      pack="$PWD/target/debug/cowboy-plugin-pack"
    fi
    pids=()
    for manifest in plugins/*/plugin.json; do
      plugin="${manifest%/plugin.json}"
      COWBOY_CHECK_DEDUPE=1 COWBOY_PLUGIN_PACK_BIN="$pack" just plugin-build "${plugin##*/}" &
      pids+=("$!")
    done
    status=0
    for pid in "${pids[@]}"; do wait "$pid" || status=$?; done
    exit "$status"

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
    pack="${COWBOY_PLUGIN_PACK_BIN:-}"
    if [ -z "$pack" ]; then
      cargo build --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
      pack="$repo_root/target/debug/cowboy-plugin-pack"
    fi
    git ls-files --cached --others --exclude-standard -z -- "plugins/{{PLUGIN}}" | tar --null -T - -cf - | tar -xf - -C "$isolation_root"
    mv "$isolation_root/plugins/{{PLUGIN}}" "$isolation_root/source"
    cd "$isolation_root"
    "$pack" build "$isolation_root/source" "{{PLUGIN}}.cowboy-plugin"
    test -s "{{PLUGIN}}.cowboy-plugin"
    test -s "{{PLUGIN}}.release.json"
    if test -s "$repo_root/dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin"; then
      cmp "{{PLUGIN}}.cowboy-plugin" "$repo_root/dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin"
    fi
    if test "$(jq -r .kind source/plugin.json)" = agent_provider; then
      test -s "{{PLUGIN}}.hostbundle.json"
      host_digest="$(sha256sum "{{PLUGIN}}.hostbundle.json" | cut -d' ' -f1)"
      release_schema="$(jq -r .release_schema "{{PLUGIN}}.release.json")"
      test "$release_schema" = 2 || test "$release_schema" = 4
      test "$(jq -r .host_bundle_digest "{{PLUGIN}}.release.json")" = "sha256:$host_digest"
      if test -s "$repo_root/dist/plugins/{{PLUGIN}}/{{PLUGIN}}.hostbundle.json"; then
        cmp "{{PLUGIN}}.hostbundle.json" "$repo_root/dist/plugins/{{PLUGIN}}/{{PLUGIN}}.hostbundle.json"
      fi
    fi

# Agent Plugin payload helper. Its output is the generic runtime manifest
# consumed by plugin-bind-runtime; it is not an independent release lifecycle.
agent-plugin-runtime-build PLUGIN BASE_URL:
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    test "$(jq -r .kind "plugins/{{PLUGIN}}/plugin.json")" = agent_provider
    if test -f "plugins/{{PLUGIN}}/runtime/build.ts"; then bun "plugins/{{PLUGIN}}/runtime/build.ts" "{{BASE_URL}}"; else bun components/provider-runtime/build.ts "plugins/{{PLUGIN}}" "{{BASE_URL}}"; fi

# No Service credentials or inference: copy an existing rollout into a private
# home and exercise the real packaged ACP launch in a network namespace.
codex-resume-conformance ADAPTER CODEX ROLLOUT RECEIPT:
    python3 plugins/codex/runtime/resume_conformance.py --adapter "{{ADAPTER}}" --codex "{{CODEX}}" --rollout "{{ROLLOUT}}" --receipt "{{RECEIPT}}"

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
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{PLUGIN}}" in (*[!a-z0-9-]*|"") echo "invalid plugin id" >&2; exit 2;; esac
    host_args=()
    host_bundle="dist/plugins/{{PLUGIN}}/{{PLUGIN}}.hostbundle.json"
    if [ -f "$host_bundle" ]; then
      host_args+=("$host_bundle")
    fi
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- sign \
      "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
      "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" \
      "{{PRIVATE_KEY}}" "${host_args[@]}"

plugin-verify PLUGIN PUBLIC_KEY:
    #!/usr/bin/env bash
    set -euo pipefail
    host_args=()
    host_bundle="dist/plugins/{{PLUGIN}}/{{PLUGIN}}.hostbundle.json"
    if [ -f "$host_bundle" ]; then
      host_args+=("$host_bundle")
    fi
    cargo run --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack -- verify \
      "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.cowboy-plugin" \
      "dist/plugins/{{PLUGIN}}/{{PLUGIN}}.release.json" \
      "{{PUBLIC_KEY}}" "${host_args[@]}"

plugin-publish PLUGIN CATALOG PUBLIC_KEY:
    just plugin-verify "{{PLUGIN}}" "{{PUBLIC_KEY}}"
    bun tools/publish-plugin-release.ts "{{PLUGIN}}" "{{CATALOG}}" "{{PUBLIC_KEY}}"

# Exact Linux adapter/server bytes; never install or update a Machine here.
zed-plugin-runtime-build ARTIFACT_BASE:
    bun plugins/zed/runtime/build.ts "{{ARTIFACT_BASE}}"

zed-plugin-conformance ADAPTER SERVER:
    cargo build --offline --locked --manifest-path plugins/zed/adapter/Cargo.toml --example navigation_lsp
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; export TMPDIR=/tmp; mount -t tmpfs -o ro,nosuid,nodev,noexec none /sys/fs/cgroup; export COWBOY_TEST_ZED_ADAPTER="$1" COWBOY_TEST_ZED_SERVER="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib machine_plugins::tests::released_zed_runtime_installs_and_drains -- --ignored --exact --nocapture' conformance "{{ADAPTER}}" "{{SERVER}}"

# Actual private native conditional effect, isolated from ordinary Zed and the
# live Machine. No network, host PID visibility or writable cgroup hierarchy.
# TMPDIR=/tmp keeps the private server's Unix socket paths within their limit.
zed-native-sync-conformance SERVER:
    cargo build --offline --locked --manifest-path plugins/zed/adapter/Cargo.toml --example navigation_lsp
    COWBOY_TEST_NAVIGATION_LSP="$(cargo metadata --offline --locked --manifest-path plugins/zed/adapter/Cargo.toml --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug/examples/navigation_lsp")')" unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; export TMPDIR=/tmp; mount -t tmpfs -o ro,nosuid,nodev,noexec none /sys/fs/cgroup; export COWBOY_TEST_NATIVE_SYNC_SERVER="$1"; exec cargo test --offline --locked --manifest-path plugins/zed/adapter/Cargo.toml sync_native::connected::immutable_native_sync -- --ignored --exact --nocapture' conformance "{{SERVER}}"

# The same isolated gate also drives the exact distributable adapter's socket.
zed-native-navigation-conformance ADAPTER SERVER:
    COWBOY_TEST_NATIVE_NAVIGATION_ADAPTER="{{ADAPTER}}" just zed-native-sync-conformance "{{SERVER}}"

# Real authenticated Controller/Machine/Code chain. No production state or egress.
code-buffer-connected-conformance INPUT RECEIPT:
    cargo build --offline --locked --manifest-path plugins/zed/adapter/Cargo.toml --example navigation_lsp
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; mount -t tmpfs -o ro,nosuid,nodev,noexec none /sys/fs/cgroup; export COWBOY_TEST_CODE_CONNECTED_INPUT="$1" COWBOY_TEST_CODE_CONNECTED_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib immutable_connected_code_buffers -- --ignored --nocapture' conformance "{{INPUT}}" "{{RECEIPT}}"

# Agent artifact probes run on the actual target OS; Code uses the gate above.
# Worker acceptance additionally
# isolates all networking to a private Linux loopback and uses only fake auth.
plugin-runtime-probe RELEASE ARTIFACTS *ARGS:
    python3 tools/plugin_runtime_conformance.py "{{RELEASE}}" "{{ARTIFACTS}}" {{ARGS}}

agent-worker-conformance RELEASE ARTIFACTS WORKER *ARGS:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/plugin_runtime_conformance.py "$@"' conformance "{{RELEASE}}" "{{ARTIFACTS}}" --worker "{{WORKER}}" {{ARGS}}

# Collector checks for a Grok or native Codex pin change, offline in a loopback-only
# namespace with fake logins: re-capture the Grok billing fixture the collector
# reproduces, and show that a refresh-free Codex read reaches no token endpoint.
grok-billing-capture GROK:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/grok_billing_capture.py "$@"' capture --grok "{{GROK}}" --fixture plugins/grok-billing-cli.fixture.json --write plugins/grok-billing-cli.fixture.json

codex-refresh-guard-probe CODEX:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/codex_refresh_guard_probe.py "$@"' probe --codex "{{CODEX}}"

# An `agy` pin must still log the Antigravity proxy's turn-barrier request
# (unauthenticated, closed HOME, no network; see tools/agy-barrier-probe.mjs).
agy-barrier-probe AGY RECEIPT: claude-remote-prepare
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec node --import ./tools/register-memory-client.mjs tools/agy-barrier-probe.mjs "$@"' probe --agy "{{AGY}}" --receipt "{{RECEIPT}}"

# Native execution interfaces only: closed homes, loopback scripted API, no
# production credentials/model inference/Provider install or Machine activation.
execution-codex-turn-conformance CLI VERSION SHA256 RECEIPT:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/execution_environment_codex_turn_probe.py "$@"' conformance --native-cli "{{CLI}}" --version "{{VERSION}}" --sha256 "{{SHA256}}" --receipt "{{RECEIPT}}"

# Native local/remote child cancellation comparison; pins observed CodeAct behavior.
execution-child-stop-conformance CLI SHA256 RECEIPT:
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; exec setpriv --inh-caps=-all --ambient-caps=-all python3 tools/execution_native_child_stop_probe.py "$@"' conformance --native-cli "{{CLI}}" --sha256 "{{SHA256}}" --receipt "{{RECEIPT}}"

execution-claude-turn-conformance CLI VERSION SHA256 RECEIPT:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/execution_environment_claude_probe.py "$@"' conformance --native-cli "{{CLI}}" --version "{{VERSION}}" --sha256 "{{SHA256}}" --receipt "{{RECEIPT}}"

# Compare native TaskStop with a resident keeper, including the unforwarded negative control.
execution-claude-task-stop-conformance KEEPER CLAUDE CLAUDE_SHA EXECUTOR EXECUTOR_SHA RECEIPT:
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; exec setpriv --inh-caps=-all --ambient-caps=-all python3 tools/claude_native_task_stop_probe.py "$@"' conformance --keeper "{{KEEPER}}" --claude "{{CLAUDE}}" --claude-sha256 "{{CLAUDE_SHA}}" --executor "{{EXECUTOR}}" --executor-sha256 "{{EXECUTOR_SHA}}" --receipt "{{RECEIPT}}"

# Research child tool/context inheritance with native Claude and a resident keeper.
execution-claude-agent-research KEEPER CLAUDE CLAUDE_SHA EXECUTOR EXECUTOR_SHA RECEIPT *ARGS:
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; exec setpriv --inh-caps=-all --ambient-caps=-all python3 tools/claude_native_agent_probe.py "$@"' research --keeper "{{KEEPER}}" --claude "{{CLAUDE}}" --claude-sha256 "{{CLAUDE_SHA}}" --executor "{{EXECUTOR}}" --executor-sha256 "{{EXECUTOR_SHA}}" --receipt "{{RECEIPT}}" {{ARGS}}

# Includes two real 35-second observations of executor process retention/expiry.
execution-lifetime-conformance CLI VERSION SHA256 RECEIPT:
    bun run typecheck
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/execution_environment_lifetime_probe.ts "$@"' conformance --native-cli "{{CLI}}" --version "{{VERSION}}" --sha256 "{{SHA256}}" --receipt "{{RECEIPT}}"

# Real detached target keeper; its control clients all disconnect for 35 seconds.
execution-keeper-conformance KEEPER CLI VERSION SHA256 RECEIPT:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/execution_keeper_conformance.py "$@"' conformance --parent-net-namespace "$(readlink /proc/self/ns/net)" --keeper "{{KEEPER}}" --native-cli "{{CLI}}" --version "{{VERSION}}" --sha256 "{{SHA256}}" --receipt "{{RECEIPT}}"

# Actual native Provider launcher -> worker WS -> Machine manager -> keeper.
# The PID namespace owns all detached fixture processes; no production auth.
execution-worker-conformance INPUT RECEIPT:
    cargo build --locked --no-default-features --features machine-host -p cowboy-machine --bin cowboy-execution-host
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    # Namespace setup needs capabilities; native sandbox helpers expect an ordinary user afterward.
    # The target's home is a fresh directory: its user files are fixtures, never this account's.
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; export COWBOY_TEST_EXECUTION_INPUT="$1" COWBOY_TEST_EXECUTION_RECEIPT="$2" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"; export HOME="$(mktemp -d)"; exec setpriv --inh-caps=-all --ambient-caps=-all cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib native_worker_execution -- --ignored --nocapture' conformance "{{INPUT}}" "{{RECEIPT}}"

# A packaged managed Claude turn through a real read-only keeper (see
# docs/managed-agent-calls.md). Same isolation as the worker gate.
execution-managed-conformance INPUT RECEIPT:
    cargo build --locked --no-default-features --features machine-host -p cowboy-machine --bin cowboy-execution-host
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    # Namespace setup needs capabilities; native sandbox helpers expect an ordinary user afterward.
    # The target's home is a fresh directory: its user files are fixtures, never this account's.
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; export COWBOY_TEST_EXECUTION_INPUT="$1" COWBOY_TEST_EXECUTION_RECEIPT="$2" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"; export HOME="$(mktemp -d)"; exec setpriv --inh-caps=-all --ambient-caps=-all cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib native_managed_claude_execution -- --ignored --nocapture' conformance "{{INPUT}}" "{{RECEIPT}}"

execution-image-conformance INPUT RECEIPT:
    cargo build --locked --no-default-features --features machine-host -p cowboy-machine --bin cowboy-execution-host
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; export COWBOY_TEST_EXECUTION_INPUT="$1" COWBOY_TEST_EXECUTION_RECEIPT="$2"; exec setpriv --inh-caps=-all --ambient-caps=-all cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib native_managed_image_execution -- --ignored --nocapture' conformance "{{INPUT}}" "{{RECEIPT}}"

# Actual public login, signed fixture installation and two enrolled Machines.
# All state is disposable; the fixture Agent makes no model requests.
execution-session-conformance INPUT RECEIPT:
    unshare --user --map-current-user --keep-caps --net --pid --fork --mount-proc bash -euc 'ip link set lo up; exec python3 tools/execution_session_conformance.py "$@"' conformance "{{INPUT}}" "{{RECEIPT}}"

# Diagnostic only: does not satisfy release coexistence or authorize migration.
agent-generation-failure-isolation RELEASE ARTIFACTS WORKER *ARGS:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/plugin_generation_failure_isolation.py "$@"' diagnostic "{{RELEASE}}" "{{ARTIFACTS}}" --worker "{{WORKER}}" {{ARGS}}

# Actual immutable readers, public old bytes and a temporary signed future
# fixture. Add --publication <bound-release.json> for each exact candidate,
# including older outer schemas with newer payloads. Never publishes/activates.
catalog-reader-conformance BRIDGE BASELINE CANDIDATE SDK LEGACY_PACKAGE RECEIPT *ARGS:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec python3 tools/catalog_reader_conformance.py "$@"' conformance "{{BRIDGE}}" "{{BASELINE}}" "{{CANDIDATE}}" "{{SDK}}" "{{LEGACY_PACKAGE}}" --receipt "{{RECEIPT}}" {{ARGS}}

# Machine attempt codecs against immutable active/recovery/cold readers.
# Clean source, no external network, installation admission or live state.
plugin-machine-install-reader-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_MACHINE_INSTALL_READER_MATRIX="$1" COWBOY_TEST_MACHINE_INSTALL_READER_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::machine_installation::immutable_machine_installation_readers -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Service-only installation journal floor; Machine attempts have their own gate.
plugin-install-reader-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_INSTALL_READER_MATRIX="$1" COWBOY_TEST_INSTALL_READER_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::installation::immutable_installation_readers -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Real active writer pair; nine reader pairs independently reopen each stopped
# result twice. Recovery readers are NOT asked to admit fresh installation.
# No production credentials, Plugin installation, policy, export or sessions.
plugin-install-connected-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_INSTALL_CONNECTED_MATRIX="$1" COWBOY_TEST_INSTALL_CONNECTED_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::installation_connected::immutable_connected_installation -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Separate telemetry evidence and policy floors remain mandatory.
telemetry-reader-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_TELEMETRY_READER_MATRIX="$1" COWBOY_TEST_TELEMETRY_READER_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::immutable_telemetry_readers -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Separate from journal-reader acceptance: explicit managed startup policy,
# stopped optional export, local recording and no legacy fallback on restart.
telemetry-background-startup-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_TELEMETRY_STARTUP_MATRIX="$1" COWBOY_TEST_TELEMETRY_STARTUP_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::startup::immutable_background_startup -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Actual private writer-policy startup, independent finite purposes and durable
# reopen. Synthetic Operator/peer only; never admits production writers.
telemetry-writer-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_TELEMETRY_WRITER_MATRIX="$1" COWBOY_TEST_TELEMETRY_WRITER_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::admission::immutable_writer_admission -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Connected immutable release pairs, real fixture authentication and bounded
# packet-loss/disconnect faults plus managed OTLP delivery to an isolated HTTP
# receiver. No production credentials, policy, external destination or sessions.
telemetry-connected-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_TELEMETRY_CONNECTED_MATRIX="$1" COWBOY_TEST_TELEMETRY_CONNECTED_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::connected::immutable_connected_telemetry -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Real disposable Victoria databases; no caller-supplied endpoint, credential or
# state path. Separate from the mandatory 45-flow protocol/fault receiver gate.
telemetry-victoria-conformance MATRIX DATABASES RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_TELEMETRY_VICTORIA_MATRIX="$1" COWBOY_TEST_TELEMETRY_VICTORIA_DATABASES="$2" COWBOY_TEST_TELEMETRY_VICTORIA_RECEIPT="$3"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib server::telemetry_binding::resolution::tests::reader_conformance::victoria::immutable_victoria_telemetry -- --ignored --exact --nocapture' conformance "{{MATRIX}}" "{{DATABASES}}" "{{RECEIPT}}"

# Deployment preflight: every embedded Agent Provider version must have an
# exact signed publication receipt and immutable artifact set in the target
# Service Catalog before a Controller carrying those manifests is activated.
provider-release-coverage CATALOG:
    bun tools/check-provider-release-coverage.ts "{{CATALOG}}"

# Cheap Remote Claude feedback before building an upstream upgrade candidate.
claude-remote-check: claude-remote-prepare claude-remote-relay claude-remote-scripts

# The execution host and the pinned Claude ACP sources the runtime tests load.
# The host builds under the Machine test slices' profile, beside the main graph,
# and is linked into target/debug, where the suites run it; no other gate build
# writes that path. The rename is atomic, so a suite already running
# never finds the host missing while a later invocation relinks it.
claude-remote-prepare: claude-source-deps
    #!/usr/bin/env bash
    set -euo pipefail
    {{rust_slice_env}} cargo build --profile {{rust_slice_profile}} --locked --no-default-features --features machine-host -p cowboy-machine --bin cowboy-execution-host
    host=target/gate-slices/cowboy-execution-host
    link=target/debug/cowboy-execution-host
    mkdir -p target/debug
    if ! test "$link" -ef "$host"; then
      ln -f "$host" "$link.$$"
      # A concurrent invocation may have linked the same file first.
      mv -f "$link.$$" "$link" || test "$link.$$" -ef "$link"
      rm -f "$link.$$"
    fi

# The pinned Claude ACP packages that `tools/register-memory-client.mjs`
# resolves `ws` from. It reinstalls only when the pinned manifest changes, so
# suites already running beside a later invocation never lose node_modules.
claude-source-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    src=components/provider-runtime/packages/claude-agent-acp
    dst=dist/claude-source-tests
    if cmp -s "$src/package.json" "$dst/package.json" && cmp -s "$src/package-lock.json" "$dst/package-lock.json" &&
      test -f "$dst/node_modules/.package-lock.json"; then
      exit 0
    fi
    mkdir -p "$dst"
    cp "$src"/package{,-lock}.json "$dst/"
    npm ci --prefer-offline --ignore-scripts --no-audit --no-fund --prefix "$dst"

claude-remote-relay:
    cargo fmt --manifest-path plugins/claude-code/relay/Cargo.toml -- --check
    cargo clippy --locked --manifest-path plugins/claude-code/relay/Cargo.toml --all-targets
    cargo test --locked --manifest-path plugins/claude-code/relay/Cargo.toml
    cargo build --locked --release --manifest-path plugins/claude-code/relay/Cargo.toml
    COWBOY_MCP_RELAY="$PWD/plugins/claude-code/relay/target/release/cowboy-mcp-relay" node --test tools/claude-mcp-relay.test.mjs
    bun x --no-install biome format tools/claude-mcp-relay.test.mjs

# Node and Python suites over sources and the prepared execution host.
claude-remote-scripts: claude-remote-prepare
    node --import ./tools/register-memory-client.mjs --test plugins/claude-code/runtime/*.test.mjs components/memory-client/*.test.mjs tools/memory-provider.test.mjs tools/claude-remote-routing.test.mjs tools/claude-remote-agents.test.mjs tools/claude-remote-permissions.test.mjs tools/claude-remote-hooks.test.mjs tools/claude-remote-shell.test.mjs tools/claude-remote-shell-jobs.test.mjs tools/claude-remote-context.test.mjs tools/claude-remote-skills.test.mjs tools/claude-remote-mcp.test.mjs tools/claude-remote-native-baseline.test.mjs tools/claude-remote-hooks-baseline.test.mjs tools/claude-remote-mods-baseline.test.mjs tools/claude-remote-permissions-baseline.test.mjs tools/claude-remote-context-baseline.test.mjs tools/claude-remote-tools-baseline.test.mjs
    bun x --no-install biome format plugins/claude-code/runtime
    node --import ./tools/register-memory-client.mjs --test tools/claude-remote-privacy.test.mjs
    bun x --no-install biome format tools/claude-remote-privacy.test.mjs
    bun x --no-install biome format tools/claude-remote-routing.test.mjs tools/claude-remote-agents.test.mjs tools/claude-remote-permissions.test.mjs tools/claude-remote-hooks.test.mjs tools/claude-remote-shell.test.mjs tools/claude-remote-shell-jobs.test.mjs tools/claude-remote-shell-fixture.mjs tools/claude-remote-context.test.mjs tools/claude-remote-skills.test.mjs tools/claude-remote-mcp.test.mjs tools/claude-remote-native-baseline.test.mjs tools/claude-remote-hooks-baseline.test.mjs tools/claude-remote-mods-baseline.test.mjs tools/claude-remote-permissions-baseline.test.mjs tools/claude-remote-context-baseline.test.mjs tools/claude-remote-tools-baseline.test.mjs
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    python3 -m unittest discover -s tools -p remote_impact_test.py

# Cross-language package/linker conformance. This is also the Agent Plugin
# payload gate used by the generic Plugin release workflow.
provider-check: plugin-check claude-remote-check provider-scripts-check provider-build-check

# The Bun, Node and Python suites of `provider-check`; they share no build
# directory, so `check` runs them in the background.
provider-scripts-check: provider-antigravity-check provider-runtime-scripts

# Its facade suite waits out real command deadlines; sharding facade.test.mjs
# across processes keeps the hashed plugin source untouched while cutting the
# suite's wall clock nearly in half. Facade tests publish through the prepared
# execution host in target/debug.
provider-antigravity-check: claude-remote-prepare
    #!/usr/bin/env bash
    set -euo pipefail
    other=()
    for f in components/provider-runtime/packages/antigravity-acp/*.test.mjs; do
      if [[ "$f" != */facade.test.mjs ]]; then other+=("$f"); fi
    done
    node --import ./tools/register-memory-client.mjs --test "${other[@]}" &
    p_other=$!
    node tools/run-sharded-node-test.mjs --shards 8 components/provider-runtime/packages/antigravity-acp/facade.test.mjs &
    p_facade=$!
    status=0
    wait "$p_other" || status=$?
    wait "$p_facade" || status=$?
    if ((status != 0)); then exit "$status"; fi
    bun x --no-install biome format components/provider-runtime/packages/antigravity-acp components/provider-runtime/owned-sources.ts tools/shard-node-test.mjs tools/shard-node-test-proxy.mjs tools/run-sharded-node-test.mjs

provider-runtime-scripts: claude-source-deps
    bun test ./tools/mcp-bindings_test.ts
    node --import ./tools/register-memory-client.mjs --test tools/execution-probe_test.mjs
    bun x --no-install biome format components/provider-runtime/execution-connection.mjs components/provider-runtime/execution-probe.mjs tools/execution-probe_test.mjs plugins/codex/runtime/bindings.mjs
    bun x --no-install biome format components/provider-runtime/mcp-bindings.mjs tools/mcp-bindings_test.ts
    node --test components/provider-runtime/packages/codex-acp/launch_test.mjs
    node --test components/provider-runtime/packages/deepseek-harness/launch_test.mjs
    bun tools/generate-plan-name-table.ts --check
    bun x --no-install biome format tools/generate-plan-name-table.ts
    node --import ./tools/register-memory-client.mjs --test plugins/codex/runtime/launch.test.mjs
    node --import ./tools/register-memory-client.mjs --test plugins/codex/runtime/execution-mcp.test.mjs
    bun x --no-install biome format plugins/codex/runtime/build.ts plugins/codex/runtime/launch.mjs plugins/codex/runtime/launch.test.mjs plugins/codex/runtime/memory.mjs plugins/codex/runtime/execution-mcp.mjs plugins/codex/runtime/execution-mcp.test.mjs plugins/codex/runtime/package.json plugins/codex/runtime/package-lock.json
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    bun test ./tools/check-provider-release-coverage_test.ts
    bun test ./tools/provider-runtime-platforms_test.ts
    bun test ./.agents/skills/release-cowboy-plugin/scripts/audit-dependencies_test.ts
    bun test ./tools/third-party_test.ts
    bun tools/third-party.ts check
    bun x --no-install biome format tools/third-party.ts tools/third-party_test.ts tools/lib/third-party.ts third_party
    bun test ./.agents/skills/release-cowboy-plugin/scripts/converge-machine_test.ts
    bun test ./.agents/skills/model-intel/scripts/score_test.ts
    bun test ./.agents/skills/model-intel/scripts/field_test.ts
    bun test ./.agents/skills/model-intel/scripts/periodic-audit_test.ts
    bun test ./tools/plugin-publication-receipt_test.ts
    bun test ./tools/immutable-publication_test.ts
    python3 -m unittest discover -s tools -p plugin_runtime_conformance_test.py
    python3 -m unittest discover -s tools -p plugin_generation_failure_isolation_test.py
    python3 -m unittest discover -s tools -p catalog_reader_conformance_test.py
    bun components/provider-runtime/check.ts

# The Rust tests, Plugin builds and package checks of `provider-check`.
provider-build-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then
      cargo test --locked -p cowboy-provider-sdk --all-targets
      cargo test --locked -p cowboy-plugin-sdk --all-targets
    fi
    pack="${COWBOY_PLUGIN_PACK_BIN:-}"
    if [ -z "$pack" ]; then
      {{rust_slice_env}} cargo build --profile {{rust_slice_profile}} --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
      pack="$PWD/target/{{rust_slice_profile}}/cowboy-plugin-pack"
    fi
    export COWBOY_PLUGIN_PACK_BIN="$pack"
    just plugin-build-all &
    p_plugins=$!
    just example-auth-build-all &
    p_auth=$!
    just example-telemetry-bundle victoria &
    p_tel=$!
    status=0
    wait "$p_plugins" || status=$?
    wait "$p_auth" || status=$?
    wait "$p_tel" || status=$?
    if ((status != 0)); then exit "$status"; fi
    pids=()
    for manifest in plugins/*/plugin.json; do
      plugin="${manifest#plugins/}"
      just plugin-isolation-check "${plugin%/plugin.json}" &
      pids+=("$!")
    done
    for pid in "${pids[@]}"; do wait "$pid" || status=$?; done
    if ((status != 0)); then exit "$status"; fi
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then
      (cd web && bun run typecheck)
    fi
    bun components/provider-ui/validate-packages.ts dist/plugins/*/*.cowboy-plugin
    bun x --no-install biome format tools/check-provider-ui-execution.ts
    bun tools/check-provider-ui-execution.ts dist/plugins/*/*.cowboy-plugin
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then
      (cd web && bun test ./src/providerSdk.test.ts)
    fi

# Quality gates.
composition-generate:
    bun tools/generate-composition-contract.ts --write

composition-check:
    bun x --no-install biome format tools/generate-composition-contract.ts tools/generate-composition-contract_test.ts tools/composition-link-conformance.ts contracts
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then bun run typecheck; fi
    bun test ./tools/generate-composition-contract_test.ts
    bun tools/generate-composition-contract.ts
    bun test ./contracts/composition.test.ts ./contracts/composition-check.test.ts ./contracts/widget-v1.test.ts
    if [ "${COWBOY_CHECK_DEDUPE:-0}" != "1" ]; then {{rust_test_env}} cargo test {{rust_test_args}} composition::; fi
    cowboy="$(just test-graph-bin cowboy)" && bun tools/composition-link-conformance.ts "$cowboy"

fmt:
    cargo fmt --check
    rustfmt --edition 2024 --check plugins/zed/runtime/server/cowboy_worktree_attempts.rs plugins/zed/runtime/server/worktree_attempts_tests.rs
    cd plugins/zed/adapter && cargo fmt --check
    cd plugins/claude-code/relay && cargo fmt --check
    rustfmt --edition 2024 --check plugins/zed/runtime/server/cowboy_sync.rs plugins/zed/runtime/server/cowboy_bounded.rs plugins/zed/runtime/server/cowboy_lsp_input.rs plugins/zed/runtime/server/cowboy_navigation.rs plugins/zed/runtime/server/navigation_tests.rs plugins/zed/runtime/server/tests.rs plugins/zed/runtime/server/cowboy_close.rs plugins/zed/runtime/server/close_tests.rs plugins/zed/runtime/server/reload_tests.rs plugins/zed/runtime/server/cowboy_buffer_budget.rs plugins/zed/runtime/server/cowboy_acquisition.rs plugins/zed/runtime/server/acquisition_tests.rs plugins/zed/runtime/server/cowboy_replacement.rs plugins/zed/runtime/server/replacement_tests.rs plugins/zed/runtime/server/cowboy_full_offsets.rs plugins/zed/runtime/server/cowboy_remote_edits.rs plugins/zed/runtime/server/remote_edits_tests.rs plugins/zed/runtime/server/cowboy_frontier.rs plugins/zed/runtime/server/frontier_tests.rs plugins/zed/runtime/server/cowboy_open_attempts.rs plugins/zed/runtime/server/open_attempts_tests.rs

fmt-write:
    cargo fmt
    cd plugins/zed/adapter && cargo fmt
    cd plugins/claude-code/relay && cargo fmt

lint: clippy zed-lint lint-plugins

# Clippy checks the same crates as the test graph but in check mode, which
# shares no artifacts with it: under the graph's profile it would only wait for
# the build lock. Its own profile (`gate-clippy`, identical to dev) has a lock
# of its own, so `check` and an interactive `just clippy` reuse each other. It
# keeps rustc incremental state even inside `check-compact`: after a one-file
# edit it rechecks the workspace in about 19s against 50s without (2026-11-22 on
# Hawk), for about 3.7G that the host's build cache retention manages.
clippy:
    env -u CARGO_INCREMENTAL cargo clippy --profile gate-clippy --workspace --all-targets --all-features --locked -- -D warnings

# The Zed adapter is its own Cargo workspace with its own build directory.
zed-lint:
    cd plugins/zed/adapter && cargo clippy --all-targets --locked -- -D warnings

zed-test:
    cd plugins/zed/adapter && cargo test --all-targets --locked

zed-check: zed-lint zed-test

# Everything `lint` checks besides workspace Clippy and the Zed adapter.
lint-plugins:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${COWBOY_CHECK_DEDUPE:-0}" = "1" ]; then
      (cd web && bun x --no-install oxlint --ignore-pattern public/wasm_exec.js)
    else
      (cd web && bun run lint)
      (cd plugins/claude-code/relay && cargo clippy --all-targets --locked -- -D warnings)
    fi

dependencies:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo deny check
    # Audit declared workspace members; independent experiments own their gates
    # and may require path-patched locks that recursive metadata would rewrite.
    python3 - <<'PY'
    import json
    import subprocess
    document = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version=1",
    ]))
    members = set(document["workspace_members"])
    paths = [p["manifest_path"] for p in document["packages"] if p["id"] in members]
    if not paths:
        raise SystemExit("dependency audit requires a non-empty workspace")
    subprocess.run(["cargo", "machete", "--with-metadata", *paths], check=True)
    PY
    (cd plugins/zed/adapter && cargo deny check && cargo machete --with-metadata)
    (cd plugins/claude-code/relay && cargo deny check && cargo machete --with-metadata)

typecheck:
    cd web && bun run typecheck

# Keep independently packaged feature slices honest. An all-features build can
# hide accidental dependencies on modules that are absent from these releases.
feature-check:
    cargo check --locked --no-default-features --features machine-host -p cowboy-machine --bin cowboy-machine --bin cowboy-machine-install
    cargo check --locked --no-default-features --features code-adapter --bin cowboy-code-adapter

# Rust-serialized wire fixtures (contracts/protocol, owned by
# src/protocol_contract.rs) must be current in their generated TypeScript
# module, whose literals the web typecheck holds to web/src/protocol.ts.
protocol-contract-check:
    bun x --no-install biome format tools/generate-protocol-fixtures.ts
    bun tools/generate-protocol-fixtures.ts --check

# docs/INDEX.md links every docs/ Markdown page and no local link is broken.
docs-index-check:
    bash tools/check-docs-index.sh

# Size ratchet: no production file, function or TSX hook count over its limit
# may appear or grow (tools/structure-baseline.json). After moving an offender
# unchanged, record the move with: bun tools/check-structure.ts --update
structure-check:
    bun x --no-install biome format tools/check-structure.ts tools/check-structure_test.ts
    bun test ./tools/check-structure_test.ts
    bun tools/check-structure.ts
    bun x --no-install biome format tools/check-module-layers.ts tools/check-module-layers_test.ts
    bun test ./tools/check-module-layers_test.ts
    bun tools/check-module-layers.ts

# The Machine and code-adapter packages compile hand-listed source closures
# (machine-src, code-adapter-src in flake.nix), which a full-checkout cargo run
# never sees. Type-check inside the exact filtered trees with the package
# features, so a module missing from a closure fails here rather than in a
# release build. One shared target directory keeps reruns incremental.
release-sources-check:
    #!/usr/bin/env bash
    set -euo pipefail
    target="$PWD/target/release-sources"
    # A stable tree path keeps Cargo fingerprints valid between runs.
    scratch="$target/trees"
    # Evaluate and materialize both source closures in one Nix invocation.
    readarray -t built_sources < <(nix build --no-link --print-out-paths .#cowboy-machine-src .#cowboy-code-adapter-src)
    check() {
      local output="$1" source="$2"; shift 2
      local tree="$scratch/$output"
      source="$(readlink -f "$source")"
      # The source is content-addressed: an unchanged store path means an
      # unchanged tree, and keeping its mtimes keeps Cargo's fingerprints.
      if test "$(cat "$tree.source" 2>/dev/null)" != "$source"; then
        rm -rf "$tree" "$tree.source"
        mkdir -p "$scratch"
        cp -r --no-preserve=mode "$source" "$tree"
        printf '%s\n' "$source" > "$tree.source"
      fi
      echo "== $output"
      # Intermediates stay under the checkout too, where check-compact's
      # SCCACHE_BASEDIRS covers them, and the directories are flags, not
      # CARGO_* variables (see rust_test_args): another worktree's identical
      # dependencies then hit the cache.
      (cd "$tree" && cargo check --target-dir "$target" --config "build.build-dir=\"$target\"" --locked "$@")
    }
    check cowboy-machine-src "${built_sources[0]}" --no-default-features --features machine-host \
      -p cowboy --bin cowboy -p cowboy-machine \
      --bin cowboy-machine --bin cowboy-machine-install --bin cowboy-execution-host
    check cowboy-code-adapter-src "${built_sources[1]}" --no-default-features --features code-adapter \
      --bin cowboy-code-adapter

test: test-rust zed-test test-web

test-web:
    cd web && bun run test

# The main workspace's Rust test selections, which share its build directory.
test-rust:
    # nextest builds the same `cargo test` executables but runs every test
    # binary concurrently; libtest runs them one after another, so the slowest
    # library suites used to add up (152s against 62s for the same 3553 tests).
    {{rust_test_env}} cargo nextest run {{rust_test_args}}
    # The shared wire crate is a workspace member, so the root package does not run its tests.
    {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_wire_args}}
    # The standalone Machine ships without Controller modules. Compile and run
    # its test slice too; cargo check alone does not cover cfg(test) boundaries.
    {{rust_slice_env}} cargo nextest run --cargo-profile {{rust_slice_profile}} {{rust_machine_args}}
    {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_code_adapter_args}}
    python3 -m unittest discover -s tools -p ovh_machine_idle_sessions_test.py

# Compiles the shared test graph, so `check` can start it before the static
# checks rather than after them, and records the JSON artifact map so later
# steps (`test-graph-bin`, `test-postgres`, `logs-conformance`) do not rerun
# Cargo's fingerprint/link pass just to discover executable paths.
test-graph-build:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/test-graph
    tmp="target/test-graph/artifacts.$$.json"
    trap 'rm -f "$tmp"' EXIT
    {{rust_test_env}} cargo test {{rust_test_args}} --no-run --message-format=json-render-diagnostics > "$tmp"
    mv -f "$tmp" target/test-graph/artifacts.json

# Compiles the test slices `test-rust` runs, so `check` can build them beside
# the main graph; `test-rust` then only runs them. The execution host shares
# their directory and comes first: the script suites wait for it.
test-rust-slices-build: claude-remote-prepare
    {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_wire_args}} --no-run
    {{rust_slice_env}} cargo build --profile {{rust_slice_profile}} --locked -p cowboy-plugin-sdk --bin cowboy-plugin-pack
    {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_machine_args}} --no-run
    {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_code_adapter_args}} --no-run

# Ignored PostgreSQL tests each get a fresh database in a private, temporary
# cluster. Never consumes a caller-provided database URL or starts a TCP listener.
test-postgres:
    {{rust_test_env}} COWBOY_RUST_TEST_ARGS="{{rust_test_args}}" bash tools/test-postgres.sh

# Actual immutable private writer ELFs; no production writer or host namespace.
session-deletion-writer-conformance OLD NEW READER RECEIPT:
    unshare --user --map-root-user --mount --pid --net --fork --mount-proc python3 tools/session_deletion_writer_conformance.py --old-fixture "{{OLD}}" --new-fixture "{{NEW}}" --reader-release "{{READER}}" --receipt "{{RECEIPT}}"

# Actual production binaries need real root ownership, but all authority mounts,
# broker state, processes and networking live in independent private namespaces.
session-deletion-production-conformance OLD NEW READER DEFAULT_READER RECEIPT:
    #!/usr/bin/env bash
    set -euo pipefail
    host_mount=$(readlink /proc/self/ns/mnt)
    host_net=$(readlink /proc/self/ns/net)
    sudo -n unshare --mount --propagation private --pid --net --fork --mount-proc \
      "$(command -v python3)" -B tools/session_deletion_production_conformance.py \
      --old-writer "{{OLD}}" --new-writer "{{NEW}}" --reader-release "{{READER}}" \
      --default-reader-release "{{DEFAULT_READER}}" --receipt "{{RECEIPT}}" \
      --host-mount-ns "$host_mount" --host-net-ns "$host_net"

# Complete gate. Release binaries are not rebuilt here: the debug graphs above
# compile the same code, and the Nix release packages are the release build.
# `build-web` keeps the production frontend bundle covered. `feature-check` is
# not repeated: `release-sources-check` checks the same packages, features and
# binaries inside the exact release source closures.
# The shared test graph's compile starts first: on a fresh worktree it is the
# longest chain, and Cargo's build lock makes later main-directory steps wait for
# it rather than compete. Workspace Clippy, the feature-sliced Rust tests, the
# release-source slices and the Zed adapter have Cargo build directories of
# their own, and the script suites and the frontend build share none, so they
# start with it and run beside the rest of the gate: the main graph's compile is
# a long single-crate chain that leaves most cores idle. The second-scale static checks then run in the
# foreground, so a formatting, ratchet or lint failure still surfaces within
# seconds. A failed background job stops the gate at the next foreground step
# instead of after the Rust builds. Each background log prints once it is
# collected, before the main Rust tests run.
check: toolchain-check
    #!/usr/bin/env bash
    set -euo pipefail
    logs="$(mktemp -d "${TMPDIR:-/tmp}/cowboy-check.XXXXXX")"
    js_fifo="$logs/jobserver.fifo"
    mkfifo "$js_fifo"
    exec 9<>"$js_fifo"
    # Coordinate the 6 concurrent Cargo lanes through one POSIX FIFO jobserver
    # so they share the host's cores instead of spawning 6 * nproc rustc jobs.
    cores="$(nproc 2>/dev/null || echo 8)"
    tokens=$((cores > 4 ? cores - 4 : 2))
    printf "%${tokens}s" "" | tr ' ' '+' >&9
    cargo_js="MAKEFLAGS=--jobserver-auth=fifo:$js_fifo"
    export COWBOY_CHECK_DEDUPE=1
    rm -f target/test-graph/artifacts.json
    pending=()
    cleanup() {
      local job
      # Before setsid runs, the job is not yet a group of its own.
      for job in "${pending[@]}"; do
        kill -- "-${job%%:*}" 2>/dev/null || kill "${job%%:*}" 2>/dev/null || true
      done
      { exec 9>&-; } 2>/dev/null || true
      rm -rf "$logs"
    }
    trap cleanup EXIT
    background() {
      local name="$1"
      shift
      # A process group of its own, so an early failure stops the whole job.
      # The status file lets `reap` see the outcome before the job is collected.
      setsid bash -c 'SECONDS=0; env "${@:3}" just "$2"; status=$?; echo "($2 took ${SECONDS}s)"; echo "$status" >"$1.status"; exit "$status"' \
        background "$logs/$name" "$name" "$@" >"$logs/$name.log" 2>&1 &
      pending+=("$!:$name")
    }
    collect() {
      local job="$1" status=0
      wait "${job%%:*}" || status=$?
      echo "==> ${job#*:} (background, exit $status)"
      cat "$logs/${job#*:}.log"
      if ((status != 0)); then exit "$status"; fi
    }
    collect_named() {
      local target="$1" next=() job found=""
      for job in "${pending[@]}"; do
        if [[ "${job#*:}" == "$target" ]]; then
          found="$job"
        else
          next+=("$job")
        fi
      done
      pending=("${next[@]}")
      if [[ -n "$found" ]]; then collect "$found"; fi
    }
    # Between foreground steps: stop at a background job that already failed.
    reap() {
      local job status
      for job in "${pending[@]}"; do
        status="$(cat "$logs/${job#*:}.status" 2>/dev/null || true)"
        if [[ -n "$status" && "$status" != 0 ]]; then collect "$job"; fi
      done
    }
    # The shared test graph is the longest chain on a fresh worktree, so it
    # starts first; composition-check and provider-build-check consume its
    # recorded artifact map once collected.
    background test-graph-build "$cargo_js"
    # The Node suites resolve `ws` from the prepared Claude ACP packages.
    just claude-source-deps
    # These three each start with `claude-remote-prepare`: whichever builds the
    # execution host first, the others find it built. The suites are collected
    # before the main Rust tests, so they never overlap them.
    background provider-antigravity-check "$cargo_js"
    background claude-remote-scripts "$cargo_js"
    background test-rust-slices-build "$cargo_js"
    background clippy "$cargo_js"
    background release-sources-check "$cargo_js"
    background zed-check "$cargo_js"
    background provider-runtime-scripts
    background test-web
    background build-web
    background dependencies
    # The MCP relay is a Cargo workspace of its own (plugins/claude-code/relay).
    background claude-remote-relay "$cargo_js"
    just fmt structure-check docs-index-check protocol-contract-check typecheck
    reap
    just idb-conformance-check site-check native-shell-check lint-plugins plugin-check
    reap
    # provider-build-check only needs cowboy-plugin-pack from test-rust-slices-build,
    # so start it in the background as soon as the slices finish while test-graph-build
    # is still compiling.
    collect_named test-rust-slices-build
    background provider-build-check "COWBOY_PLUGIN_PACK_BIN=$PWD/target/{{rust_slice_profile}}/cowboy-plugin-pack"
    collect_named test-graph-build
    export COWBOY_TEST_GRAPH_ARTIFACTS="$PWD/target/test-graph/artifacts.json"
    just composition-check
    reap
    while ((${#pending[@]})); do
      job="${pending[0]}"
      pending=("${pending[@]:1}")
      collect "$job"
    done
    # Stop sharing the compile jobserver before timing-sensitive test suites run.
    unset MAKEFLAGS
    { exec 9>&-; } 2>/dev/null || true
    # Run the two full-core nextest suites alone first, then overlap the small
    # post-suites (wire, code-adapter, python, postgres, logs-conformance).
    {{rust_test_env}} cargo nextest run {{rust_test_args}}
    {{rust_slice_env}} cargo nextest run --cargo-profile {{rust_slice_profile}} {{rust_machine_args}}
    post_jobs=()
    run_post() {
      local name="$1"; shift
      (
        SECONDS=0
        "$@"
        status=$?
        echo "($name took ${SECONDS}s)"
        exit "$status"
      ) >"$logs/post-$name.log" 2>&1 &
      post_jobs+=("$!:$name")
    }
    run_post wire {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_wire_args}}
    run_post code-adapter {{rust_slice_env}} cargo test --profile {{rust_slice_profile}} {{rust_code_adapter_args}}
    run_post idle-sessions python3 -m unittest discover -s tools -p ovh_machine_idle_sessions_test.py
    run_post test-postgres just test-postgres
    run_post logs-conformance just logs-conformance
    post_status=0
    for job in "${post_jobs[@]}"; do
      st=0
      wait "${job%%:*}" || st=$?
      echo "==> ${job#*:} (post, exit $st)"
      cat "$logs/post-${job#*:}.log"
      if ((st != 0 && post_status == 0)); then post_status=$st; fi
    done
    exit "$post_status"

# Real diagnostic CLI, typed OTel fixture, query/metrics/tail/watch/RPC contracts.
# An optional explicit --ssh in the standalone tool adds same-host SSH acceptance.
logs-conformance:
    #!/usr/bin/env bash
    set -euo pipefail
    # Reuse the shared test graph's library test and CLI executables; the
    # tool's own root-only `cargo test --lib` selection, like a separate
    # `cargo build --bin cowboy`, would compile the workspace again.
    if [[ -n "${COWBOY_TEST_GRAPH_ARTIFACTS:-}" && -r "$COWBOY_TEST_GRAPH_ARTIFACTS" ]]; then
      artifacts="$(cat "$COWBOY_TEST_GRAPH_ARTIFACTS")"
    else
      artifacts="$({{rust_test_env}} cargo test {{rust_test_args}} --no-run --message-format=json-render-diagnostics)"
    fi
    fixture="$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "cowboy" and .profile.test and (.target.kind | index("lib"))) | .executable // empty' <<<"$artifacts")"
    cowboy="$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "cowboy" and (.profile.test | not) and (.target.kind | index("bin"))) | .executable // empty' <<<"$artifacts")"
    test -x "$fixture"
    test -x "$cowboy"
    python3 tools/logs_conformance.py --cowboy "$cowboy" --test-executable "$fixture" --receipt target/logs-conformance.json

# The shared test graph's executable for binary NAME. Recipes run it instead of
# a `cargo build --bin`, whose own feature selection compiles the workspace again.
[private]
test-graph-bin NAME:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -n "${COWBOY_TEST_GRAPH_ARTIFACTS:-}" && -r "$COWBOY_TEST_GRAPH_ARTIFACTS" ]]; then
      executable="$(jq -r --arg name '{{NAME}}' 'select(.reason == "compiler-artifact" and .target.name == $name and (.profile.test | not) and (.target.kind | index("bin"))) | .executable // empty' "$COWBOY_TEST_GRAPH_ARTIFACTS")"
    else
      executable="$({{rust_test_env}} cargo test {{rust_test_args}} --no-run --message-format=json-render-diagnostics |
        jq -r --arg name '{{NAME}}' 'select(.reason == "compiler-artifact" and .target.name == $name and (.profile.test | not) and (.target.kind | index("bin"))) | .executable // empty')"
    fi
    test -x "$executable"
    printf '%s\n' "$executable"

# Run the complete quality gate through a compiler cache shared by every
# worktree of this user. Only the shared test graph (`rust_test_env`), its
# feature slices (`rust_slice_env`) and Clippy keep incremental state; every
# other selection compiles without growing one. The server is
# private to this checkout (its own socket, started in the caller's cgroup and
# stopped on exit), so one task's compiles never run in another task's process
# tree. SCCACHE_BASEDIRS strips the checkout and build paths from compiler
# arguments, so a fresh worktree reuses registry dependencies compiled by
# another checkout. Workspace crates (their absolute CARGO_MANIFEST_DIR) and
# crates with build scripts (their OUT_DIR) still carry checkout paths in their
# keys and never hit across worktrees.
# Concurrent gates' servers share the disk cache; each evicts by its own size
# accounting, which can cost hits but never yields a wrong artifact. A reused
# rlib keeps its producer's debug paths; debug a gate build only after rebuilding.
check-compact: (compact "check")

# Prepare a fresh worktree while reading the task: install the dependencies and
# compile every Rust lane of the gate (the shared test graph, Clippy, the
# feature slices with the execution host, the release-source slices, the Zed
# adapter and the MCP relay) through the same compiler cache, so the first
# `check-compact` starts warm instead of spending minutes compiling. A lane
# that fails only stays cold; the gate reports the failure itself.
warm: install (compact "warm-lanes")

[private]
warm-lanes:
    #!/usr/bin/env bash
    set -uo pipefail
    js_dir="$(mktemp -d "${TMPDIR:-/tmp}/cowboy-warm-js.XXXXXX")"
    js_fifo="$js_dir/jobserver.fifo"
    mkfifo "$js_fifo"
    exec 9<>"$js_fifo"
    cores="$(nproc 2>/dev/null || echo 8)"
    tokens=$((cores > 4 ? cores - 4 : 2))
    printf "%${tokens}s" "" | tr ' ' '+' >&9
    export MAKEFLAGS="--jobserver-auth=fifo:$js_fifo"
    trap '{ exec 9>&-; } 2>/dev/null || true; rm -rf "$js_dir"' EXIT
    lane() {
      local log status
      log="$(mktemp "${TMPDIR:-/tmp}/cowboy-warm.XXXXXX")"
      SECONDS=0
      just "$1" >"$log" 2>&1
      status=$?
      echo "warm: $1 exit $status after ${SECONDS}s"
      if ((status != 0)); then tail -n 20 "$log"; fi
      rm -f "$log"
    }
    lane test-graph-build &
    lane clippy &
    lane test-rust-slices-build &
    lane release-sources-check &
    lane zed-check &
    lane claude-remote-relay &
    wait

# Run RECIPE through the private compiler cache described above.
[private]
compact RECIPE:
    #!/usr/bin/env bash
    set -euo pipefail
    # Full gates from parallel task worktrees share one host. Unbounded, they
    # drove Hawk's load past 400 and timed out worker heartbeats, so a gate
    # takes one of COWBOY_GATE_SLOTS host-wide slots (default 2; 0 disables)
    # and waits for a free one. Hold the lock on FD 8 and close 8>&- on every
    # child (especially daemonized sccache) so orphaned children can never pin
    # a slot after this script exits.
    slots="${COWBOY_GATE_SLOTS:-2}"
    if [[ "$slots" =~ ^[1-9][0-9]*$ ]] && command -v flock >/dev/null; then
      slot_dir="${XDG_RUNTIME_DIR:-/tmp}/cowboy-gate-slots"
      mkdir -p "$slot_dir"
      announced=""
      while :; do
        for ((i = 0; i < slots; i++)); do
          exec 8>"$slot_dir/$i.lock"
          if flock -n 8; then break 2; fi
          exec 8>&-
        done
        if [[ -z "$announced" ]]; then
          echo "{{RECIPE}}: all $slots gate slots are busy; waiting (COWBOY_GATE_SLOTS)" >&2
          announced=1
        fi
        sleep 5
      done
    fi
    build_dir() { (cd "$1" && cargo metadata --format-version 1 --no-deps --locked | jq -r '.build_directory // .target_directory') 8>&-; }
    root="$PWD"
    main_build="$(build_dir .)"
    zed_build="$(build_dir plugins/zed/adapter)"
    export CARGO_INCREMENTAL=0 SCCACHE_IGNORE_SERVER_IO_ERROR=1
    export SCCACHE_DIR="${SCCACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/cowboy-sccache}"
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-40G}"
    export SCCACHE_BASEDIRS="$root:$main_build:$zed_build"
    # One server per invocation: a short private socket directory, so a second
    # gate in the same checkout neither stops nor reuses this one.
    unset RUSTC_WRAPPER
    stop() {
      (sccache --show-stats --stats-format json 2>/dev/null | jq -c '{requests: .stats.compile_requests, hits: ([.stats.cache_hits.counts[]?] | add // 0), misses: ([.stats.cache_misses.counts[]?] | add // 0)}' || true) 8>&-
      sccache --stop-server >/dev/null 2>&1 8>&- || true
      rm -rf "$socket_dir"
    }
    # The cache only speeds the gate up: if its server cannot start, run the
    # same gate uncached instead of failing it.
    if socket_dir="$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/cowboy-sccache.XXXXXX" 2>/dev/null)" &&
      SCCACHE_SERVER_UDS="$socket_dir/server.sock" sccache --start-server >/dev/null 2>&1 8>&-; then
      export SCCACHE_SERVER_UDS="$socket_dir/server.sock" RUSTC_WRAPPER=sccache
      trap stop EXIT
    else
      echo "{{RECIPE}}: sccache unavailable; running uncached" >&2
      if [[ -n "${socket_dir:-}" ]]; then rm -rf "$socket_dir"; fi
    fi
    just {{RECIPE}} 8>&-

# Exercise the actual private Operator CLI/Controller and enrolled Machine.
# Only active entries write; other matrix entries are resolved, not reader-tested.
plugin-local-operator-conformance MATRIX RECEIPT:
    cargo test --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib --no-run
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; export COWBOY_TEST_LOCAL_OPERATOR_MATRIX="$1" COWBOY_TEST_LOCAL_OPERATOR_RECEIPT="$2"; exec cargo test --offline --locked -p cowboy -p cowboy-server -p cowboy-machine -p cowboy-integration-tests -p cowboy-wire --all-features --lib immutable_local_operator_installation -- --ignored --nocapture' conformance "{{MATRIX}}" "{{RECEIPT}}"

# Iterate on part of the shared test graph: `just test-rust-filter <nextest
# filters or options>`, for example `just test-rust-filter admin_api` or
# `just test-rust-filter -E 'package(cowboy-machine)'`. A narrower `cargo test
# -p` or feature selection compiles the workspace again, for minutes and then
# for every edit, and leaves another multi-gigabyte copy in the build directory.
[positional-arguments]
test-rust-filter *ARGS:
    {{rust_test_env}} cargo nextest run {{rust_test_args}} "$@"

[positional-arguments]
test-fast *ARGS:
    {{rust_test_env}} cargo nextest run {{rust_test_args}} -E 'package(cowboy) | package(cowboy-store)' "$@"

# Compact-control-plane memory fixture against a local SQLite Durable Object.
do-memory-mock:
    bun tools/do-memory-mock/mock.ts
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
    RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo build -p cowboy -p cowboy-server -p cowboy-machine --all-targets --all-features --locked
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

# Actual cover/footer DOM with overlay and resized keyboard geometry.
sheet-keyboard-browser-conformance BROWSER:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/idb-browser-conformance.ts "$1" sheet-keyboard' conformance "{{BROWSER}}"

# Actual cached transcript recovery after failed/malformed bootstrap responses.
transcript-recovery-browser BROWSER BUNDLE:
    unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/send-latency-browser.ts "$1" "$2" --transcript-recovery' recovery "{{BROWSER}}" "{{BUNDLE}}"

# Real local presentation before slow restoration/save, and receipt/echo gaps.
# Pass a pinned Firefox or Chromium executable; no normal browser profile.
local-presentation-browser-conformance BROWSER:
    #!/usr/bin/env bash
    set -euo pipefail
    fixture=$(mktemp -d /tmp/cowboy-local-presentation.XXXXXX)
    trap 'rm -rf "$fixture"' EXIT
    for suite in local metadata; do
      if ! node tools/send-latency-bundle.mjs "$fixture/$suite" "--$suite" > "$fixture/$suite-build.log" 2>&1; then
        tail -80 "$fixture/$suite-build.log"
        exit 1
      fi
    done
    for suite in local queued metadata; do
      bundle=local
      if [ "$suite" = metadata ]; then bundle=metadata; fi
      unshare --user --map-current-user --keep-caps --net bash -euc 'ip link set lo up; exec bun tools/send-latency-browser.ts "$1" "$2" "$3"' conformance "{{BROWSER}}" "$fixture/$bundle" "--$suite"
    done
