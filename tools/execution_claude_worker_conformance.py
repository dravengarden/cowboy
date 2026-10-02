#!/usr/bin/env python3
"""Actual packaged Claude turns through Cowboy's worker endpoint and keeper.

The Rust execution-worker gate owns disposable Machines, target worktree,
transport outage and cleanup. This client supplies only loopback scripted model
responses. It never reads subscription state or sends a real inference request.
"""
import argparse
import base64
import hashlib
import json
import os
import random
from pathlib import Path
import shutil
import socket
import struct
import time
import zlib

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool as native_tool
from execution_environment_probe import Executor, ProbeFailure, require
from plugin_runtime_conformance import closed_environment


def tool(name, arguments):
    alias = {"Read": "ReadFile", "Write": "WriteFile", "Edit": "EditFile", "Glob": "GlobFiles",
             "Grep": "GrepFiles", "NotebookEdit": "EditNotebook"}.get(name, name)
    return native_tool(alias, arguments)


def outputs(request):
    for message in request.get("messages", []):
        content = message.get("content", [])
        if isinstance(content, list):
            for block in content:
                if block.get("type") == "tool_result":
                    yield block


def stop_background(requests):
    for block in reversed(list(outputs(requests[-1]))):
        for item in block.get("content", []) if isinstance(block.get("content"), list) else []:
            if item.get("type") == "text":
                try:
                    result = json.loads(item["text"])
                except ValueError:
                    continue
                if result.get("task_id"):
                    return tool("TaskStop", {"task_id": result["task_id"]})
    raise ProbeFailure("background tool did not return a handle")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["native-cli", "descriptor", "runtime", "target", "receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new receipt required")
    inputs = json.loads(Path(os.environ["COWBOY_TEST_EXECUTION_INPUT"]).read_text())
    binary = Path(inputs["claude_cli"])
    require(hashlib.sha256(binary.read_bytes()).hexdigest() == inputs["claude_sha256"], "Claude artifact changed")
    launcher = Path(inputs["adapter_launcher"])
    wrapper = launcher.parent.parent / "bin/cowboy-configured-cli"
    require(wrapper.is_file(), "packaged Claude execution launcher missing")
    checks = ["machine_owned_worktree", "machine_restart_reattaches_same_keeper_and_binding"]
    (args.runtime / "CLAUDE.md").write_text("RUNTIME_CLAUDE_GUIDANCE_MUST_NOT_REACH_MODEL")
    (args.target / "CLAUDE.md").write_text("TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL")
    def png_chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    pixel = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0)) +
             png_chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\0\0" * 2) * 2)) + png_chunk(b"IEND", b""))
    (args.target / "pixel.png").write_bytes(pixel)
    random_text = "".join(random.Random(7).choices("abcdefghijklmnopqrstuvwxyz0123456789", k=58000))
    (args.target / "large.txt").write_text(random_text)
    quoted = "quoted '\" $() 中文 🐎.txt"
    content = "target after '\" $() 中文 🐎\r\n"
    def conflict(_requests):
        (args.target / quoted).write_text("external change\n")
        return tool("Edit", {"file_path": quoted, "old_string": "hello", "new_string": "lost"})
    api = ScriptedApi([
        tool("Read", {"file_path": "fixture.txt"}),
        tool("Edit", {"file_path": "fixture.txt", "old_string": "target before\n", "new_string": content}),
        tool("Bash", {"command": "pwd; cat fixture.txt; printf once >> once.txt"}),
        tool("Write", {"file_path": quoted, "content": "hello\r\n"}),
        tool("Read", {"file_path": quoted}), conflict,
        tool("Read", {"file_path": "pixel.png"}),
        tool("Read", {"file_path": "large.txt"}),
        tool("Glob", {"pattern": "*.txt"}),
        tool("Grep", {"pattern": "target after", "output_mode": "content"}),
        tool("Bash", {"command": "printf background_started >> jobs.txt; while :; do sleep 1; printf tick >> jobs.txt; done", "run_in_background": True}),
        stop_background,
    ])
    environment = closed_environment(args.runtime.parent / "claude-home")
    environment.update({
        "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
        "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
        "COWBOY_PRIVATE_CLAUDE_EXECUTABLE": str(binary),
        "CLAUDE_CODE_EXECUTABLE": str(binary),
        "COWBOY_EXECUTION_DESCRIPTOR": str(args.descriptor),
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
    })
    fixture = WorkspaceFixture(args.target)
    def native(resume=None):
        return Claude(str(wrapper), environment, args.runtime, fixture, resume=resume)
    def context_checked(requests):
        for request in api.token_requests:
            require(str(args.runtime) not in json.dumps(request) and
                    str(args.runtime.parent / "claude-home") not in json.dumps(request),
                    "token counting leaked runtime context")
        for request in requests:
            encoded = json.dumps(request)
            exposed = next((path for path in [str(args.runtime), str(args.runtime.parent / "claude-home")]
                            if path in encoded), None)
            if exposed:
                position = encoded.index(exposed)
                print("runtime context diagnostic:", encoded[max(0, position-180):position+500])
            require(str(args.runtime) not in encoded and str(args.runtime.parent / "claude-home") not in encoded and
                    "RUNTIME_CLAUDE_GUIDANCE" not in encoded and
                    "RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in encoded, "runtime context reached model")
            require(str(args.target) in encoded and "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL" in encoded and
                    "TARGET_GUIDANCE_MUST_REACH_MODEL" in encoded, "target guidance missing")
        require(not fixture.calls, "test client unexpectedly supplied target tools")
        require(api.failure is None, api.failure or "scripted API failed")
    client = None
    try:
        client = native()
        client.ready()
        require(not api.requests, "readiness called the model")
        client.send({"type": "control_request", "request_id": "forbidden-mcp", "request": {
            "subtype": "mcp_set_servers", "servers": {"local": {"type": "stdio", "command": "false"}},
        }})
        denied = client.until(lambda frame: frame.get("type") == "control_response" and
                              frame["response"].get("request_id") == "forbidden-mcp")
        require(denied["response"]["subtype"] == "error", "execution override was accepted")
        checks.append("native_control_cannot_replace_bound_execution")
        result = client.prompt(timeout=150)
        session = result["session_id"]
        context_checked(api.requests)
        require((args.target / "fixture.txt").read_bytes() == content.encode(), "target edit bytes changed")
        require((args.target / "once.txt").read_text() == "once", "command did not execute exactly once")
        require((args.target / quoted).read_text() == "external change\n", "stale edit overwrote external change")
        errors = [block for block in outputs(api.requests[-1]) if block.get("is_error")]
        require(len(errors) == 1, f"expected only the edit conflict; received {len(errors)} tool errors")
        require((args.runtime / "fixture.txt").read_text() == "runtime remains untouched\n" and
                not (args.runtime / "once.txt").exists(), "runtime filesystem was modified")
        jobs = (args.target / "jobs.txt").read_text()
        require(jobs.count("background_started") == 1, "background start replayed")
        stopped = list(outputs(api.requests[-1]))[-1]
        state = json.loads(stopped["content"][0]["text"])
        require(state["closed"] and state["exited"], "background cancellation did not settle")
        time.sleep(1.2)
        require((args.target / "jobs.txt").read_text() == jobs, "background descendants survived cancellation")
        images = [item for block in outputs(api.requests[-1]) if isinstance(block.get("content"), list) for item in block["content"]
                  if item.get("type") == "image"]
        require(images and base64.b64decode(images[0]["source"]["data"]) == pixel, "target image bytes changed")
        require(any(random_text in json.dumps(request) for request in api.requests),
                "large tool result spilled or was truncated before reaching the model")
        checks.extend(["module_readiness_uses_no_model_request", "native_aliases_use_target_files_and_processes",
                       "runtime_context_and_guidance_are_replaced", "unicode_quotes_crlf_bytes_preserved",
                       "stale_edit_is_refused", "target_image_enters_model_context", "background_cancel_settles",
                       "bounded_large_read_stays_in_target_tool_result",
                       "lost_start_receipt_and_transport_outage_do_not_replay"])
        api.steps.extend([[], tool("Bash", {"command": "printf retained_started >> retained.txt; while :; do sleep 1; printf tick >> retained.txt; done", "run_in_background": True})])
        client.prompt(timeout=90)
        client.close(); client = None
        api.steps.extend([[], stop_background, tool("Edit", {"file_path": "fixture.txt", "old_string": "target after", "new_string": "target resumed"}),
                          tool("Bash", {"command": "cat fixture.txt; cat once.txt"})])
        previous = len(api.requests)
        client = native(session)
        client.ready()
        require(len(api.requests) == previous, "resume readiness called model")
        client.prompt(timeout=90)
        context_checked(api.requests)
        require((args.target / "fixture.txt").read_bytes() == content.replace("target after", "target resumed").encode(), "cold resume lost read stamps")
        require((args.target / "once.txt").read_text() == "once", "cold resume replayed a command")
        retained = (args.target / "retained.txt").read_text()
        time.sleep(1.2)
        require((args.target / "retained.txt").read_text() == retained, "resumed process handle did not stop target job")
        checks.extend(["cold_native_resume_preserves_context_reads_and_effects", "background_process_handle_survives_native_resume"])
        api.steps.extend([[], [{"type": "text", "text": "<summary>Continue the target fixture. The target project instructions remain authoritative.</summary>"}],
                          tool("Read", {"file_path": "fixture.txt"})])
        client.prompt(text="/compact", timeout=90)
        client.prompt(timeout=90)
        context_checked(api.requests)
        checks.append("real_compaction_and_next_turn_preserve_target_context")
        client.close(); client = None
        client = native(session)
        client.ready()
        api.steps.extend([[], tool("Read", {"file_path": "fixture.txt"})])
        client.prompt(timeout=90)
        context_checked(api.requests)
        checks.append("cold_resume_after_compaction_has_no_runtime_file_locators")
        # Both commands must start before either finishes. A global facade
        # queue deadlocks this barrier; the native dispatcher and target must
        # actually overlap, rather than just accepting two tool-use blocks.
        parallel = []
        for own, other in [("a", "b"), ("b", "a")]:
            parallel.extend(tool("Bash", {"command":
                f"printf started > parallel-{own}.txt; "
                f"for i in $(seq 1 100); do "
                f"if test -f parallel-{other}.txt; then printf complete >> parallel-{own}.txt; exit 0; fi; "
                "sleep 0.1; done; exit 1"}))
        api.steps.extend([[], parallel])
        client.prompt(timeout=90)
        for name in ["a", "b"]:
            require((args.target / f"parallel-{name}.txt").read_text() == "startedcomplete",
                    "independent native commands did not overlap")
        context_checked(api.requests)
        checks.append("independent_native_commands_execute_concurrently")
        api.steps.extend([[], tool("Bash", {"command": "printf foreground_started >> foreground.txt; while :; do sleep 1; printf tick >> foreground.txt; done", "timeout": 600000})])
        messages_before = len(client.messages)
        client.send({"type": "user", "message": {"role": "user", "content": "Run the foreground cancellation fixture."},
                     "parent_tool_use_id": None, "session_id": ""})
        deadline = time.monotonic() + 20
        while not (args.target / "foreground.txt").exists():
            require(time.monotonic() < deadline, "foreground command did not start")
            time.sleep(0.05)
        client.send({"type": "control_request", "request_id": "cancel-foreground", "request": {"subtype": "interrupt"}})
        client.until(lambda frame: frame.get("type") == "control_response" and
                     frame["response"].get("request_id") == "cancel-foreground")
        if not any(frame.get("type") == "result" for frame in client.messages[messages_before:]):
            client.until(lambda frame: frame.get("type") == "result")
        foreground = (args.target / "foreground.txt").read_text()
        time.sleep(1.2)
        require((args.target / "foreground.txt").read_text() == foreground, "foreground descendants survived interruption")
        context_checked(api.requests)
        checks.append("native_interrupt_stops_foreground_target_process")
        client.close(); client = None
        native_requests = len(api.requests)
        api.close()

        # Drive the actual bundled ACP adapter, not just its private CLI shim.
        api = ScriptedApi([tool("Bash", {"command": "cat fixture.txt; printf acp_once >> acp-once.txt"})])
        environment["ANTHROPIC_BASE_URL"] = f"http://127.0.0.1:{api.server_port}"
        class Acp(Executor):
            def send(self, message):
                super().send({"jsonrpc": "2.0", **message})

            def frame(self, deadline):
                while True:
                    frame = super().frame(deadline)
                    if frame.get("error"):
                        print("ACP fixture rejection:", json.dumps(frame["error"]))
                    require(not ("id" in frame and "method" in frame), "unexpected ACP client operation")
                    if "method" not in frame:
                        return frame
        def acp():
            instance = Acp([str(launcher.parent.parent / "bin/claude-agent-acp")], 120,
                           environment=environment, cwd=args.runtime)
            instance.request("initialize", {"protocolVersion": 1, "clientCapabilities": {},
                                             "clientInfo": {"name": "cowboy-fixture", "version": "1"}})
            return instance
        client = acp()
        created = client.request("session/new", {"cwd": str(args.runtime), "mcpServers": []})
        acp_session = created["sessionId"]
        client.request("session/set_mode", {"sessionId": acp_session, "modeId": "bypassPermissions"})
        client.request("session/set_config_option", {"sessionId": acp_session, "configId": "effort", "value": "high"})
        prompt = {"sessionId": acp_session, "prompt": [{"type": "text", "text": "Run the fixture."}]}
        client.request("session/prompt", prompt)
        context_checked(api.requests)
        require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP did not execute on target")
        client.close(); client = None
        api.steps.extend([[], tool("Bash", {"command": "cat acp-once.txt"})])
        client = acp()
        client.request("session/load", {"sessionId": acp_session, "cwd": str(args.runtime), "mcpServers": []})
        client.request("session/set_mode", {"sessionId": acp_session, "modeId": "bypassPermissions"})
        client.request("session/prompt", prompt)
        context_checked(api.requests)
        require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP replayed an effect")
        checks.extend(["packaged_acp_native_spawn_routes_target_tools", "packaged_acp_cold_load_preserves_binding_history_and_effects"])
        checks.append("hot_effort_configuration_preserves_bound_execution")
        client.close(); client = None
        before = len(api.requests)
        # Negative candidates are disposable copies; never modify supplied
        # packaged input bytes or production Plugin state.
        broken = args.runtime.parent / "broken-claude-package"
        (broken / "app").mkdir(parents=True)
        shutil.copytree(launcher.parent.parent / "bin", broken / "bin")
        (broken / "runtime").symlink_to(launcher.parent.parent / "runtime", target_is_directory=True)
        (broken / "app/node_modules").symlink_to(launcher.parent / "node_modules", target_is_directory=True)
        for name in ["cowboy-launch.mjs", "connection.mjs", "tools.mjs"]:
            shutil.copyfile(launcher.parent / name, broken / "app" / name)
        (broken / "app/context-mod.js").write_text("export function register() { throw new Error('fixture broken module'); }\n")
        for extra in [(), ("--bare",)]:
            failed = Claude(str(broken / "bin/cowboy-configured-cli"), environment, args.runtime, fixture,
                            extra_arguments=extra)
            try:
                try:
                    failed.ready()
                except ProbeFailure:
                    pass
                else:
                    raise ProbeFailure("invalid execution module or bare mode was accepted")
                failed.process.wait(timeout=10)
                require(len(api.requests) == before, "failed readiness reached model")
                failed.stderr.seek(0)
                diagnostic = failed.stderr.read().decode()
                require(("--bare" if extra else "execution module is missing or disabled") in diagnostic,
                        "readiness failed for an unrelated reason")
            finally:
                failed.close()
        checks.extend(["broken_module_refused_before_model_request", "bare_mode_refused_without_changing_authentication"])
        receipt = {
            "schema": "cowboy.claude-execution-worker-conformance/v1", "accepted": False, "checks": checks,
            "claude_version": inputs["claude_version"], "claude_sha256": inputs["claude_sha256"],
            "executor_sha256": inputs["sha256"], "executor_version": inputs["version"],
            "packaged_launcher_sha256": hashlib.sha256(launcher.read_bytes()).hexdigest(),
            "scripted_api_requests": native_requests + len(api.requests),
            "production_credentials": False, "production_activation": False,
            "not_checked": ["real_subscription_inference", "cross_host_latency", "native_subagents_and_project_hooks"],
        }
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"accepted {len(checks)} packaged Claude worker checks")
    finally:
        if client:
            if isinstance(client, Claude) and client.process.poll() is not None:
                client.stderr.seek(0)
                print(client.stderr.read().decode()[-2000:])
            client.close()
        api.close()


if __name__ == "__main__":
    main()
