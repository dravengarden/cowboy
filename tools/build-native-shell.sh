#!/usr/bin/env bash
# Build Cowboy's native product from this clean Git revision. Build-only:
# no signing credentials, provisioning updates, installation or app launch.
set -euo pipefail
native_platform="${1:-ios-sim}"
shift || true
native_flags=(--ci)
native_profile=release
if [ "${1:-}" = --debug ]; then native_flags+=(--debug); native_profile=debug; shift; fi
native_report_target=
if [ "${1:-}" = --receipt-path ]; then
  native_report_target="${2:?--receipt-path requires an absolute path}"
  shift 2
  case "$native_report_target" in /*) ;; *) echo "receipt path must be absolute" >&2; exit 2;; esac
  test ! -e "$native_report_target" && test ! -L "$native_report_target" || {
    echo "refusing to overwrite a native receipt" >&2; exit 1;
  }
fi
if [ "$#" != 0 ]; then
  echo "usage: bash tools/build-native-shell.sh {macos|ios-sim|ios|android|android-emu} [--debug] [--receipt-path /absolute/new.json]" >&2
  exit 2
fi
case "$native_platform" in macos|ios-sim|ios|android|android-emu) ;; *) echo "unknown native platform" >&2; exit 2;; esac
native_repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$native_repo"
case "$native_platform" in
  android*)
    test "$(uname -s)" = Linux || { echo "Android builds run on a Linux build host" >&2; exit 1; }
    test "${COWBOY_NATIVE_ANDROID_SHELL:-}" = 1 || {
      echo "Run Android builds inside: nix develop .#native-android" >&2; exit 1;
    }
    ;;
  *)
    test "$(uname -s)" = Darwin || { echo "Apple builds require a Mac with Xcode" >&2; exit 1; }
    test "$(uname -m)" = arm64 || { echo "This build entry targets an arm64 Mac" >&2; exit 1; }
    ;;
esac
test -z "$(git status --porcelain)" || { echo "Native builds require clean, committed source" >&2; exit 1; }
native_revision="$(git rev-parse HEAD)"
deno run --allow-read tools/check-native-shell.ts
native_rust="$(python3 -c 'import json; print(json.load(open("apps/native-shell/toolchain.json"))["rust"])')"
native_cli="$(python3 -c 'import json; print(json.load(open("apps/native-shell/toolchain.json"))["tauriCli"])')"
case "$native_platform" in android*)
  # The native-android Nix shell provides the pinned rustc/Android targets and
  # Tauri CLI. The SDK/NDK remain SDK Manager-owned; require their exact pins.
  test "$(rustc --version | cut -d ' ' -f 2)" = "$native_rust" || {
    echo "Expected rustc $native_rust from the native-android shell" >&2; exit 1;
  }
  test "$(cargo tauri --version)" = "tauri-cli $native_cli" || {
    echo "Expected tauri-cli $native_cli from the native-android shell" >&2; exit 1;
  }
  : "${ANDROID_HOME:?Set ANDROID_HOME to the SDK Manager-owned Android SDK}"
  native_android() { python3 -c 'import json,sys; print(json.load(open("apps/native-shell/toolchain.json"))["android"][sys.argv[1]])' "$1"; }
  native_ndk="$(native_android ndk)"
  native_sdk_platform="$(native_android platform)"
  native_build_tools="$(native_android buildTools)"
  export NDK_HOME="$ANDROID_HOME/ndk/$native_ndk"
  grep -Fxq "Pkg.Revision = $native_ndk" "$NDK_HOME/source.properties" || {
    echo "Install NDK $native_ndk with the SDK Manager" >&2; exit 1;
  }
  test -f "$ANDROID_HOME/platforms/$native_sdk_platform/android.jar" || {
    echo "Install platforms;$native_sdk_platform with the SDK Manager" >&2; exit 1;
  }
  test -x "$ANDROID_HOME/build-tools/$native_build_tools/aapt2" || {
    echo "Install build-tools;$native_build_tools with the SDK Manager" >&2; exit 1;
  }
  native_abi=arm64-v8a
  if [ "$native_platform" = android-emu ]; then native_abi=x86_64; fi
  read -r native_tauri_target native_rust_target < <(python3 -c 'import json,sys
for abi in json.load(open("apps/native-shell/toolchain.json"))["android"]["abis"]:
    if abi["name"] == sys.argv[1]: print(abi["tauriTarget"], abi["rustTarget"])' "$native_abi")
  test -d "$(rustc --print sysroot)/lib/rustlib/$native_rust_target" || {
    echo "The native-android shell lacks Rust target $native_rust_target" >&2; exit 1;
  }
  # Signing is a separate release step. Never let an inherited keystore or
  # Tauri updater key sign this build-only artifact.
  unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD TAURI_CONFIG
  unset TAURI_ANDROID_KEYSTORE_PATH TAURI_ANDROID_KEYSTORE_PASSWORD ANDROID_KEYSTORE
  unset CARGO_TARGET_DIR RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
  export LANG=C.UTF-8
  mkdir -p dist/native-shell
  native_build="$(mktemp -d "$native_repo/dist/native-shell/$native_platform-${native_revision:0:12}.XXXXXX")"
  echo "Native build directory: $native_build"
  native_source="$native_build/apps/native-shell"
  mkdir -p "$native_source"
  git archive "$native_revision:apps/native-shell" | tar -xf - -C "$native_source"
  export CARGO_TARGET_DIR="$native_build/target"
  cd "$native_source/tauri"
  # The Gradle project itself is generated, but every handwritten Kotlin source
  # and resource comes from this commit.
  cargo tauri android init --ci --skip-targets-install
  cp -R ../android/app/src/. gen/android/app/src/
  cargo tauri android build --apk --target "$native_tauri_target" "${native_flags[@]}"
  native_apk="$native_source/tauri/gen/android/app/build/outputs/apk/universal/$native_profile/app-universal-$native_profile.apk"
  if [ "$native_profile" = release ]; then
    native_apk="${native_apk%.apk}-unsigned.apk"
  fi
  test -f "$native_apk" || { echo "Build returned no APK at its exact output path: $native_apk" >&2; exit 1; }
  cmp "$native_source/tauri/Cargo.lock" "$native_repo/apps/native-shell/tauri/Cargo.lock"
  python3 - "$native_apk" "$native_revision" "$native_platform" "$native_profile" "$native_build" "$native_report_target" "$native_abi" <<'PY'
import hashlib, json, os, pathlib, re, subprocess, sys
apk, revision, platform, profile, build, receipt_target, abi = sys.argv[1:]
apk, build = pathlib.Path(apk), pathlib.Path(build)
native = build / "apps/native-shell"
toolchain = json.loads((native / "toolchain.json").read_text())
android = toolchain["android"]
sdk = pathlib.Path(os.environ["ANDROID_HOME"])
aapt2 = sdk / "build-tools" / android["buildTools"] / "aapt2"
badging = subprocess.check_output([str(aapt2), "dump", "badging", str(apk)], text=True)
def field(pattern):
    match = re.search(pattern, badging, re.M)
    if not match:
        raise SystemExit("APK badging lacks " + pattern)
    return match.group(1)
package = field(r"^package: name='([^']+)'")
if package != "top.thundersparrow.cowboy":
    raise SystemExit("Unexpected Android application id: " + package)
min_sdk = int(field(r"^(?:minSdkVersion|sdkVersion):'(\d+)'"))
if min_sdk != android["minSdk"]:
    raise SystemExit(f"APK minSdk {min_sdk} differs from pinned {android['minSdk']}")
abis = field(r"^native-code: (.+)$").replace("'", "").split()
if abis != [abi]:
    raise SystemExit(f"APK native code {abis} differs from requested {abi}")
gen = native / "tauri/gen/android"
gradle = re.search(r"gradle-([0-9.]+)-", (gen / "gradle/wrapper/gradle-wrapper.properties").read_text()).group(1)
agp = re.search(r"com\.android\.tools\.build:gradle:([0-9.]+)", (gen / "buildSrc/build.gradle.kts").read_text() + (gen / "build.gradle.kts").read_text()).group(1)
apksigner = sdk / "build-tools" / android["buildTools"] / "apksigner"
signing = "unsigned"
if subprocess.run([str(apksigner), "verify", str(apk)], capture_output=True).returncode == 0:
    signing = "android-debug-keystore" if profile == "debug" else "signed"
report = dict(source_revision=revision, platform=platform, profile=profile, abi=abi,
    apk=str(apk), apk_sha256=hashlib.sha256(apk.read_bytes()).hexdigest(),
    application_id=package, version_code=int(field(r"versionCode='(\d+)'")),
    version_name=field(r"versionName='([^']*)'"), min_sdk=min_sdk,
    target_sdk=int(field(r"^targetSdkVersion:'(\d+)'")),
    lock_sha256=hashlib.sha256((native / "tauri/Cargo.lock").read_bytes()).hexdigest(),
    toolchain=toolchain, gradle=gradle, android_gradle_plugin=agp,
    ndk=android["ndk"], java=subprocess.run(["java", "-version"], capture_output=True, text=True).stderr.splitlines()[0],
    rustc=subprocess.check_output(["rustc", "--version", "--verbose"], text=True).strip(),
    tauri_cli=subprocess.check_output(["cargo", "tauri", "--version"], text=True).strip(),
    signing=signing, installed=False, real_login="not_checked", physical_device="not_checked")
(build / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
if receipt_target:
    with pathlib.Path(receipt_target).open("x") as receipt:
        receipt.write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
  exit 0
  ;;
esac
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
native_source="$native_build/apps/native-shell"
mkdir -p "$native_source"
# Archive the subtree itself: partial clones must not hydrate unrelated Web
# artwork merely to stage the native product's committed source.
git archive "$native_revision:apps/native-shell" | tar -xf - -C "$native_source"
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
if [ "$native_platform" = ios ]; then
  # The exact unsigned archive becomes a SideStore input. Signing and version
  # publication stay with the machine-owned publisher, never this build step.
  mkdir "$native_build/Payload"
  ditto "$native_app" "$native_build/Payload/Cowboy.app"
  ditto -c -k --keepParent "$native_build/Payload" "$native_build/Cowboy.ipa"
fi
# Record the actual executable and source bytes, not a glob-selected DerivedData
# product or an old receipt. Failure above leaves no success receipt.
python3 - "$native_app" "$native_revision" "$native_platform" "$native_profile" "$native_build" "$native_report_target" <<'PY'
import hashlib, json, pathlib, plistlib, subprocess, sys
app, revision, platform, profile, build, receipt_target = sys.argv[1:]
app, build = pathlib.Path(app), pathlib.Path(build)
info_path = app / ("Contents/Info.plist" if platform == "macos" else "Info.plist")
info = plistlib.loads(info_path.read_bytes())
binary = app / ("Contents/MacOS" if platform == "macos" else "") / info["CFBundleExecutable"]
if info["CFBundleIdentifier"] != "top.thundersparrow.cowboy":
    raise SystemExit("Unexpected native bundle identifier")
toolchain = json.loads((build / "apps/native-shell/toolchain.json").read_text())
swift_packages = {}
for state_path in (build / "target").rglob("workspace-state.json"):
    if "swift-rs" not in state_path.parts:
        continue
    for dependency in json.loads(state_path.read_text())["object"]["dependencies"]:
        package = dependency["packageRef"]
        if package["kind"] != "remoteSourceControl":
            continue
        state = dependency["state"]["checkoutState"]
        consumer = state_path.parent.name
        pin = dict(url=package["location"], version=state.get("version"), revision=state.get("revision"))
        if toolchain["swiftPackages"].get(consumer) != pin:
            raise SystemExit("Unpinned Swift package for " + consumer + ": " + package["location"])
        swift_packages[consumer] = pin
if platform != "macos" and swift_packages != toolchain["swiftPackages"]:
    raise SystemExit("Incomplete Swift dependency receipt")
alternate_icons = []
if platform != "macos":
    expected = {path.stem for path in (build / "apps/native-shell/apple/Assets.xcassets").glob("Cowboy-*.appiconset")}
    for key in ("CFBundleIcons", "CFBundleIcons~ipad"):
        actual = set(info.get(key, {}).get("CFBundleAlternateIcons", {}))
        if actual != expected:
            raise SystemExit(f"Incomplete compiled alternate icons in {key}: missing {sorted(expected - actual)}, unexpected {sorted(actual - expected)}")
    alternate_icons = sorted(expected)
report = dict(source_revision=revision, platform=platform, profile=profile,
    app=str(app), executable_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
    lock_sha256=hashlib.sha256((build / "apps/native-shell/tauri/Cargo.lock").read_bytes()).hexdigest(),
    xcode=subprocess.check_output(["xcodebuild", "-version"], text=True).strip(),
    toolchain=toolchain, swift_packages=swift_packages, alternate_icons=alternate_icons,
    rustc=subprocess.check_output(["rustc", "--version", "--verbose"], text=True).strip(),
    signing="ad-hoc" if platform == "macos" else "unsigned",
    installed=False, real_login="not_checked", physical_device="not_checked")
if platform == "ios":
    ipa = build / "Cowboy.ipa"
    report.update(ipa=str(ipa), ipa_sha256=hashlib.sha256(ipa.read_bytes()).hexdigest())
(build / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
if receipt_target:
    # Exclusive creation rejects a racing file/symlink too. The caller owns
    # the parent directory; failure above never leaves a success receipt here.
    with pathlib.Path(receipt_target).open("x") as receipt:
        receipt.write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
