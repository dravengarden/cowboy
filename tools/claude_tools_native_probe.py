#!/usr/bin/env python3
"""Record the pinned native CLI's default tool inventory.

Runs the native CLI with its default tool set against a scripted loopback API
(no credentials, no inference) and records every tool name with a digest of
its input schema and of its description. A candidate CLI that adds, removes or
changes a tool shows up in the diff with tools/claude_tools_native_baseline.json;
claude-remote-tools-baseline.test.mjs requires the plugin to classify every
tool (routed, run on the runtime, refused).

Usage: python3 tools/claude_tools_native_probe.py --claude <native cli> --receipt <new path>
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

from claude_mods_native_probe import FIXTURE_ENVIRONMENT
from claude_native_behavior_probe import clean_root, stable
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture
from plugin_runtime_conformance import closed_environment


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()[:16]


def inventory(claude, root):
    runtime = root / "runtime"
    runtime.mkdir()
    api = ScriptedApi([[{"type": "text", "text": "ok"}]], native_titles=True)
    env = closed_environment(root / "home")
    env.update(FIXTURE_ENVIRONMENT, ANTHROPIC_BASE_URL=f"http://127.0.0.1:{api.server_port}")
    original = subprocess.Popen

    def spawn(argv, *args, **kwargs):
        argv = list(argv)
        argv[argv.index("--tools") + 1] = "default"
        argv[argv.index("--disallowedTools") + 1] = ""
        # As the plugin launches it: host prompts over stdio.
        argv.extend(["--permission-prompt-tool", "stdio"])
        return original(argv, *args, **kwargs)
    client = None
    try:
        with patch.object(subprocess, "Popen", spawn):
            client = Claude(str(claude), env, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True)
        client.ready()
        client.prompt(text="hi", timeout=120)
        tools = api.requests[0].get("tools", [])
        return {tool["name"]: {"input_schema": digest(tool.get("input_schema")),
                               "description": digest(tool.get("description"))}
                for tool in sorted(tools, key=lambda tool: tool["name"])
                if not tool["name"].startswith("mcp__")}
    finally:
        if client:
            client.close()
        api.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.receipt.exists():
        parser.error("use a new receipt path")
    with tempfile.TemporaryDirectory(prefix="cowboy-native-tools-", dir=clean_root()) as temp:
        receipt = {"default_tools": json.loads(stable(json.dumps(inventory(args.claude, Path(temp))), temp))}
    args.receipt.write_text(json.dumps(receipt, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
