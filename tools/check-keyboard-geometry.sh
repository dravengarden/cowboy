#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
test_binary=$(mktemp "${TMPDIR:-/tmp}/cowboy-keyboard-geometry.XXXXXX")
trap 'rm -f -- "$test_binary"' EXIT

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
