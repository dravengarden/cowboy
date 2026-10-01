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


def complete_turn(client, thread):
    result = client.request("turn/start", {"threadId": thread, "input": [{
        "type": "text", "text": "Run the fixture.", "text_elements": [],
    }]})
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        frame = client.frame(deadline)
        require(not ("id" in frame and "method" in frame), "unexpected approval")
        if frame.get("method") == "turn/completed":
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

    def png_chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    (args.target / "pixel.png").write_bytes(b"\x89PNG\r\n\x1a\n" +
        png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0)) +
        png_chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\0\0" * 2) * 2)) + png_chunk(b"IEND", b""))
    background = command("fixture_background", "printf background_started >> jobs.txt; trap 'printf cancelled >> jobs.txt; exit 0' INT TERM; sleep 120 & wait")
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
    environment = closed_environment(args.runtime.parent / "agent-home")
    environment.update({
        "COWBOY_OFFLINE_FIXTURE_KEY": "not-a-production-credential",
        "COWBOY_PRIVATE_CODEX_EXECUTABLE": str(args.native_cli),
        "COWBOY_PRIVATE_CODEX_ARGUMENTS": "[]",
        "COWBOY_EXECUTION_DESCRIPTOR": str(args.descriptor),
    })
    home = Path(environment["CODEX_HOME"])
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
        require((args.target / "fixture.txt").read_text() == "target after '\" $() 中文 🐎\n", "target edit failed")
        require((args.target / "once.txt").read_text() == "once", "target command did not execute exactly once")
        require((args.runtime / "fixture.txt").read_text() == "runtime remains untouched\n"
                and not (args.runtime / "once.txt").exists(), "runtime was modified")
        require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(api.requests[0]), "target instructions missing")
        require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests), "runtime instructions leaked")
        require((args.target / "jobs.txt").read_text() == "background_startedcancelled", "background process replayed or cancellation did not reach target")
        require("data:image/" in json.dumps(api.requests), "target image did not reach native model input: " + json.dumps([
            item for item in api.requests[-1].get("input", []) if item.get("call_id") == "fixture_image"
        ])[:1500])
        checks.extend(["target_image_read_reaches_native_model_input", "background_job_survives_lost_reply_and_35_second_transport_gap", "cancel_reaches_original_target_job_without_replay"])
        checks.extend(["native_apply_patch_and_command_use_target", "runtime_files_unchanged", "native_target_instructions"])
        client.close()
        client = native()
        resumed = client.request("thread/resume", {"threadId": thread, "excludeTurns": True})
        require(resumed["thread"]["id"] == thread, "native thread changed")
        complete_turn(client, thread)
        require(api.failure is None, api.failure or "scripted resume failed")
        require((args.target / "once.txt").read_text() == "once", "cold resume repeated a command")
        require("TARGET_GUIDANCE_MUST_REACH_MODEL" in json.dumps(api.requests[-2]), "resume target instructions missing")
        require("RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in json.dumps(api.requests), "resume runtime instructions leaked")
        require(not (home / "environments.toml").exists(), "global environments changed")
        checks.extend(["cold_native_resume_automatically_rebinds_each_turn", "cold_resume_retains_history_without_effect_replay", "global_environment_config_unchanged"])
        if packaged:
            client.close()
            client = None
            api.steps.extend([
                command("packaged_acp_command", "cat fixture.txt; printf acp_once >> acp-once.txt"),
                final("packaged_acp_final"),
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
        receipt = {
            "schema": "cowboy.execution-worker-conformance/v1", "accepted": False, "checks": checks,
            "native_sha256": hashlib.sha256(args.native_cli.read_bytes()).hexdigest(),
            "launcher_sha256": hashlib.sha256(launcher.read_bytes()).hexdigest(),
            "packaged_bridge_sha256": hashlib.sha256((packaged.parent / "cowboy-execution.mjs").read_bytes()).hexdigest() if packaged else None,
            "packaged_adapter_sha256": hashlib.sha256((packaged.parent / "node_modules/@agentclientprotocol/codex-acp/dist/index.js").read_bytes()).hexdigest() if packaged else None,
            "scripted_api_requests": len(api.requests), "real_model_requests": 0,
            "production_credentials": False, "production_activation": False,
            "not_checked": ["authenticated_enrolled_transport", "cross_host_network", "signed_provider_release",
                            "subscription_inference", "inline_user_attachments", "native_nested_agents"],
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
