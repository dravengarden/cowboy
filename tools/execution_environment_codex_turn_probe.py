#!/usr/bin/env python3
"""Offline native Codex edit/command/resume acceptance with a scripted API.

Both native processes are exact pinned bytes with separate disposable homes.
The only model endpoint is a loopback fixture returning fixed tool calls.
"""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time

from execution_environment_native_probe import stop
from execution_environment_probe import Executor, ProbeFailure, require
from plugin_runtime_conformance import closed_environment


class Api(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, steps):
        self.steps = steps
        self.dispatcher = None
        self.extra_requests = 0
        self.dispatch_lock = threading.Lock()
        self.requests = []
        self.failure = None
        super().__init__(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
            require(0 < length < 8 * 1024 * 1024, "fixture API body exceeds limit")
            require(self.path == "/v1/responses", "unexpected fixture API endpoint")
            request = json.loads(self.rfile.read(length))
            with self.server.dispatch_lock:
                index = len(self.server.requests)
                self.server.requests.append(request)
                require(index < len(self.server.steps) + self.server.extra_requests, "unexpected extra native API request")
                response_id = f"fixture_response_{index}"
                step = self.server.dispatcher(self.server.requests) if self.server.dispatcher else self.server.steps[index]
                if callable(step):
                    step = step(self.server.requests)
            events = [
                {"type": "response.created", "response": {"id": response_id}},
                {"type": "response.output_item.done", "item": step},
                {"type": "response.completed", "response": {
                    "id": response_id,
                    "usage": {"input_tokens": 0, "input_tokens_details": None, "output_tokens": 0,
                              "output_tokens_details": None, "total_tokens": 0},
                }},
            ]
            encoded = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n"
                              for event in events).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
        except Exception as error:
            self.server.failure = str(error)
            self.send_error(500)


def final(identity):
    return {"type": "message", "role": "assistant", "id": identity,
            "content": [{"type": "output_text", "text": "offline fixture complete"}]}


def command(identity, script):
    return {"type": "function_call", "call_id": identity, "name": "exec_command",
            "arguments": json.dumps({"cmd": script, "yield_time_ms": 1000, "max_output_tokens": 1000})}


def app(binary, environment, runtime):
    client = Executor([binary, "app-server"], 40, environment=environment, cwd=runtime)
    client.request("initialize", {"clientInfo": {"name": "cowboy-native-turn-fixture", "version": "1"},
                                  "capabilities": {"experimentalApi": True}})
    client.send({"method": "initialized", "params": {}})
    return client


def add_environment(client, url):
    client.request("environment/add", {"environmentId": "cowboy-fixture-target", "execServerUrl": url,
                                       "connectTimeoutMs": 1000})


def turn(client, thread_id, selection):
    started = client.request("turn/start", {
        "threadId": thread_id, "input": [{"type": "text", "text": "Run the offline fixture.",
                                          "text_elements": []}],
        "environments": selection,
    })
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        frame = client.frame(deadline)
        require(not ("id" in frame and "method" in frame), "unexpected native approval request")
        if frame.get("method") == "turn/completed":
            require(frame["params"]["threadId"] == thread_id, "turn changed native thread")
            completed = frame["params"]["turn"]
            require(completed["id"] == started["turn"]["id"], "turn identity changed")
            require(completed["status"] == "completed", "native fixture turn failed")
            return
    raise ProbeFailure("native fixture turn timed out")


