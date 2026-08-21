#!/usr/bin/env bash
set -euo pipefail

mock="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$mock/../.." && pwd)"
port="${COWBOY_DO_MOCK_PORT:-8789}"
fixture="${COWBOY_DO_FIXTURE_OUT:-$(mktemp)}"
persist="${COWBOY_DO_PERSIST:-/tmp/cowboy-do-memory-state-seed}"

cd "$root"
COWBOY_DO_FIXTURE_OUT="$fixture" nix develop -c cargo test --locked --lib \
  core::core_tests::exports_production_shaped_do_fixture -- --exact --test-threads=1

rm -rf "$persist"
bash "$mock/run.sh" dev --local --ip 127.0.0.1 --port "$port" --persist-to "$persist" &
wrangler_pid=$!
cleanup() {
  kill "$wrangler_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 80); do
  if curl -sf -X POST "http://127.0.0.1:$port/reset" >/dev/null; then
    break
  fi
  sleep 0.25
done

curl -sf -X POST "http://127.0.0.1:$port/seed" \
  -H "content-type: application/json" \
  --data-binary @"$fixture" >/tmp/cowboy-do-seed-response.json
python3 -m json.tool /tmp/cowboy-do-seed-response.json

python3 - <<'PY'
import json, sys
report = json.load(open("/tmp/cowboy-do-seed-response.json"))
checks = [
    report.get("source") == "cowboy-hub-fixture",
    report.get("sessions") == 17,
    report.get("sharedFrame") is True,
    report.get("droppedRawOutput") is True,
    report.get("liveImageExternalized") is True,
    report.get("fitsTarget") is True,
    report.get("fitsIsolate") is True,
    report.get("estimatedWorkingSetBytes", 10**18) < 1024 * 1024,
]
if not all(checks):
    print("DO seed assertions failed", file=sys.stderr)
    sys.exit(1)
print(
    f"ok: {report['sessions']} sessions, "
    f"{report['durableEvents']} sqlite rows, "
    f"live {report['compactLiveBytes']} B, "
    f"working set {report['estimatedWorkingSetBytes']} B "
    f"(raw would have been {report['rawFanoutWouldHaveBeen']} B)"
)
PY
