#!/usr/bin/env python3
"""Packaged Claude managed read-only turn through a real keeper-enforced target.

Run by `just execution-managed-conformance INPUT RECEIPT`, which prepares a
Machine-owned snapshot, its read-only keeper and the worker transport, then
calls this driver. Native Claude talks to a scripted loopback API; no model,
subscription or production credential is used.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from execution_environment_probe import require
from plugin_runtime_conformance import closed_environment

PROBE = ("touch managed-probe.txt; echo ws=$?; touch /tmp/managed-probe-escape; "
         "echo tmp=$?; cat fixture.txt")
REMOVED = {"Write", "Edit", "NotebookEdit", "Agent", "WebFetch", "WebSearch", "Skill", "AskUserQuestion"}


def tool_results(request):
    texts = []
    for message in request.get("messages", []):
        content = message.get("content")
        for block in content if isinstance(content, list) else []:
            if block.get("type") != "tool_result":
                continue
            value = block.get("content")
            if isinstance(value, list):
                value = "".join(item.get("text", "") for item in value if item.get("type") == "text")
            texts.append(str(value))
    return texts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["descriptor", "runtime", "target", "receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new receipt required")
    inputs = json.loads(Path(os.environ["COWBOY_TEST_EXECUTION_INPUT"]).read_text())
    binary = Path(inputs["claude_cli"])
    require(hashlib.sha256(binary.read_bytes()).hexdigest() == inputs["claude_sha256"], "Claude artifact changed")
    wrapper = Path(inputs["adapter_launcher"]).parent.parent / "bin/cowboy-configured-cli"
    require(wrapper.is_file(), "packaged Claude execution launcher missing")
    observed = {}

    def structured(requests):
        request = requests[-1]
        observed["tools"] = sorted(item["name"] for item in request.get("tools", []))
        observed["results"] = tool_results(request)
        return tool("StructuredOutput", {"verdict": "approve"})

    api = ScriptedApi([
        tool("Bash", {"command": PROBE}),
        tool("Read", {"file_path": "fixture.txt"}),
        structured,
    ], native_titles=True)
    environment = closed_environment(args.runtime.parent / "claude-home")
    environment.update({
        "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
        "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
        "COWBOY_PRIVATE_CLAUDE_EXECUTABLE": str(binary),
        "CLAUDE_CODE_EXECUTABLE": str(binary),
        "COWBOY_EXECUTION_DESCRIPTOR": str(args.descriptor),
        # Set by the adapter entry from the signed profile's flag.
        "COWBOY_MANAGED_PROFILE": "read-only-v1",
        "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
    })
    checks = []
    client = Claude(str(wrapper), environment, args.runtime, WorkspaceFixture(args.target),
                    bound_native=True, disallowed="AskUserQuestion")
    try:
        client.ready()
        # A managed child's permission mode belongs to Cowboy.
        client.send({"type": "control_request", "request_id": "mode-change",
                     "request": {"subtype": "set_permission_mode", "mode": "acceptEdits"}})
        refused = client.until(lambda frame: frame.get("type") == "control_response"
                               and frame["response"].get("request_id") == "mode-change")
        require(refused["response"]["subtype"] == "error", "managed permission mode change was accepted")
        checks.append("permission_mode_change_refused")
        try:
            result = client.prompt(text="Review the snapshot.", timeout=180)
        except Exception:
            # Bounded diagnostics: native frames and stderr carry no credential.
            for frame in client.messages[-6:]:
                print(json.dumps(frame)[:1500])
            print("api failure:", api.failure)
            for request in api.requests:
                names = sorted(item.get("name", "") for item in request.get("tools", []))
                last = request["messages"][-1]
                content = last.get("content")
                print("request tools:", names[:40])
                print("request last:", json.dumps(content)[:1200])
            client.stderr.seek(0)
            print(client.stderr.read()[-3000:].decode(errors="replace"))
            raise
    finally:
        client.close()
        api.close()
    require(result.get("structured_output") == {"verdict": "approve"}, "native structured output missing")
    checks.append("native_structured_output_validated")
    messages = client.messages
    final = [frame for frame in messages if frame.get("type") == "assistant"
             and frame.get("parent_tool_use_id") is None][-1]
    require(final["message"]["content"] == [{"type": "text", "text": '{"verdict":"approve"}'}],
            "structured output is not the turn's final message")
    require(messages.index(final) < messages.index(result), "structured message follows the result")
    checks.append("structured_output_is_final_message")
    tools = set(observed.get("tools", []))
    require({"Bash", "Read", "StructuredOutput"} <= tools, "managed read tools or StructuredOutput missing")
    require(not tools & REMOVED, "managed turn advertises removed tools: " + ", ".join(sorted(tools & REMOVED)))
    require(not any(name.startswith("mcp__") for name in tools), "managed turn advertises MCP tools")
    checks.append("read_only_tool_surface")
    results = "\n".join(observed.get("results", []))
    for expected in ["ws=1", "tmp=1", "Read-only file system", "snapshot fixture", "working tree"]:
        require(expected in results, "missing managed tool evidence: " + expected)
    checks.append("keeper_sandbox_refuses_workspace_and_tmp_writes")
    checks.append("snapshot_includes_working_tree")
    require(not (args.target / "managed-probe.txt").exists(), "managed turn wrote its workspace")
    require(not Path("/tmp/managed-probe-escape").exists(), "managed turn wrote /tmp")
    checks.append("no_files_written")
    args.receipt.write_text(json.dumps({
        "schema": "cowboy.claude-managed-execution-conformance/v1",
        "accepted": True,
        "checks": checks,
        "claude_sha256": inputs["claude_sha256"],
        "packaged_launcher_sha256": hashlib.sha256(Path(inputs["adapter_launcher"]).read_bytes()).hexdigest(),
        "scripted_api_requests": len(api.requests),
        "production_credentials": False,
        "production_activation": False,
    }, indent=1) + "\n")


if __name__ == "__main__":
    main()
