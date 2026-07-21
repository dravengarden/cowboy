#!/usr/bin/env bash
# Cowboy's project-owned control surface for its DEBUG iOS Simulator shell.
# This script runs on the Mac and is normally invoked through ios-sim-remote.
set -euo pipefail

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

BID="top.thundersparrow.cowboy"
DEVPORT="${COWBOY_SIM_DEVPORT:-4171}"
FALLBACK_SIM="${COWBOY_SIM_UDID:-D89613B8-4B25-4486-A690-5A7205AC2788}"

sim_udid() {
  local booted
  booted="$(xcrun simctl list devices booted 2>/dev/null | grep -oE '[0-9A-F-]{36}' | head -1 || true)"
  printf '%s\n' "${booted:-$FALLBACK_SIM}"
}

SIM="$(sim_udid)"

boot_simulator() {
  xcrun simctl boot "$SIM" 2>/dev/null || true
  xcrun simctl bootstatus "$SIM" -b >/dev/null
}

bridge_ping() {
  curl -fsS -m 3 "http://127.0.0.1:$DEVPORT/ping"
}

dev_eval() {
  local source="$1" tries=0 output=""
  while [ "$tries" -lt 8 ]; do
    output="$(curl -fsS -m 6 --data-binary "$source" "http://127.0.0.1:$DEVPORT/eval" 2>/dev/null)" && {
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
    output="${1:-$HOME/cowboysim.png}"
    xcrun simctl io "$SIM" screenshot "$output"
    echo "$output"
    ;;
  ping)
    bridge_ping || echo "(down)"
    ;;
  eval)
    dev_eval "${1:?JavaScript expression required}"
    ;;
  aeval)
    curl -fsS -m 20 --data-binary "${1:?JavaScript body required}" \
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
  help | *)
    sed -n '2,23p' "$0"
    ;;
esac
