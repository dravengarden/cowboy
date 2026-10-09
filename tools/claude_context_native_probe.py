#!/usr/bin/env python3
"""Measure native-local session context the remote lane reproduces for the target.

Runs the pinned native CLI against a scripted loopback API (no credentials, no
inference) and records which instruction files native loads at the start and
which it attaches as Reads enter subdirectories (CLAUDE.md, CLAUDE.local.md,
.claude/CLAUDE.md, rules with and without paths, @imports, AGENTS.md, parent,
user and outside directories), how its session context frames Git status
inside and outside a repository, and how it labels context a Mod's tool.call
returns. instructions.mjs and context-mod.js reproduce these for the target.
Diff a fresh receipt with tools/claude_context_native_baseline.json for every
Claude CLI candidate.

Usage: python3 tools/claude_context_native_probe.py --claude <native cli> --receipt <new path>
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile
from unittest.mock import patch

from claude_mods_native_probe import FIXTURE_ENVIRONMENT
from claude_native_behavior_probe import clean_root, stable
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment

FILES = {
    "CLAUDE.md": "ROOT_CLAUDE_MD\n@docs/imported.md\n",
    "docs/imported.md": "IMPORTED_BY_AT\n",
    "CLAUDE.local.md": "ROOT_CLAUDE_LOCAL_MD\n",
    ".claude/CLAUDE.md": "DOT_CLAUDE_CLAUDE_MD\n",
    ".claude/rules/always.md": "RULE_ALWAYS\n",
    ".claude/rules/scoped.md": "---\npaths:\n  - \"sub/**\"\n---\nRULE_SCOPED_SUB\n",
    "AGENTS.md": "ROOT_AGENTS_MD\n",
    "sub/CLAUDE.md": "NESTED_SUB_CLAUDE_MD\n",
    "sub/AGENTS.md": "NESTED_SUB_AGENTS_MD\n",
    "sub/file.txt": "sub file\n",
    "sub2/CLAUDE.md": "NESTED_SUB2_CLAUDE_MD\n",
    "sub3/CLAUDE.md": "NESTED_SUB3_CLAUDE_MD\n",
    "sub3/e.txt": "edit me\n",
}
AGENTS_ONLY = {"AGENTS.md": "ROOT_AGENTS_MD\n", "sub/AGENTS.md": "NESTED_SUB_AGENTS_MD\n", "sub/file.txt": "sub file\n"}
MARKERS = sorted({line for text in FILES.values() for line in text.split("\n") if line.isupper() and "_" in line} |
                 {"PARENT_CLAUDE_MD", "PARENT_AGENTS_MD", "OUTSIDE_OTHER_CLAUDE_MD", "USER_CLAUDE_MD"})

# A Mod that adds context to every Bash call it forwards.
MOD = r'''export function register(on) {
  on("tool.call", async ($, event, next) => {
    const result = await next(event);
    return event.tool === "Bash" ? { ...result, context: ["MOD_TOOL_CONTEXT"] } : result;
  });
}
'''


def markers(value):
    text = json.dumps(value, ensure_ascii=False)
    return [marker for marker in MARKERS if marker in text]


def launch(claude, root, runtime, api, tools, extra_arguments=()):
    env = closed_environment(root / "home")
    (root / "home" / "claude").mkdir(parents=True, exist_ok=True)
    (root / "home" / "claude" / "CLAUDE.md").write_text("USER_CLAUDE_MD\n")
    env.update(FIXTURE_ENVIRONMENT, ANTHROPIC_BASE_URL=f"http://127.0.0.1:{api.server_port}")
    original = subprocess.Popen

    def spawn(argv, *args, **kwargs):
        argv = list(argv)
        argv[argv.index("--tools") + 1] = tools
        argv[argv.index("--disallowedTools") + 1] = "Skill"
        argv[argv.index("--setting-sources") + 1] = "user,project,local"
        return original(argv, *args, **kwargs)
    with patch.object(subprocess, "Popen", spawn):
        return Claude(str(claude), env, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True,
                      extra_arguments=list(extra_arguments))


def discovery(claude, root, files):
    parent = root / "parent"
    runtime = parent / "project"
    runtime.mkdir(parents=True)
    (parent / "CLAUDE.md").write_text("PARENT_CLAUDE_MD\n")
    (parent / "AGENTS.md").write_text("PARENT_AGENTS_MD\n")
    (parent / "other").mkdir()
    (parent / "other/CLAUDE.md").write_text("OUTSIDE_OTHER_CLAUDE_MD\n")
    (parent / "other/f.txt").write_text("other\n")
    for name, text in files.items():
        (runtime / name).parent.mkdir(parents=True, exist_ok=True)
        (runtime / name).write_text(text)
    subprocess.run(["git", "init", "-q", str(runtime)], check=True)
    steps = [("read sub/file.txt", tool("Read", {"file_path": str(runtime / "sub/file.txt")})),
             ("read sub/file.txt again", tool("Read", {"file_path": str(runtime / "sub/file.txt")})),
             ("write sub2/new.txt", tool("Write", {"file_path": str(runtime / "sub2/new.txt"), "content": "x\n"})),
             ("read sub3/e.txt", tool("Read", {"file_path": str(runtime / "sub3/e.txt")})),
             ("read ../other/f.txt", tool("Read", {"file_path": str(parent / "other/f.txt")}))]
    api = ScriptedApi([step for _, step in steps] + [[{"type": "text", "text": "DONE"}]], native_titles=True)
    client = None
    try:
        client = launch(claude, root, runtime, api, "Read,Write")
        client.ready()
        client.prompt(text="go", timeout=60)
        first = api.requests[0]
        return {"start": markers({"system": first.get("system"), "messages": first["messages"]}),
                "after": {label: markers(request["messages"][-1])
                          for (label, _), request in zip(steps, api.requests[1:])}}
    finally:
        if client:
            client.close()
        api.close()


def session_context(claude, root, repository):
    runtime = root / "runtime"
    runtime.mkdir()
    (runtime / "a.txt").write_text("a\n")
    if repository:
        git = ["git", "-C", str(runtime), "-c", "user.name=probe", "-c", "user.email=probe@invalid"]
        subprocess.run(git[:3] + ["init", "-q", "-b", "main"], check=True)
        subprocess.run(git + ["add", "a.txt"], check=True)
        subprocess.run(git + ["commit", "-q", "-m", "first"], check=True)
        (runtime / "b.txt").write_text("b\n")
    api = ScriptedApi([[{"type": "text", "text": "DONE"}]], native_titles=True)
    client = None
    try:
        client = launch(claude, root, runtime, api, "Read")
        client.ready()
        client.prompt(text="go", timeout=60)
        text = json.dumps(api.requests[0], ensure_ascii=False)
        section = re.search(r"# gitStatus\\n(.*?)(?:\\n# |\\n\\nClaude Code attached)", text)
        inside = subprocess.run(["git", "-C", str(runtime), "rev-parse", "--is-inside-work-tree"],
                                capture_output=True, text=True).stdout.strip() == "true"
        # Native puts it in a reminder of the first user message.
        block = re.search(r"# Environment\\n(You have been invoked.*?)\\n</system-reminder>", text)
        environment = None if not block else [
            re.sub(r"(OS Version: ).*", r"\1OS", re.sub(r"(Shell: ).*", r"\1SHELL", line))
            for line in block.group(1).split("\\n")]
        return {"runtime_is_repository": inside, "environment_lines": environment,
                "git_status_header": "# gitStatus\\n" in text,
                "attached_trailer": "Claude Code attached this context automatically" in text,
                "git_status_lines": None if not section else [
                    re.sub(r"[0-9a-f]{7,40}", "COMMIT", line) for line in section.group(1).split("\\n")]}
    finally:
        if client:
            client.close()
        api.close()


def tool_context_label(claude, root):
    runtime = root / "runtime"
    runtime.mkdir()
    plugin = root / "plugin"
    (plugin / ".claude-plugin").mkdir(parents=True)
    (plugin / "hooks").mkdir()
    (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "context-probe", "version": "1.0.0"}))
    (plugin / "hooks/hooks.json").write_text(json.dumps({"modules": ["./register.js"]}))
    (plugin / "hooks/register.js").write_text(MOD)
    api = ScriptedApi([tool("Bash", {"command": "echo hi"}), [{"type": "text", "text": "DONE"}]], native_titles=True)
    client = None
    try:
        client = launch(claude, root, runtime, api, "Bash", ["--plugin-dir", str(plugin)])
        client.ready()
        client.prompt(text="go", timeout=60)
        text = json.dumps(api.requests[-1]["messages"][-1], ensure_ascii=False)
        return {"labeled": "tool.call hook additional context: MOD_TOOL_CONTEXT" in text,
                "present": "MOD_TOOL_CONTEXT" in text}
    finally:
        if client:
            client.close()
        api.close()


def run(claude):
    probes = [("discovery", lambda root: discovery(claude, root, FILES)),
              ("agents_only", lambda root: discovery(claude, root, AGENTS_ONLY)),
              ("git_repository", lambda root: session_context(claude, root, True)),
              ("no_repository", lambda root: session_context(claude, root, False)),
              ("tool_context_label", lambda root: tool_context_label(claude, root))]
    receipt = {}
    for name, probe in probes:
        with tempfile.TemporaryDirectory(prefix="cowboy-native-context-", dir=clean_root()) as temp:
            try:
                value = probe(Path(temp))
            except Exception as error:  # A failed case is a recorded observation, not a crash.
                value = {"error": str(error)[:300]}
            receipt[name] = json.loads(stable(json.dumps(value), temp))
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
