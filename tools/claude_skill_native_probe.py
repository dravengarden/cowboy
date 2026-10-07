#!/usr/bin/env python3
"""Native-local Skill results and skill listing for the shared cases.

Runs pinned native Claude with project settings, its own Skill and Bash tools
and the fixture project's .claude directory against a scripted loopback API;
refresh tools/claude_skill_native_baseline.json from it for a candidate CLI.
No model or production credentials are used.

Usage: PYTHONPATH=tools python3 tools/claude_skill_native_probe.py
  --claude /absolute/pinned/claude --receipt /absolute/new-receipt.json
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

from claude_skill_cases import CASES, contents, listing, normalize, setup
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment


def run(claude):
    with tempfile.TemporaryDirectory(prefix="cowboy-skill-baseline-") as temp:
        root = Path(temp)
        project = root / "project"
        (project / ".git").mkdir(parents=True)
        # Native's user directory is CLAUDE_CONFIG_DIR (closed_environment).
        user = root / "home" / "claude"
        setup(project, user)
        steps = [tool("Skill", arguments) for _, arguments in CASES] + [[{"type": "text", "text": "SKILL_DONE"}],
                                                                       [{"type": "text", "text": "TYPED_DONE"}]]
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
            argv[argv.index("--tools") + 1] = "Skill,Bash"
            argv[argv.index("--disallowedTools") + 1] = "Agent"
            argv[argv.index("--setting-sources") + 1] = "user,project,local"
            kwargs["cwd"] = str(project)
            return original(argv, *args, **kwargs)

        client = None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(claude), environment, project, WorkspaceFixture(project),
                                aliases=False, bound_native=True)
            client.ready()
            client.prompt(text="go", timeout=300)
            results = {}
            for message in api.requests[-1]["messages"]:
                for block in message["content"] if isinstance(message.get("content"), list) else []:
                    if block.get("type") == "tool_result" and block.get("tool_use_id") in ids:
                        results[ids[block["tool_use_id"]]] = {
                            "is_error": block.get("is_error", False),
                            "content": normalize(block.get("content"), project, user),
                            "follows": contents(message, project, user),
                        }
            shown = next(block["text"] for block in api.requests[0]["messages"][0]["content"]
                         if "The following skills are available" in block.get("text", ""))
            client.prompt(text="/grp:inner typed-arg", timeout=120)
            typed = [normalize(block["text"], project, user) for block in api.requests[-1]["messages"][-1]["content"]
                     if block.get("type") == "text" and not block["text"].startswith("<system-reminder>")]
            return {"results": results, "listing": listing(shown), "typed": typed}
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
