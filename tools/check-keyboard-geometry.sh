#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
test_binary=$(mktemp "${TMPDIR:-/tmp}/cowboy-keyboard-geometry.XXXXXX")
trap 'rm -f -- "$test_binary"' EXIT

native_tweaks="$repo_root/src-tauri/gen/apple/Sources/cowboy-app/CowboyNativeTweaks.mm"
apply_overlap=$(sed -n '/^- (void)applyOverlap:/,/^}/p' "$native_tweaks")
for required_option in \
  UIViewAnimationOptionAllowUserInteraction \
  UIViewAnimationOptionBeginFromCurrentState \
  'options:options'; do
  if ! grep -Fq -- "$required_option" <<<"$apply_overlap"; then
    printf 'applyOverlap is missing required animation contract: %s\n' "$required_option" >&2
    exit 1
  fi
done

compiler=${CC:-cc}
"$compiler" \
  -std=c11 \
  -Wall \
  -Wextra \
  -Werror \
  -pedantic \
  -I "$repo_root/src-tauri/gen/apple/Sources/cowboy-app" \
  "$repo_root/tools/cowboy-keyboard-geometry-test.c" \
  -lm \
  -o "$test_binary"
"$test_binary"
