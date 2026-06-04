#!/usr/bin/env bash
# Launch wrapper for Zed's `agent_servers["cowboy (hawk, …)"]` entry.
#
# WHY this exists at all: when Zed spawns `cowboy serve-acp` via its `sh -c`
# wrapper, stderr lands in a pipe that ONLY zed-remote-server reads. From a
# hawk shell it's `Permission denied` to /proc/<pid>/fd/2, so first-spawn
# failures (e.g. upstream `npx` not on PATH for the zed-remote-server's env,
# or an unexpected cwd) are invisible. This wrapper tees stderr to
# /tmp/cowboy-zed.log so we can read it from hawk WITHOUT touching Zed's
# spawn machinery.
#
# Once the integration is stable this can be removed and the Mac
# settings.json entry can point at the release binary directly.
set -euo pipefail

LOG_DIR="${COWBOY_ZED_LOG_DIR:-/tmp}"
LOG_FILE="${LOG_DIR}/cowboy-zed.log"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Resolve the binary among the artifacts that actually exist in this worktree,
# newest-intent first. `just build` produces target/release; `nix build` (the
# same source the deployed systemd service runs) leaves ./result; target/debug
# is the cargo-run dev build. Picking the first that exists keeps the bridge
# launchable without forcing a release build, while never out-ranking one.
BIN=""
for cand in \
  "$ROOT/target/release/cowboy" \
  "$ROOT/result/bin/cowboy" \
  "$ROOT/target/debug/cowboy"; do
  if [ -x "$cand" ]; then
    BIN="$cand"
    break
  fi
done

{
  echo "--- $(date -Iseconds) launch pid=$$ ppid=$PPID cwd=$(pwd) ---"
  echo "args: $*"
  echo "bin: ${BIN:-<none found>}"
  echo "PATH=$PATH"
} >> "$LOG_FILE"

if [ -z "$BIN" ]; then
  echo "cowboy binary not found under $ROOT (tried target/release, result/bin, target/debug); run 'just build'" >> "$LOG_FILE"
  exit 127
fi

exec "$BIN" "$@" 2>>"$LOG_FILE"
