#!/usr/bin/env python3
"""Probe exact release bytes; optionally drive the real detached ACP worker.

Worker mode runs in the recipe's loopback-only Linux network namespace. It never
uses Service credentials, installs on a Machine, or sends an inference prompt.
The release must independently pass the SDK/signature gate before publication;
this harness supplies behavioral evidence, not a replacement trust authority.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import signal
import socket
import struct
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import urlparse
import urllib.request


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    require(path.is_file() and not path.is_symlink(), "artifact is not a regular file")
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return "sha256:" + result.hexdigest()


def extract_archive(source, target):
    """Extract only bounded regular files/directories into a new private root."""
    with tarfile.open(source, "r:gz") as archive:
        count, total = 0, 0
        for member in archive:
            count += 1
            total += member.size
            require(count <= 100_000 and total <= 2 * 1024**3, "archive exceeds bounds")
            path = PurePosixPath(member.name)
            require(
                not path.is_absolute() and ".." not in path.parts
                and "\\" not in member.name and "\x00" not in member.name,
                "unsafe archive path",
            )
            require(member.isdir() or member.isfile(), "archive contains a link or special file")
            destination = target.joinpath(*path.parts)
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True, mode=0o700)
                continue
            require(bool(path.parts), "archive file has an empty path")
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with archive.extractfile(member) as content, destination.open("xb") as output:
                shutil.copyfileobj(content, output)
            require(destination.stat().st_size == member.size, "truncated archive member")
            destination.chmod(0o700 if member.mode & 0o111 else 0o600)


def closed_environment(private_home):
    private_home.mkdir(parents=True, exist_ok=True, mode=0o700)
    environment = {
        name: os.environ[name]
        for name in ("PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE")
        if name in os.environ
    }
    environment.update({
        "HOME": str(private_home), "XDG_CONFIG_HOME": str(private_home / "config"),
        "XDG_CACHE_HOME": str(private_home / "cache"),
        "XDG_DATA_HOME": str(private_home / "data"), "TMPDIR": str(private_home),
        "CODEX_HOME": str(private_home / "codex"),
        "CLAUDE_CONFIG_DIR": str(private_home / "claude"),
    })
    return environment


def stop_group(process):
    # Every subprocess here is created with start_new_session=True.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=10)


def probe(command, arguments, private_home, timeout):
    process = subprocess.Popen(
        [str(command), *arguments], cwd=private_home,
        env=closed_environment(private_home), start_new_session=True,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        require(process.wait(timeout=timeout) == 0, f"probe failed: {command.name}")
    finally:
        stop_group(process)


class Candidate:
    def __init__(self, release_path, artifacts, root):
        self.release = json.loads(release_path.read_text())
        self.id = self.release["plugin_id"]
        self.version = self.release["plugin_version"]
        self.root = root
        root.mkdir(mode=0o700)
        package_path = release_path.with_name(release_path.name.removesuffix(".release.json") + ".cowboy-plugin")
        require(digest(package_path) == self.release["package_digest"], "package digest mismatch")
        outer = json.loads(package_path.read_text())
        require(outer["payload"]["kind"] == "agent_provider", "not an Agent Plugin")
        package = outer["payload"]["contract"]
        self.manifest = package["manifest"]
        require(self.manifest["id"] == self.id and self.manifest["version"] == self.version,
                "release and Provider identity disagree")
        self.package_path = root / "provider.json"
        self.package_path.write_text(json.dumps(package))
        os_name = {"Linux": "linux", "Darwin": "macos"}.get(platform.system())
        architecture = {"x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(platform.machine())
        matches = [target for target in self.release["runtime_artifacts"]
                   if target["os"] == os_name and target["architecture"] == architecture]
        require(len(matches) == 1, "release does not cover this exact platform")
        self.target = matches[0]
        self.commands = {}
        self.probes = []
        for component in self.target["components"]:
            name = component["command"]
            require(name and all(c.isalnum() or c in "-_." for c in name), "unsafe command")
            relative = f"{name}.tar.gz" if component["artifact_format"] == "tar_gz" else name
            source = artifacts / "runtime" / f"{os_name}-{architecture}" / relative
            if not source.exists():
                url_name = PurePosixPath(urlparse(component["artifact_url"]).path).name
                source = artifacts / "artifacts" / component["artifact_digest"].removeprefix("sha256:") / url_name
            require(digest(source) == component["artifact_digest"], f"runtime digest mismatch: {name}")
            destination = root / name
            destination.mkdir(mode=0o700)
            if component["artifact_format"] == "tar_gz":
                extract_archive(source, destination)
                entrypoint = component["entrypoint"]
                require(not PurePosixPath(entrypoint).is_absolute()
                        and ".." not in PurePosixPath(entrypoint).parts, "unsafe entrypoint")
                executable = destination / entrypoint
            else:
                executable = destination / name
                shutil.copyfile(source, executable)
                executable.chmod(0o700)
            require(executable.is_file() and not executable.is_symlink(), "missing executable")
            self.commands[name] = str(executable)
            private_home = root / f"probe-{name}"
            closed_environment(private_home)
            probe(executable, component["probe"]["args"], private_home,
                  min(component["probe"]["timeout_ms"] / 1000, 120))
            self.probes.append({"command": name, "version": component["version"],
                                "artifact_digest": component["artifact_digest"], "status": "passed"})


def write_frame(connection, frame):
    data = json.dumps(frame).encode()
    require(len(data) <= 4 * 1024**2, "frame exceeds bound")
    connection.sendall(struct.pack(">I", len(data)) + data)


def read_exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise EOFError("worker connection closed")
        data.extend(chunk)
    return data


def read_frame(connection):
    size = struct.unpack(">I", read_exact(connection, 4))[0]
    require(0 < size <= 4 * 1024**2, "invalid worker frame size")
    return json.loads(read_exact(connection, size))


def descendants(pid):
    rows = subprocess.check_output(["ps", "-eo", "pid=,ppid="], text=True)
    parents = dict(tuple(map(int, row.split())) for row in rows.splitlines())
    owned = {pid}
    while True:
        expanded = owned | {child for child, parent in parents.items() if parent in owned}
        if expanded == owned:
            return owned - {pid}
        owned = expanded


def is_running(pid):
    try:
        # Zombies are already terminated; the parent/init owns their final reap.
        return Path(f"/proc/{pid}/stat").read_text().split(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def fixture_session_id(plugin_id, root):
    # Session IDs also own ACP cgroups. Concurrent harness invocations must
    # never share that identity, even for the same Plugin version.
    return f"conformance-{plugin_id}-{root.parent.name}-{root.name}"


class Worker:
    def __init__(self, candidate, executable, root):
        self.candidate = candidate
        self.connection = None
        self.process = None
        self.child_pids = set()
        self.sidecars = []
        self.ready = False
        self.session = fixture_session_id(candidate.id, root)
        root.mkdir(mode=0o700)
        private_home = root / "home"
        environment = closed_environment(private_home)
        for name in candidate.manifest["authentication"]["environment_projection"]:
            environment[name] = "cowboy-conformance-not-a-credential"
        for credential in candidate.manifest["authentication"]["credential_files"]:
            if credential["required"]:
                require(credential["bundle_key"] == "api_key", "required credential has no hermetic fixture")
                path = PurePosixPath(credential["relative_path"])
                require(not path.is_absolute() and ".." not in path.parts, "unsafe credential fixture path")
                destination = private_home.joinpath(*path.parts)
                destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                destination.write_text("cowboy-conformance-not-a-credential")
                destination.chmod(0o600)
        environment.update({
            "COWBOY_PROVIDER_PACKAGE_PATH": str(candidate.package_path),
            "COWBOY_PROVIDER_COMPONENT_COMMANDS": json.dumps(candidate.commands),
            "RUST_LOG": "info",
        })
        workspace = root / "workspace"
        workspace.mkdir()
        self.log = (root / "worker.log").open("w+")
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(root / "broker.sock"))
        self.listener.listen()
        self.listener.settimeout(20)
        try:
            self.process = subprocess.Popen([
                str(executable), "--socket", str(root / "broker.sock"),
                "--session-id", self.session, "--provider", candidate.id,
                "--provider-version", candidate.version, "--provider-generation-digest",
                candidate.release["artifact_digest"], "--provider-auth-generation", "1",
                "--cwd", str(workspace), "--generation", "conformance",
            ], cwd=workspace, env=environment, start_new_session=True,
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=self.log)
            self.connection, _ = self.listener.accept()
            self.connection.settimeout(90)
            hello = read_frame(self.connection)
            require(hello["type"] == "hello" and hello["session_id"] == self.session,
                    "worker handshake mismatch")
            write_frame(self.connection, {"type": "welcome", "protocol": 1, "controller_epoch": 1})
            self.wait_ready()
            self.capture_sidecars()
        except BaseException as error:
            self.log.flush()
            self.log.seek(0)
            diagnostic = self.log.read()[-4000:]
            self.cleanup()
            raise RuntimeError(f"{candidate.id}@{candidate.version}: {error}\n{diagnostic}") from error

    def receive(self):
        frame = read_frame(self.connection)
        if frame["type"] == "worker_event":
            write_frame(self.connection, {"type": "ack", "session_id": self.session,
                        "worker_epoch": frame["worker_epoch"], "runtime_seq": frame["runtime_seq"]})
        return frame

    def wait_ready(self):
        deadline = time.monotonic() + 90
        native_session = None
        running = False
        while time.monotonic() < deadline:
            frame = self.receive()
            if frame["type"] == "snapshot":
                launch = frame["worker"]["launch"]
                require(launch["provider_generation_digest"] == self.candidate.release["artifact_digest"],
                        "worker selected another generation")
                native_session = frame["worker"].get("agent_session_id")
                running = frame["worker"]["state"] == "running"
            event = frame.get("event", {})
            if event.get("event") == "ready":
                native_session = event.get("agent_session_id")
                running = True
            if event.get("event") == "agent_session_id":
                native_session = event.get("agent_session_id")
            if event.get("event") == "status" and event.get("state") == "running":
                running = True
            if native_session and running:
                self.ready = True
                return
            require(event.get("state") not in ("crashed", "exited"), f"session failed: {event}")
        raise TimeoutError("ACP initialize/session-new timed out")

    def capture_sidecars(self):
        self.child_pids.update(descendants(self.process.pid))
        for sidecar in self.candidate.manifest["runtime"].get("sidecars", []):
            requirement = next(component for component in self.candidate.target["components"]
                               if component["slot"] == sidecar["component"]["slot"])
            executable = Path(self.candidate.commands[requirement["command"]])
            matches = [pid for pid in self.child_pids
                       if Path(f"/proc/{pid}/exe").exists()
                       and Path(f"/proc/{pid}/exe").resolve() == executable]
            require(len(matches) == 1, "worker did not own its exact sidecar executable")
            pid = matches[0]
            arguments = Path(f"/proc/{pid}/cmdline").read_bytes().decode().strip("\x00").split("\x00")
            address = arguments[arguments.index(sidecar["transport"]["listen_argument"]) + 1]
            require(address.startswith("127.0.0.1:"), "sidecar is not on private loopback")
            url = "http://" + address + sidecar["transport"]["health_path"]
            self.sidecars.append((pid, url))
        self.assert_alive()

    def assert_alive(self):
        require(self.process.poll() is None, "worker exited before drain")
        for pid, url in self.sidecars:
            require(is_running(pid), "sidecar exited before worker drain")
            with urllib.request.urlopen(url, timeout=2) as response:
                require(response.status == 200, "sidecar readiness changed")

    def stop(self):
        self.child_pids.update(descendants(self.process.pid))
        write_frame(self.connection, {"type": "worker_command", "session_id": self.session,
                    "command": {"command": "stop", "command_id": "conformance-stop"}})
        self.connection.settimeout(20)
        try:
            while True:
                self.receive()
        except EOFError:
            pass
        require(self.process.wait(timeout=15) == 0, "worker failed during stop")
        deadline = time.monotonic() + 5
        while any(is_running(pid) for pid in self.child_pids) and time.monotonic() < deadline:
            time.sleep(0.05)
        require(not any(is_running(pid) for pid in self.child_pids), "worker left a running descendant")

    def cleanup(self):
        if self.process:
            self.child_pids.update(descendants(self.process.pid))
            # ACP owns another process group. Kill only recorded fixture descendants.
            for pid in self.child_pids:
                if is_running(pid):
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            stop_group(self.process)
        if self.connection:
            self.connection.close()
        self.listener.close()
        self.log.close()


def require_worker_isolation():
    require(platform.system() == "Linux", "worker harness currently requires Linux")
    require(os.geteuid() != 0, "worker conformance must preserve a non-root user")
    require(set(socket.if_nameindex()) == {(1, "lo")}, "worker conformance requires an isolated loopback-only namespace")


def validate_receipt_paths(receipt, failure_receipt):
    paths = [path for path in (receipt, failure_receipt) if path is not None]
    require(len({path.resolve() for path in paths}) == len(paths),
            "acceptance and failure receipts must use different paths")
    for path in paths:
        require(not os.path.lexists(path), "receipt already exists; use a new evidence path")


def write_receipt(path, receipt):
    """Commit complete private evidence without replacing any existing file/link."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                         prefix=".conformance-", delete=False) as output:
            temporary = Path(output.name)
            output.write(json.dumps(receipt, indent=2) + "\n")
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink()


