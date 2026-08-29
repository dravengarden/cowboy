#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_root="$(cd "$script_dir/.." && pwd)"
app_bundle="${1:-$app_root/dist/Cowboy Manager.app}"

swift test --package-path "$app_root"
test -x "$app_bundle/Contents/MacOS/CowboyInstaller"
test -x "$app_bundle/Contents/Resources/bin/cowboy"
test -x "$app_bundle/Contents/Resources/bin/cowboy-machine"
test -x "$app_bundle/Contents/Resources/bin/cowboy-machine-install"
test -x "$app_bundle/Contents/Resources/bin/cowboy-code-adapter"
test "$(plutil -extract CFBundleIdentifier raw "$app_bundle/Contents/Info.plist")" = \
    "xyz.stormbird.cowboy.manager"
test "$(plutil -extract CFBundleDisplayName raw "$app_bundle/Contents/Info.plist")" = \
    "Cowboy Manager"
test "$(plutil -extract LSUIElement raw "$app_bundle/Contents/Info.plist")" = "true"
test "$(plutil -extract CFBundleURLTypes.0.CFBundleURLSchemes.0 raw "$app_bundle/Contents/Info.plist")" = \
    "xyz.stormbird.cowboy.manager"
codesign --verify --deep --strict --verbose=2 "$app_bundle"
"$app_bundle/Contents/Resources/bin/cowboy" --version
