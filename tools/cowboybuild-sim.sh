#!/bin/bash
# Build, install, and launch Cowboy's DEBUG iOS Simulator shell on the Mac.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

SIM="${1:-D89613B8-4B25-4486-A690-5A7205AC2788}"
BID="top.thundersparrow.cowboy"
ROOT="$HOME/cowboy-shell"

cd "$ROOT/src-tauri"
mkdir -p gen/apple/assets

# Re-glob hand-authored Sources such as CowboyDevBridge.swift before building.
(cd gen/apple && xcodegen generate)

cargo tauri ios build --debug --target aarch64-sim --ci \
  || echo "note: simulator IPA export returned non-zero; locating the .app directly"

# Choose the newest DerivedData product; paths are quoted below.
# shellcheck disable=SC2012
APP="$(ls -dt "$HOME"/Library/Developer/Xcode/DerivedData/cowboy-app-*/Build/Products/*-iphonesimulator/Cowboy.app 2>/dev/null | head -1)"
if [ -z "$APP" ]; then
  echo "FATAL: no Cowboy simulator .app found in DerivedData" >&2
  exit 1
fi
echo "APP=$APP"

xcrun simctl boot "$SIM" 2>/dev/null || true
xcrun simctl install "$SIM" "$APP"
xcrun simctl terminate "$SIM" "$BID" 2>/dev/null || true
xcrun simctl launch "$SIM" "$BID"
sleep 5
"$ROOT/tools/cowboysim.sh" status
echo "COWBOYSIM_OK"
