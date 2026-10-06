#!/usr/bin/env python3
"""Exercise native Claude tool dispatch against an offline scripted API.

The API supplies fixed tool-use blocks, not model inference. A fixture MCP
server edits a separate temporary target. No production credentials, installed
settings, enrolled Machines or real model requests participate in this gate.
"""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading
import time
import uuid

from execution_environment_probe import ProbeFailure, require
from plugin_runtime_conformance import closed_environment, stop_group


MAX_FRAME = 8 * 1024 * 1024
LOCAL_GUIDANCE = "RUNTIME_PROJECT_GUIDANCE_MUST_NOT_REACH_MODEL"
TARGET_GUIDANCE = "TARGET_PROJECT_GUIDANCE_MUST_REACH_MODEL"
ALIASES = {name: f"mcp__workspace__{name.lower()}" for name in (
    "Bash", "Read", "Edit", "Write", "Glob", "Grep", "NotebookEdit", "TaskOutput", "TaskStop",
)}
DISALLOWED = [*ALIASES, "Agent", "Task", "Skill", "EnterWorktree", "ExitWorktree"]


class ScriptedApi(ThreadingHTTPServer):
    """Serve actual native request/response framing with deterministic tool uses."""

    daemon_threads = True

    def __init__(self, steps, *, native_titles=False):
        self.steps = steps
        self.requests = []
        self.token_requests = []
        self.title_requests = []
        self.native_titles = native_titles
        self.failure = None
        super().__init__(("127.0.0.1", 0), ApiHandler)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class ApiHandler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
            require(0 < length <= MAX_FRAME, "API request exceeds fixture limit")
            request = json.loads(self.rfile.read(length))
            if self.path.split("?")[0] == "/v1/messages/count_tokens":
                self.server.token_requests.append(request)
                encoded = json.dumps({"input_tokens": 1000}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
                return
            require(self.path.split("?")[0] == "/v1/messages", f"unexpected API endpoint {self.path.split('?')[0][:256]}")
            index = len(self.server.requests)
            is_title = self.server.native_titles and not request.get("tools") and any(
                block.get("type") == "text" and block.get("text", "").startswith("You are naming a coding session so the user can pick it out")
                for block in request.get("system", []) if isinstance(block, dict))
            if is_title:
                self.server.title_requests.append(request)
                content = [{"type": "text", "text": '{"title":"Fixture session"}'}]
            else:
                self.server.requests.append(request)
                require(index <= len(self.server.steps), "unexpected extra native API request")
                content = self.server.steps[index] if index < len(self.server.steps) else [
                    {"type": "text", "text": "fixture complete"},
                ]
            if callable(content):
                content = content(self.server.requests)
            reason = "tool_use" if any(block["type"] == "tool_use" for block in content) else "end_turn"
            message = {
                "id": f"msg_fixture_{index}", "type": "message", "role": "assistant",
                "model": request["model"], "content": content, "stop_reason": reason,
                "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 10},
            }
            if not request.get("stream"):
                encoded = json.dumps(message).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
                return
            events = [("message_start", {"message": dict(message, content=[], stop_reason=None)})]
            for block_index, block in enumerate(content):
                start = dict(block)
                start["input" if block["type"] == "tool_use" else "text"] = {} if block["type"] == "tool_use" else ""
                events.append(("content_block_start", {"index": block_index, "content_block": start}))
                delta = ({"type": "input_json_delta", "partial_json": json.dumps(block["input"])}
                         if block["type"] == "tool_use" else {"type": "text_delta", "text": block["text"]})
                events.append(("content_block_delta", {"index": block_index, "delta": delta}))
                events.append(("content_block_stop", {"index": block_index}))
            events.extend([
                ("message_delta", {"delta": {"stop_reason": reason, "stop_sequence": None},
                                   "usage": {"output_tokens": 10}}),
                ("message_stop", {}),
            ])
            encoded = "".join(f"event: {name}\ndata: {json.dumps(dict(body, type=name))}\n\n"
                              for name, body in events).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
        except Exception as error:
            self.server.failure = str(error)
            self.send_error(500)


class WorkspaceFixture:
    """Small fixture tool host; it is not a production execution implementation."""

    def __init__(self, target):
        self.target = target
        self.calls = []
        self.outputs = []

    def message(self, message):
        method = message.get("method")
        if method == "initialize":
            result = {"protocolVersion": message["params"]["protocolVersion"],
                      "capabilities": {"tools": {}},
                      "serverInfo": {"name": "cowboy-execution-fixture", "version": "1"}}
        elif method == "tools/list":
            result = {"tools": [{"name": name.lower(), "description": f"Fixture {name} on the target",
                                 "inputSchema": {"type": "object", "additionalProperties": True}}
                                for name in ALIASES]}
        elif method == "tools/call":
            params = message["params"]
            self.calls.append(params)
            name, arguments = params["name"], params["arguments"]
            if name == "bash":
                process = subprocess.run(["/run/current-system/sw/bin/bash", "-c", arguments["command"]],
                                         cwd=self.target, env={"PATH": "/run/current-system/sw/bin"},
                                         capture_output=True, timeout=5, check=False)
                require(process.returncode == 0, "fixture target command failed")
                output = process.stdout.decode()
            else:
                path = self.target / arguments["file_path"]
                require(path.parent == self.target, "fixture path is outside target")
                if name == "write":
                    path.write_text(arguments["content"])
                    output = "File written."
                elif name == "edit":
                    content = path.read_bytes().decode()
                    require(content.count(arguments["old_string"]) == 1, "edit is not unique")
                    path.write_text(content.replace(arguments["old_string"], arguments["new_string"]))
                    output = "File edited."
                elif name == "read":
                    output = path.read_bytes().decode()
                else:
                    raise ProbeFailure("fixture received an unexpected tool")
            self.outputs.append(output)
            result = {"content": [{"type": "text", "text": output}], "isError": False}
        elif method.startswith("notifications/"):
            return None
        else:
            raise ProbeFailure("unexpected fixture MCP request")
        return {"jsonrpc": "2.0", "id": message["id"], "result": result}


class Claude:
    def __init__(self, binary, environment, runtime, fixture, *, aliases=True, resume=None,
                 custom_system_prompt=False, extra_arguments=(), model="claude-sonnet-4-6", bound_native=False,
                 disallowed=None):
        self.fixture = fixture
        self.frames = queue.Queue(maxsize=1000)
        self.messages = []
        self.stderr = tempfile.TemporaryFile()
        arguments = [binary, "--print", "--input-format", "stream-json", "--output-format", "stream-json",
                     "--verbose", "--permission-mode", "bypassPermissions", "--tools", "",
                     "--disallowedTools", disallowed if disallowed is not None
                     else "Agent,Task,Skill" if bound_native else ",".join(DISALLOWED), "--setting-sources", "",
                     "--strict-mcp-config", "--model", model, *extra_arguments]
        if resume:
            arguments.extend(["--resume", resume])
        self.process = subprocess.Popen(arguments, cwd=runtime, env=environment, start_new_session=True,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr)
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        context = f"Execution directory: {fixture.target}\n{TARGET_GUIDANCE}"
        self.send({"type": "control_request", "request_id": "initialize-fixture", "request": {
            "subtype": "initialize", "sdkMcpServers": ["workspace"],
            "toolAliases": ALIASES if aliases else {},
            **({"systemPrompt": [context]} if custom_system_prompt
               else {"appendSystemPrompt": context}),
        }})

    def read(self):
        try:
            while True:
                line = self.process.stdout.readline(MAX_FRAME + 1)
                if not line:
                    self.frames.put(None, timeout=5)
                    return
                require(len(line) <= MAX_FRAME, "native frame exceeds fixture limit")
                self.frames.put(json.loads(line), timeout=5)
        except Exception as error:
            self.frames.put(error, timeout=5)

    def send(self, message):
        self.process.stdin.write(json.dumps(message).encode() + b"\n")
        self.process.stdin.flush()

    def until(self, predicate, timeout=40):
        deadline = time.monotonic() + timeout
        while True:
            try:
                frame = self.frames.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty as error:
                raise ProbeFailure("native Claude response timed out") from error
            if isinstance(frame, Exception):
                raise frame
            require(frame is not None, f"native Claude exited with status {self.process.poll()}")
            self.messages.append(frame)
            if frame.get("type") == "control_request":
                request = frame["request"]
                require(request["subtype"] == "mcp_message", "unexpected native control request")
                require(request["server_name"] == "workspace", "unknown native MCP server")
                response = self.fixture.message(request["message"])
                self.send({"type": "control_response", "response": {
                    "subtype": "success", "request_id": frame["request_id"],
                    "response": {"mcp_response": response} if response is not None else {},
                }})
            if predicate(frame):
                return frame
            require(time.monotonic() < deadline, "native Claude response timed out")

    def ready(self):
        frame = self.until(lambda frame: frame.get("type") == "control_response"
                           and frame["response"].get("request_id") == "initialize-fixture")
        require(frame["response"]["subtype"] == "success", "native initialize rejected")

    def prompt(self, *, client_composed=False, text="Run the offline tool fixture.", timeout=40):
        self.send({"type": "user", "message": {"role": "user", "content": text},
                   **({"client_composed": True} if client_composed else {}),
                   "parent_tool_use_id": None, "session_id": ""})
        result = self.until(lambda frame: frame.get("type") == "result", timeout=timeout)
        require(not result.get("is_error"), "native fixture turn failed")
        return result

    def close(self):
        stop_group(self.process)
        self.reader.join(timeout=5)
        self.process.stdin.close()
        self.process.stdout.close()
        self.stderr.close()


def tool(name, arguments):
    return [{"type": "tool_use", "id": f"toolu_{uuid.uuid4().hex}", "name": name, "input": arguments}]


def context_probes(binary):
    """Check proposed public context controls over a complete tool round trip.

    A custom system prompt, documented reminder suppression and client-composed
    prompts have distinct semantics. None is assumed to replace native context.
    Observing only initialize/stream output would miss the actual API reminder.
    """
    suppressed = {
        "CLAUDE_CODE_DISABLE_ATTACHMENTS": "1",
        "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS": "1",
        "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
    }
    variants = [
        ("custom_system_prompt", True, False, {}),
        ("documented_context_suppression", False, False, suppressed),
        ("client_composed_prompt", False, True, {}),
        ("composed_prompt_and_context_suppression", False, True, suppressed),
    ]
    checks, requests = [], 0
    for label, custom, composed, flags in variants:
        with tempfile.TemporaryDirectory(prefix="cowboy-claude-context-") as temporary:
            root = Path(temporary)
            runtime, target = root / "runtime", root / "target"
            runtime.mkdir()
            target.mkdir()
            (runtime / "CLAUDE.md").write_text(LOCAL_GUIDANCE)
            (target / "fixture.txt").write_text("target context fixture")
            fixture = WorkspaceFixture(target)
            api = ScriptedApi([tool("Read", {"file_path": "fixture.txt"})])
            environment = closed_environment(root / "home")
            environment.update({
                "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
                "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
                "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1", **flags,
            })
            client = Claude(binary, environment, runtime, fixture, custom_system_prompt=custom)
            try:
                client.ready()
                client.prompt(client_composed=composed)
                require(api.failure is None and len(api.requests) == 2,
                        f"{label}: unexpected scripted API requests")
                require(len(fixture.calls) == 1 and fixture.outputs == ["target context fixture"],
                        f"{label}: target dispatch changed")
                encoded = json.dumps(api.requests)
                require(TARGET_GUIDANCE in encoded and LOCAL_GUIDANCE not in encoded,
                        f"{label}: project guidance differs")
                require(str(runtime) in json.dumps(api.requests[0]["messages"]),
                        f"{label}: native context behavior changed; reassess the blocker")
                checks.append(f"runtime_context_persists_with_{label}")
                requests += len(api.requests)
            finally:
                client.close()
                api.close()
    return checks, requests


def probe(binary, version):
    checks = []
    requests = 0
    with tempfile.TemporaryDirectory(prefix="cowboy-claude-execution-") as temporary:
        root = Path(temporary)
        runtime, target = root / "runtime", root / "target"
        runtime.mkdir()
        target.mkdir()
        (runtime / "CLAUDE.md").write_text(LOCAL_GUIDANCE)
        (runtime / "fixture.txt").write_text("runtime must remain unchanged")
        environment = closed_environment(root / "home")
        version_output = subprocess.run([binary, "--version"], cwd=runtime, env=environment,
                                        capture_output=True, timeout=10, check=True).stdout.decode().strip()
        require(version_output == f"{version} (Claude Code)", "native Claude version differs")
        checks.append("exact_native_version")
        text = "quotes: '\" `$() ${HOME} \\\n中文 🐎\r\n"
        steps = [tool("Write", {"file_path": "fixture.txt", "content": text}),
                 tool("Edit", {"file_path": "fixture.txt", "old_string": "quotes:", "new_string": "edited:"}),
                 tool("Read", {"file_path": "fixture.txt"}),
                 tool("Bash", {"command": "pwd; cat fixture.txt"})]
        api = ScriptedApi(steps)
        fixture = WorkspaceFixture(target)
        environment.update({
            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
            "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
            "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
        })
        client = Claude(binary, environment, runtime, fixture)
        try:
            client.ready()
            result = client.prompt()
            require(api.failure is None, api.failure or "API fixture failed")
            require([call["name"] for call in fixture.calls] == ["write", "edit", "read", "bash"],
                    "native alias dispatch did not reach target tools")
            checks.append("native_model_emitted_aliases_dispatch_to_mcp")
            require((target / "fixture.txt").read_bytes() == text.replace("quotes:", "edited:").encode(),
                    "target edit bytes differ")
            require((runtime / "fixture.txt").read_text() == "runtime must remain unchanged",
                    "native tools changed runtime files")
            require(fixture.outputs[-1] == f"{target}\n" + text.replace("quotes:", "edited:"),
                    "shell cwd or file bytes do not match the target")
            checks.append("file_edits_and_commands_use_target_without_runtime_changes")
            encoded = json.dumps(api.requests, ensure_ascii=False)
            require(LOCAL_GUIDANCE not in encoded, "runtime project guidance reached the model")
            require(TARGET_GUIDANCE in encoded, "target guidance did not reach the model")
            checks.append("only_explicit_target_project_guidance_reaches_model")
            # This is a measured limitation, not successful context projection.
            # Native environment metadata is injected into a USER message even
            # with local project tools/settings disabled. Do not remove this
            # observation to turn tool routing into full environment acceptance.
            require(str(runtime) in json.dumps(api.requests[0]["messages"]),
                    "native runtime context behavior changed; reassess this blocker")
            checks.append("native_runtime_cwd_still_injected_into_model_context")
            advertised = {entry["name"] for entry in api.requests[0]["tools"]}
            require(not advertised.intersection(DISALLOWED), "native project tools remain advertised")
            require(set(ALIASES.values()).issubset(advertised), "target tools missing from native schema")
            checks.append("one_advertised_project_tool_surface_without_local_builtins")
            require(len(api.requests) == len(steps) + 1, "tool dispatch added native model round trips")
            checks.append("one_scripted_response_per_tool_no_connection_turn")
            requests += len(api.requests)
            session_id = result["session_id"]
        finally:
            client.close()
            api.close()
        # Without the aliases, even a model-emitted native name must not run
        # locally. This proves denial of model calls, not every internal harness
        # path (project hooks/checkpointing/nested agents need separate gates).
        api = ScriptedApi([
            tool("Bash", {"command": "printf local-fallback > must-not-exist"}),
            tool("Read", {"file_path": str(runtime / "fixture.txt")}),
        ])
        environment["ANTHROPIC_BASE_URL"] = f"http://127.0.0.1:{api.server_port}"
        calls_before = len(fixture.calls)
        client = Claude(binary, environment, runtime, fixture, aliases=False)
        try:
            client.ready()
            client.prompt()
            require(api.failure is None, api.failure or "API fixture failed")
            require(len(fixture.calls) == calls_before, "unaliasing still dispatched a tool")
            require(not (runtime / "must-not-exist").exists(), "disabled local Bash executed")
            require("runtime must remain unchanged" not in json.dumps(api.requests),
                    "disabled local Read returned runtime bytes")
            errors = [block for message in api.requests[-1]["messages"]
                      for block in message["content"] if isinstance(block, dict)
                      and block.get("type") == "tool_result" and block.get("is_error")]
            require(len(errors) == 2, "disabled local tools did not return errors")
            checks.append("disabled_native_model_calls_fail_without_local_fallback")
            requests += len(api.requests)
        finally:
            client.close()
            api.close()
        # A real native session record must resume after the native process exits.
        api = ScriptedApi([tool("Read", {"file_path": "fixture.txt"})])
        environment["ANTHROPIC_BASE_URL"] = f"http://127.0.0.1:{api.server_port}"
        client = Claude(binary, environment, runtime, fixture, resume=session_id)
        try:
            client.ready()
            resumed = client.prompt()
            require(resumed["session_id"] == session_id, "native resume changed session identity")
            require(len(fixture.calls) == 5 and fixture.calls[-1]["name"] == "read",
                    "resumed native session did not retain target dispatch")
            require(len(api.requests[0]["messages"]) > 1, "native resume lost conversation")
            checks.append("cold_native_resume_retains_conversation_and_target_dispatch")
            requests += len(api.requests)
        finally:
            client.close()
            api.close()
    return checks, requests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require(args.native_cli.is_absolute(), "native CLI must be absolute")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "receipt must be a new absolute path")
    require(os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net"),
            "run inside a fresh network namespace")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "only loopback may exist")
    with args.native_cli.open("rb") as executable:
        digest = hashlib.file_digest(executable, "sha256").hexdigest()
    require(digest == args.sha256, "native CLI digest differs")
    checks, requests = probe(str(args.native_cli), args.version)
    context_checks, context_requests = context_probes(str(args.native_cli))
    checks.extend(context_checks)
    requests += context_requests
    receipt = {
        "schema": "cowboy.claude-tool-dispatch-probe/v1", "accepted": True,
        "native_cli": {"version": args.version, "sha256": digest}, "checks": checks,
        "topology": "isolated_loopback_scripted_api_and_fixture_mcp",
        "scripted_api_requests": requests, "real_model_requests": 0,
        "production_credentials": False, "proves_cowboy_routing": False,
        "proves_subscription": False, "proves_token_savings": False,
        "remote_execution_ready": False,
        "blocking_findings": ["native_runtime_cwd_injected_into_model_visible_user_message"],
        "not_checked": ["native_nested_agents", "target_hooks_and_settings", "background_tasks",
                        "search_images_notebooks", "native_checkpoints", "production_provider_release"],
    }
    descriptor = os.open(args.receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"accepted": True, "checks": len(checks), "real_model_requests": 0,
                      "remote_execution_ready": False, "blocking_findings": len(receipt["blocking_findings"])}))


if __name__ == "__main__":
    main()
