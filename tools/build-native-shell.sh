#!/usr/bin/env bash
# Build Cowboy's native product from this clean Git revision. Build-only:
# no signing credentials, provisioning updates, installation or app launch.
set -euo pipefail
native_platform="${1:-ios-sim}"
shift || true
native_flags=(--ci)
native_profile=release
if [ "${1:-}" = --debug ]; then native_flags+=(--debug); native_profile=debug; shift; fi
if [ "$#" != 0 ]; then
  echo "usage: bash tools/build-native-shell.sh {macos|ios-sim|ios} [--debug]" >&2
  exit 2
fi
case "$native_platform" in macos|ios-sim|ios) ;; *) echo "unknown native platform" >&2; exit 2;; esac
native_repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$native_repo"
test "$(uname -s)" = Darwin || { echo "Apple builds require a Mac with Xcode" >&2; exit 1; }
test "$(uname -m)" = arm64 || { echo "This build entry targets an arm64 Mac" >&2; exit 1; }
test -z "$(git status --porcelain)" || { echo "Native builds require clean, committed source" >&2; exit 1; }
native_revision="$(git rev-parse HEAD)"
deno run --allow-read tools/check-native-shell.ts
native_rust="$(python3 -c 'import json; print(json.load(open("apps/native-shell/toolchain.json"))["rust"])')"
native_cli="$(python3 -c 'import json; print(json.load(open("apps/native-shell/toolchain.json"))["tauriCli"])')"
native_xcodegen="$(python3 -c 'import json; print(json.load(open("apps/native-shell/toolchain.json"))["xcodegen"])')"
# Select a preinstalled toolchain explicitly; never install an ambient latest.
rustup toolchain list | grep -Fq "$native_rust-aarch64-apple-darwin" || {
  echo "Install the pinned Rust $native_rust toolchain and required Apple targets first" >&2
  exit 1
}
export RUSTUP_TOOLCHAIN="$native_rust"
test "$(rustc --version | cut -d ' ' -f 2)" = "$native_rust"
test "$(cargo tauri --version)" = "tauri-cli $native_cli" || {
  echo "Expected tauri-cli $native_cli; see apps/native-shell/README.md" >&2; exit 1;
}
if [ "$native_platform" != macos ]; then
  test "$(xcodegen --version)" = "Version: $native_xcodegen"
  native_triple=aarch64-apple-ios
  if [ "$native_platform" = ios-sim ]; then native_triple=aarch64-apple-ios-sim; fi
  rustup target list --installed | grep -Fxq "$native_triple" || {
    echo "Install the pinned Rust target first: rustup target add --toolchain $native_rust $native_triple" >&2
    exit 1
  }
fi
xcodebuild -version
# Credentials are neither inputs to nor side effects of this unsigned/ad-hoc
# build. Do not let an inherited release environment open a signing keychain.
unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_DEVELOPMENT_TEAM
unset APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
unset APPLE_ID APPLE_PASSWORD APPLE_PROVISIONING_PROFILE APPLE_TEAM_ID
unset TAURI_CONFIG TAURI_ENV_TARGET_TRIPLE TAURI_SIGNING_PRIVATE_KEY
unset CARGO_TARGET_DIR RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
export LANG=en_US.UTF-8
mkdir -p dist/native-shell
native_build="$(mktemp -d "$native_repo/dist/native-shell/$native_platform-${native_revision:0:12}.XXXXXX")"
echo "Native build directory: $native_build"
# Git archive is a local staging operation, not a cross-machine source copy.
# It includes tracked icons/bridges and cannot borrow an unversioned Mac shell.
git archive "$native_revision" apps/native-shell | tar -xf - -C "$native_build"
native_source="$native_build/apps/native-shell"
export CARGO_TARGET_DIR="$native_build/target"
cd "$native_source/tauri"
if [ "$native_platform" = macos ]; then
  cargo tauri build --bundles app --target aarch64-apple-darwin "${native_flags[@]}" -- --locked
  native_app="$CARGO_TARGET_DIR/aarch64-apple-darwin/$native_profile/bundle/macos/Cowboy.app"
else
  mkdir -p gen/apple
  # The Xcode project itself is generated, but ALL handwritten sources,
  # entitlements, launch resources and XcodeGen settings come from this commit.
  cp -R ../apple/Sources ../apple/Assets.xcassets ../apple/cowboy-app_iOS gen/apple/
  cp ../apple/project.yml ../apple/LaunchScreen.storyboard ../apple/ExportOptions.plist gen/apple/
  mkdir -p gen/apple/Externals gen/apple/assets
  xcodegen generate --spec gen/apple/project.yml
  native_target=aarch64
  if [ "$native_platform" = ios-sim ]; then native_target=aarch64-sim; fi
  cargo tauri ios build --no-sign --archive-only --target "$native_target" "${native_flags[@]}" -- --locked
  native_app="$native_source/tauri/gen/apple/build/cowboy-app_iOS.xcarchive/Products/Applications/Cowboy.app"
fi
test -d "$native_app" || { echo "Build returned no app at its exact output path: $native_app" >&2; exit 1; }
cmp "$native_source/tauri/Cargo.lock" "$native_repo/apps/native-shell/tauri/Cargo.lock"
# Record the actual executable and source bytes, not a glob-selected DerivedData
# product or an old receipt. Failure above leaves no success receipt.
python3 - "$native_app" "$native_revision" "$native_platform" "$native_profile" "$native_build" <<'PY'
import hashlib, json, pathlib, plistlib, subprocess, sys
app, revision, platform, profile, build = sys.argv[1:]
app, build = pathlib.Path(app), pathlib.Path(build)
info_path = app / ("Contents/Info.plist" if platform == "macos" else "Info.plist")
info = plistlib.loads(info_path.read_bytes())
binary = app / ("Contents/MacOS" if platform == "macos" else "") / info["CFBundleExecutable"]
if info["CFBundleIdentifier"] != "top.thundersparrow.cowboy":
    raise SystemExit("Unexpected native bundle identifier")
report = dict(source_revision=revision, platform=platform, profile=profile,
    app=str(app), executable_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
    lock_sha256=hashlib.sha256((build / "apps/native-shell/tauri/Cargo.lock").read_bytes()).hexdigest(),
    xcode=subprocess.check_output(["xcodebuild", "-version"], text=True).strip(),
    toolchain=json.loads((build / "apps/native-shell/toolchain.json").read_text()),
    signing="ad-hoc" if platform == "macos" else "unsigned",
    installed=False, real_login="not_checked", physical_device="not_checked")
(build / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
