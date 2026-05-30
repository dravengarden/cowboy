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
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/cowboy"

{
  echo "--- $(date -Iseconds) launch pid=$$ ppid=$PPID cwd=$(pwd) ---"
  echo "args: $*"
  echo "PATH=$PATH"
} >> "$LOG_FILE"

exec "$BIN" "$@" 2>>"$LOG_FILE"
