#!/usr/bin/env bash
# Cowboy's project-owned control surface for its DEBUG iOS Simulator shell.
# This script runs on the Mac; cowboysim-remote.sh supplies the owned SSH path.
set -euo pipefail

BID="top.thundersparrow.cowboy"
DEVPORT="${COWBOY_SIM_DEVPORT:-4171}"
if [ "${1:-help}" = help ]; then
  echo "COWBOY_SIM_UDID=<explicit simulator> bash tools/cowboysim.sh {boot|launch|status|ping|eval|aeval|url|reload|appearance|shot|log}"
  exit 0
fi
SIM="${COWBOY_SIM_UDID:?select a Simulator explicitly; no implicit booted device}"
[[ "$SIM" =~ ^[0-9A-Fa-f-]{36}$ ]] || { echo "invalid simulator UDID" >&2; exit 2; }
[[ "$DEVPORT" =~ ^[0-9]+$ ]] && ((DEVPORT > 1023 && DEVPORT <= 65535)) || { echo "invalid bridge port" >&2; exit 2; }

bridge_curl() {
  curl -H "X-Cowboy-Simulator: $SIM" "$@"
}

boot_simulator() {
  xcrun simctl boot "$SIM" 2>/dev/null || true
  xcrun simctl bootstatus "$SIM" -b >/dev/null
}

bridge_ping() {
  bridge_curl -fsS -m 3 "http://127.0.0.1:$DEVPORT/ping"
}

dev_eval() {
  local source="$1" tries=0 output=""
  while [ "$tries" -lt 8 ]; do
    output="$(bridge_curl -fsS -m 6 --data-raw "$source" "http://127.0.0.1:$DEVPORT/eval" 2>/dev/null)" && {
      if [ -n "$output" ]; then
        printf '%s\n' "$output"
        return 0
      fi
    }
    tries=$((tries + 1))
    sleep 1
  done
  echo "FATAL: CowboyDevBridge is not answering on 127.0.0.1:$DEVPORT; launch a DEBUG simulator build" >&2
  return 1
}

command="${1:-help}"
shift || true
case "$command" in
  boot)
    boot_simulator
    echo "booted $SIM"
    ;;
  launch)
    # `launch` is intentionally cold-start safe: the generic bridge contract
    # should not require callers to remember a separate `boot` first.
    boot_simulator
    xcrun simctl terminate "$SIM" "$BID" 2>/dev/null || true
    SIMCTL_CHILD_COWBOY_SIM_BRIDGE=1 SIMCTL_CHILD_COWBOY_SIM_DEVPORT="$DEVPORT" \
      xcrun simctl launch "$SIM" "$BID"
    dev_eval 'document.title' >/dev/null
    ;;
  appearance)
    boot_simulator
    xcrun simctl ui "$SIM" appearance "${1:-dark}"
    echo "appearance=${1:-dark}"
    ;;
  shot)
    boot_simulator
    output="${1:?explicit screenshot output path required}"
    xcrun simctl io "$SIM" screenshot "$output"
    echo "$output"
    ;;
  ping)
    bridge_ping
    ;;
  eval)
    dev_eval "${1:?JavaScript expression required}"
    ;;
  eval64)
    # Compatibility for callers that already encode their script; the owned
    # SSH wrapper preserves ordinary eval arguments without requiring base64.
    dev_eval "$(printf '%s' "${1:?base64 JavaScript required}" | base64 -D)"
    ;;
  aeval)
    bridge_curl -fsS -m 20 --data-raw "${1:?JavaScript body required}" \
      "http://127.0.0.1:$DEVPORT/aeval"
    ;;
  aeval64)
    bridge_curl -fsS -m 20 \
      --data-raw "$(printf '%s' "${1:?base64 JavaScript required}" | base64 -D)" \
      "http://127.0.0.1:$DEVPORT/aeval"
    ;;
  url)
    dev_eval 'location.href'
    ;;
  reload)
    dev_eval 'location.reload()'
    ;;
  log)
    xcrun simctl spawn "$SIM" log stream --level debug \
      --predicate "processImagePath CONTAINS[c] 'Cowboy'"
    ;;
  status)
    echo "sim=$SIM"
    xcrun simctl list devices booted | grep -i booted || true
    if bridge_ping >/dev/null 2>&1; then
      echo "CowboyDevBridge: ok"
      printf 'app origin: '
      dev_eval 'location.origin'
      printf 'document title: '
      dev_eval 'document.title'
      printf 'user agent: '
      dev_eval 'navigator.userAgent'
    else
      echo "CowboyDevBridge: down"
    fi
    ;;
  *)
    echo "unknown command: $command" >&2
    exit 2
    ;;
esac
