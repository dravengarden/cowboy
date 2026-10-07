#!/usr/bin/env python3
"""Native-local Bash process lifetimes for the shared lifecycle cases.

Runs pinned native Claude with its own Bash and TaskStop against a scripted
loopback API; refresh tools/claude_lifecycle_native_baseline.json from it for a
candidate CLI. No model or production credentials are used.

Usage (dev shell): PYTHONPATH=tools python3 tools/claude_lifecycle_native_probe.py
  --claude /absolute/pinned/claude --receipt /absolute/new-receipt.json
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time
from unittest.mock import patch

from claude_lifecycle_cases import CASES, STOP, alive, left_running_notified, normalize, stop_all, task_id
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment


def run(claude):
    with tempfile.TemporaryDirectory(prefix="cowboy-lifecycle-baseline-") as temp:
        root = Path(temp)
        runtime = root / "runtime"
        runtime.mkdir()
        ids, tasks, raw = {}, [], {}

        def router(requests):
            last = requests[-1]["messages"][-1]
            for block in last.get("content") if isinstance(last.get("content"), list) else []:
                if block.get("type") == "tool_result" and block.get("tool_use_id") in ids:
                    content = block.get("content")
                    if isinstance(content, list):
                        content = "".join(item.get("text", "") for item in content if item.get("type") == "text")
                    raw[ids[block["tool_use_id"]]] = (block.get("is_error", False), content)
                    if task_id(content):
                        tasks.append(task_id(content))
            index = len(raw)
            if index >= len(CASES):
                return [{"type": "text", "text": "LIFECYCLE_DONE"}]
            case, (name, arguments) = CASES[index]
            call = tool(name, {"task_id": tasks[-1]} if name == STOP else arguments)
            ids[call[0]["id"]] = case
            return call

        api = ScriptedApi([router] * (len(CASES) + 10), native_titles=True)
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
            argv[argv.index("--tools") + 1] = "Bash,TaskStop"
            argv[argv.index("--disallowedTools") + 1] = "Skill"
            return original(argv, *args, **kwargs)

        client = None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(claude), environment, runtime, WorkspaceFixture(runtime),
                                aliases=False, bound_native=True)
            client.ready()
            client.prompt(text="Run the lifecycle parity fixture.", timeout=180)
            time.sleep(2)
            processes = alive(runtime)
            notified = left_running_notified(api.requests)
        finally:
            if client:
                client.close()
            api.close()
            stop_all(runtime)
        results = {case: {"is_error": error, "content": normalize(content, runtime, tasks)}
                   for case, (error, content) in raw.items()}
        return {"results": results, "alive": processes, "left_running_notified": notified}


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
