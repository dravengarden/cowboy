#!/usr/bin/env bash
# Hawk-side convenience wrapper around the generic iOS Simulator Bridge plugin.
set -euo pipefail

# Reach the Mac through Stormbird's stable overlay address. Callers can still
# override this when validating another Mac or an isolated bridge.
export IOS_SIM_MAC_HOST="${IOS_SIM_MAC_HOST:-dravenchen@100.64.0.2}"

resolver="${IOS_SIM_REMOTE_RESOLVER:-}"
if [ -z "$resolver" ]; then
  for candidate in \
    "$HOME"/.codex/plugins/cache/liveview-development/ios-simulator-bridge/*/scripts/ios-sim-remote
  do
    if [ -x "$candidate" ]; then
      resolver="$candidate"
    fi
  done
fi

if [ -z "$resolver" ] || [ ! -x "$resolver" ]; then
  echo "FATAL: installed ios-simulator-bridge resolver not found" >&2
  exit 1
fi

if { [ "${1:-}" = "eval" ] || [ "${1:-}" = "aeval" ]; } && [ "$#" -eq 2 ]; then
  # Encode locally so complex JavaScript remains one shell-safe SSH argument.
  encoded="$(printf '%s' "$2" | base64 --wrap=0)"
  exec "$resolver" '$HOME/cowboy-shell/tools/cowboysim.sh' "${1}64" "$encoded"
fi

exec "$resolver" '$HOME/cowboy-shell/tools/cowboysim.sh' "$@"
