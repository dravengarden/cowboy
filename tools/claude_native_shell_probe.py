#!/usr/bin/env python3
"""Research native Bash background tasks across a separate native executor.

Same-host, shared-filesystem experiment only. The whole native shell envelope
is forwarded deliberately to expose its runtime-path coupling. Not a shipping
remote shell implementation; cancellation and reconnect are not implemented.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import tempfile
from unittest.mock import patch

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from execution_environment_probe import require
from plugin_runtime_conformance import closed_environment


def probe(claude, executor):
    with tempfile.TemporaryDirectory(prefix="cowboy-native-shell-research-") as temporary:
        root = Path(temporary)
        runtime, target = root / "runtime", root / "target"
        runtime.mkdir()
        target.mkdir()
        target_environment = closed_environment(root / "target-home")
        helper = root / "prefix.py"
        # All interpolated fields are fixture-owned literal paths, not model input.
        helper.write_text(
            "import json,os,sys\n"
            f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
            "from execution_environment_probe import Executor\n"
            f"client=Executor([{str(executor)!r},'exec-server','--listen','stdio'],20,environment={target_environment!r},cwd={str(target)!r})\n"
            "try:\n"
            " client.request('initialize',{'clientName':'cowboy-shell-research'})\n"
            " client.send({'method':'initialized','params':{}})\n"
            f" job=client.start(['/run/current-system/sw/bin/bash','-c',sys.argv[1]],{target.as_uri()!r})\n"
            " result,output=client.collect(job)\n"
            " sys.stdout.buffer.write(output['stdout']);sys.stdout.buffer.flush()\n"
            " sys.stderr.buffer.write(output['stderr']);sys.stderr.buffer.flush()\n"
            " code=result['exitCode']\n"
            "finally: client.close()\n"
            "sys.exit(code if isinstance(code,int) else 125)\n"
        )
        wrapper = root / "prefix"
        wrapper.write_text("#!/run/current-system/sw/bin/bash\nexec "
                           + shlex.quote(sys.executable) + " " + shlex.quote(str(helper)) + ' "$@"\n')
        wrapper.chmod(0o700)
        api = ScriptedApi([tool("Bash", {
            "command": "sleep 1; printf target-proof > native-marker.txt; printf complete",
            "run_in_background": True,
        })], native_titles=True)
        environment = closed_environment(root / "runtime-home")
        environment.update({"CLAUDE_CODE_SHELL_PREFIX": str(wrapper),
                            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
                            "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
                            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                            "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"})
        original_spawn = subprocess.Popen

        def spawn(argv, *args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = "Bash"
            return original_spawn(argv, *args, **kwargs)

        with patch.object(subprocess, "Popen", spawn):
            client = Claude(str(claude), environment, runtime, WorkspaceFixture(target),
                            aliases=False, bound_native=True)
        try:
            client.ready()
            client.prompt()
            completion = client.until(lambda f: f.get("type") == "system"
                                      and f.get("subtype") == "task_notification", timeout=25)
            require(completion.get("status") == "completed", "native background job failed")
            require((target / "native-marker.txt").read_text() == "target-proof", "missing target effect")
            require(not (runtime / "native-marker.txt").exists(), "effect leaked to runtime")
            output = Path(completion["output_file"]).read_text()
            require("complete" in output, "native task output lost executor output")
            require(api.failure is None and len(api.requests) == 2, "unexpected API calls")
            return {"target_effect": True, "runtime_effect": False,
                    "native_task_completion": True, "native_task_output": True,
                    "scripted_api_requests": 2}
        finally:
            client.close()
            api.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("claude", "executor", "receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--claude-sha256", required=True)
    parser.add_argument("--executor-sha256", required=True)
    args = parser.parse_args()
    require(os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net"), "new network namespace required")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "only loopback may exist")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    for path, expected in ((args.claude, args.claude_sha256), (args.executor, args.executor_sha256)):
        require(path.is_absolute(), "absolute binary path required")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, "binary digest differs")
    result = {"schema": "cowboy.claude-native-shell-research/v1",
              "claude_sha256": args.claude_sha256, "executor_sha256": args.executor_sha256,
              "topology": "same-host separate native executor, shared filesystem",
              "real_model_requests": 0, "production_credentials": False,
              "not_checked": ["cross_host", "cancellation", "reconnect", "permissions", "autonomous_model_continuation"]}
    result.update(probe(args.claude, args.executor))
    with args.receipt.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