def probe(binary, version):
    checks = []
    with tempfile.TemporaryDirectory(prefix="cowboy-codex-turn-") as temporary:
        root = Path(temporary)
        runtime, target = root / "runtime", root / "target"
        runtime.mkdir()
        target.mkdir()
        (runtime / "AGENTS.md").write_text("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL")
        (target / "AGENTS.md").write_text("TARGET_GUIDANCE_MUST_REACH_MODEL")
        (runtime / "fixture.txt").write_text("runtime remains untouched\n")
        (target / "fixture.txt").write_text("target before\n")
        api = Api([
            {"type": "custom_tool_call", "call_id": "fixture_patch", "name": "apply_patch",
             "input": "*** Begin Patch\n*** Update File: fixture.txt\n@@\n-target before\n+target after '\" $() 中文 🐎\n*** End Patch"},
            command("fixture_command", "pwd; cat fixture.txt; printf once >> once.txt"),
            final("fixture_final_one"),
            command("fixture_resume", "pwd; cat fixture.txt; cat once.txt"),
            final("fixture_final_two"),
        ])
        environment = closed_environment(root / "agent-home")
        # Disable implicit local execution. The exact session selection is added
        # through the native API, without shared environments.toml mutation.
        environment["CODEX_EXEC_SERVER_URL"] = "none"
        environment["COWBOY_OFFLINE_FIXTURE_KEY"] = "not-a-production-credential"
        codex_home = Path(environment["CODEX_HOME"])
        codex_home.mkdir()
        (codex_home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "cowboy_fixture"\n'
            '[features]\nremote_control = false\nshell_snapshot = false\n'
            '[model_providers.cowboy_fixture]\nname = "Offline fixture"\n'
            f'base_url = "http://127.0.0.1:{api.server_port}/v1"\n'
            'wire_api = "responses"\nenv_key = "COWBOY_OFFLINE_FIXTURE_KEY"\n'
            'request_max_retries = 0\nstream_max_retries = 0\n'
        )
        executor_environment = closed_environment(root / "executor-home")
        observed = subprocess.run([binary, "--version"], cwd=runtime, env=environment, check=True,
                                  capture_output=True, timeout=10).stdout.decode().strip()
        require(observed == f"codex-cli {version}", "native CLI version differs")
        checks.append("exact_native_version")
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        url = f"ws://127.0.0.1:{port}"
        executor = subprocess.Popen([binary, "exec-server", "--listen", url], cwd=target,
                                    env=executor_environment, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        client = None
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.05):
                        break
                except OSError:
                    require(executor.poll() is None, "executor stopped before readiness")
                    time.sleep(0.05)
            else:
                raise ProbeFailure("executor readiness timed out")
            client = app(binary, environment, runtime)
            add_environment(client, url)
            selection = [{"environmentId": "cowboy-fixture-target", "cwd": str(target),
                          "runtimeWorkspaceRoots": [str(target)]}]
            started = client.request("thread/start", {
                "cwd": str(runtime), "approvalPolicy": "never", "sandbox": "danger-full-access",
                "environments": selection,
            })
            thread_id = started["thread"]["id"]
            turn(client, thread_id, selection)
            require(api.failure is None, api.failure or "API fixture failed")
            require((target / "fixture.txt").read_text() == "target after '\" $() 中文 🐎\n",
                    "native patch did not edit the target")
            require((target / "once.txt").read_text() == "once", "native command did not run once")
            require((runtime / "fixture.txt").read_text() == "runtime remains untouched\n"
                    and not (runtime / "once.txt").exists(), "native tools modified runtime files")
            checks.append("native_patch_and_command_dispatch_to_target")
            encoded = json.dumps(api.requests, ensure_ascii=False)
            require("TARGET_GUIDANCE_MUST_REACH_MODEL" in encoded, "target project guidance missing")
            require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in encoded, "runtime guidance leaked")
            checks.append("native_target_guidance_without_runtime_guidance")
            require(len(api.requests) == 3, "native dispatch added fixture model round trips")
            checks.append("one_scripted_response_per_tool_no_connection_turn")
            client.close()
            client = None
            client = app(binary, environment, runtime)
            add_environment(client, url)
            resumed = client.request("thread/resume", {
                "threadId": thread_id, "cwd": str(runtime), "excludeTurns": True,
            })
            require(resumed["thread"]["id"] == thread_id and resumed["thread"]["environments"] == [],
                    "native cold resume identity or empty selection differs")
            checks.append("cold_resume_requires_host_to_reassert_execution_selection")
            # thread/resume has no environments parameter in this version; the
            # selection is live thread state, not persisted conversation state.
            # The Provider must inject its core binding into EVERY turn/start.
            turn(client, thread_id, selection)
            require(api.failure is None and len(api.requests) == 5, "resumed fixture requests differ")
            require("fixture_patch" in json.dumps(api.requests[3]), "native resume lost conversation")
            require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests[3]),
                    "cold resume loaded runtime project guidance")
            selected = client.request("thread/read", {"threadId": thread_id, "includeTurns": False})
            require(selected["thread"]["environments"] == selection, "resumed turn did not select target")
            require((target / "once.txt").read_text() == "once", "native cold resume replayed a command")
            checks.append("cold_native_resume_retains_history_target_and_single_effect")
            client.close()
            client = None
            client = app(binary, environment, runtime)
            client.request("thread/resume", {"threadId": thread_id, "excludeTurns": True})
            client.request("turn/start", {
                "threadId": thread_id, "input": [{"type": "text", "text": "Must not execute."}],
                "environments": selection,
            }, expect_error=True)
            require(len(api.requests) == 5, "missing environment contacted model API")
            checks.append("missing_environment_on_resume_refused_without_local_fallback")
            require(not (codex_home / "environments.toml").exists(), "global environment config changed")
            checks.append("no_global_environment_configuration_written")
        finally:
            if client is not None:
                client.close()
            stop(executor)
            api.close()
    return checks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require(args.native_cli.is_absolute(), "native CLI must be absolute")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "receipt must be a new absolute path")
    require(os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net"), "use a new network namespace")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "only loopback may exist")
    with args.native_cli.open("rb") as executable:
        digest = hashlib.file_digest(executable, "sha256").hexdigest()
    require(digest == args.sha256, "native CLI digest differs")
    checks = probe(str(args.native_cli), args.version)
    receipt = {
        "schema": "cowboy.codex-execution-turn-probe/v1", "accepted": True,
        "native_cli": {"version": args.version, "sha256": digest}, "checks": checks,
        "topology": "isolated_loopback_scripted_api_and_native_executor",
        "scripted_api_requests": 5, "real_model_requests": 0, "production_credentials": False,
        "proves_cowboy_routing": False, "proves_subscription": False, "proves_token_savings": False,
        "not_checked": ["cross_host_transport", "target_keeper", "background_task_resume",
                        "production_provider_release", "native_nested_agents", "uploads_images"],
    }
    descriptor = os.open(args.receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"accepted": True, "checks": len(checks), "real_model_requests": 0}))


if __name__ == "__main__":
    main()
