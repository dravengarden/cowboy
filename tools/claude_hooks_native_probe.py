#!/usr/bin/env python3
"""Measure the native-local Claude hook behavior the remote lane reproduces.

Runs the pinned native CLI against a scripted loopback API (no credentials, no
inference) with project settings hooks and records what native does: model-
visible results, permission prompts, stop reasons, hook stdin, matcher rules,
PermissionRequest races, shell-prefix argument passing and agent hook input.
context-mod.js and hook-proxy.mjs reproduce these for target tools; the
packaged "hooks" phase checks the reproduction. Diff a fresh receipt with
tools/claude_hooks_native_baseline.json for every Claude CLI candidate.

Usage: python3 tools/claude_hooks_native_probe.py --claude <native cli> --receipt <new path>
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time
from unittest.mock import patch

from claude_native_behavior_probe import clean_root, stable
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment

FIXTURE_ENVIRONMENT = {"ANTHROPIC_API_KEY": "offline-fixture-not-a-credential", "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
                       "DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"}
FAIL_OUT = {"hookSpecificOutput": {"hookEventName": "PostToolUseFailure", "additionalContext": "FAIL_CONTEXT"}}
ALLOW = {"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}}}


def decide(behavior, delay):
    body = {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                   "decision": {"behavior": behavior, "message": f"{behavior.upper()}_{delay}"}}}
    return f"cat > /dev/null; sleep {delay}; printf '%s' '" + json.dumps(body) + "'"


# name: (event, matcher, hook action(s), appended Bash command, permission mode, host prompt delay).
# Two PermissionRequest decisions arriving together are a native race (2.1.287
# resolved them to deny once and to allow once), so no case pins their order.
TOOL_HOOKS = {
    "pre_exit2": ("PreToolUse", "Bash", "echo PRE_BLOCK_STDERR >&2; exit 2"),
    "pre_exit1": ("PreToolUse", "Bash", "echo PRE_FAIL_STDERR >&2; exit 1"),
    "pre_deny": ("PreToolUse", "Bash", {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                                                                "permissionDecisionReason": "PRE_DENY_REASON"}}),
    "pre_allow_update": ("PreToolUse", "Bash", {"hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "allow",
        "updatedInput": {"command": "printf updated >> ran.txt"}}}),
    "pre_ask": ("PreToolUse", "Bash", {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask",
                                                               "permissionDecisionReason": "PRE_ASK_REASON"}}),
    "pre_legacy_block": ("PreToolUse", "Bash", {"decision": "block", "reason": "LEGACY_BLOCK_REASON"}),
    "pre_context": ("PreToolUse", "Bash", {"hookSpecificOutput": {"hookEventName": "PreToolUse",
                                                                   "additionalContext": "PRE_CONTEXT"}}),
    "pre_stop": ("PreToolUse", "Bash", {"continue": False, "stopReason": "PRE_STOP_REASON"}),
    "pre_plain_stdout": ("PreToolUse", "Bash", "echo PRE_PLAIN_STDOUT"),
    "match_partial": ("PreToolUse", "Bas", "echo MATCHED >&2; exit 2"),
    "match_regex": ("PreToolUse", "B.*h", "echo MATCHED >&2; exit 2"),
    "match_alternation": ("PreToolUse", "Edit|Bash", "echo MATCHED >&2; exit 2"),
    "match_lowercase": ("PreToolUse", "bash", "echo MATCHED >&2; exit 2"),
    "match_star": ("PreToolUse", "*", "echo MATCHED >&2; exit 2"),
    "post_exit2": ("PostToolUse", "Bash", "echo POST_FEEDBACK >&2; exit 2"),
    "post_block": ("PostToolUse", "Bash", {"decision": "block", "reason": "POST_BLOCK_REASON"}),
    "post_context": ("PostToolUse", "Bash", {"hookSpecificOutput": {"hookEventName": "PostToolUse",
                                                                     "additionalContext": "POST_CONTEXT"}}),
    "post_stop": ("PostToolUse", "Bash", {"continue": False, "stopReason": "POST_STOP_REASON"}),
    "post_exit1": ("PostToolUse", "Bash", "echo POST_FAIL >&2; exit 1"),
    "post_stdin": ("PostToolUse", "Bash", "cat > post-stdin.json"),
    "pre_context_then_fail": ("PreToolUse", "Bash", {"hookSpecificOutput": {"hookEventName": "PreToolUse",
                                                                             "additionalContext": "PRE_CONTEXT"}}, "exit 3"),
    "invalid_project_settings": ("PreToolUse", "NoMatch", "true", "true"),
    "bash_nonzero_post": ("PostToolUse", "Bash", "cat > post-stdin.json; echo POST_RAN >&2; exit 2", "exit 3"),
    "bash_nonzero_failure": ("PostToolUseFailure", "Bash",
                             "cat > post-stdin.json; printf '%s' '" + json.dumps(FAIL_OUT) + "'", "echo partial; exit 3"),
    "failure_stop": ("PostToolUseFailure", "Bash", {"continue": False, "stopReason": "FAIL_STOP_REASON"}, "exit 3"),
    "failure_exit2": ("PostToolUseFailure", "Bash", "echo FAIL_FEEDBACK >&2; exit 2", "exit 3"),
    "pre_stop_then_fail": ("PreToolUse", "Bash", {"continue": False, "stopReason": "PRE_STOP_REASON"}, "exit 3"),
    "streams_fail": ("PreToolUse", "NoMatch", "true", "echo OUT1; echo ERR1 >&2; echo OUT2; exit 3"),
    "streams_ok": ("PreToolUse", "NoMatch", "true", "echo OUT1; echo ERR1 >&2; echo OUT2"),
    "silent_fail": ("PreToolUse", "NoMatch", "true", "exit 4"),
    "permreq_allow": ("PermissionRequest", "Bash", ALLOW, "true", "default"),
    "permreq_allow_update": ("PermissionRequest", "Bash", {"hookSpecificOutput": {
        "hookEventName": "PermissionRequest",
        "decision": {"behavior": "allow", "updatedInput": {"command": "printf amended >> ran.txt"}}}}, "true", "default"),
    "permreq_deny": ("PermissionRequest", "Bash", {"hookSpecificOutput": {
        "hookEventName": "PermissionRequest", "decision": {"behavior": "deny", "message": "PERMREQ_DENY"}}},
        "true", "default"),
    "permreq_deny_interrupt": ("PermissionRequest", "Bash", {"hookSpecificOutput": {
        "hookEventName": "PermissionRequest",
        "decision": {"behavior": "deny", "message": "PERMREQ_STOP", "interrupt": True}}}, "true", "default"),
    "permreq_none": ("PermissionRequest", "Bash", "cat > post-stdin.json", "true", "default"),
    "permreq_exit2": ("PermissionRequest", "Bash", "echo PERMREQ_EXIT2 >&2; exit 2", "true", "default"),
    "permreq_nomatch": ("PermissionRequest", "Read", ALLOW, "true", "default"),
    "permreq_allow_slow_host": ("PermissionRequest", "Bash", ALLOW, "true", "default", 4),
    "permreq_fast_allow_slow_deny": ("PermissionRequest", "Bash", [decide("allow", 0), decide("deny", 2)], "true",
                                     "default", 6),
    "permreq_fast_deny_slow_allow": ("PermissionRequest", "Bash", [decide("deny", 0), decide("allow", 2)], "true",
                                     "default", 6),
}


def launch(claude, root, runtime, api, tools, extra_argv=(), environment=None, extra_arguments=(), mode=None):
    env = closed_environment(root / "home")
    env.update(FIXTURE_ENVIRONMENT, ANTHROPIC_BASE_URL=f"http://127.0.0.1:{api.server_port}", **(environment or {}))
    original = subprocess.Popen

    def spawn(argv, *args, **kwargs):
        argv = list(argv)
        argv[argv.index("--tools") + 1] = tools
        argv[argv.index("--disallowedTools") + 1] = "Skill"
        if mode:
            argv[argv.index("--permission-mode") + 1] = mode
        argv.extend(["--settings", str(root / "settings.json"), *extra_argv])
        return original(argv, *args, **kwargs)
    with patch.object(subprocess, "Popen", spawn):
        return Claude(str(claude), env, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True,
                      extra_arguments=list(extra_arguments))


def tool_hook(claude, root, name):
    spec = TOOL_HOOKS[name]
    event, matcher, action = spec[:3]
    bash, mode, delay = (list(spec[3:]) + [None, None, 0][len(spec) - 3:])[:3]
    runtime = root / "runtime"
    runtime.mkdir()
    actions = action if isinstance(action, list) else [action]
    commands = []
    for index, item in enumerate(actions):
        if isinstance(item, dict):
            (root / f"out{index}.json").write_text(json.dumps(item))
            commands.append(f"cat > /dev/null; cat {root}/out{index}.json")
        else:
            commands.append(item if "cat >" in item else "cat > /dev/null; " + item)
    (root / "settings.json").write_text(json.dumps({"hooks": {event: [{"matcher": matcher, "hooks": [
        {"type": "command", "command": command} for command in commands]}]}}))
    if name == "invalid_project_settings":
        (runtime / ".claude").mkdir()
        (runtime / ".claude/settings.json").write_text('{"hooks": {bad')
    api = ScriptedApi([tool("Bash", {"command": "printf original >> ran.txt" + (f"; {bash}" if bash else "")}),
                       [{"type": "text", "text": "AFTER_TOOL"}]], native_titles=True)
    extra = ["--permission-prompt-tool", "stdio"]
    asked, client = [], None
    try:
        client = launch(claude, root, runtime, api, "Bash", extra, mode=mode)
        client.permission = lambda request: (asked.append(request) or time.sleep(delay) or
                                             {"behavior": "allow", "updatedInput": request["input"]})
        client.ready()
        result = client.prompt(text="go", timeout=60)
        last = api.requests[-1]["messages"] if len(api.requests) > 1 else []
        stdin = runtime / "post-stdin.json"
        return {"event": event, "matcher": matcher, "api_requests": len(api.requests),
                "ran": (runtime / "ran.txt").read_text() if (runtime / "ran.txt").exists() else None,
                "asked": [request.get("decision_reason") for request in asked],
                "model_tail": last[-1] if last else None, "result_text": result.get("result"),
                "stop_reason": result.get("stop_reason"),
                "hook_stdin": json.loads(stdin.read_text()) if stdin.exists() else None}
    finally:
        if client:
            client.close()
        api.close()


# Hooks of one event run concurrently: each invocation logs to its own file.
PREFIX = r'''#!/bin/sh
{
  printf 'argc=%s\n' "$#"
  i=0; for a in "$@"; do i=$((i+1)); printf 'arg%s=%s\n' "$i" "$a"; done
  printf 'project_dir=%s\n' "$CLAUDE_PROJECT_DIR"
  printf 'stdin_is_tty=%s\n' "$( [ -t 0 ] && echo yes || echo no)"
} > "$HOOK_LOG.$$"
exec "$@"
'''

# A facade Mod answering Read itself, as context-mod.js does for target tools.
FACADE = r'''export function register(on) {
  on("tool.call", async ($, event, next) => event.tool === "Read"
    ? { result: { type: "text", file: { filePath: "/facade/x", content: "FACADE", numLines: 1, startLine: 1, totalLines: 1 } } }
    : next(event));
}
'''


def shell_prefix(claude, root, prefixed):
    runtime = root / "runtime"
    runtime.mkdir()
    (runtime / "a.txt").write_text("a\n")
    log, hooklog = root / "prefix.log", root / "hooks.log"
    prefix = root / "prefix.sh"
    prefix.write_text(PREFIX)
    prefix.chmod(0o755)
    plugin = root / "plugin"
    (plugin / ".claude-plugin").mkdir(parents=True)
    (plugin / "hooks").mkdir()
    (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "hooks-facade", "version": "1.0.0"}))
    (plugin / "hooks/hooks.json").write_text(json.dumps({"modules": ["./register.js"]}))
    (plugin / "hooks/register.js").write_text(FACADE)

    def hook(name):
        return {"type": "command", "command": f"cat > {hooklog}.{name}.stdin; printf '{name}\\n' >> {hooklog}"}
    (root / "settings.json").write_text(json.dumps({"hooks": {
        "SessionStart": [{"hooks": [hook("SessionStart"), {"type": "command", "command":
                                                           "printf '%s' \"${CLAUDE_PROJECT_DIR}\" > /dev/null # BRACED"},
                                    {"type": "command", "command": "printf", "args": ["%s", "${CLAUDE_PROJECT_DIR}",
                                                                                      "EXEC_FORM"]}]}],
        "UserPromptSubmit": [{"hooks": [hook("UserPromptSubmit")]}],
        "PreToolUse": [{"matcher": "Bash|Read", "hooks": [hook("PreToolUse")]}],
        "PostToolUse": [{"matcher": "Bash|Read", "hooks": [hook("PostToolUse")]}],
        "Stop": [{"hooks": [hook("Stop")]}],
    }}))
    api = ScriptedApi([tool("Read", {"file_path": "a.txt"}), tool("Bash", {"command": "echo bash-ran"}),
                       [{"type": "text", "text": "done"}]], native_titles=True)
    environment = {"HOOK_LOG": str(log)} | ({"CLAUDE_CODE_SHELL_PREFIX": str(prefix)} if prefixed else {})
    client = None
    try:
        client = launch(claude, root, runtime, api, "Bash,Read", environment=environment,
                        extra_arguments=["--plugin-dir", str(plugin)])
        client.ready()
        client.prompt(text="go", timeout=60)
        time.sleep(1)
        stdins = {path.name.split(".")[-2]: json.loads(path.read_text() or "null") for path in root.glob("hooks.log.*.stdin")}
        return {"hooks_ran": hooklog.read_text().split() if hooklog.exists() else [],
                "prefix_invocations": sorted(path.read_text() for path in root.glob("prefix.log.*")),
                "stdin_keys": {key: sorted(value) if isinstance(value, dict) else value for key, value in sorted(stdins.items())},
                "api_requests": len(api.requests)}
    finally:
        if client:
            client.close()
        api.close()


CHILD = "CHILD_HOOK_PROBE run the command"


def agent_hooks(claude, root, background):
    runtime = root / "runtime"
    runtime.mkdir()
    capture = f"cat >> {root}/hooks.jsonl; echo >> {root}/hooks.jsonl"
    (root / "settings.json").write_text(json.dumps({"hooks": {
        "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": capture}]}],
        "SubagentStop": [{"hooks": [{"type": "command", "command": capture}]}],
    }}))

    def router(requests):
        request = requests[-1]
        last = request["messages"][-1]
        results = isinstance(last.get("content"), list) and any(
            block.get("type") == "tool_result" for block in last["content"])
        if any(CHILD in json.dumps(message.get("content")) for message in request.get("messages", [])[:1]):
            return [{"type": "text", "text": "CHILD_DONE"}] if results else \
                tool("Bash", {"command": "printf child >> ran.txt"})
        # Launch once: a background child's notification starts another turn.
        if results or "Hook child" in json.dumps(request["messages"][:-1]):
            return [{"type": "text", "text": "PARENT_DONE"}]
        return tool("Agent", {"description": "Hook child", "prompt": CHILD, "subagent_type": "general-purpose",
                              "run_in_background": background})
    api = ScriptedApi([router] * 12, native_titles=True)
    client = None
    try:
        client = launch(claude, root, runtime, api, "Bash,Agent")
        client.ready()
        client.prompt(text="go", timeout=60)
        if background:
            time.sleep(5)
        lines = [json.loads(line) for line in (root / "hooks.jsonl").read_text().splitlines() if line.strip()] \
            if (root / "hooks.jsonl").exists() else []
        return {"ran": (runtime / "ran.txt").read_text() if (runtime / "ran.txt").exists() else None,
                "hooks": [{key: value for key, value in sorted(line.items()) if key != "tool_response"}
                          for line in lines]}
    finally:
        if client:
            client.close()
        api.close()


def run(claude):
    probes = [(f"tool_hooks.{name}", lambda root, name=name: tool_hook(claude, root, name)) for name in TOOL_HOOKS]
    probes += [("shell_prefix.plain", lambda root: shell_prefix(claude, root, False)),
               ("shell_prefix.prefixed", lambda root: shell_prefix(claude, root, True)),
               ("agent_hooks.foreground", lambda root: agent_hooks(claude, root, False)),
               ("agent_hooks.background", lambda root: agent_hooks(claude, root, True))]
    receipt = {}
    for key, probe in probes:
        group, name = key.split(".")
        with tempfile.TemporaryDirectory(prefix="cowboy-native-hooks-", dir=clean_root()) as temp:
            try:
                value = probe(Path(temp))
            except Exception as error:  # A failed case is a recorded observation, not a crash.
                value = {"error": str(error)[:300]}
            receipt.setdefault(group, {})[name] = json.loads(stable(json.dumps(value), temp))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.receipt.exists():
        parser.error("use a new receipt path")
    args.receipt.write_text(json.dumps(run(args.claude), indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
