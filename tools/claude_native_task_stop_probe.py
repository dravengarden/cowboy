#!/usr/bin/env python3
"""Native TaskStop research with local, transient and resident executors.

The signal-forwarding wrapper is an experiment, not a shipping remote shell.
Shared paths, full access and loopback fixtures do not establish fleet parity.
"""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import time
from unittest.mock import patch

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from execution_environment_probe import require
from execution_keeper_conformance import Keeper, identity, process_arguments
from plugin_runtime_conformance import closed_environment, stop_group


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def process_identity(path):
    try:
        pid = path.read_text()
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return (pid, fields[19]) if fields[0] != "Z" else None
    except FileNotFoundError:
        return None


def wait_file(path):
    deadline = time.monotonic() + 10
    while not path.exists() and time.monotonic() < deadline:
        time.sleep(.01)
    require(path.exists(), f"fixture did not admit {path.name}")


@contextmanager
def resident(args, root, target, runtime, environment):
    state = root / "keeper-state"
    state.mkdir(mode=0o700)
    contract = {
        "schema": 1, "session_id": "claude-stop-fixture", "binding": {
            "schema": 1, "id": "binding-fixture", "revision": 1,
            "runtime": {"machine_id": "ovh", "cwd": str(runtime)},
            "environment": {"machine_id": "hawk", "id": "environment-fixture", "incarnation": identity(),
                            "executor_digest": "sha256:" + args.executor_sha256, "protocol": 1},
            "workspace": {"id": "fixture", "worktree_id": "fixture-tree", "source_path": str(target),
                          "cwd": str(target)}, "access": "project",
        },
        "executor": {"command": str(args.executor), "sha256": args.executor_sha256, "version": "0.159.3"},
        "capability": identity() + identity(),
        "environment": {"HOME": str(root / "target-home"), "PATH": "/run/current-system/sw/bin",
                        "SHELL": "/run/current-system/sw/bin/bash"},
    }
    path = root / "contract.json"
    path.write_text(json.dumps(contract))
    path.chmod(0o600)
    with tempfile.TemporaryFile() as log:
        daemon = subprocess.Popen([str(args.keeper), "--contract", str(path), "--state-dir", str(state)],
                                  env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                  start_new_session=True)
        client = Keeper(state / "keeper.sock", contract)
        try:
            for _ in range(100):
                require(daemon.poll() is None, "keeper exited during startup")
                try:
                    require(client.request({"kind": "describe"})["kind"] == "ready", "keeper not ready")
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("keeper readiness timed out")
            yield daemon, client, state / "keeper.sock", contract
        finally:
            stop_group(daemon)


