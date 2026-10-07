#!/usr/bin/env python3
"""Native-local file tool results and on-disk effects for the shared cases.

Runs pinned native Claude with its own Read, Write and Edit against a
scripted loopback API; refresh tools/claude_file_native_baseline.json from it
for a candidate CLI. No model or production credentials are used.

Usage (dev shell): PYTHONPATH=tools python3 tools/claude_file_native_probe.py
  --claude /absolute/pinned/claude --receipt /absolute/new-receipt.json
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

from claude_file_cases import CASES, effects, normalize, setup
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment


def run(claude):
    with tempfile.TemporaryDirectory(prefix="cowboy-file-baseline-") as temp:
        root = Path(temp)
        runtime = root / "runtime"
        runtime.mkdir()
        inodes = setup(runtime)
        steps = [tool(name, arguments) for _, (name, arguments) in CASES] + [[{"type": "text", "text": "FILES_DONE"}]]
        ids = {call[0]["id"]: case for (case, _), call in zip(CASES, steps)}
        api = ScriptedApi(steps, native_titles=True)
        environment = closed_environment(root / "home")
        environment.update({
            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
            "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "DISABLE_AUTOUPDATER": "1",
            "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
        })
        original = subprocess.Popen

        def spawn(argv, *args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = "Read,Write,Edit"
            argv[argv.index("--disallowedTools") + 1] = "Skill"
            return original(argv, *args, **kwargs)

        client = None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(claude), environment, runtime, WorkspaceFixture(runtime),
                                aliases=False, bound_native=True)
            client.ready()
            client.prompt(text="go", timeout=180)
            results = {}
            for message in api.requests[-1]["messages"]:
                for block in message["content"] if isinstance(message.get("content"), list) else []:
                    if block.get("type") == "tool_result" and block.get("tool_use_id") in ids:
                        content = block.get("content")
                        if isinstance(content, list):
                            content = "".join(item.get("text", "") for item in content)
                        results[ids[block["tool_use_id"]]] = {
                            "is_error": block.get("is_error", False),
                            "content": normalize(content, runtime) if isinstance(content, str) else content,
                        }
            return {"results": results, "effects": effects(runtime, inodes)}
        finally:
            if client:
                client.close()
            api.close()


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
