#!/usr/bin/env bash
# Run on an arm64 Mac with Xcode. Use an exclusively-created Simulator, never
# the developer's existing app, simulator, keychain, or authenticated browser.
set -euo pipefail
test "$(uname -s)" = Darwin
test "$(uname -m)" = arm64
test -z "$(git status --porcelain)"
native_revision="$(git rev-parse HEAD)"
native_root="$(mktemp -d /tmp/cowboy-native-conformance.XXXXXX)"
native_simulator=""
cleanup() {
  if [ -n "$native_simulator" ]; then
    xcrun simctl shutdown "$native_simulator" >/dev/null 2>&1 || true
    xcrun simctl delete "$native_simulator" >/dev/null 2>&1 || true
  fi
  rm -r -- "$native_root"
}
trap cleanup EXIT
native_app="$native_root/CowboyPluginConformance.app"
mkdir "$native_app"
cp apps/native-shell/apple/Conformance/Info.plist "$native_app/Info.plist"
xcrun --sdk iphonesimulator clang++ -fobjc-arc -fblocks \
  -target arm64-apple-ios15.0-simulator \
  -framework UIKit -framework WebKit -framework AuthenticationServices \
  -framework SafariServices -framework UniformTypeIdentifiers -framework CoreGraphics \
  apps/native-shell/apple/Conformance/main.mm \
  apps/native-shell/apple/Sources/cowboy-app/CowboyPasskeyBridge.mm \
  apps/native-shell/apple/Sources/cowboy-app/CowboyNativeTweaks.mm \
  apps/native-shell/apple/Sources/cowboy-app/CowboyAppIconBridge.mm \
  -o "$native_app/CowboyPluginConformance"
codesign --sign - "$native_app"
native_runtime="$(xcrun simctl list runtimes --json | python3 -c 'import json,sys; values=[r for r in json.load(sys.stdin)["runtimes"] if r.get("isAvailable") and ".iOS-" in r["identifier"]]; values.sort(key=lambda r: tuple(map(int,r["version"].split(".")))); print(values[-1]["identifier"])')"
native_simulator="$(xcrun simctl create "Cowboy Plugin Conformance ${native_revision:0:8}" \
  com.apple.CoreSimulator.SimDeviceType.iPhone-16 "$native_runtime")"
xcrun simctl boot "$native_simulator"
xcrun simctl bootstatus "$native_simulator" -b
xcrun simctl install "$native_simulator" "$native_app"
xcrun simctl launch "$native_simulator" dev.cowboy.plugin-conformance
native_data="$(xcrun simctl get_app_container "$native_simulator" dev.cowboy.plugin-conformance data)"
for _ in $(seq 1 90); do
  if [ -s "$native_data/Documents/conformance.json" ]; then
    mkdir -p dist/native-plugin-conformance
    python3 -c 'import json,sys; from pathlib import Path; report=json.loads(Path(sys.argv[1]).read_text()); report.update(source_revision=sys.argv[2], simulator_runtime=sys.argv[3], real_login="not_checked", product_shell_bundle="not_checked"); print(json.dumps(report,indent=2)); Path("dist/native-plugin-conformance/receipt.json").write_text(json.dumps(report,indent=2)+"\n"); sys.exit(0 if report.get("ok") and len(report.get("tests",[]))==13 else 1)' \
      "$native_data/Documents/conformance.json" "$native_revision" "$native_runtime"
    exit 0
  fi
  sleep 1
done
echo "Native Plugin ABI conformance timed out" >&2
exit 1
