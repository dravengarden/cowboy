#!/usr/bin/env bash
set -euo pipefail

mock="$(cd "$(dirname "$0")" && pwd)"
config="$mock/wrangler.toml"

if [[ -x "$mock/node_modules/.bin/wrangler" ]]; then
  wrangler="$mock/node_modules/.bin/wrangler"
elif command -v wrangler >/dev/null 2>&1; then
  wrangler="$(command -v wrangler)"
else
  echo "wrangler not found; install tools/do-memory-mock or put wrangler on PATH" >&2
  exit 1
fi

echo "using wrangler: $wrangler ($("$wrangler" --version))"
exec "$wrangler" "$@" --config "$config"
