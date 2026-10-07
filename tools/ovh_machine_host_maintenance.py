#!/usr/bin/env python3
"""OVH-only host maintenance; detached ACP workers keep their existing runtime.

Run from an independent root systemd unit with the prepared receipt directory.
An independent timer must invoke rollback unless acceptance commits the receipt.
This does not enroll Machines, install Plugins, or access Provider credentials.
"""

import hashlib
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

UNIT = "cowboy-machine-svc-4e4d5154f3df9aa109d7d841dd925fd7.service"
TARGET = Path("/home/ubuntu/.config/systemd/user") / (UNIT + ".d/10-plugin-admission.conf")
RUNTIME = Path("/run/user/1000/systemd/user") / (UNIT + ".d/99-usage-host-maintenance.conf")
RECEIPT = Path(os.environ.get(
    "COLUMBUS_COWBOY_MAINTENANCE_RECEIPT",
    "/var/lib/columbus/ovh-cowboy-usage-20261001",
))


def systemctl(*args):
    return subprocess.check_output(
        ["runuser", "-u", "ubuntu", "--", "env", "XDG_RUNTIME_DIR=/run/user/1000",
         "systemctl", "--user", *args], text=True, timeout=45,
    ).strip()


def process(pid):
    root = Path("/proc") / str(pid)
    try:
        fields = (root / "stat").read_text().rpartition(")")[2].split()
        return {"pid": int(pid), "start": fields[19], "parent": int(fields[1]),
                "exe": str((root / "exe").readlink())}
    except (FileNotFoundError, ProcessLookupError):
        return None


def preserved():
    for before in json.loads((RECEIPT / "workers.json").read_text()):
        after = process(before["pid"])
        if after is None or any(after[key] != before[key] for key in ("start", "exe")):
            raise RuntimeError("A retained ACP worker changed; acceptance refused")


def write_atomic(path, data, mode=0o644):
    # Never perform privileged writes through a user-controlled directory.
    if path in (TARGET, RUNTIME):
        subprocess.run([
            "runuser", "-u", "ubuntu", "--", "python3", "-c",
            "import os,pathlib,sys,tempfile; p=pathlib.Path(sys.argv[1]); "
            "p.parent.mkdir(parents=True,exist_ok=True); "
            "fd,name=tempfile.mkstemp(dir=p.parent); "
            "f=os.fdopen(fd,'wb'); f.write(sys.stdin.buffer.read()); f.close(); "
            "os.chmod(name,int(sys.argv[2])); os.replace(name,p)",
            str(path), str(mode),
        ], input=data, check=True, timeout=15)
        return
    descriptor, name = tempfile.mkstemp(dir=path.parent)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(data)
        os.fchmod(handle.fileno(), mode)
    os.replace(name, path)


def host_only_mode():
    write_atomic(RUNTIME, b"[Service]\nKillMode=process\n")
    systemctl("daemon-reload")
    if systemctl("show", UNIT, "--value", "-p", "KillMode") != "process":
        raise RuntimeError("Host-only signal boundary was not applied")


def signal_if_alive(pid, requested_signal):
    try:
        os.kill(pid, requested_signal)
    except ProcessLookupError:
        pass