def prefix_source(args, target, environment, keeper_data, forward):
    imports = f"import sys\nsys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
    if keeper_data is None:
        return imports + (
            "from execution_environment_probe import Executor\n"
            f"client=Executor([{str(args.executor)!r},'exec-server','--listen','stdio'],20,environment={environment!r},cwd={str(target)!r})\n"
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
    _, _, path, contract = keeper_data
    # Native TaskStop signals the wrapper. The explicit request targets only
    # this admitted job. This deliberately does not solve pending-start races.
    signals = (
        "def cancel(signum, frame):\n"
        " client.result('process/terminate',{'processId':job})\n"
        " sys.exit(128+signum)\n"
        "signal.signal(signal.SIGTERM,cancel);signal.signal(signal.SIGINT,cancel)\n"
    ) if forward else ""
    return imports + (
        "import base64,signal\nfrom pathlib import Path\n"
        "from execution_keeper_conformance import Keeper,identity,process_arguments\n"
        f"client=Keeper({str(path)!r},{contract!r})\njob=identity()\n"
        f"client.result('process/start',process_arguments(Path({str(target)!r}),job,sys.argv[1]))\n"
    ) + signals + (
        "seq=None\nwhile True:\n"
        " result=client.result('process/read',{'processId':job,'afterSeq':seq,'maxBytes':32768,'waitMs':1000})\n"
        " for chunk in result['chunks']:\n"
        "  seq=chunk['seq'];stream=sys.stdout if chunk['stream']=='stdout' else sys.stderr\n"
        "  stream.buffer.write(base64.b64decode(chunk['chunk']));stream.buffer.flush()\n"
        " if result['closed']: sys.exit(result['exitCode'] if isinstance(result['exitCode'],int) else 125)\n"
    )


def exercise(args, mode, root, runtime, target, environment, keeper_data):
    effect = runtime if mode == "local" else target
    other = target if mode == "local" else runtime
    originals = {}
    task_id = stop_call = None
    acknowledged = None
    alive_at_ack = None
    peer = None
    if keeper_data:
        keeper_data[1].result("process/start", process_arguments(target, identity(),
                             'printf "%s" "$$" > peer-pid; exec sleep 90'))
        wait_file(target / "peer-pid")
        peer = process_identity(target / "peer-pid")
        require(peer is not None, "independent keeper job not admitted")
    helper = root / "prefix.py"
    helper.write_text(prefix_source(args, target, environment, keeper_data, mode == "keeper-forward"))
    wrapper = root / "prefix"
    wrapper.write_text("#!/run/current-system/sw/bin/bash\nexec " + shlex.quote(sys.executable) + " " +
                       shlex.quote(str(helper)) + ' "$@"\n')
    wrapper.chmod(0o700)

    def alive(name):
        return originals.get(name) is not None and process_identity(effect / name) == originals[name]

    def results(requests):
        return [block for message in requests[-1]["messages"] if isinstance(message.get("content"), list)
                for block in message["content"] if block.get("type") == "tool_result"]

    def stop_task(requests):
        nonlocal task_id, stop_call
        match = re.search(r"ID: ([a-zA-Z0-9_-]+)", json.dumps(results(requests)))
        require(match is not None, "native background task ID missing")
        task_id = match[1]
        wait_file(effect / "started")
        for name in ("shell-pid", "child-pid"):
            originals[name] = process_identity(effect / name)
            require(alive(name), "expected positively admitted live process")
        require(not (effect / "late").exists(), "effect preceded stop request")
        response = tool("TaskStop", {"task_id": task_id})
        stop_call = response[0]["id"]
        return response

    def confirm_stop(requests):
        nonlocal acknowledged, alive_at_ack
        outputs = [block for block in results(requests) if block.get("tool_use_id") == stop_call]
        require(len(outputs) == 1 and not outputs[0].get("is_error"), "TaskStop failed")
        body, _ = json.JSONDecoder().raw_decode(outputs[0]["content"].lstrip())
        require(body.get("task_id") == task_id and "Successfully stopped" in body.get("message", ""),
                "TaskStop did not acknowledge the original task")
        require(not (effect / "late").exists(), "effect preceded stop acknowledgement")
        acknowledged = time.monotonic()
        alive_at_ack = {name: alive(name) for name in ("shell-pid", "child-pid")}
        return [{"type": "text", "text": "fixture complete"}]

    api = ScriptedApi([tool("Bash", {
        "command": 'printf "%s" "$$" > shell-pid; (sleep 4; printf late > late; sleep 60) & '
                   'printf "%s" "$!" > child-pid; printf started > started; wait',
        "run_in_background": True,
    }), stop_task, confirm_stop], native_titles=True)
    runtime_env = closed_environment(root / "runtime-home")
    runtime_env.update({"ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
                        "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
                        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                        "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"})
    if mode != "local":
        runtime_env["CLAUDE_CODE_SHELL_PREFIX"] = str(wrapper)
    spawn = subprocess.Popen

    def native_spawn(argv, *a, **kw):
        argv = list(argv)
        argv[argv.index("--tools") + 1] = "Bash,TaskStop"
        return spawn(argv, *a, **kw)

    client = None
    try:
        with patch.object(subprocess, "Popen", native_spawn):
            client = Claude(str(args.claude), runtime_env, runtime, WorkspaceFixture(target),
                            aliases=False, bound_native=True)
        client.ready()
        client.prompt()
        require(api.failure is None and len(api.requests) == 3, "scripted API failed: " + str(api.failure))
        require(acknowledged is not None, "missing stop acknowledgement")
        while time.monotonic() - acknowledged < 6 and (alive("shell-pid") or alive("child-pid")):
            time.sleep(.05)
        retained = mode == "keeper-unforwarded"
        require(alive("shell-pid") == retained and alive("child-pid") == retained,
                "native cancellation behavior differs from the recorded topology")
        require((effect / "late").exists() == retained, "post-stop effect differs")
        require(not any((other / name).exists() for name in ("started", "shell-pid", "child-pid", "late")),
                "command affected the wrong execution directory")
        if keeper_data:
            require(keeper_data[0].poll() is None, "cancellation killed the keeper")
            require(process_identity(target / "peer-pid") == peer, "cancellation killed independent keeper job")
        return {"mode": mode, "task_stop_acknowledged": True, "shell_alive": alive("shell-pid"),
                "child_alive": alive("child-pid"), "post_stop_effect": retained,
                "alive_at_native_stop_ack": alive_at_ack,
                "observation_seconds": time.monotonic() - acknowledged, "scripted_api_requests": len(api.requests),
                "keeper_and_original_peer_survive": True if keeper_data else None}
    finally:
        if client is not None:
            client.close()
        api.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("claude", "executor", "keeper", "receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--claude-sha256", required=True)
    parser.add_argument("--executor-sha256", required=True)
    args = parser.parse_args()
    require(os.getpid() == 1 and [name for _, name in socket.if_nameindex()] == ["lo"],
            "disposable PID and loopback-only network namespaces required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    for path in (args.claude, args.executor, args.keeper):
        require(path.is_absolute() and path.is_file(), "absolute binary path required")
    require(digest(args.claude) == args.claude_sha256 and digest(args.executor) == args.executor_sha256,
            "native binary digest differs")
    cases = []
    for mode in ("local", "transient-prefix", "keeper-unforwarded", "keeper-forward"):
        with tempfile.TemporaryDirectory(prefix="cowboy-claude-stop-") as directory:
            root = Path(directory)
            runtime, target = root / "runtime", root / "target"
            runtime.mkdir()
            target.mkdir()
            environment = closed_environment(root / "target-home")
            if mode.startswith("keeper-"):
                with resident(args, root, target, runtime, environment) as keeper_data:
                    cases.append(exercise(args, mode, root, runtime, target, environment, keeper_data))
            else:
                cases.append(exercise(args, mode, root, runtime, target, environment, None))
    receipt = {"schema": "cowboy.claude-native-task-stop-research/v1", "accepted": True,
               "cases": cases, "claude_sha256": args.claude_sha256, "executor_sha256": args.executor_sha256,
               "keeper_sha256": digest(args.keeper), "probe_sha256": digest(Path(__file__)),
               "real_model_requests": 0, "production_changes": False,
               "topology": "same-host, shared filesystem, persistent keeper in an independent process group",
               "not_checked": ["pending-start cancellation", "SIGKILL", "disconnect/reconnect", "cold resume",
                               "foreground interrupt", "cross-host paths", "permissions", "hooks/MCP shell classification"]}
    with args.receipt.open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
