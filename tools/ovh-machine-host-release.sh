#!/usr/bin/env bash
# Replace the OVH Machine host binary; detached ACP workers keep running.
# With --workers, also select the release's worker bundle: idle workers adopt
# it, busy ones at their native safe boundary. See docs/ovh-machine-maintenance.md.
# Run on Hawk from the repository root:
#   tools/ovh-machine-host-release.sh [--workers] /nix/store/<hash>-cowboy-machine-release
set -euo pipefail

workers=0
if (($# == 2)) && [[ $1 == --workers ]]; then
  workers=1
  shift
fi
if (($# != 1)); then
  echo "usage: $0 [--workers] /nix/store/<hash>-cowboy-machine-release" >&2
  exit 2
fi
release=$1
host=${OVH_SSH_ALIAS:-ovh}
unit_dropin=/home/ubuntu/.config/systemd/user/cowboy-machine-svc-4e4d5154f3df9aa109d7d841dd925fd7.service.d/10-plugin-admission.conf
script_dir=$(cd "$(dirname "$0")" && pwd)

[[ $release =~ ^/nix/store/[a-z0-9]{32}-cowboy-machine-release$ ]] || {
  echo "release must be an immutable cowboy-machine-release store path" >&2
  exit 2
}
source_json=$release/etc/cowboy-release/source.json
revision=$(python3 - "$source_json" <<'PY'
import json, re, sys
source = json.load(open(sys.argv[1]))
assert source.get("component") == "cowboy" and source.get("lane") == "machine", source
assert source.get("dirty") is False and not source.get("bootstrap", False), source
assert re.fullmatch(r"[a-f0-9]{40}", source.get("revision", "")), source
print(source["revision"])
PY
)
git fetch -q origin
git merge-base --is-ancestor "$revision" origin/main || {
  echo "release revision $revision is not on origin/main" >&2
  exit 1
}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
short=${revision:0:12}
receipt=/var/lib/columbus/ovh-machine-$short-$stamp
units=columbus-machine-$short-$stamp
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

remote() { ssh -o BatchMode=yes "$host" "$@"; }

echo "== target identity"
remote "sudo -n grep -q -- '--machine-id ovh ' $unit_dropin"
remote "sudo -n grep -c '^ExecStart=/nix/store/[a-z0-9]\{32\}-cowboy-machine-release/libexec/cowboy-machine ' $unit_dropin" |
  grep -qx 1 || {
  echo "live drop-in does not name exactly one Machine host release" >&2
  exit 1
}

echo "== closure"
nix-store -qR "$release" >"$work/closure"
remote 'ls -d /nix/store/*' >"$work/present"
grep -vxF -f "$work/present" "$work/closure" >"$work/missing" || true
if [[ -s $work/missing ]]; then
  tar -C / -cpf "$work/closure.tar" $(sed 's#^/##' "$work/missing")
  local_sum=$(sha256sum "$work/closure.tar" | cut -d' ' -f1)
  scp -o BatchMode=yes -q "$work/closure.tar" "$host:/var/tmp/ovh-machine-$short.tar"
  remote "echo '$local_sum  /var/tmp/ovh-machine-$short.tar' | sha256sum -c --quiet &&
    sudo -n tar -C / -xpf /var/tmp/ovh-machine-$short.tar && rm -f /var/tmp/ovh-machine-$short.tar"
fi
python3 - "$revision" "$release" "$work/closure" >"$work/manifest.json" <<'PY'
import json, sys
paths = [line.strip() for line in open(sys.argv[3]) if line.strip()]
print(json.dumps({"revision": sys.argv[1], "release": sys.argv[2], "paths": paths}))
PY
remote "sudo -n install -d -m 0755 /nix/var/nix/gcroots/ovh-cowboy-machine-$short &&
  sudo -n tee /nix/var/nix/gcroots/ovh-cowboy-machine-$short/closure-manifest.json >/dev/null &&
  sudo -n python3 -c 'import json,os,sys; m=json.load(open(sys.argv[1])); missing=[p for p in m[\"paths\"] if not os.path.exists(p)]; assert not missing, missing' \
    /nix/var/nix/gcroots/ovh-cowboy-machine-$short/closure-manifest.json" <"$work/manifest.json"

echo "== receipt $receipt"
remote "sudo -n bash -c 'test ! -e $receipt && install -d -m 0700 -o root -g root $receipt && cat > $receipt/maintenance.py && chmod 0700 $receipt/maintenance.py'" \
  <"$script_dir/ovh_machine_host_maintenance.py"
remote "sudo -n env RELEASE='$release' REVISION='$revision' WORKERS='$workers' D='$receipt' CONF='$unit_dropin' bash -s" <<'EOF'
set -euo pipefail
install -m 0600 "$CONF" "$D/previous.conf"
python3 - <<'PY'
import json, os, re
from pathlib import Path
d = Path(os.environ["D"])
release = os.environ["RELEASE"]
text = (d / "previous.conf").read_text()
pattern = r"^(ExecStart=)/nix/store/[a-z0-9]{32}-cowboy-machine-release(/libexec/cowboy-machine )"
text, count = re.subn(pattern, r"\g<1>" + release + r"\g<2>", text, flags=re.MULTILINE)
assert count == 1, "expected one Machine host ExecStart"
text = re.sub(r"^# Machine host: Cowboy [a-f0-9]{40}$",
              "# Machine host: Cowboy " + os.environ["REVISION"], text, flags=re.MULTILINE)
if os.environ["WORKERS"] == "1":
    source = json.loads((Path(release) / "etc/cowboy-release/source.json").read_text())
    generation = source["workerGeneration"]
    assert re.fullmatch(r"worker-[a-f0-9]{20}", generation), source
    text, count = re.subn(r"(--desired-generation )worker-[a-f0-9]{20}", r"\g<1>" + generation, text)
    assert count == 1, "expected one desired worker generation"
    text, count = re.subn(r"(--worker-command )/nix/store/[a-z0-9]{32}-cowboy-machine-release/bin/cowboy-acp-worker",
                          r"\g<1>" + release + "/bin/cowboy-acp-worker", text)
    assert count == 1, "expected one worker command"
    (d / "worker-rollout.json").write_text(json.dumps({"schema": 1, "release": release}))
    os.chmod(d / "worker-rollout.json", 0o600)
(d / "candidate.conf").write_text(text)
os.chmod(d / "candidate.conf", 0o600)
PY
touch "$D/maintenance.lock"
cd "$D"
COLUMBUS_COWBOY_MAINTENANCE_RECEIPT="$D" python3 - <<'PY'
import importlib.util, json, os
from pathlib import Path
spec = importlib.util.spec_from_file_location("m", "maintenance.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
host = m.process(int(m.systemctl("show", m.UNIT, "--value", "-p", "MainPID")))
assert host is not None, "Machine host is not running"
Path("host.json").write_text(json.dumps({k: host[k] for k in ("pid", "start", "exe")}))
if Path("worker-rollout.json").exists():
    # A rollout recognizes each session's retained or replacement worker.
    workers = [item for entry in Path("/proc").iterdir()
               if entry.name.isdigit() and (item := m.worker_identity(entry.name))]
else:
    workers = [{k: item[k] for k in ("pid", "start", "exe")}
               for entry in Path("/proc").iterdir()
               if entry.name.isdigit() and (item := m.process(entry.name))
               and Path(item["exe"]).name == "cowboy-acp-worker"]
Path("workers.json").write_text(json.dumps(workers))
os.chmod("host.json", 0o600)
os.chmod("workers.json", 0o600)
print(f"host pid {host['pid']}, {len(workers)} retained workers, candidate {m.candidate_host()}")
PY
rm -rf "$D/__pycache__"
diff "$D/previous.conf" "$D/candidate.conf" || true
EOF

activate=activate
accept=accept
if ((workers)); then
  activate=activate-workers
  accept=accept-workers
fi

echo "== $activate (automatic rollback in 4 minutes unless accepted)"
remote "sudo -n systemd-run --unit=$units-rollback --on-active=4min \
    --setenv=COLUMBUS_COWBOY_MAINTENANCE_RECEIPT=$receipt /usr/bin/python3 $receipt/maintenance.py rollback"
# A Cowboy session on OVH may lose this SSH connection while its host restarts;
# the activation unit and rollback timer are independent of it.
remote "sudo -n systemd-run --unit=$units-activate --wait --collect \
    --setenv=COLUMBUS_COWBOY_MAINTENANCE_RECEIPT=$receipt /usr/bin/python3 $receipt/maintenance.py $activate" || true

echo "== verify"
pid=0
for _ in $(seq 1 30); do
  if remote "sudo -n test -e $receipt/awaiting-acceptance.json"; then
    pid=$(remote "sudo -n python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"pid\"])' $receipt/awaiting-acceptance.json")
    if remote "sudo -n journalctl _PID=$pid --no-pager -o cat | grep -q 'Machine controller authenticated'"; then
      break
    fi
  fi
  sleep 4
done
remote "sudo -n journalctl _PID=$pid --no-pager -o cat | grep -q 'Machine controller authenticated'" || {
  echo "candidate host did not reconnect to the controller; the rollback timer will restore $receipt/previous.conf" >&2
  exit 1
}

echo "== $accept"
# Idle workers are replaced right after the new host connects; a session can
# be between its old and new worker for a moment. Acceptance never changes
# anything until it succeeds, so repeat it within the rollback window.
accepted=0
for _ in $(seq 1 $((workers ? 20 : 1))); do
  if remote "sudo -n env COLUMBUS_COWBOY_MAINTENANCE_RECEIPT=$receipt python3 $receipt/maintenance.py $accept"; then
    accepted=1
    break
  fi
  sleep 8
done
((accepted)) || {
  echo "acceptance failed; the rollback timer will restore $receipt/previous.conf" >&2
  exit 1
}
remote "sudo -n systemctl stop $units-rollback.timer && sudo -n rm -rf $receipt/__pycache__ &&
  sudo -n cat $receipt/committed.json"
echo
echo "OVH Machine host now runs Cowboy $revision ($receipt)"
