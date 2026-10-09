#!/usr/bin/env python3
"""Native-local behaviors the remote Claude lane reproduces, in one baseline.

Runs pinned native Claude against a scripted loopback API and records:
tool descriptions, background deadlines (explicit and moved commands), Bash
stdin and terminal state, MCP scopes and precedence, and where a background
agent's background command is notified. Refresh
tools/claude_native_behavior_baseline.json from it for a candidate CLI; the
unit tests compare the plugin's own native strings and rules with it. No
model or production credentials are used.

Usage (dev shell): PYTHONPATH=tools python3 tools/claude_native_behavior_probe.py
  --claude /absolute/pinned/claude --receipt /absolute/new-receipt.json
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from unittest.mock import patch

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment

DESCRIBED = ["Bash", "Read", "Write", "Edit", "NotebookEdit", "TaskStop"]
MCP_SERVER = r'''
import json, os, sys
name = sys.argv[1]
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("id") is None:
        continue
    method = msg.get("method")
    if method == "initialize":
        result = {"protocolVersion": msg["params"].get("protocolVersion", "2025-06-18"),
                  "capabilities": {"tools": {}}, "serverInfo": {"name": name, "version": "1"},
                  "instructions": name + " instructions"}
    elif method == "tools/list":
        result = {"tools": [{"name": "where", "description": "where",
                             "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": json.dumps({
            "server": name, "cwd": os.getcwd(), "argv": sys.argv[2:], "env": os.environ.get("FIXTURE")})}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "no"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
'''


class Session:
    """One native process in a fresh project with the given tools and steps."""

    def __init__(self, claude, root, tools, steps, sources="", extra=(), environment=None, cwd=None):
        self.root = root
        self.project = cwd or root / "project"
        self.project.mkdir(parents=True, exist_ok=True)
        self.api = ScriptedApi(steps, native_titles=True)
        env = closed_environment(root / "home")
        env.update({"ANTHROPIC_BASE_URL": f"http://127.0.0.1:{self.api.server_port}",
                    "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
                    "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "DISABLE_AUTOUPDATER": "1",
                    "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1", **(environment or {})})
        original = subprocess.Popen

        def spawn(argv, *args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = tools
            argv[argv.index("--disallowedTools") + 1] = "AskUserQuestion"
            argv[argv.index("--setting-sources") + 1] = sources
            if sources:
                argv.remove("--strict-mcp-config")
            kwargs["cwd"] = str(self.project)
            return original(argv + list(extra), *args, **kwargs)

        with patch.object(subprocess, "Popen", spawn):
            self.client = Claude(str(claude), env, self.project, WorkspaceFixture(self.project),
                                 aliases=False, bound_native=True)
        self.client.ready()

    def pump(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                self.client.until(lambda frame: frame.get("type") == "result", timeout=2)
            except Exception:
                pass

    def normalize(self, value):
        return json.loads(stable(json.dumps(value), str(self.root)))

    def close(self):
        self.client.close()
        self.api.close()


def stable(text, root):
    """Replace per-run values (root, its session-directory key, ids) so receipts of one binary compare equal."""
    text = text.replace(root, "<ROOT>").replace(re.sub(r"[^A-Za-z0-9]", "-", root), "-ROOT")
    text = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "SESSION", text)
    tasks = {}
    for found in re.findall(r"(?:ID: |<task-id>|tasks/)([a-z0-9]{6,})\b", text):
        tasks.setdefault(found, "task%d" % (len(tasks) + 1))
    for found, name in tasks.items():
        text = re.sub(r"\b%s\b" % found, name, text)
    uses = {}
    for found in re.findall(r"toolu_[A-Za-z0-9]+", text):
        uses.setdefault(found, "toolu_%d" % (len(uses) + 1))
    for found, name in uses.items():
        text = text.replace(found, name)
    agents = {}
    for found in re.findall(r"\ba[0-9a-f]{16}\b", text):
        agents.setdefault(found, "agent%d" % (len(agents) + 1))
    for found, name in agents.items():
        text = re.sub(r"\b%s\b" % found, name, text)
    text = re.sub(r"snapshot-bash-\d+-[a-z0-9]+\.sh", "snapshot-bash-SNAPSHOT.sh", text)
    text = re.sub(r"claude-[0-9a-f]{4}-cwd", "claude-CWD-cwd", text)
    return re.sub(r'"duration_ms": \d+', '"duration_ms": 0', text)


def results(api):
    found = {}
    for request in api.requests:
        for message in request["messages"]:
            for block in message["content"] if isinstance(message.get("content"), list) else []:
                if block.get("type") == "tool_result":
                    content = block.get("content")
                    found[block["tool_use_id"]] = content if isinstance(content, str) else content[0].get("text")
    return found


def notifications(api):
    return [block["text"] for request in api.requests for message in request["messages"]
            for block in (message["content"] if isinstance(message.get("content"), list) else [])
            if "<task-notification>" in block.get("text", "")]


def descriptions(claude, root):
    session = Session(claude, root, "default", [[{"type": "text", "text": "DONE"}]])
    try:
        session.client.prompt(text="go", timeout=120)
        tools = {item["name"]: item.get("description") for item in session.api.requests[0]["tools"]}
        return session.normalize({name: tools[name] for name in DESCRIBED})
    finally:
        session.close()


def deadlines_and_stdin(claude, root):
    explicit = tool("Bash", {"command": "echo start; sleep 12", "run_in_background": True, "timeout": 3000})
    moved = tool("Bash", {"command": "echo moved; sleep 14", "timeout": 2000})
    stdin = tool("Bash", {"command": "test -t 0 && echo in-tty || echo in-notty; test -t 1 && echo out-tty || "
                                    "echo out-notty; read -t 2 x; echo read-status:$?; readlink /proc/self/fd/0"})
    session = Session(claude, root, "Bash,TaskStop",
                      [stdin, explicit, moved, [{"type": "text", "text": "T"}]] + [[{"type": "text", "text": "N"}]] * 6)
    try:
        session.client.prompt(text="go", timeout=120)
        session.pump(25)
        found = results(session.api)
        stopped = next(note for note in notifications(session.api) if "<status>killed</status>" in note)
        return session.normalize({
            "stdin": found[stdin[0]["id"]].split("\n\n<system-reminder>")[0],
            "explicit_result": found[explicit[0]["id"]].split("\n\n<system-reminder>")[0],
            "moved_result": found[moved[0]["id"]].split("\n\n<system-reminder>")[0],
            "deadline_notification": stopped,
        })
    finally:
        session.close()


def mcp(claude, root):
    project = root / "project"
    (project / ".git").mkdir(parents=True)
    (project / "sub").mkdir()
    server_file = root / "server.py"
    server_file.write_text(MCP_SERVER)

    def server(name):
        return {"command": sys.executable, "args": [str(server_file), name, "${FIXTURE:-dflt}", "${UNSET_FIXTURE:-d2}"],
                "env": {"FIXTURE": "${FIXTURE}-x"}}
    (project / ".mcp.json").write_text(json.dumps({"mcpServers": {
        "projsrv": server("projsrv"), "dup": server("dup-project"), "hidden": server("hidden")}}))
    config = root / "home" / "claude"
    config.mkdir(parents=True)
    (config / ".claude.json").write_text(json.dumps({
        "mcpServers": {"usersrv": server("usersrv"), "dup": server("dup-user"), "off": server("off")},
        "projects": {str(project): {"mcpServers": {"localsrv": server("localsrv"), "dup": server("dup-local")},
                                    "disabledMcpjsonServers": ["hidden"], "disabledMcpServers": ["off"]}}}))
    calls = {name: tool(f"mcp__{name}__where", {}) for name in ["projsrv", "usersrv", "localsrv", "dup"]}
    session = Session(claude, root, "default", list(calls.values()) + [[{"type": "text", "text": "DONE"}]],
                      sources="user,project,local", environment={"FIXTURE": "fx"}, cwd=project / "sub")
    try:
        session.client.prompt(text="go", timeout=120)
        found = results(session.api)
        init = next(frame for frame in session.client.messages if frame.get("subtype") == "init")
        listed = sorted(item["name"] for item in session.api.requests[0]["tools"] if item["name"].startswith("mcp__"))
        return session.normalize({
            "servers": sorted([item["name"], item["status"], item.get("source")] for item in init["mcp_servers"]),
            "tools": listed,
            "calls": {name: json.loads(found[call[0]["id"]]) for name, call in calls.items()},
        })
    finally:
        session.close()


def agent_background(claude, root):
    """Where a background agent's background command is notified."""
    seen = []

    def router(requests):
        request = requests[-1]
        child = "AGENT_CHILD" in json.dumps(request["messages"][0])
        last = json.dumps(request["messages"][-1])
        seen.append({"child": child, "notified": "<task-notification>" in last})
        if child:
            if len(request["messages"]) == 1:
                return tool("Bash", {"command": "sleep 2; echo child-bg", "run_in_background": True})
            if "<task-notification>" in last or "waited" in last:
                return [{"type": "text", "text": "CHILD_DONE"}]
            return tool("Bash", {"command": "sleep 5; echo waited"})
        if len(request["messages"]) == 1:
            return tool("Agent", {"description": "child", "prompt": "AGENT_CHILD", "subagent_type": "general-purpose",
                                  "run_in_background": True})
        return [{"type": "text", "text": "PARENT"}]
    session = Session(claude, root, "Agent,Bash", [router] * 30)
    try:
        session.client.prompt(text="go", timeout=120)
        session.pump(20)
        return {"child_notified": any(item["child"] and item["notified"] for item in seen),
                "parent_notified": any(not item["child"] and item["notified"] for item in seen)}
    finally:
        session.close()


def run(claude):
    baseline = {}
    for name, probe in [("tool_descriptions", descriptions), ("bash", deadlines_and_stdin), ("mcp", mcp),
                        ("agent_background", agent_background)]:
        with tempfile.TemporaryDirectory(prefix=f"cowboy-native-{name}-") as temp:
            baseline[name] = probe(claude, Path(temp))
    return baseline


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.receipt.exists():
        raise SystemExit("new receipt required")
    args.receipt.write_text(json.dumps(run(args.claude), indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
