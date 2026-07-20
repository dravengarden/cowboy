#!/bin/bash
# Headless control surface for Cowboy's DEBUG iOS Simulator shell.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

BID="top.thundersparrow.cowboy"
DEVPORT=4171

sim_udid() {
  local booted
  booted="$(xcrun simctl list devices booted 2>/dev/null | grep -oE '[0-9A-F-]{36}' | head -1 || true)"
  echo "${booted:-D89613B8-4B25-4486-A690-5A7205AC2788}"
}
SIM="$(sim_udid)"

dev_eval() {
  local source="$1" tries=0 output=""
  while [ "$tries" -lt 8 ]; do
    output="$(curl -s -m 6 --data-binary "$source" "http://127.0.0.1:$DEVPORT/eval" 2>/dev/null)" && {
      [ -n "$output" ] && {
        printf '%s\n' "$output"
        return 0
      }
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
    xcrun simctl boot "$SIM" 2>/dev/null || true
    echo "booted $SIM"
    ;;
  launch)
    xcrun simctl terminate "$SIM" "$BID" 2>/dev/null || true
    xcrun simctl launch "$SIM" "$BID"
    sleep 2
    ;;
  appearance)
    xcrun simctl ui "$SIM" appearance "${1:-dark}"
    echo "appearance=${1:-dark}"
    ;;
  shot)
    output="${1:-$HOME/cowboysim.png}"
    xcrun simctl io "$SIM" screenshot "$output"
    echo "$output"
    ;;
  ping)
    curl -s -m 4 "http://127.0.0.1:$DEVPORT/ping" || echo "(down)"
    ;;
  eval)
    dev_eval "$1"
    ;;
  aeval)
    curl -s -m 20 --data-binary "$1" "http://127.0.0.1:$DEVPORT/aeval"
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
    printf "CowboyDevBridge: "
    curl -s -m 3 "http://127.0.0.1:$DEVPORT/ping" 2>/dev/null || echo "down"
    echo
    printf "app origin: "
    dev_eval 'location.origin' 2>/dev/null || true
    ;;
  help | *)
    sed -n '2,18p' "$0"
    ;;
esac
