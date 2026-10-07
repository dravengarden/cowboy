#!/usr/bin/env python3
"""Delay a real native initialize reply past 10s; require bridge recovery.

Uses temporary homes, a loopback byte relay and no model or credentials. The
relay never fabricates native frames. Run inside the pinned development shell.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time

from execution_environment_probe import Executor, require


def relay(source, target, delay_handshake):
    try:
        if delay_handshake:
            headers = b""
            while b"\r\n\r\n" not in headers:
                chunk = source.recv(65536)
                if not chunk:
                    return
                headers += chunk
            headers, remainder = headers.split(b"\r\n\r\n", 1)
            target.sendall(headers + b"\r\n\r\n")
            time.sleep(12)
            if remainder:
                target.sendall(remainder)
        while chunk := source.recv(65536):
            target.sendall(chunk)
    except OSError:
        pass
    finally:
        source.close()
        target.close()


def run(binary, launcher):
    with tempfile.TemporaryDirectory(prefix="cowboy-handshake-recovery-") as temporary:
        root = Path(temporary)
        target, runtime = root / "target", root / "runtime"
        for path in [target, runtime, root / "home", root / "codex"]:
            path.mkdir()
        (target / "AGENTS.md").write_text("# Target\nRead only this target.\n")
        environment = {
            key: os.environ[key]
            for key in ["PATH", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE"]
            if key in os.environ
        }
        environment.update(HOME=str(root / "home"), CODEX_HOME=str(root / "codex"),
                           CODEX_EXEC_SERVER_URL="none")
        with socket.socket() as allocated:
            allocated.bind(("127.0.0.1", 0))
            port = allocated.getsockname()[1]
        executor = subprocess.Popen(
            [str(binary), "exec-server", "--listen", f"ws://127.0.0.1:{port}"],
            cwd=target, env=environment, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        connections = []
        count = [0]

        def accept():
            while True:
                try:
                    peer, _ = listener.accept()
                    upstream = socket.create_connection(("127.0.0.1", port))
                    connections.extend([peer, upstream])
                    count[0] += 1
                    threading.Thread(target=relay, args=(peer, upstream, False), daemon=True).start()
                    threading.Thread(target=relay, args=(upstream, peer, count[0] == 1), daemon=True).start()
                except OSError:
                    return

        client = None
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.05):
                        break
                except OSError:
                    require(executor.poll() is None, "native executor exited")
                    time.sleep(0.05)
            else:
                raise RuntimeError("executor readiness timed out")
            threading.Thread(target=accept, daemon=True).start()
            descriptor = root / "descriptor.json"
            descriptor.write_text(json.dumps({
                "schema": 1, "endpoint": f"ws://127.0.0.1:{listener.getsockname()[1]}/",
                "bearer_token": "a" * 64,
                "binding": {"schema": 1, "environment": {"protocol": 1, "id": "recovery-target"},
                            "workspace": {"cwd": str(target)}},
            }))
            descriptor.chmod(0o600)
            environment.update(COWBOY_EXECUTION_DESCRIPTOR=str(descriptor),
                               COWBOY_PRIVATE_CODEX_EXECUTABLE=str(binary),
                               COWBOY_PRIVATE_CODEX_ARGUMENTS="[]")
            client = Executor([shutil.which("node"), str(launcher), "--cowboy-private-cli", "app-server"],
                              40, environment=environment, cwd=runtime)
            started = time.monotonic()
            client.request("initialize", {"clientInfo": {"name": "handshake-recovery", "version": "1"},
                                          "capabilities": {"experimentalApi": True}})
            duration = time.monotonic() - started
            require(duration >= 10 and count[0] >= 2, "probe did not cross native handshake failure and recovery")
            client.send({"method": "initialized", "params": {}})
            thread = client.request("thread/start", {"cwd": str(runtime), "ephemeral": True,
                                                    "approvalPolicy": "never", "sandbox": "danger-full-access"})
            require(thread["instructionSources"] == [str(target / "AGENTS.md")],
                    "recovered thread did not load its target guidance")
            require(thread["cwd"] == str(runtime), "runtime placement changed")
            return {"schema": "cowboy.execution-handshake-recovery/v1", "accepted": True,
                    "native_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "bridge_sha256": hashlib.sha256(launcher.read_bytes()).hexdigest(),
                    "initialize_seconds": round(duration, 3), "connections": count[0],
                    "checks": ["real_10_second_initialize_failure", "native_info_recovers_same_environment",
                               "thread_starts_only_after_recovery", "target_guidance_loaded", "no_model_request"]}
        finally:
            if client:
                client.close()
            listener.close()
            for connection in connections:
                connection.close()
            executor.terminate()
            executor.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--launcher", type=Path, default=(
        Path(__file__).resolve().parents[1] /
        "components/provider-runtime/packages/codex-acp/launch.mjs"
    ))
    args = parser.parse_args()
    print(json.dumps(run(args.native_cli.resolve(), args.launcher.resolve()), indent=2))
