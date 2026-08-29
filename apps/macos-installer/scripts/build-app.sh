#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_root="$(cd "$script_dir/.." && pwd)"
repo_root="$(cd "$app_root/../.." && pwd)"
configuration="${CONFIGURATION:-release}"
version="${COWBOY_INSTALLER_VERSION:-0.1.2}"
build_number="${COWBOY_INSTALLER_BUILD:-3}"
output_root="${COWBOY_INSTALLER_OUTPUT_DIR:-$app_root/dist}"
app_bundle="$output_root/Cowboy Manager.app"
backend_dir="${COWBOY_BOOTSTRAP_DIR:-$repo_root/target/release}"

if [[ "${1:-}" == "--build-backend" ]]; then
    (
        cd "$repo_root"
        cargo build --release --locked --no-default-features --features machine-host \
            --bin cowboy --bin cowboy-machine --bin cowboy-machine-install
        cargo build --release --locked --no-default-features --features code-adapter \
            --bin cowboy-code-adapter
    )
fi

for executable in cowboy cowboy-machine cowboy-machine-install cowboy-code-adapter; do
    if [[ ! -x "$backend_dir/$executable" ]]; then
        echo "missing executable backend: $backend_dir/$executable" >&2
        echo "run with --build-backend or set COWBOY_BOOTSTRAP_DIR" >&2
        exit 1
    fi
done

swift build --package-path "$app_root" --configuration "$configuration" --product CowboyInstaller
binary_path="$(swift build --package-path "$app_root" --configuration "$configuration" --show-bin-path)/CowboyInstaller"

rm -rf -- "$app_bundle"
mkdir -p "$app_bundle/Contents/MacOS" "$app_bundle/Contents/Resources/bin"
cp "$binary_path" "$app_bundle/Contents/MacOS/CowboyInstaller"
cp "$app_root/Resources/Info.plist" "$app_bundle/Contents/Info.plist"
for executable in cowboy cowboy-machine cowboy-machine-install cowboy-code-adapter; do
    cp "$backend_dir/$executable" "$app_bundle/Contents/Resources/bin/$executable"
done

plutil -replace CFBundleShortVersionString -string "$version" "$app_bundle/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$build_number" "$app_bundle/Contents/Info.plist"

signing_identity="${CODE_SIGN_IDENTITY:--}"
for executable in cowboy cowboy-machine cowboy-machine-install cowboy-code-adapter; do
    codesign --force --options runtime --sign "$signing_identity" \
        "$app_bundle/Contents/Resources/bin/$executable"
done
codesign --force --options runtime --entitlements "$app_root/Resources/CowboyInstaller.entitlements" \
    --sign "$signing_identity" "$app_bundle"
codesign --verify --deep --strict --verbose=2 "$app_bundle"

echo "$app_bundle"