def evidence_identity(candidate):
    return {"plugin_id": candidate.id, "plugin_version": candidate.version,
            "artifact_digest": candidate.release["artifact_digest"]}


def run_conformance(args, progress):
    with tempfile.TemporaryDirectory(prefix="cw-conformance-", dir="/tmp") as temporary:
        root = Path(temporary).resolve()
        progress["phase"] = "candidate_artifacts"
        candidate = Candidate(args.release.resolve(), args.artifacts.resolve(), root / "candidate")
        progress["candidate"] = evidence_identity(candidate)
        progress["completed"].append("candidate_artifact_probes")
        receipt = {"schema_version": 1, "plugin_id": candidate.id,
                   "plugin_version": candidate.version, "artifact_digest": candidate.release["artifact_digest"],
                   "platform": {"os": candidate.target["os"], "architecture": candidate.target["architecture"]},
                   "probes": candidate.probes, "uses_service_credentials": False, "sends_prompt": False}
        workers = []
        try:
            if args.previous:
                progress["phase"] = "previous_artifacts"
                previous = Candidate(args.previous.resolve(), args.previous_artifacts.resolve(), root / "previous")
                progress["previous"] = evidence_identity(previous)
                progress["completed"].append("previous_artifact_probes")
                progress["phase"] = "previous_identity"
                require(previous.id == candidate.id and previous.release["artifact_digest"] != candidate.release["artifact_digest"],
                        "coexistence must use two distinct exact releases of one Plugin")
                progress["phase"] = "previous_worker_startup"
                workers.append(Worker(previous, args.worker, root / "old"))
                progress["completed"].append("previous_worker_ready")
                receipt["previous"] = {"plugin_version": previous.version,
                                       "artifact_digest": previous.release["artifact_digest"]}
            if args.worker:
                progress["phase"] = "candidate_worker_startup"
                workers.append(Worker(candidate, args.worker, root / "new"))
                progress["completed"].append("candidate_worker_ready")
                progress["phase"] = "generation_coexistence"
                ports = [url for worker in workers for _, url in worker.sidecars]
                require(len(ports) == len(set(ports)), "generations shared a sidecar port")
                for worker in workers:
                    worker.assert_alive()
                if len(workers) == 2:
                    progress["phase"] = "previous_worker_drain"
                    workers[0].stop()
                    progress["completed"].append("previous_worker_drained")
                    progress["phase"] = "candidate_survives_previous_drain"
                    workers[1].assert_alive()
                    progress["completed"].append("distinct_generation_coexistence")
                progress["phase"] = "candidate_worker_drain"
                workers[-1].stop()
                progress["completed"].append("candidate_worker_drained")
                receipt["worker"] = {"executable_digest": digest(args.worker), "initialize_and_session_new": "passed",
                                     "stop_and_descendant_drain": "passed", "sidecar_count": len(ports),
                                     "distinct_generation_coexistence": "passed" if len(workers) == 2 else "not_checked"}
        finally:
            for worker in reversed(workers):
                try:
                    worker.cleanup()
                except Exception:
                    progress["phase"] = "fixture_cleanup"
                    raise
        return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("release", type=Path)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("--worker", type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--previous-artifacts", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--failure-receipt", type=Path,
                        help="write separate non-acceptance evidence on failure; never changes the exit status")
    args = parser.parse_args()
    # Refuse stale/colliding evidence paths before any runtime is extracted or
    # started. Atomic create-only publication repeats that check at commit time.
    validate_receipt_paths(args.receipt, args.failure_receipt)
    progress = {"phase": "input_validation", "completed": []}
    try:
        if args.worker:
            require_worker_isolation()
            require(args.worker.is_absolute() and args.worker.is_file(), "worker must be an exact absolute build result")
            progress["worker_executable_digest"] = digest(args.worker)
        require(bool(args.previous) == bool(args.previous_artifacts), "previous release needs its artifact root")
        require(args.previous is None or args.worker is not None,
                "previous generation needs worker conformance")
        receipt = run_conformance(args, progress)
        progress["phase"] = "acceptance_receipt"
        if args.receipt:
            write_receipt(args.receipt, receipt)
    except Exception as error:
        if args.failure_receipt:
            # Do not copy exception messages, stdout, logs, argv, configuration,
            # environment or credentials into durable diagnostic evidence.
            write_receipt(args.failure_receipt, {
                "schema": "dravengarden.cowboy.plugin-runtime-failure/v1",
                "status": "failed", **progress, "exception_type": type(error).__name__,
                "acceptance_receipt_created": False,
                "uses_service_credentials": False, "sends_prompt": False,
                "signature_verification": "separate_required_gate",
            })
        raise
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
