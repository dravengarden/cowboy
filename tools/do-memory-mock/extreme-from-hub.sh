#!/usr/bin/env bash
set -euo pipefail

mock="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$mock/../.." && pwd)"
port="${COWBOY_DO_MOCK_PORT:-8792}"
fixture="${COWBOY_DO_FIXTURE_OUT:-$(mktemp)}"
persist="${COWBOY_DO_PERSIST:-/tmp/cowboy-do-memory-state-extreme}"

export COWBOY_DO_SESSIONS="${COWBOY_DO_SESSIONS:-50}"
export COWBOY_DO_FOCUSED="${COWBOY_DO_FOCUSED:-4}"
export COWBOY_DO_TERMINALS="${COWBOY_DO_TERMINALS:-8}"
export COWBOY_DO_IDLE_CHUNK_BYTES="${COWBOY_DO_IDLE_CHUNK_BYTES:-8192}"
export COWBOY_DO_IDLE_CHUNKS="${COWBOY_DO_IDLE_CHUNKS:-24}"
export COWBOY_DO_BUSY_CHUNKS="${COWBOY_DO_BUSY_CHUNKS:-40}"
export COWBOY_DO_TOOLS="${COWBOY_DO_TOOLS:-1000}"
export COWBOY_DO_FAT_EVENTS="${COWBOY_DO_FAT_EVENTS:-3}"
export COWBOY_DO_ARCHIVE_ROWS="${COWBOY_DO_ARCHIVE_ROWS:-40000}"
export COWBOY_DO_ARCHIVE_BYTES="${COWBOY_DO_ARCHIVE_BYTES:-1024}"

cd "$root"
echo "generating extreme Hub fixture..."
COWBOY_DO_FIXTURE_OUT="$fixture" nix develop -c cargo test --locked --lib \
  core::core_tests::exports_extreme_do_fixture -- --exact --test-threads=1
echo "fixture bytes: $(wc -c < "$fixture")"

rm -rf "$persist"
bash "$mock/run.sh" dev --local --ip 127.0.0.1 --port "$port" --persist-to "$persist" &
wrangler_pid=$!
cleanup() {
  kill "$wrangler_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 80); do
  if curl -sf -o /dev/null -X POST "http://127.0.0.1:$port/reset"; then
    break
  fi
  sleep 0.25
done

echo "seeding Durable Object (archive rows=${COWBOY_DO_ARCHIVE_ROWS})..."
curl -sf -X POST "http://127.0.0.1:$port/seed?mode=storage" \
  -H "content-type: application/json" \
  --data-binary @"$fixture" >/tmp/cowboy-do-extreme-storage.json

sample_rss() {
  local pid
  pid="$(pgrep -P "$wrangler_pid" -u "$(id -u)" workerd | head -n 1 || true)"
  if [[ -z "${pid:-}" ]]; then
    pid="$(pgrep -u "$(id -u)" workerd | tail -n 1 || true)"
  fi
  if [[ -n "${pid:-}" && -r "/proc/$pid/status" ]]; then
    awk -v pid="$pid" '
      $1=="VmRSS:" { rss=$2*1024 }
      $1=="VmHWM:" { hwm=$2*1024 }
      END { printf "{\"workerdPid\":%s,\"workerdRssBytes\":%s,\"workerdHwmBytes\":%s}\n", pid, rss+0, hwm+0 }
    ' "/proc/$pid/status"
  else
    echo '{"workerdPid":null,"workerdRssBytes":null,"workerdHwmBytes":null}'
  fi
}

for mode in storage focused all-hot all-rows; do
  curl -sf "http://127.0.0.1:$port/report?mode=$mode" \
    >"/tmp/cowboy-do-extreme-$mode.json" || \
    curl -sf "http://127.0.0.1:$port/?mode=$mode" \
    >"/tmp/cowboy-do-extreme-$mode.json"
done

python3 - <<'PY'
import json, pathlib, sys

def load(name):
    path = pathlib.Path(f"/tmp/cowboy-do-extreme-{name}.json")
    return json.loads(path.read_text())

storage = load("storage")
focused = load("focused")
all_hot = load("all-hot")
all_rows = load("all-rows")
print(json.dumps({
    "storage": storage,
    "focused": focused,
    "allHot": all_hot,
    "allRows": all_rows,
}, indent=2)[:2000])
print("---")
rows = [
    ("storage  (SQL only, no row parse)", storage),
    ("focused (busy tails + shared live)", focused),
    ("all-hot (every session hot tail)", all_hot),
    ("all-rows (hot + archive parsed)", all_rows),
]
print(f"{'mode':<42} {'heap':>12} {'sqlite':>12} {'rows':>8} {'fits100':>8}")
for label, report in rows:
    heap = report.get("isolateHeapBytes", -1)
    sqlite = report.get("sqliteTotalBytes", -1)
    n = report.get("materializedRows", report.get("durableEvents", -1))
    fits = report.get("fitsTarget")
    print(f"{label:<42} {heap:12} {sqlite:12} {n:8} {str(fits):>8}")
print(
    "raw fan-out if uncompacted:",
    focused.get("rawFanoutWouldHaveBeen"),
    "live", focused.get("compactLiveBytes"),
    "shared", focused.get("sharedFrame"),
    "image", focused.get("liveImageExternalized"),
)
checks = [
    focused.get("sessions") == 50,
    focused.get("sharedFrame") is True,
    focused.get("droppedRawOutput") is True,
    focused.get("liveImageExternalized") is True,
    focused.get("fitsTarget") is True,
    all_hot.get("fitsTarget") is True,
    focused.get("isolateHeapBytes", 10**18) < all_hot.get("isolateHeapBytes", 0) + 1
    or focused.get("focusedSessions", 0) == focused.get("sessions", -1),
    storage.get("isolateHeapBytes", 10**18) < focused.get("isolateHeapBytes", 0) + 4096
    or focused.get("materializedBytes", 1) == 0,
]
if not all(checks):
    print("extreme DO assertions failed", checks, file=sys.stderr)
    sys.exit(1)
PY

echo "workerd RSS:"
sample_rss
python3 -m json.tool /tmp/cowboy-do-extreme-focused.json
