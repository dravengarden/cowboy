#!/usr/bin/env python3
"""Disposable public session APIs, real enrolled Machines and a signed ACP fixture.

The fixture Agent makes no model requests. Native Codex turns are accepted by
the separate execution-worker gate. No existing Service or credential is read.
"""
import argparse
import base64
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid


def require(value, message):
    if not value:
        raise RuntimeError(message)


class WebSocket:
    """Small bounded fixture client; supports server ping and masked client frames."""
    def __init__(self, url, headers=None):
        url = urllib.parse.urlsplit(url)
        require(url.hostname == "127.0.0.1", "fixture must stay on loopback")
        self.socket = socket.create_connection((url.hostname, url.port), timeout=60)
        self.reader = self.socket.makefile("rb")
        key = base64.b64encode(os.urandom(16)).decode()
        fields = {"Host": url.netloc, "Connection": "Upgrade", "Upgrade": "websocket",
                  "Sec-WebSocket-Key": key, "Sec-WebSocket-Version": "13", **(headers or {})}
        target = (url.path or "/") + ("?" + url.query if url.query else "")
        request = f"GET {target} HTTP/1.1\r\n" + "".join(f"{k}: {v}\r\n" for k, v in fields.items()) + "\r\n"
        self.socket.sendall(request.encode())
        require(self.reader.readline().split()[1] == b"101", "WebSocket admission refused")
        reply = {}
        while True:
            line = self.reader.readline(8192)
            require(line, "WebSocket upgrade closed")
            if line == b"\r\n":
                break
            name, value = line.decode().split(":", 1)
            reply[name.lower()] = value.strip()
        expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        require(reply.get("sec-websocket-accept") == expected, "WebSocket handshake changed")
        self.sequence = 0

    def send_bytes(self, payload, opcode=1):
        size = len(payload)
        prefix = bytes([0x80 | opcode])
        prefix += bytes([0x80 | size]) if size < 126 else b"\xfe" + struct.pack(">H", size) if size < 65536 else b"\xff" + struct.pack(">Q", size)
        mask = os.urandom(4)
        self.socket.sendall(prefix + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def send(self, value):
        self.send_bytes(json.dumps(value).encode())

    def receive(self):
        parts = bytearray()
        while True:
            header = self.reader.read(2)
            require(len(header) == 2, "WebSocket closed")
            flags, length = header
            require(not length & 0x80, "masked server frame")
            length &= 127
            if length == 126:
                length = struct.unpack(">H", self.reader.read(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self.reader.read(8))[0]
            require(length + len(parts) <= 16 * 1024 * 1024, "oversized frame")
            payload = self.reader.read(length)
            require(len(payload) == length, "truncated frame")
            if flags & 15 == 9:
                self.send_bytes(payload, 10)
                continue
            require(flags & 15 in (0, 1), "unexpected WebSocket frame")
            parts.extend(payload)
            if flags & 0x80:
                return json.loads(parts)

    def rpc(self, method, params):
        self.sequence += 1
        self.send({"id": self.sequence, "method": method, "params": params})
        while True:
            reply = self.receive()
            if reply.get("id") == self.sequence:
                require("error" not in reply, f"fixture RPC refused: {method}")
                return reply["result"]

    def close(self):
        self.reader.close()
        self.socket.close()


def agent():
    for line in sys.stdin:
        request = json.loads(line)
        if "id" not in request:
            continue
        try:
            method = request["method"]
            if method == "initialize":
                result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}, "authMethods": []}
            elif method in ("session/new", "session/load", "session/resume"):
                result = {"sessionId": request["params"].get("sessionId", "execution-fixture-native")}
            elif method == "session/prompt":
                descriptor = json.loads(Path(os.environ["COWBOY_EXECUTION_DESCRIPTOR"]).read_text())
                client = WebSocket(descriptor["endpoint"], {"Authorization": "Bearer " + descriptor["bearer_token"]})
                try:
                    initialized = client.rpc("initialize", {})
                    cwd = Path(descriptor["binding"]["workspace"]["cwd"])
                    require(initialized["environmentInfo"]["cwd"] == cwd.as_uri(), "wrong native cwd")
                    client.rpc("fs/writeFile", {"path": (cwd / "route.txt").as_uri(), "dataBase64": base64.b64encode("target '\" $() 中文 🐎\n".encode()).decode()})
                    process = str(uuid.uuid4())
                    client.rpc("process/start", {"processId": process, "argv": ["/bin/sh", "-c", "printf '%s\n' once >> effects"],
                        "cwd": cwd.as_uri(), "env": {}, "tty": False, "pipeStdin": False, "arg0": None,
                        "envPolicy": {"inherit": "all", "ignoreDefaultExcludes": False, "exclude": [], "set": {}, "includeOnly": []}})
                    for _ in range(100):
                        result = client.rpc("process/read", {"processId": process, "waitMs": 100, "maxBytes": 1024})
                        if result.get("closed"):
                            break
                    require(result.get("closed") and result.get("exitCode") == 0, "target command failed")
                finally:
                    client.close()
                result = {"stopReason": "end_turn"}
            else:
                result = {}
            response = {"jsonrpc": "2.0", "id": request["id"], "result": result}
        except Exception as error:
            response = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": str(error)}}
        print(json.dumps(response), flush=True)


