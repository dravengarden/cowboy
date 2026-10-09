#!/usr/bin/env python3
"""Measure native $.tool.check decisions against native permission prompts, per mode.

context-mod.js gates target tools with native $.tool.check and raises its
asks as native can_use_tool requests. This runs the pinned native CLI with a
recording Mod against a scripted loopback API (no credentials, no inference)
and records, for each permission mode and tool call (inside and outside the
workspace, through symlinks, read-only and mutating commands), what
$.tool.check decided and whether native itself prompted. Diff a fresh receipt
with tools/claude_permissions_native_baseline.json for every Claude CLI
candidate.

Usage: python3 tools/claude_permissions_native_probe.py --claude <native cli> --receipt <new path>
"""
import argparse
import json
from pathlib import Path
import secrets
import subprocess
import tempfile
import threading
import time
from unittest.mock import patch

from claude_mods_native_probe import FIXTURE_ENVIRONMENT, Observer, Recorder
from claude_native_behavior_probe import clean_root, stable
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment

# plan is not measured: its auto-mode classifier sends its own model requests,
# which a scripted API cannot answer as the model would.
MODES = ["default", "acceptEdits", "bypassPermissions", "dontAsk"]

MOD = r'''let context;
async function observe($, body) {
  context ??= JSON.parse(await $.fs.read(await $.env.get("COWBOY_CLAUDE_CONTEXT")));
  try {
    await $.http.fetch("http://cowboy-execution/check", { socketPath: context.socketPath, method: "POST",
      headers: { Authorization: "Bearer " + context.token }, body: JSON.stringify({ kind: "check", ...body }) });
  } catch {}
}
export function register(on) {
  on("tool.call", async ($, event, next) => {
    const input = { ...event };
    for (const key of ["tool", "tool_use_id", "agentId", "consent"]) delete input[key];
    let check;
    try {
      check = await $.tool.check({ tool: event.tool, input });
    } catch (error) {
      check = { error: String(error).slice(0, 200) };
    }
    await observe($, { tool: event.tool, id: event.tool_use_id ?? null, check });
    return next(event);
  });
}
'''


def calls(outside):
    return [
        tool("Bash", {"command": "ls"}),
        tool("Bash", {"command": "cat inside.txt"}),
        tool("Bash", {"command": "git status"}),
        tool("Bash", {"command": "rm -f gone.txt"}),
        tool("Bash", {"command": "mkdir newdir"}),
        tool("Bash", {"command": "touch made.txt"}),
        tool("Bash", {"command": "cp inside.txt copy.txt"}),
        tool("Bash", {"command": "echo hi > out.txt"}),
        tool("Bash", {"command": "mkdir link/new"}),
        tool("Bash", {"command": "touch link/made.txt"}),
        tool("Read", {"file_path": "inside.txt"}),
        tool("Read", {"file_path": str(outside / "outside.txt")}),
        tool("Edit", {"file_path": "inside.txt", "old_string": "inside", "new_string": "edited"}),
        # Native refuses an unread file before asking, so read through the links first.
        tool("Read", {"file_path": "link/outside.txt"}),
        tool("Edit", {"file_path": "link/outside.txt", "old_string": "outside", "new_string": "x"}),
        tool("Read", {"file_path": "file-link.txt"}),
        tool("Write", {"file_path": "file-link.txt", "content": "via link"}),
        tool("Write", {"file_path": str(outside / "outside.txt"), "content": "w"}),
    ]


def describe(step):
    block = step[0]
    return block["name"] + " " + json.dumps(block["input"], sort_keys=True)


def run_mode(claude, root, mode):
    runtime, outside = root / "runtime", root / "outside"
    runtime.mkdir()
    outside.mkdir()
    (runtime / "inside.txt").write_text("inside\n")
    (outside / "outside.txt").write_text("outside\n")
    (runtime / "link").symlink_to(outside)
    (runtime / "file-link.txt").symlink_to(outside / "outside.txt")
    with tempfile.TemporaryDirectory(prefix="cowboy-mods-", dir="/tmp") as sockets:
        socket_path = str(Path(sockets) / "observer.sock")
        server = Observer(socket_path, Recorder)
        server.token, server.events, server.start = secrets.token_hex(16), [], time.monotonic()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        plugin = root / "plugin"
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / "hooks").mkdir()
        (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "permissions-probe", "version": "1.0.0"}))
        (plugin / "hooks/hooks.json").write_text(json.dumps({"modules": ["./register.js"]}))
        (plugin / "hooks/register.js").write_text(MOD)
        (root / "context.json").write_text(json.dumps({"socketPath": socket_path, "token": server.token}))
        steps = calls(outside)
        api = ScriptedApi(steps + [[{"type": "text", "text": "done"}]], native_titles=True)
        env = closed_environment(root / "home")
        env.update(FIXTURE_ENVIRONMENT, ANTHROPIC_BASE_URL=f"http://127.0.0.1:{api.server_port}",
                   COWBOY_CLAUDE_CONTEXT=str(root / "context.json"))
        original = subprocess.Popen

        def spawn(argv, *args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = "Bash,Read,Edit,Write"
            argv[argv.index("--disallowedTools") + 1] = "Skill"
            argv[argv.index("--permission-mode") + 1] = mode
            argv.extend(["--permission-prompt-tool", "stdio"])
            return original(argv, *args, **kwargs)
        asked, client = [], None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(claude), env, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True,
                                extra_arguments=["--plugin-dir", str(plugin)])
            client.permission = lambda request: (asked.append(request.get("tool_use_id")) or
                                                 {"behavior": "allow", "updatedInput": request["input"]})
            client.ready()
            client.prompt(text="go", timeout=120)
            checks = {event["id"]: event for event in server.events if event["kind"] == "check"}
            results = {block["tool_use_id"]: block for request in api.requests for message in request["messages"]
                       if isinstance(message.get("content"), list)
                       for block in message["content"] if block.get("type") == "tool_result"}
            observed = []
            for step in steps:
                tool_use = step[0]["id"]
                check = checks.get(tool_use, {}).get("check")
                result = results.get(tool_use)
                text = None if result is None else json.dumps(result["content"]) if not isinstance(
                    result["content"], str) else result["content"]
                observed.append({"call": describe(step), "check": None if check is None else check.get("decision", check),
                                 "reason": None if check is None else check.get("reason"),
                                 "native_prompted": tool_use in asked,
                                 "result": None if result is None else {"is_error": bool(result.get("is_error")),
                                                                        "text": text.split("\n\n<system-reminder>")[0][:1200]}})
            return {"calls": observed, "outside_after": (outside / "outside.txt").read_text(),
                    "stop_reason": api.requests[-1]["messages"][-1]["role"] if api.requests else None}
        finally:
            if client:
                client.close()
            api.close()
            server.shutdown()
            server.server_close()


def run(claude, modes=MODES):
    receipt = {}
    for mode in modes:
        with tempfile.TemporaryDirectory(prefix="cowboy-native-permissions-", dir=clean_root()) as temp:
            try:
                value = run_mode(claude, Path(temp), mode)
            except Exception as error:  # A failed mode is a recorded observation, not a crash.
                value = {"error": str(error)[:300]}
            receipt[mode] = json.loads(stable(json.dumps(value), temp))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--mode", choices=MODES, action="append")
    args = parser.parse_args()
    if args.receipt.exists():
        parser.error("use a new receipt path")
    args.receipt.write_text(json.dumps(run(args.claude, args.mode or MODES), indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
