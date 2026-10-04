#!/usr/bin/env python3
"""Immutable private writer ELF acceptance; never enables a production writer."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import tempfile
import time


CHILD = "machine_broker::tests::deletion_process::child"
SERVICE = "svc-0123456789abcdef0123456789abcdef"
MAX_FRAME = 32 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def artifact(value, fixture):
    root = Path(value)
    require(root.is_absolute() and root.parent == Path("/nix/store")
            and root.resolve() == root, "artifact must be an exact immutable store root")
    source = json.loads((root / "etc/cowboy-release/source.json").read_text())
    require(source["schema"] == 1 and not source["dirty"]
            and len(source["revision"]) == 40, "clean revision required")
    require(source["repository"] == "git@github.com:dravengarden/cowboy.git",
            "unexpected repository")
    if fixture:
        require(source["component"] == "cowboy-test" and source["lane"] == "conformance"
                and source["fixture"] == "session-deletion-private-broker"
                and source["sessionDeletionJournal"] == {"readerSchema": 1, "writerSchema": 1},
                "only nondeployable private broker fixtures may supply a writer")
        binary = root / "bin/cowboy-deletion-conformance-native"
    else:
        require(source["component"] == "cowboy" and source["lane"] == "machine"
                and source["sessionDeletionJournal"] == {"readerSchema": 1, "writerSchema": 0},
                "production reader must remain writer-disabled")
        binary = (root / "libexec/cowboy-machine").resolve().parent / ".cowboy-machine-wrapped"
    with binary.open("rb") as file:
        require(file.read(4) == b"\x7fELF", "native ELF required")
        file.seek(0)
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    return {"release": str(root), "source": source, "native": str(binary), "sha256": digest}


class Process:
    def __init__(self, release, root, mode="reader", checkpoint="", fixture=True, extra_env=None):
        self.root = root
        log_fd, log_path = tempfile.mkstemp(prefix="process-", suffix=".log", dir=root)
        os.close(log_fd)
        self.log = Path(log_path)
        self.socket = root / "runtime.sock"
        existing = self.socket.lstat() if self.socket.exists() else None
        self.original_socket = (existing.st_dev, existing.st_ino) if existing else None
        self.controller = None
        env = {"HOME": str(root / "home"), "TMPDIR": str(root), "LANG": "C.UTF-8",
               "XDG_CONFIG_HOME": str(root / "config"), "XDG_CACHE_HOME": str(root / "cache"),
               "XDG_DATA_HOME": str(root / "data"), "RUST_LOG": "info"}
        env.update(extra_env or {})
        if fixture:
            env.update(COWBOY_TEST_DELETION_ROOT=str(root), COWBOY_TEST_DELETION_MODE=mode,
                       COWBOY_TEST_DELETION_CHECKPOINT=checkpoint)
            command = [release["native"], "--exact", CHILD, "--ignored", "--nocapture",
                       "--test-threads=1"]
        else:
            self.controller = socket.socket()
            self.controller.bind(("127.0.0.1", 0))
            self.controller.listen()
            command = [str(Path(release["release"]) / "bin/cowboy-machine"),
                       "--controller-url", f"http://127.0.0.1:{self.controller.getsockname()[1]}",
                       "--service-id", SERVICE, "--machine-id", "release-fixture",
                       "--spawn-mode", "direct", "--worker-command", "/no-fixture-worker",
                       "--state-dir", str(root), "--socket", str(self.socket),
                       "--provider-usage-socket", str(root / "usage.sock"),
                       "--workspace-config", str(root / "absent-workspaces.json")]
        with self.log.open("wb") as output:
            self.child = subprocess.Popen(command, env=env, cwd=root, stdin=subprocess.DEVNULL,
                                          stdout=output, stderr=output, start_new_session=True)

    def output(self):
        return self.log.read_text(errors="replace")

    def marker(self, marker):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            text = self.output()
            require(self.child.poll() is None, f"process exited before {marker}: {text}")
            if marker in text:
                return
            time.sleep(0.01)
        raise RuntimeError(f"missing {marker}: {self.output()}")

    def ready(self, fixture=True):
        self.marker("COWBOY_DELETION_CHILD_READY" if fixture
                    else "Session deletion journal reader ready")
        deadline = time.monotonic() + 10
        while not self.socket.exists():
            require(self.child.poll() is None and time.monotonic() < deadline,
                    f"broker did not bind: {self.output()}")
            time.sleep(0.01)

    def refused(self, reason):
        try:
            status = self.child.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(f"refusal hung: {self.output()}") from error
        require(status != 0 and reason in self.output(), f"wrong refusal: {self.output()}")
        if self.original_socket is None:
            require(not self.socket.exists(), "invalid namespace bound a broker")
        else:
            current = self.socket.lstat()
            require((current.st_dev, current.st_ino) == self.original_socket,
                    "refused process replaced the existing broker socket")

    def kill(self):
        if self.child.poll() is None:
            os.killpg(self.child.pid, signal.SIGKILL)
            require(self.child.wait(timeout=10) == -signal.SIGKILL, "SIGKILL not confirmed")

    def close(self):
        self.kill()
        if self.controller:
            self.controller.close()

    def __enter__(self):
        return self

    def __exit__(self, *unused):
        self.close()


def send(peer, frame):
    payload = json.dumps(frame, separators=(",", ":")).encode()
    require(len(payload) <= MAX_FRAME, "oversized request")
    peer.sendall(struct.pack("!I", len(payload)) + payload)


def exact(peer, count, clean_eof=False):
    data = bytearray()
    while len(data) < count:
        chunk = peer.recv(count - len(data))
        if not chunk and not data and clean_eof:
            return None
        require(bool(chunk), "truncated IPC frame")
        data.extend(chunk)
    return data


def receive(peer):
    header = exact(peer, 4, clean_eof=True)
    if header is None:
        return None
    size = struct.unpack("!I", header)[0]
    require(size <= MAX_FRAME, "oversized response")
    return json.loads(exact(peer, size))


def connect(process, role):
    peer = socket.socket(socket.AF_UNIX)
    peer.settimeout(5)
    peer.connect(str(process.socket))
    send(peer, {"type": "hello", "role": role, "min_protocol": 1, "max_protocol": 2,
                "build": "immutable-writer-fixture", "session_id": "sess-1" if role == "worker" else None,
                "worker_epoch": "old-epoch" if role == "worker" else None,
                "generation": "gen-1", "executable": "/bin/false", "fallback_for": None})
    return peer, receive(peer)


def stop(peer, command_id="immutable-delete"):
    send(peer, {"type": "core_command", "command": {"command": "stop_session",
               "session_id": "sess-1", "command_id": command_id}})


def ack(peer, accepted, command_id="immutable-delete"):
    for _ in range(10):
        frame = receive(peer)
        require(frame is not None, "missing deletion ACK")
        if frame["type"] == "command_ack" and frame["command_id"] == command_id:
            require(frame["accepted"] == accepted, f"wrong deletion outcome: {frame}")
            return frame
    raise RuntimeError("ACK response budget exceeded")


def cold(release, root, deleted):
    with Process(release, root) as process:
        process.ready()
        peer, reply = connect(process, "worker")
        with peer:
            require(reply["type"] == ("reject" if deleted else "welcome"), f"wrong cold fence: {reply}")
            if deleted:
                require("deleted" in reply["reason"], "wrong worker rejection")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old-fixture", required=True)
    parser.add_argument("--new-fixture", required=True)
    parser.add_argument("--reader-release", required=True)
    parser.add_argument("--receipt", required=True)
    args = parser.parse_args()
    old = artifact(args.old_fixture, True)
    new = artifact(args.new_fixture, True)
    reader = artifact(args.reader_release, False)
    require(old["source"]["revision"] != new["source"]["revision"]
            and old["sha256"] != new["sha256"], "independent fixture revisions/ELFs required")
    observations = []
    for release in [old, new]:
        with tempfile.TemporaryDirectory(prefix="cowboy-deletion-writer-", dir="/tmp") as temporary:
            root = Path(temporary)
            record = root / "deletions/deletions.json"
            # One writer's ACK must survive old/new/old independently built readers.
            with Process(release, root, "writer") as process:
                process.ready()
                peer, welcome = connect(process, "core")
                with peer:
                    require(welcome["type"] == "welcome", "core admission failed")
                    stop(peer)
                    ack(peer, True)
                    captured = record.read_bytes()
                    inode = record.stat().st_ino
                    stop(peer, "duplicate-delete")
                    ack(peer, True, "duplicate-delete")
                    require(record.read_bytes() == captured and record.stat().st_ino == inode,
                            "duplicate deletion rewrote durable evidence")
                require(json.loads(captured)["deleted"] == ["sess-1"], "wrong committed set")
                contender = new if release is old else old
                with Process(contender, root, "writer") as rejected:
                    rejected.refused("already owned")
                require(record.read_bytes() == captured, "competing writer changed evidence")
            for reopener in [old, new, old]:
                cold(reopener, root, True)
                require(record.read_bytes() == captured, "reader rewrote committed evidence")
            observations.append({"revision": release["source"]["revision"], "case": "ack-dedup-old-new-old",
                                 "recordSha256": hashlib.sha256(captured).hexdigest()})
        for checkpoint, published in [("Staged", False), ("FileSynced", False),
                                      ("Renamed", True), ("DirectorySynced", True)]:
            with tempfile.TemporaryDirectory(prefix="cowboy-deletion-crash-", dir="/tmp") as temporary:
                root = Path(temporary)
                with Process(release, root, "writer", checkpoint) as process:
                    process.ready()
                    peer, welcome = connect(process, "core")
                    with peer:
                        require(welcome["type"] == "welcome", "core admission failed")
                        stop(peer)
                        process.marker(f"COWBOY_DELETION_WRITE_CHECKPOINT {checkpoint}")
                        process.kill()
                        require(receive(peer) is None, "positive ACK escaped interrupted commit")
                namespace = root / "deletions"
                require((namespace / "deletions.json").exists() == published, "wrong crash publication")
                pending = sorted(path.name for path in namespace.glob(".pending-*"))
                require(len(pending) == int(not published), "wrong staging retention")
                cold(new if release is old else old, root, published)
                require(sorted(path.name for path in namespace.glob(".pending-*")) == pending,
                        "reader replayed or removed uncommitted staging")
                observations.append({"revision": release["source"]["revision"], "case": checkpoint,
                                     "publishedBeforeKill": published, "positiveAck": False})
        with tempfile.TemporaryDirectory(prefix="cowboy-deletion-lock-", dir="/tmp") as temporary:
            root = Path(temporary)
            with Process(release, root, "writer") as process:
                process.ready()
                namespace = root / "deletions"
                (namespace / ".lock").rename(namespace / "retained-lock")
                replacement = b"replacement lock evidence"
                (namespace / ".lock").write_bytes(replacement)
                peer, welcome = connect(process, "core")
                with peer:
                    require(welcome["type"] == "welcome", "core admission failed")
                    stop(peer)
                    outcome = ack(peer, False)
                    require("lock was replaced" in outcome["reason"], "wrong lock replacement refusal")
                require((namespace / ".lock").read_bytes() == replacement
                        and sorted(path.name for path in namespace.iterdir()) == [".lock", "retained-lock"],
                        "lock replacement refusal mutated namespace evidence")
            observations.append({"revision": release["source"]["revision"], "case": "lock-replacement",
                                 "negativeAck": True, "namespaceUnchanged": True})
        with tempfile.TemporaryDirectory(prefix="cowboy-deletion-failure-", dir="/tmp") as temporary:
            root = Path(temporary)
            with Process(release, root, "writer") as process:
                process.ready()
                worker, welcome = connect(process, "worker")
                with worker:
                    require(welcome["type"] == "welcome", "initial worker admission failed")
                    peer, welcome = connect(process, "core")
                    with peer:
                        require(welcome["type"] == "welcome", "core admission failed")
                        replay = receive(worker)
                        require(replay["type"] == "replay" and replay["session_id"] == "sess-1",
                                "missing initial Controller reconnect replay")
                        (root / "deletions/deletions.json").mkdir()
                        stop(peer)
                        outcome = ack(peer, False)
                        require("durable Session deletion was not confirmed" in outcome["reason"],
                                "wrong failed-commit outcome")
                    worker.settimeout(0.2)
                    try:
                        worker.recv(1)
                        raise RuntimeError("failed storage changed surviving worker channel")
                    except socket.timeout:
                        pass
                    rejected, outcome = connect(process, "worker")
                    with rejected:
                        require(outcome["type"] == "reject"
                                and "fenced after a storage failure" in outcome["reason"],
                                f"wrong poisoned-owner reconnect outcome: {outcome}")
            for reopener in [old, new]:
                with Process(reopener, root) as process:
                    process.refused("not a regular file")
            observations.append({"revision": release["source"]["revision"], "case": "storage-failure",
                                 "negativeAck": True, "survivingWorkerUntouched": True,
                                 "furtherAdmissionFenced": True})
    # The new fixture also emits the production reader's exact owner/namespace.
    with tempfile.TemporaryDirectory(prefix="cowboy-deletion-reader-bridge-", dir="/tmp") as temporary:
        root = Path(temporary)
        with Process(new, root, "release-writer") as process:
            process.ready()
            peer, welcome = connect(process, "core")
            with peer:
                require(welcome["type"] == "welcome", "bridge core admission failed")
                stop(peer)
                ack(peer, True)
        record = root / "session-deletions/deletions.json"
        captured = record.read_bytes()
        for _ in range(2):
            with Process(reader, root, fixture=False) as process:
                process.ready(fixture=False)
                require("writer_enabled=false" in process.output(), "production writer enabled")
                peer, outcome = connect(process, "worker")
                with peer:
                    require(outcome["type"] == "reject" and "deleted" in outcome["reason"],
                            "real release ignored writer-produced terminal ID")
            require(record.read_bytes() == captured, "real reader changed writer evidence")
        observations.append({"case": "production-reader-bridge", "recordSha256": hashlib.sha256(captured).hexdigest(),
                             "readerRevision": reader["source"]["revision"], "reopenAfterSigkill": True})
    receipt = {"schema": 1, "accepted": True, "oldFixture": old, "newFixture": new,
               "productionReader": reader, "observations": observations,
               "productionWriterEnabled": False, "powerLossAcceptance": False,
               "scope": "independently built private writer fixtures; not production writer release admission"}
    Path(args.receipt).write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"accepted": True, "cases": len(observations), "receipt": args.receipt}))


if __name__ == "__main__":
    main()
