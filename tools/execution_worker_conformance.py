#!/usr/bin/env python3
"""Native Codex turns through Cowboy's worker endpoint and target keeper.

Started by the Rust gate in a disposable PID/network namespace. The model API
is scripted loopback only. No native environment selection is supplied by this
client: the actual Provider private launcher must supply it on every turn.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import socket
import struct
import time
import zlib

from execution_environment_codex_turn_probe import Api, command, final
from execution_environment_probe import Executor, ProbeFailure, require
from plugin_runtime_conformance import closed_environment
from matrix_execution_fixture import MatrixFixture


def complete_turn(client, thread, prompt="Run the fixture."):
    result = client.request("turn/start", {"threadId": thread, "input": [{
        "type": "text", "text": prompt, "text_elements": [],
    }]})
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        frame = client.frame(deadline)
        require(not ("id" in frame and "method" in frame), "unexpected approval")
        if frame.get("method") == "turn/completed" and frame["params"]["threadId"] == thread:
            turn = frame["params"]["turn"]
            require(turn["id"] == result["turn"]["id"], "turn identity changed")
            require(turn["status"] == "completed", "native turn failed")
            return
    raise ProbeFailure("native turn timed out")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["native-cli", "descriptor", "runtime", "target", "receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    launcher = Path("components/provider-runtime/packages/codex-acp/launch.mjs").resolve()
    inputs = json.loads(Path(os.environ["COWBOY_TEST_EXECUTION_INPUT"]).read_text())
    packaged = Path(inputs["adapter_launcher"]) if inputs.get("adapter_launcher") else None
    if packaged:
        require(packaged.is_absolute() and packaged.is_file(), "packaged launcher missing")
        launcher = packaged
    node = shutil.which("node")
    require(node, "pinned Node is missing")
    def cancel_background(requests):
        outputs = [item.get("output", "") for item in requests[-1].get("input", [])
                   if item.get("type") == "function_call_output" and item.get("call_id") == "fixture_background"]
        match = re.search(r"(?:session ID|session_id)[\s:=]+(\d+)", str(outputs), re.IGNORECASE)
        require(match is not None, "background command did not return a process handle")
        return {"type": "function_call", "call_id": "fixture_cancel", "name": "write_stdin",
                "arguments": json.dumps({"session_id": int(match[1]), "chars": "\u0003", "yield_time_ms": 1000})}

    def wait_codeact(requests):
        output = [item for item in requests[-1].get("input", [])
                  if item.get("call_id") == "fixture-native-codeact-yield" and item.get("type") == "custom_tool_call_output"]
        match = re.search(r"Script running with cell ID ([a-zA-Z0-9_-]+)", json.dumps(output))
        require(match is not None, "native CodeAct did not yield a resumable cell: " + json.dumps(output)[-2000:])
        return {"type": "function_call", "call_id": "fixture-native-codeact-wait", "namespace": "functions", "name": "wait",
                "arguments": json.dumps({"cell_id": match[1], "yield_time_ms": 10000})}

    def png_chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    (args.target / "pixel.png").write_bytes(b"\x89PNG\r\n\x1a\n" +
        png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0)) +
        png_chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\0\0" * 2) * 2)) + png_chunk(b"IEND", b""))
    disconnected_output = "" if inputs.get("legacy_event_gap") else "head -c 8388608 /dev/zero | tr '\\0' x; "
    background = command("fixture_background", "printf background_started >> jobs.txt; trap 'printf cancelled >> jobs.txt; exit 0' INT TERM; " + disconnected_output + "sleep 120 & wait")
    background["arguments"] = json.dumps({**json.loads(background["arguments"]), "tty": True})
    api = Api([
        {"type": "custom_tool_call", "call_id": "fixture_patch", "name": "apply_patch",
         "input": "*** Begin Patch\n*** Update File: fixture.txt\n@@\n-target before\n+target after '\" $() 中文 🐎\n*** End Patch"},
        command("fixture_command", "pwd; cat fixture.txt; printf once >> once.txt"),
        {"type": "function_call", "call_id": "fixture_image", "name": "view_image",
         "arguments": json.dumps({"path": str(args.target / "pixel.png")})},
        background,
        cancel_background,
        final("fixture_final_one"),
        command("fixture_resume", "pwd; cat fixture.txt; cat once.txt"),
        final("fixture_final_two"),
    ])
    (args.target / "codeact.txt").write_text("target before\n")
    (args.runtime / "codeact.txt").write_text("runtime remains untouched\n")
    (args.target / "codeact-pixel.png").write_bytes((args.target / "pixel.png").read_bytes())
    patch = "*** Begin Patch\n*** Update File: codeact.txt\n@@\n-target before\n+native codeact target\n*** End Patch"
    program = "text(await tools.apply_patch(" + json.dumps(patch) + ")); "
    program += "const results = await Promise.all(["
    program += "tools.exec_command(" + json.dumps({"cmd": "pwd; cat codeact.txt; printf once >> codeact-once.txt", "max_output_tokens": 1000}) + "),"
    program += "tools.exec_command(" + json.dumps({"cmd": "printf native_codeact_expected_failure >&2; exit 37", "max_output_tokens": 1000}) + ")]); "
    program += "for (const result of results) text(result);"
    program += "const picture = await tools.view_image(" + json.dumps({"path": str(args.target / "codeact-pixel.png")}) + "); image(picture.image_url);"
    api.steps.insert(5, {"type": "custom_tool_call", "call_id": "fixture-native-codeact", "namespace": "functions", "name": "exec", "input": program})
    api.steps[6:6] = [{"type": "custom_tool_call", "call_id": "fixture-native-codeact-yield", "namespace": "functions", "name": "exec",
                       "input": '// @exec: {"yield_time_ms": 1}\ntext(await tools.exec_command({cmd:"sleep 2; printf yielded >> codeact-yielded.txt; cat codeact-yielded.txt",yield_time_ms:3000,max_output_tokens:1000}));'}, wait_codeact]
    api.steps.insert(-1, {"type": "custom_tool_call", "call_id": "fixture-native-codeact-resume", "namespace": "functions", "name": "exec",
                         "input": "text(await tools.exec_command({cmd:'cat codeact.txt; cat codeact-once.txt',max_output_tokens:1000}));"})
    child_counts = {"none": 0, "all": 0}
    child_requests = {mode: [] for mode in child_counts}
    child_steps = []
    for mode in child_counts:
        child_steps.extend([
            {"type": "function_call", "namespace": "collaboration", "name": "spawn_agent", "call_id": "fixture_spawn_" + mode,
             "arguments": json.dumps({"task_name": "remote_" + mode, "message": "Execute the isolated child fixture.", "fork_turns": mode})},
            {"type": "function_call", "namespace": "collaboration", "name": "wait_agent", "call_id": "fixture_wait_" + mode,
             "arguments": json.dumps({"timeout_ms": 10000})},
        ])
    api.steps[8:8] = child_steps
    parent_step = 0

    def dispatch(requests):
        nonlocal parent_step
        request = requests[-1]
        recipient = next((item.get("recipient") for item in reversed(request.get("input", []))
                          if item.get("type") == "agent_message" and item.get("recipient") in
                          ["/root/remote_" + mode for mode in child_counts]), None)
        if recipient:
            mode = recipient.rsplit("_", 1)[1]
            step = child_counts[mode]
            child_counts[mode] += 1
            child_requests[mode].append(request)
            require(step < 2, "unexpected extra child request")
            if step == 1:
                return final("fixture_child_final_" + mode)
            script = "pwd; printf once >> native-child-" + mode + ".txt; cat native-child-" + mode + ".txt"
            if mode == "none":
                return command("fixture_child_none", script)
            return {"type": "custom_tool_call", "namespace": "functions", "name": "exec", "call_id": "fixture_child_" + mode,
                    "input": "text(await tools.exec_command(" + json.dumps({"cmd": script, "max_output_tokens": 1000}) + "));"}
        require(parent_step < len(api.steps), "unexpected extra parent request")
        step = api.steps[parent_step]
        parent_step += 1
        return step

    api.dispatcher = dispatch
    api.extra_requests = 4
    if inputs.get("legacy_event_gap"):
        api.steps[0:0] = [
            command("fixture_legacy_flood", "printf once >> legacy-starts; head -c 8388608 /dev/zero"),
            command("fixture_legacy_health", "printf healthy > legacy-health"),
        ]
    environment = closed_environment(args.runtime.parent / "agent-home")
    environment.update({
        "COWBOY_OFFLINE_FIXTURE_KEY": "not-a-production-credential",
        "COWBOY_PRIVATE_CODEX_EXECUTABLE": str(args.native_cli),
        "COWBOY_PRIVATE_CODEX_ARGUMENTS": "[]",
        "COWBOY_EXECUTION_DESCRIPTOR": str(args.descriptor),
    })
    home = Path(environment["CODEX_HOME"])
    memory = MatrixFixture(inputs, args.runtime.parent, args.descriptor, "codex", environment)
    if memory.enabled:
        api.steps.insert(0, {"type": "custom_tool_call", "call_id": "fixture-memory", "namespace": "functions", "name": "exec", "input": "text(await tools.mcp__matrix__" + memory.tool + "(" + json.dumps(memory.arguments) + "));"})
    home.mkdir()
    (home / "config.toml").write_text(
        'model = "gpt-6-astra"\nmodel_provider = "cowboy_fixture"\n'
        '[features]\nremote_control = false\nshell_snapshot = false\n'
        '[model_providers.cowboy_fixture]\nname = "Offline fixture"\n'
        f'base_url = "http://127.0.0.1:{api.server_port}/v1"\n'
        'wire_api = "responses"\nenv_key = "COWBOY_OFFLINE_FIXTURE_KEY"\n'
        'request_max_retries = 0\nstream_max_retries = 0\n'
    )

    def native():
        client = Executor([node, str(launcher), "--cowboy-private-cli", "app-server"],
                          90, environment=environment, cwd=args.runtime)
        client.request("initialize", {"clientInfo": {"name": "cowboy-worker-fixture", "version": "1"}})
        client.send({"method": "initialized", "params": {}})
        return client

    client = None
    checks = ["machine_owned_worktree", "long_state_path_uses_private_short_socket",
              "machine_restart_reattaches_same_keeper_and_binding"]
    try:
        client = native()
        client.request("environment/add", {"environmentId": "forbidden", "execServerUrl": "stdio://"}, expect_error=True)
        checks.append("provider_refuses_environment_override")
        started = client.request("thread/start", {
            "cwd": str(args.runtime), "approvalPolicy": "never", "sandbox": "danger-full-access",
        })
        thread = started["thread"]["id"]
        complete_turn(client, thread)
        require(api.failure is None, api.failure or "scripted API failed")
        require(child_counts == {"none": 2, "all": 2}, "both native children must execute and complete")
        for mode, requests in child_requests.items():
            name = "native-child-" + mode + ".txt"
            require((args.target / name).read_text() == "once" and not (args.runtime / name).exists(),
                    "native child missed target or repeated an effect: " + mode)
            require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(requests[0]) and
                    "RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(requests), "child guidance binding differs: " + mode)
            results = [item for request in requests for item in request.get("input", [])
                       if item.get("call_id") == "fixture_child_" + mode and item.get("type", "").endswith("call_output")]
            require(str(args.target) in json.dumps(results) and "once" in json.dumps(results), "child target result missing: " + mode)
        checks.append("native_fresh_and_forked_children_inherit_target_guidance_and_execute_once")
        require((args.target / "fixture.txt").read_text() == "target after '\" $() 中文 🐎\n", "target edit failed")
        require((args.target / "once.txt").read_text() == "once", "target command did not execute exactly once")
        if inputs.get("legacy_event_gap"):
            require((args.target / "legacy-starts").read_text() == "once", "legacy recovery replayed a command")
            require((args.target / "legacy-health").read_text() == "healthy", "legacy gap poisoned subsequent commands")
            checks.append("native_codex_recovers_legacy_event_gap_without_poisoning_session_or_replaying_effects")
        require((args.runtime / "fixture.txt").read_text() == "runtime remains untouched\n"
                and not (args.runtime / "once.txt").exists(), "runtime was modified")
        require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(api.requests[0]), "target instructions missing")
        require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests), "runtime instructions leaked")
        require((args.target / "jobs.txt").read_text() == "background_startedcancelled", "background process replayed or cancellation did not reach target")
        direct_images = [item for request in api.requests for item in request.get("input", [])
                         if item.get("call_id") == "fixture_image" and item.get("type") == "function_call_output"]
        require(any(block.get("type") == "input_image" and str(block.get("image_url", "")).startswith("data:image/")
                    for item in direct_images for block in item.get("output", []) if isinstance(block, dict)),
                "direct target image did not reach native model input: " + json.dumps(direct_images)[:1500])
        checks.extend(["target_image_read_reaches_native_model_input", "background_job_survives_lost_reply_and_35_second_transport_gap", "cancel_reaches_original_target_job_without_replay"])
        if not inputs.get("legacy_event_gap"):
            checks.append("output_backpressure_survives_35_second_transport_gap")
        checks.extend(["native_apply_patch_and_command_use_target", "runtime_files_unchanged", "native_target_instructions"])
        client.close()
        client = native()
        resumed = client.request("thread/resume", {"threadId": thread, "excludeTurns": True})
        require(resumed["thread"]["id"] == thread, "native thread changed")
        complete_turn(client, thread)
        require(api.failure is None, api.failure or "scripted resume failed")
        require((args.target / "once.txt").read_text() == "once", "cold resume repeated a command")
        resumed_outputs = [item for request in api.requests for item in request.get("input", [])
                           if item.get("type") == "function_call_output" and item.get("call_id") == "fixture_resume"]
        require(any("target after" in str(item.get("output")) for item in resumed_outputs),
                "cold resumed command did not actually read target bytes: " + json.dumps(resumed_outputs)[-1500:])
        require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(api.requests[-2]), "resume target instructions missing")
        require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests), "resume runtime instructions leaked")
        require(not (home / "environments.toml").exists(), "global environments changed")
        checks.extend(["cold_native_resume_automatically_rebinds_each_turn", "cold_resume_retains_history_without_effect_replay", "cold_native_resumed_command_returns_target_bytes", "global_environment_config_unchanged"])
        require((args.target / "codeact.txt").read_text() == "native codeact target\n", "native CodeAct patch missed target")
        require((args.target / "codeact-once.txt").read_text() == "once", "native CodeAct shell missed target or replayed")
        require((args.target / "codeact-yielded.txt").read_text() == "yielded", "yielded CodeAct effect was lost or replayed")
        waited = [item for request in api.requests for item in request.get("input", [])
                  if item.get("call_id") == "fixture-native-codeact-wait" and item.get("type") == "function_call_output"]
        require("yielded" in json.dumps(waited), "CodeAct wait did not return the completed target result: " + json.dumps(waited)[-2000:])
        require((args.runtime / "codeact.txt").read_text() == "runtime remains untouched\n" and
                not (args.runtime / "codeact-once.txt").exists(), "native CodeAct mutated runtime files")
        results = [item for request in api.requests for item in request.get("input", [])
                   if item.get("type") == "custom_tool_call_output" and item.get("call_id") in
                   ["fixture-native-codeact", "fixture-native-codeact-resume"]]
        command_results = []
        for item in results:
            for block in item.get("output", []):
                if not isinstance(block, dict) or block.get("type") != "input_text":
                    continue
                try:
                    value = json.loads(block["text"])
                except (ValueError, KeyError):
                    continue
                if isinstance(value, dict):
                    command_results.append(value)
        require(any(value.get("exit_code") == 37 and "native_codeact_expected_failure" in value.get("output", "")
                    for value in command_results), "nested failed command lost its output or exit code")
        require(any(block.get("type") == "input_image" for item in results
                    for block in item.get("output", []) if isinstance(block, dict)),
                "native CodeAct did not return target image content: " + json.dumps(results)[-2000:])
        require(any(item.get("call_id") == "fixture-native-codeact-resume" and "native codeact target" in str(item.get("output"))
                    for item in results), "cold resumed CodeAct result missing: " + json.dumps(results)[-4000:])
        checks.extend(["native_codeact_patch_parallel_shell_and_failure_use_target", "native_codeact_target_image_reaches_model", "native_codeact_yield_and_wait_keep_target_and_single_effect", "native_codeact_cold_resume_preserves_binding_and_single_effect"])
        if packaged:
            client.close()
            client = None
            child_counts["acp"] = 0
            child_requests["acp"] = []
            api.extra_requests += 2
            parent_result_id = "packaged_parent_after_child"
            parent_polls = 0

            def finish_parent(requests):
                nonlocal parent_result_id, parent_polls
                outputs = [item.get("output", "") for item in requests[-1].get("input", [])
                           if item.get("type") == "function_call_output" and item.get("call_id") == parent_result_id]
                require(len(outputs) == 1, "parent continuation result missing")
                output = str(outputs[0])
                running = re.search(r"(?:session ID|session_id)[\s:=]+(\d+)", output, re.IGNORECASE)
                if running:
                    parent_polls += 1
                    require(parent_polls <= 10, "parent continuation did not exit within fixture poll budget")
                    parent_result_id = "packaged_parent_poll_" + str(parent_polls)
                    api.steps.insert(parent_step, finish_parent)
                    return {"type": "function_call", "name": "write_stdin", "call_id": parent_result_id,
                            "arguments": json.dumps({"session_id": int(running[1]), "chars": "", "yield_time_ms": 1000})}
                require("Process exited with code 0" in output, "parent continuation failed: " + output[-1000:])
                return final("packaged_acp_final")

            api.steps.extend([
                command("packaged_acp_command", "cat fixture.txt; printf acp_once >> acp-once.txt"),
                {"type": "function_call", "namespace": "collaboration", "name": "spawn_agent", "call_id": "fixture_spawn_acp",
                 "arguments": json.dumps({"task_name": "remote_acp", "message": "Execute the isolated ACP child fixture.", "fork_turns": "none"})},
                {"type": "function_call", "namespace": "collaboration", "name": "wait_agent", "call_id": "fixture_wait_acp",
                 "arguments": json.dumps({"timeout_ms": 10000})},
                command("packaged_parent_after_child", "sleep 2; printf after >> parent-after-child.txt"),
                finish_parent,
                command("packaged_acp_resume", "cat fixture.txt; cat acp-once.txt"),
                final("packaged_acp_resumed_final"),
            ])
            class Acp(Executor):
                def send(self, frame):
                    super().send({"jsonrpc": "2.0", **frame})

                def frame(self, deadline):
                    frame = super().frame(deadline)
                    if "error" in frame:
                        # This client exists only in a disposable loopback
                        # fixture; keep diagnostics bounded to its error class.
                        print("ACP fixture:", json.dumps({key: frame["error"].get(key)
                            for key in ("code", "message", "data")})[:1800], flush=True)
                    return frame

            acp_environment = {**environment, "CODEX_PATH": str(args.native_cli)}

            def acp():
                instance = Acp([node, str(packaged), "-c", "approval_policy=never",
                    "-c", "sandbox_mode=danger-full-access"], 90,
                    environment=acp_environment, cwd=args.runtime)
                instance.request("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
                instance.request("authenticate", {"methodId": "api-key", "_meta": {
                    "api-key": {"apiKey": "not-a-production-credential"}}})
                return instance

            client = acp()
            created = client.request("session/new", {"cwd": str(args.runtime), "mcpServers": []})
            acp_session = created["sessionId"]
            client.request("session/set_config_option", {"sessionId": acp_session,
                "configId": "mode", "value": "agent-full-access"})
            prompt = {"sessionId": acp_session, "prompt": [{"type": "text", "text": "Run the fixture."}]}
            result = client.request("session/prompt", prompt)
            require(result["stopReason"] == "end_turn", "packaged ACP turn failed")
            require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP missed target")
            require(not (args.runtime / "acp-once.txt").exists(), "packaged ACP used runtime filesystem")
            require(child_counts["acp"] == 2 and (args.target / "native-child-acp.txt").read_text() == "once",
                    "packaged ACP child did not complete its target command")
            require((args.target / "parent-after-child.txt").read_text() == "after",
                    "packaged ACP returned before the parent completed after its child")
            require(not (args.runtime / "native-child-acp.txt").exists() and
                    not (args.runtime / "parent-after-child.txt").exists(), "packaged ACP child or continuation used runtime files")
            checks.append("packaged_acp_child_completion_does_not_finish_parent_prompt")
            client.close()
            client = acp()
            client.request("session/load", {"sessionId": acp_session, "cwd": str(args.runtime), "mcpServers": []})
            client.request("session/set_config_option", {"sessionId": acp_session,
                "configId": "mode", "value": "agent-full-access"})
            result = client.request("session/prompt", prompt)
            require(result["stopReason"] == "end_turn", "packaged ACP resume failed")
            require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP resume replayed an effect")
            require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(api.requests[-2]), "packaged ACP lost target instructions")
            require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests), "packaged ACP leaked runtime guidance")
            checks.extend(["packaged_acp_native_spawn_routes_target_tools", "packaged_acp_cold_load_preserves_binding_history_and_effects"])
        checks.extend(memory.accept(api.requests))
        receipt = {
            "schema": "cowboy.execution-worker-conformance/v1", "accepted": False, "checks": checks,
            "native_sha256": hashlib.sha256(args.native_cli.read_bytes()).hexdigest(),
            "sandbox_helper_sha256": hashlib.sha256((args.native_cli.parent.parent / "codex-resources/bwrap").read_bytes()).hexdigest(),
            "code_mode_host_sha256": hashlib.sha256((args.native_cli.parent / "codex-code-mode-host").read_bytes()).hexdigest()
                if (args.native_cli.parent / "codex-code-mode-host").is_file() else None,
            "launcher_sha256": hashlib.sha256(launcher.read_bytes()).hexdigest(),
            "packaged_bridge_sha256": hashlib.sha256((packaged.parent / "cowboy-execution.mjs").read_bytes()).hexdigest() if packaged else None,
            "packaged_adapter_sha256": hashlib.sha256((packaged.parent / "node_modules/@agentclientprotocol/codex-acp/dist/index.js").read_bytes()).hexdigest() if packaged else None,
            "scripted_api_requests": len(api.requests), "real_model_requests": 0,
            "packaged_parent_continuation_polls": parent_polls if packaged else 0,
            "production_credentials": False, "production_activation": False,
            "not_checked": ["authenticated_enrolled_transport", "cross_host_network", "signed_provider_release",
                            "subscription_inference", "inline_user_attachments",
                            "native_grandchildren_background_agents_and_child_crash_recovery"],
        }
        fd = os.open(args.receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump(receipt, output, indent=2)
            output.write("\n")
        print(f"accepted {len(checks)} native worker execution checks")
    finally:
        if client is not None:
            client.close()
        api.close()


if __name__ == "__main__":
    main()