def switch(configuration, *, preserve_before_start=False):
    host_only_mode()
    old_pid = int(systemctl("show", UNIT, "--value", "-p", "MainPID"))
    adapter_receipt = RECEIPT / "adapters.json"
    auxiliary = json.loads(adapter_receipt.read_text()) if adapter_receipt.exists() else []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit() and (item := process(entry.name)):
            if item["parent"] == old_pid and "cowboy-code-adapter" in Path(item["exe"]).name:
                if item not in auxiliary:
                    auxiliary.append(item)
    # Retain identities across partial activation and reparenting. The rollback
    # timer must still be able to release an orphan's socket after MainPID is 0.
    write_atomic(adapter_receipt, json.dumps(auxiliary).encode(), 0o600)
    systemctl("stop", UNIT)
    # The host's Code adapter is separately supervised, unlike retained ACP
    # workers. Stop only the exact old child so it cannot retain the socket.
    for before in auxiliary:
        current = process(before["pid"])
        if current and all(current[key] == before[key] for key in ("start", "exe")):
            signal_if_alive(before["pid"], signal.SIGTERM)
            deadline = time.monotonic() + 10
            forced = False
            while (current := process(before["pid"])) and all(
                current[key] == before[key] for key in ("start", "exe")
            ):
                if time.monotonic() >= deadline:
                    if forced:
                        raise RuntimeError("Previous Code adapter did not exit")
                    signal_if_alive(before["pid"], signal.SIGKILL)
                    forced = True
                    deadline = time.monotonic() + 2
                time.sleep(0.1)
    if preserve_before_start:
        # The transaction must preserve every original worker through its own
        # stop boundary. After start, Cowboy alone owns idle/busy generation drain.
        preserved()
    write_atomic(TARGET, configuration)
    systemctl("daemon-reload")
    systemctl("start", UNIT)
    new_pid = int(systemctl("show", UNIT, "--value", "-p", "MainPID"))
    current = process(new_pid) if new_pid else None
    if current is None:
        raise RuntimeError("Candidate host is not running")
    return {"previous_pid": old_pid, **current}


def restore_normal_mode():
    subprocess.run(["runuser", "-u", "ubuntu", "--", "rm", "-f", "--", str(RUNTIME)],
                   check=True, timeout=15)
    systemctl("daemon-reload")
    if systemctl("show", UNIT, "--value", "-p", "KillMode") != "control-group":
        raise RuntimeError("Normal service containment was not restored")


def rollout_release():
    plan = json.loads((RECEIPT / "worker-rollout.json").read_text())
    if set(plan) != {"schema", "release"} or plan["schema"] != 1:
        raise RuntimeError("Invalid worker rollout plan")
    release = Path(plan["release"])
    if release.parent != Path("/nix/store") or release.resolve() != release:
        raise RuntimeError("Worker rollout requires an immutable release")
    source = json.loads((release / "etc/cowboy-release/source.json").read_text())
    if (source.get("component") != "cowboy" or source.get("lane") != "machine"
            or source.get("dirty") is not False or source.get("bootstrap", False) is not False
            or not re.fullmatch(r"[a-f0-9]{40}", source.get("revision", ""))
            or not re.fullmatch(r"worker-[a-f0-9]{20}", source.get("workerGeneration", ""))):
        raise RuntimeError("Worker rollout lacks exact production provenance")
    return release, source


def worker_identity(pid):
    current = process(pid)
    if current is None or Path(current["exe"]).name != "cowboy-acp-worker":
        return None
    try:
        raw = (Path("/proc") / str(pid) / "cmdline").read_bytes()
    except (FileNotFoundError, ProcessLookupError):
        return None
    if len(raw) > 256 * 1024:
        raise RuntimeError("Worker command exceeds inspection limit")
    args = raw.decode().split("\0")
    # Never retain the full command/environment or any Provider credential.
    for flag in ("session-id", "socket", "cwd", "provider", "provider-version",
                 "provider-generation-digest", "generation"):
        matches = [index for index, value in enumerate(args) if value == "--" + flag]
        if len(matches) != 1 or matches[0] + 1 >= len(args):
            raise RuntimeError("Worker identity is incomplete or ambiguous")
        current[flag.replace("-", "_")] = args[matches[0] + 1]
    return current


def native_host(release):
    entry = (release / "libexec/cowboy-machine").resolve(strict=True)
    with entry.open("rb") as stream:
        if stream.read(4) == b"\x7fELF":
            return str(entry)
    # The owned Nix package adds a PATH wrapper beside this exact ELF; do not
    # execute or parse an arbitrary shell program to infer an identity.
    entry = entry.with_name(".cowboy-machine-wrapped")
    with entry.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            raise RuntimeError("Machine release lacks its declared native host")
    return str(entry.resolve(strict=True))