def wait(check, label, seconds=70):
    until = time.monotonic() + seconds
    last = None
    while time.monotonic() < until:
        try:
            result = check()
            if result:
                return result
        except (OSError, ValueError, RuntimeError) as error:
            last = error
        time.sleep(0.1)
    raise RuntimeError(f"{label} timed out ({last})")


def main():
    from plugin_runtime_conformance import closed_environment
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "isolated loopback required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    inputs = json.loads(args.input.read_text())
    identities = {name: {"path": str(Path(inputs[name]).resolve()),
        "sha256": hashlib.sha256(Path(inputs[name]).read_bytes()).hexdigest()}
        for name in ("controller", "machine", "worker", "keeper", "pack", "native_cli")}
    if inputs.get("recovery_controller"):
        path = Path(inputs["recovery_controller"])
        identities["recovery_controller"] = {"path": str(path.resolve()),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    checks = []
    children = []
    logs = []
    with tempfile.TemporaryDirectory(prefix="cw-execution-session-") as temporary:
        root = Path(temporary)
        env = closed_environment(root / "home")
        env.update({"RUST_LOG": "info", "COWBOY_PUBLIC_ORIGIN": "http://127.0.0.1"})

        def run(*command, **options):
            result = subprocess.run([str(v) for v in command], env=env, stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, **options)
            require(result.returncode == 0, "fixture command failed: " + result.stderr.decode()[-1200:])
            return result.stdout

        def write(path, value):
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            path.write_text(json.dumps(value))
            path.chmod(0o600)

        class Artifacts(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                data = (root / "fixture-agent").read_bytes()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, *_args):
                pass

        artifacts = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Artifacts)
        threading.Thread(target=artifacts.serve_forever, daemon=True).start()
        adapter = root / "fixture-agent"
        adapter.write_text(f"#!{sys.executable}\n" + Path(__file__).read_text().split("\n", 1)[1])
        adapter.chmod(0o700)
        adapter_digest = "sha256:" + hashlib.sha256(adapter.read_bytes()).hexdigest()
        native_digest = "sha256:" + hashlib.sha256(Path(inputs["native_cli"]).read_bytes()).hexdigest()
        require(native_digest == "sha256:" + inputs["sha256"], "executor input changed")
        source = json.loads(Path("plugins/codex/provider.json").read_text())
        source.update(id="execution-fixture", version="1.0.0", publisher="execution-fixture", host={"schema_version": 1}, configuration_presets=[])
        source["runtime"].update(entrypoint="fixture-agent", arguments=["--agent"], environment={}, remove_environment=[], remove_environment_prefixes=[],
            dependencies=[{"id": "fixture-agent", "version": "1.0.0", "source": "https://example.invalid/fixture", "integrity": "sha512-" + base64.b64encode(hashlib.sha512(adapter.read_bytes()).digest()).decode(), "private": True}],
            platforms=[{"os": "linux", "architecture": "x86_64", "payload_digest": "", "launch_command": "fixture-agent",
                "private_components": [{"kind": "provider_adapter", "slot": "fixture", "dependency": "fixture-agent", "command": "fixture-agent"}]}])
        source["runtime"]["behavior"].update(permission="portable_v1", configuration="portable_v1", default_preferences={})
        source["runtime"]["behavior"]["execution"]["executor_digests"] = [native_digest]
        source["authentication"].update(required=False, methods=[], credential_files=[], environment_projection={})
        manifest = json.loads(Path("plugins/codex/plugin.json").read_text())
        manifest.update(id="execution-fixture", version="1.0.0", publisher="execution-fixture")
        write(root / "plugin/provider.json", source)
        write(root / "plugin/plugin.json", manifest)
        catalog = root / "catalog"
        catalog.mkdir()
        package = catalog / "execution-fixture.cowboy-plugin"
        release = package.with_suffix(".release.json")
        run(inputs["pack"], "build", root / "plugin", package, "https://example.invalid/fixture.cowboy-plugin")
        write(root / "runtime.json", [{"os": "linux", "architecture": "x86_64", "components": [{"kind": "agent_adapter", "slot": "fixture", "dependency": "fixture-agent", "version": "1.0.0", "command": "fixture-agent",
            "artifact_url": f"http://127.0.0.1:{artifacts.server_port}/fixture-agent", "artifact_digest": adapter_digest,
            "artifact_format": "raw", "entrypoint": None, "probe": {"args": ["--version"], "timeout_ms": 5000}}]}])
        run(inputs["pack"], "bind-runtime", package, release, root / "runtime.json")
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", root / "publisher")
        run(inputs["pack"], "sign", package, release, root / "publisher")
        run(inputs["pack"], "verify", package, release, root / "publisher.pub")
        (catalog / "trusted-publishers").mkdir()
        shutil.copyfile(root / "publisher.pub", catalog / "trusted-publishers/execution-fixture.pub")
        signed = json.loads(release.read_text())
        require(signed["release_schema"] == 4, "execution fixture lacks reader-safe envelope")
        for name in ("runtime-source", "target-source"):
            directory = root / name
            directory.mkdir()
            (directory / "fixture.txt").write_text(name)
            run("git", "init", "--initial-branch=main", directory)
            run("git", "-C", directory, "add", ".")
            run("git", "-C", directory, "-c", "user.name=Fixture", "-c", "user.email=fixture@invalid", "commit", "-m", "fixture")
        write(root / "execution.json", {"schema": 1, "host_command": inputs["keeper"],
            "executor": {"command": inputs["native_cli"], "sha256": inputs["sha256"], "version": inputs["version"]}})
        write(root / "security.json", {"schema": "dravengarden.cowboy.core-security/v1", "passkeys": {"namespace_id": "passkey", "source": "fresh"}})
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
        listener.close()
        origin = f"http://127.0.0.1:{port}"
        public_origin = f"https://127.0.0.1:{port}" if inputs.get("browser_device") else origin
        env["COWBOY_PUBLIC_ORIGIN"] = public_origin
        device = None
        if inputs.get("browser_device"):
            from execution_fixture_device import FixtureDevice
            device = FixtureDevice(root, env, port, public_origin)
        cookies = {}

        def call(method, path, body=None, anonymous=False, device_proof=True):
            client = http.client.HTTPConnection("127.0.0.1", port, timeout=60)
            headers = {"Origin": public_origin, "Content-Type": "application/json"}
            if device:
                headers["X-Forwarded-Proto"] = "https"
                if not anonymous and device_proof:
                    headers["x-cowboy-browser-proof"] = device.proof(method, path)
            if not anonymous:
                headers["Cookie"] = "; ".join(f"{k}={v}" for k, v in cookies.items())
            client.request(method, path, None if body is None else json.dumps(body), headers)
            response = client.getresponse()
            data = response.read(4 * 1024 * 1024)
            for key, value in response.getheaders():
                if key.lower() == "set-cookie" and not anonymous:
                    name, value = value.split(";", 1)[0].split("=", 1)
                    cookies[name] = value
            status = response.status
            client.close()
            try:
                data = json.loads(data)
            except ValueError:
                data = data.decode()
            return status, data

        def start(name, command):
            log = (root / f"{name}-{len(logs)}.log").open("wb")
            logs.append(log)
            process = subprocess.Popen([str(v) for v in command], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
            children.append(process)
            return process

        def connected(machine):
            status, value = call("GET", f"/api/machines/{machine}/deployment-health")
            return status == 200 and isinstance(value, dict) and value.get("connected")

        controller_command = [inputs["controller"], "serve", "--bind", f"127.0.0.1:{port}", "--data-dir", root / "controller", "--database-url", f"sqlite://{root}/controller/store.sqlite3",
            "--workspace-root", root, "--web-root", root / "no-web", "--plugin-catalog-dir", catalog, "--core-security-config", root / "security.json", "--product-auth-enabled", "true", "--execution-runtime-machine", "runtime"]
        try:
            controller = start("controller", controller_command)
            wait(lambda: call("GET", "/healthz")[0] == 200, "Controller")
            setup = next((root / "controller").glob("*setup*token*"))
            require(call("POST", "/api/auth/setup", {"token": setup.read_text().strip()})[0] == 200, "fixture setup failed")
            require(call("POST", "/api/auth/register", {"account": "fixture-owner", "password": "Fixture-correct-Horse-2026!"})[0] == 200, "fixture registration failed")
            require(call("POST", "/api/auth/login", {"account": "fixture-owner", "password": "Fixture-correct-Horse-2026!"})[0] == 200, "fixture login failed")
            if device:
                require(call("GET", "/api/machines", device_proof=False)[0] == 401,
                        "cookie without device proof admitted")
                checks.append("current_browser_device_login_and_bound_api_credentials")
            service = (root / "controller/service-id").read_text().strip()
            machines = {}
            for machine in ("runtime", "target"):
                status, enrollment = call("POST", "/api/machines/enrollment", {"machine_id": machine, "display_name": machine})
                require(status == 200, "fixture enrollment refused")
                token = root / f"{machine}-token"
                token.write_text(enrollment["token"])
                token.chmod(0o600)
                state = root / machine
                command = [inputs["machine"], "--controller-url", origin, "--service-id", service, "--machine-id", machine, "--display-name", machine,
                    "--state-dir", state, "--socket", state / "broker", "--provider-usage-socket", state / "usage", "--workspace-config", state / "absent-config",
                    "--workspace", f"fixture={root}/{machine}-source", "--spawn-mode", "direct", "--worker-command", inputs["worker"], "--desired-generation", "execution-fixture",
                    "--plugin-operation-admission", "--execution-config", root / "execution.json"]
                machines[machine] = (start(machine, [*command, "--enrollment-token-file", token]), command)
                wait(lambda: connected(machine), f"{machine} enrollment")
            checks.append("real_product_setup_login_and_two_signed_machine_handshakes")
            install = {"operation_id": "execution-fixture-install", "version": "1.0.0", "digest": signed["artifact_digest"]}
            endpoint = "/api/machines/runtime/plugins/execution-fixture"
            require(call("POST", endpoint, install, anonymous=True)[0] == 401, "anonymous install admitted")
            status, result = call("POST", endpoint, install)
            require(status in (200, 201, 202, 204), f"fixture installation failed: {status} {result}")
            wait(lambda: "execution-fixture" in call("GET", "/api/execution-environments?runtime_machine_id=runtime&machine_id=target")[1].get("providers", []), "installed Provider readiness")
            checks.append("signed_fixture_installed_through_public_operator_lifecycle")
            if inputs.get("native_projects"):
                require(call("GET", "/api/project-policies", anonymous=True)[0] == 401, "anonymous policy read admitted")
                status, policies = call("GET", "/api/project-policies")
                require(status == 200, "project policies unavailable")
                policy = {"agent_mode": "remote", "hosts_projects": False, "remote_targets": ["target"]}
                update = {"expected_revision": policies["revision"], "policy": policy, "preferred": True}
                endpoint = "/api/machines/runtime/project-policy"
                require(call("PUT", endpoint, update, anonymous=True)[0] == 401, "anonymous policy mutation admitted")
                require(call("PUT", endpoint, update)[0] == 200, "remote-only policy refused")
                require(call("PUT", endpoint, update)[0] == 409, "stale policy overwrote current policy")
                require(call("POST", "/api/sessions", {"provider": "execution-fixture", "machine_id": "runtime", "cwd": "fixture", "origin": "web"})[0] == 409, "remote-only policy bypassed through local API")
                for machine in ("runtime", "target"):
                    endpoint = f"/api/machines/{machine}/projects"
                    status, registry = call("GET", endpoint)
                    require(status == 200 and len(registry["projects"]) == 1, "bootstrap project missing")
                    if machine == "runtime":
                        edit = {"action": "remove", "expected_revision": registry["revision"], "id": "fixture"}
                    else:
                        project = dict(registry["projects"][0], display_name="display/only/not/routing")
                        edit = {"action": "upsert", "expected_revision": registry["revision"], "project": project}
                    require(call("POST", endpoint, edit, anonymous=True)[0] == 401, "anonymous project mutation admitted")
                    require(call("POST", endpoint, edit)[0] == 200, "project edit refused")
                    require(call("POST", endpoint, edit)[0] == 409, "stale project mutation accepted")
                    require(call("GET", f"/api/machines/{machine}/deployment-health")[1].get("workspace_owner") == "cowboy", "host health lost the native project owner")
                status, placement = call("GET", "/api/project-placements?machine_id=target")
                require(status == 200 and {"runtime_machine_id": "runtime", "provider": "execution-fixture", "mode": "remote"} in placement["placements"], "native AI placement missing without runtime mirror roots")
                status, inventory = call("GET", "/api/machines")
                runtime = next(m for m in inventory if m["id"] == "runtime")
                require(status == 200 and runtime["connected"] and runtime["workspaces"] == [] and runtime["schedulable"], "AI Machine without projects was filtered out of browser scheduling")
                checks.append("runtime_without_projects_is_schedulable_in_browser_machine_inventory")
                checks.extend(["native_project_registry_cas_and_operator_authorization", "remote_only_policy_enforced_by_legacy_and_native_creation", "placement_needs_no_runtime_project_or_directory_mapping"])
            request = {"provider": "execution-fixture", "runtime_machine_id": "runtime", "machine_id": "target", "cwd": "fixture", "initial_prompt": "Run one fixture command"}
            require(call("POST", "/api/execution-sessions", request, anonymous=True)[0] == 401, "anonymous session admitted")
            require(call("POST", "/api/execution-sessions", dict(request, machine_id="runtime"))[0] == 409, "local fallback admitted")
            status, created = call("POST", "/api/execution-sessions", request)
            require(status == 201, f"bound session refused: {status} {created}")
            session = created["session_id"]

            def info():
                status, value = call("GET", f"/api/sessions/{session}/info")
                require(status == 200, "session missing")
                return value

            meta = wait(lambda: (value if (value := info()).get("execution_binding", {}).get("workspace") else None), "target preparation")
            binding = meta["execution_binding"]
            cwd = Path(binding["workspace"]["cwd"])
            effects = cwd / "effects"
            wait(lambda: effects.exists(), "initial remote command")
            wait(lambda: info()["status"] == "running", "initial turn completion")
            require((cwd / "route.txt").read_text() == "target '\" $() 中文 🐎\n", "target edit differs")
            require(not (Path(meta["cwd"]) / "route.txt").exists(), "runtime file was changed")
            require(not (root / "target-source/route.txt").exists(), "stable source was changed")
            checks.extend(["public_creation_persists_exact_binding_and_isolated_target_worktree", "worker_tools_cross_both_enrolled_machine_connections", "runtime_and_stable_source_remain_unchanged"])
            if inputs.get("native_projects"):
                endpoint = "/api/machines/target/projects"
                registry = call("GET", endpoint)[1]
                require(call("POST", endpoint, {"action": "remove", "expected_revision": registry["revision"], "id": "fixture"})[0] == 200, "project removal refused")
                require(cwd.is_dir() and (root / "target-source").is_dir(), "removing project deleted files")
                require(call("POST", "/api/execution-sessions", request)[0] == 409, "removed project admitted a new session")
                checks.append("project_removal_retires_new_admission_and_preserves_bound_session_and_files")
            first_effect = effects.read_bytes()
            controller.terminate()
            controller.wait(timeout=20)
            if inputs.get("recovery_controller"):
                recovery_command = [inputs["recovery_controller"], *controller_command[1:-2]]
                for index in range(2):
                    recovery = start(f"recovery-{index}", recovery_command)
                    wait(lambda: call("GET", "/healthz")[0] == 200, "recovery Controller")
                    # Recovery is a reader floor, not permission to run the new
                    # worker protocol. An older Controller may deliberately
                    # refuse the upgraded Machine while retaining this record.
                    wait(lambda: info().get("execution_binding") == binding,
                         "recovery binding retention")
                    require(effects.read_bytes() == first_effect, "recovery reader replayed tools")
                    require(not (Path(meta["cwd"]) / "route.txt").exists(), "recovery reader touched runtime")
                    recovery.terminate()
                    recovery.wait(timeout=20)
                checks.append("actual_recovery_controller_preserves_binding_across_two_cold_opens")
            controller = start("controller-restored", controller_command)
            wait(lambda: call("GET", "/healthz")[0] == 200, "Controller restart")
            wait(lambda: connected("runtime"), "runtime reconnect")
            require(info()["execution_binding"] == binding, "restart changed execution binding")
            require(effects.read_bytes() == first_effect, "restart replayed a command")
            require(call("POST", f"/api/sessions/{session}/prompt", {"text": "Run one more fixture command"})[0] == 202, "restored prompt refused")
            wait(lambda: effects.read_bytes() == first_effect * 2, "restored remote command")
            checks.append("controller_restart_retains_binding_and_resumes_original_environment_without_replay")
            target_machine, target_command = machines["target"]
            target_machine.terminate()
            target_machine.wait(timeout=20)
            wait(lambda: not connected("target"), "target disconnect")
            machines["target"] = (start("target-restored", target_command), target_command)
            wait(lambda: connected("target"), "target reconnect")
            if inputs.get("native_projects"):
                require(call("GET", "/api/machines/target/projects")[1]["projects"] == [], "bootstrap projects resurrected after restart")
                require(call("GET", "/api/project-policies")[1]["machines"]["runtime"]["agent_mode"] == "remote", "runtime policy lost after restart")
                checks.append("native_project_registry_and_policy_survive_machine_and_controller_restart")
            require(info()["execution_binding"] == binding, "target restart changed execution binding")
            require(effects.read_bytes() == first_effect * 2, "target restart replayed a command")
            require(call("POST", f"/api/sessions/{session}/prompt", {"text": "Run after target restart"})[0] == 202, "reattached prompt refused")
            wait(lambda: effects.read_bytes() == first_effect * 3, "reattached remote command")
            checks.append("target_machine_restart_reattaches_original_keeper_without_replay")
            if inputs.get("execution_recovery"):
                wait(lambda: info()["status"] == "running", "idle before explicit recovery")
                operator = [inputs["controller"], "operator", "--data-dir", root / "controller"]
                require(call("GET", "/v1/execution-sessions")[0] == 404,
                        "private recovery route leaked onto public TCP")
                run(*operator, "enable")
                planned = json.loads(run(*operator, "recover-execution", "--session", session))["data"]
                require(planned["supported"], "candidate Machine did not negotiate recovery")
                saved_request = root / "recovery-request.json"
                write(saved_request, planned["request"])
                native_id = info().get("agent_session_id")
                # Lose the Controller after it durably fences the old binding,
                # before the paused target can acknowledge replacement. Only
                # this disposable fixture's enrolled process is suspended.
                target_process = machines["target"][0]
                target_process.send_signal(signal.SIGSTOP)
                interrupted = start("interrupted-recovery", [*operator, "recover-execution",
                                    "--session", session, "--request", saved_request])
                try:
                    pending = planned["request"]["intent"]
                    wait(lambda: info()["execution_binding"] == pending, "durable recovery fence")
                    controller.kill()
                    controller.wait(timeout=20)
                finally:
                    target_process.send_signal(signal.SIGCONT)
                interrupted.wait(timeout=20)
                if inputs.get("recovery_controller"):
                    for index in range(2):
                        recovery = start(f"pending-reader-{index}", recovery_command)
                        wait(lambda: call("GET", "/healthz")[0] == 200, "pending reader")
                        require(info()["execution_binding"] == pending, "old reader changed recovery fence")
                        require(effects.read_bytes() == first_effect * 3, "old reader replayed interrupted effect")
                        recovery.terminate()
                        recovery.wait(timeout=20)
                controller = start("controller-recovery-resumed", controller_command)
                wait(lambda: call("GET", "/healthz")[0] == 200, "recovery Controller restart")
                wait(lambda: connected("runtime") and connected("target"), "recovery Machines reconnect")
                require(info()["execution_binding"] == pending, "restart lost recovery intent")
                checks.append("lost_controller_reply_and_older_cold_readers_preserve_recovery_fence")
                first = json.loads(run(*operator, "recover-execution", "--session", session,
                                       "--request", saved_request))["data"]
                recovered = first["execution_binding"]
                require(recovered["revision"] == binding["revision"] + 1 and
                        recovered["environment"]["incarnation"] != binding["environment"]["incarnation"] and
                        recovered["workspace"] == binding["workspace"] and recovered["runtime"] == binding["runtime"],
                        "recovery changed workspace or reused execution lifetime")
                wait(lambda: info()["status"] == "running", "native resume after recovery")
                require(info().get("agent_session_id") == native_id and effects.read_bytes() == first_effect * 3,
                        "recovery replaced native history or replayed an effect")
                target_process.send_signal(signal.SIGSTOP)
                try:
                    repeated_process = start("observing-recovery", [*operator, "recover-execution",
                                             "--session", session, "--request", saved_request])
                    repeated_log = Path(logs[-1].name)
                    time.sleep(0.2)
                    require(repeated_process.poll() is None, "recovery observation did not wait for target")
                    require(call("POST", f"/api/sessions/{session}/prompt", {"text": "Run while observing recovery"})[0] == 202,
                            "recovered prompt refused")
                    wait(lambda: info()["status"] == "busy", "tool during receipt observation")
                    time.sleep(0.5)
                    require(info()["status"] == "busy", "receipt observation fenced healthy tool calls")
                finally:
                    target_process.send_signal(signal.SIGCONT)
                require(repeated_process.wait(timeout=30) == 0, "saved recovery observation failed")
                repeated = json.loads(repeated_log.read_text())["data"]
                require(repeated["execution_binding"] == recovered, "same recovery operation replaced twice")
                wait(lambda: effects.read_bytes() == first_effect * 4, "recovered target tool")
                checks.extend(["private_host_recovery_preserves_native_history_and_worktree",
                               "same_recovery_receipt_never_replaces_twice_or_replays_commands",
                               "observing_recovery_receipt_does_not_fence_healthy_native_tools",
                               "recovered_environment_executes_native_target_tools"])
            status, dataset = call("GET", "/api/sync/dataset")
            require(status == 200, "browser dataset unavailable")
            query = urllib.parse.urlencode({"dataset": dataset["dataset_id"], "bootstrap": "lazy"})
            browser_headers = {"Origin": public_origin,
                "Cookie": "; ".join(f"{k}={v}" for k, v in cookies.items()), "Sec-WebSocket-Protocol": "cowboy-sync-v1"}
            if device:
                browser_headers.update({"X-Forwarded-Proto": "https",
                    "x-cowboy-browser-proof": device.proof("GET", f"/ws?{query}")})
            browser = WebSocket(f"ws://127.0.0.1:{port}/ws?{query}", browser_headers)
            browser.send({"type": "delete_session", "session_id": session})
            wait(lambda: call("GET", f"/api/sessions/{session}/info")[0] == 404, "confirmed environment deletion")
            browser.close()
            require((cwd / "route.txt").is_file() and effects.read_bytes() == first_effect * (4 if inputs.get("execution_recovery") else 3), "deletion removed work or replayed tools")
            checks.append("public_deletion_waits_for_target_stop_and_preserves_work")
        except Exception:
            for log in logs:
                log.flush()
                lines = Path(log.name).read_text(errors="replace").splitlines()
                # Fixture setup tokens and cookies never appear in diagnostics.
                lines = [line for line in lines if "token" not in line.lower() and "cookie" not in line.lower()]
                print(Path(log.name).name + "\n" + "\n".join(lines[-12:]), file=sys.stderr)
            raise
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
            for child in reversed(children):
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
            for log in logs:
                log.close()
            artifacts.shutdown()
    receipt = {"schema": "cowboy.execution-session-conformance/v1", "accepted": True, "checks": checks,
        "inputs": identities, "fixture_release": {name: signed[name] for name in
            ("release_schema", "plugin_id", "plugin_version", "artifact_digest")},
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "real_model_requests": 0, "production_credentials": False, "production_activation": False,
        "not_checked": ["native_codex_turns_covered_by_separate_gate", "cross_host_latency", "production_subscription_inference"]}
    if inputs.get("browser_device"):
        receipt["device_fixture_sha256"] = hashlib.sha256(
            Path(__file__).with_name("execution_fixture_device.py").read_bytes()).hexdigest()
        receipt["not_checked"].append("external_tls_termination_fixture_uses_trusted_loopback_proxy_boundary")
    with args.receipt.open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"accepted": True, "checks": len(checks)}))


if __name__ == "__main__":
    if sys.argv[1:] == ["--version"]:
        print("execution-fixture 1.0.0")
    elif sys.argv[1:] == ["--agent"]:
        agent()
    else:
        main()
