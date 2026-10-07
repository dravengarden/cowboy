#!/usr/bin/env python3
"""Native-local Bash results for the shared parity cases.

Runs pinned native Claude with its own Bash against a scripted loopback API and
writes normalized results; refresh tools/claude_shell_native_baseline.json from
it for a candidate CLI. No model or production credentials are used.

Usage (dev shell): PYTHONPATH=tools python3 tools/claude_shell_native_probe.py
  --claude /absolute/pinned/claude --receipt /absolute/new-receipt.json
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

from claude_shell_cases import CASES, normalize
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment


def run(claude):
    """The calls run in order in one turn, so shell state carries as natively."""
    with tempfile.TemporaryDirectory(prefix="cowboy-shell-baseline-") as temp:
        root = Path(temp)
        runtime = root / "runtime"
        (runtime / "sub").mkdir(parents=True)
        steps = [tool("Bash", arguments) for _, arguments in CASES] + [[{"type": "text", "text": "SHELL_DONE"}]]
        ids = {call[0]["id"]: name for (name, _), call in zip(CASES, steps)}
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
            argv[argv.index("--tools") + 1] = "Bash,TaskOutput,TaskStop"
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
            return {"requests": len(api.requests), "results": results}
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