def configuration_preflight():
    """Run the candidate release's own `cowboy config check` against the Device
    configuration its host will read, before anything changes. An invalid file,
    or a release that cannot read the current one, refuses maintenance while the
    running host keeps serving. A release older than Cowboy's unified
    configuration has no such command and is admitted unchanged."""
    text = (RECEIPT / "candidate.conf").read_text()
    release = re.search(
        r"^ExecStart=(/nix/store/[a-z0-9]{32}-cowboy-machine-release)/libexec/cowboy-machine(?: |$)",
        text, re.MULTILINE,
    )
    state = re.search(r"--state-dir (/\S+)", text)
    if release is None or state is None:
        return  # candidate_host() refuses a candidate without one release.
    binary = Path(release.group(1)) / "bin/cowboy"
    if not binary.exists():
        return
    result = subprocess.run(
        ["runuser", "-u", "ubuntu", "--", str(binary), "config", "check",
         "--scope", "device", "--state-dir", state.group(1)],
        capture_output=True, text=True, timeout=60,
    )
    output = (result.stdout + result.stderr).strip()
    if result.returncode != 0 and "unrecognized subcommand 'config'" not in output:
        raise RuntimeError("Candidate rejects the Device configuration; nothing was changed:\n" + output)


def candidate_host():
    matches = re.findall(
        r"^ExecStart=(/nix/store/[a-z0-9]{32}-cowboy-machine-release)/libexec/cowboy-machine(?: |$)",
        (RECEIPT / "candidate.conf").read_text(), re.MULTILINE,
    )
    if len(matches) != 1:
        raise RuntimeError("Candidate lacks one immutable Machine release")
    release = Path(matches[0])
    if release.resolve(strict=True) != release:
        raise RuntimeError("Candidate release is not immutable")
    source = json.loads((release / "etc/cowboy-release/source.json").read_text())
    if (source.get("component") != "cowboy" or source.get("lane") != "machine"
            or source.get("dirty") is not False
            or source.get("bootstrap", False) is not False
            or not re.fullmatch(r"[a-f0-9]{40}", source.get("revision", ""))):
        raise RuntimeError("Candidate lacks production provenance")
    return native_host(release)


def settled_host(result):
    expected_exe = candidate_host()
    deadline = time.monotonic() + 5
    while result["exe"] != expected_exe and time.monotonic() < deadline:
        time.sleep(0.05)
        current = process(result["pid"])
        if current is None or current["start"] != result["start"]:
            break
        result.update(current)
    if result["exe"] != expected_exe:
        raise RuntimeError("Candidate did not exec its declared native host")
    return result


def rollout_workers():
    release, source = rollout_release()
    expected_exe = str((release / "bin/cowboy-acp-worker").resolve(strict=True))
    active = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit() and (worker := worker_identity(entry.name)):
            active.append(worker)
    result = []
    for before in json.loads((RECEIPT / "workers.json").read_text()):
        # Socket is the shared Machine broker, not a per-session endpoint.
        # Ownership is scoped by both identities: other sessions use this
        # socket, and another Service can independently use the same session ID.
        owners = [worker for worker in active if all(
            worker[key] == before[key] for key in ("session_id", "socket"))]
        if len(owners) > 1:
            raise RuntimeError("An original session has ambiguous worker ownership")
        unchanged = next((worker for worker in owners if all(
            worker[key] == before[key] for key in ("pid", "start", "exe", "cwd",
                "provider", "provider_version", "provider_generation_digest", "generation")
        )), None)
        if unchanged:
            result.append({"session_id": before["session_id"], "state": "retained",
                           "pid": unchanged["pid"], "start": unchanged["start"]})
            continue
        replacements = [worker for worker in owners if worker["exe"] == expected_exe
            and worker["generation"] == source["workerGeneration"] and all(
                worker[key] == before[key] for key in ("session_id", "socket", "cwd",
                    "provider", "provider_version", "provider_generation_digest"))]
        if len(replacements) != 1:
            raise RuntimeError("An original session lacks its retained or replacement worker")
        result.append({"session_id": before["session_id"], "state": "native_generation_handoff",
                       "pid": replacements[0]["pid"], "start": replacements[0]["start"]})
    return result


def main():
    if os.geteuid() != 0 or len(sys.argv) != 2:
        raise SystemExit("Run as root with activate, accept, activate-workers, accept-workers, or rollback")
    mode = sys.argv[1]
    if mode not in ("activate", "accept", "activate-workers", "accept-workers", "rollback"):
        raise SystemExit("Unknown maintenance action")
    if (RECEIPT / "committed.json").exists():
        return
    baseline = (RECEIPT / "previous.conf").read_bytes()
    rollout = mode in ("activate-workers", "accept-workers")
    release, source = rollout_release() if rollout else (None, None)
    if mode in ("activate", "activate-workers"):
        if TARGET.read_bytes() != baseline or RUNTIME.exists() or (RECEIPT / "started.json").exists():
            raise RuntimeError("Host configuration changed before maintenance")
        expected_host = json.loads((RECEIPT / "host.json").read_text())
        current = process(int(systemctl("show", UNIT, "--value", "-p", "MainPID")))
        if current is None or any(current[key] != expected_host[key] for key in ("pid", "start", "exe")):
            raise RuntimeError("Host identity changed before maintenance")
        preserved()
        configuration_preflight()
        write_atomic(RECEIPT / "started.json", b"{}", 0o600)
        if rollout:
            result = switch((RECEIPT / "candidate.conf").read_bytes(), preserve_before_start=True)
            expected_exe = native_host(release)
            deadline = time.monotonic() + 5
            while result["exe"] != expected_exe and time.monotonic() < deadline:
                time.sleep(0.05)
                current = process(result["pid"])
                if current is None or current["start"] != result["start"]:
                    break
                result.update(current)
            if result["exe"] != expected_exe:
                raise RuntimeError("Worker rollout started a different host")
            result.update(worker_generation=source["workerGeneration"],
                          original_workers_preserved_until_host_start=True)
        else:
            result = switch((RECEIPT / "candidate.conf").read_bytes())
            result = settled_host(result)
            preserved()
        write_atomic(RECEIPT / "awaiting-acceptance.json", json.dumps(result).encode(), 0o600)
    elif mode in ("accept", "accept-workers"):
        expected = json.loads((RECEIPT / "awaiting-acceptance.json").read_text())
        if ("worker_generation" in expected) != rollout:
            raise RuntimeError("Maintenance acceptance mode differs from activation")
        if int(systemctl("show", UNIT, "--value", "-p", "MainPID")) != expected["pid"]:
            raise RuntimeError("Candidate host changed before acceptance")
        current = process(expected["pid"]) if expected["pid"] else None
        if current is None or current["start"] != expected["start"]:
            raise RuntimeError("Candidate host is not the retained live process")
        if TARGET.read_bytes() != (RECEIPT / "candidate.conf").read_bytes():
            raise RuntimeError("Candidate configuration changed before acceptance")
        if current["exe"] != candidate_host():
            raise RuntimeError("Candidate host is not its declared native executable")
        if current["exe"] != expected["exe"]:
            # Preserve the old helper's transient wrapper observation; the PID
            # and start time must be unchanged and the final ELF independently
            # matches the reviewed immutable candidate. Never rewrite admission.
            expected["launch_exe"] = expected["exe"]
            expected["exe"] = current["exe"]
        if rollout:
            if expected["worker_generation"] != source["workerGeneration"]:
                raise RuntimeError("Worker generation changed before acceptance")
            expected["workers"] = rollout_workers()
        else:
            preserved()
        restore_normal_mode()
        expected.update(accepted_at=int(time.time()),
                        configuration_sha256=hashlib.sha256(TARGET.read_bytes()).hexdigest())
        write_atomic(RECEIPT / "committed.json", json.dumps(expected).encode(), 0o600)
    else:
        if not (RECEIPT / "started.json").exists() or (RECEIPT / "rolled-back.json").exists():
            return
        try:
            result = switch(baseline)
        finally:
            restore_normal_mode()
        write_atomic(RECEIPT / "rolled-back.json", json.dumps(result).encode(), 0o600)


if __name__ == "__main__":
    with (RECEIPT / "maintenance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        main()
