#!/usr/bin/env python3
"""Compare pinned native child interruption and scoped terminal stop locally/remotely.

Research acceptance, not a production cancellation policy. Run inside a disposable
PID/network namespace: only loopback, separate homes, no real model or credentials.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

import execution_environment_codex_turn_probe as native
from execution_environment_probe import require
from plugin_runtime_conformance import closed_environment


def identity(root, name):
    try:
        pid = (root / name).read_text()
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return (pid, fields[19]) if fields[0] != "Z" else None
    except FileNotFoundError:
        return None


def run(binary, mode, tool):
    with tempfile.TemporaryDirectory(prefix="cowboy-child-stop-") as temp:
        root = Path(temp)
        runtime, target = root / "runtime", root / "target"
        runtime.mkdir()
        target.mkdir()
        effect = runtime if mode == "local" else target
        other = target if mode == "local" else runtime
        counts = {"parent": 0, "child": 0}
        originals = {}
        interrupt_sent = False
        interrupted = None
        script = (
            'printf "%s" "$$" > child-pid; '
            'printf "%s" "$COWBOY_PROBE_LOCATION" > location; '
            '(sleep 6; printf late > late; sleep 60) & '
            'printf "%s" "$!" > descendant-pid; printf started > started; wait'
        )

        def alive(name):
            return originals.get(name) is not None and identity(effect, name) == originals[name]

        def dispatch(requests):
            nonlocal interrupt_sent, interrupted
            child = any(item.get("type") == "agent_message" and
                        item.get("recipient") == "/root/stop_probe"
                        for item in requests[-1].get("input", []))
            role = "child" if child else "parent"
            step = counts[role]
            counts[role] += 1
            if child:
                require(step == 0, "interrupted child unexpectedly requested another model response")
                arguments = {"cmd": script, "yield_time_ms": 10000, "max_output_tokens": 1000}
                if tool == "direct":
                    return {"type": "function_call", "name": "exec_command", "call_id": "child-command",
                            "arguments": json.dumps(arguments)}
                return {"type": "custom_tool_call", "namespace": "functions", "name": "exec",
                        "call_id": "child-command", "input": "text(await tools.exec_command(" + json.dumps(arguments) + "));"}
            if step == 0:
                return native.command("peer", 'printf "%s" "$$" > peer-pid; sleep 90')
            if step == 1:
                originals["peer-pid"] = identity(effect, "peer-pid")
                require(originals["peer-pid"] is not None, "peer not admitted")
                return {"type": "function_call", "namespace": "collaboration", "name": "spawn_agent",
                        "call_id": "spawn", "arguments": json.dumps({"task_name": "stop_probe",
                        "message": "Run the isolated scripted command.", "fork_turns": "none"})}
            if (effect / "started").exists() and not interrupt_sent:
                for name in ("child-pid", "descendant-pid"):
                    originals[name] = identity(effect, name)
                    require(originals[name] is not None, "child not positively admitted")
                require(not (effect / "late").exists(), "delayed effect preceded interrupt")
                interrupt_sent = True
                return {"type": "function_call", "namespace": "collaboration", "name": "interrupt_agent",
                        "call_id": "interrupt", "arguments": json.dumps({"target": "stop_probe"})}
            if interrupt_sent and interrupted is None:
                require(any(item.get("call_id") == "interrupt" and item.get("type") == "function_call_output"
                            for item in requests[-1].get("input", [])), "interrupt acknowledgement missing")
                require(not (effect / "late").exists(), "delayed effect preceded interrupt acknowledgement")
                interrupted = time.monotonic()
            if interrupted is not None and (
                not (alive("child-pid") or alive("descendant-pid")) or time.monotonic() - interrupted >= 12
            ):
                return native.final("parent-final")
            return {"type": "function_call", "namespace": "clock", "name": "sleep",
                    "call_id": f"wait-{step}", "arguments": json.dumps({"duration_ms": 50})}

        api = native.Api([dispatch] * 500)
        env = closed_environment(root / "agent-home")
        env["COWBOY_PROBE_LOCATION"] = "runtime"
        if mode == "remote":
            env["CODEX_EXEC_SERVER_URL"] = "none"
        else:
            env.pop("CODEX_EXEC_SERVER_URL", None)
        env["COWBOY_OFFLINE_FIXTURE_KEY"] = "not-a-production-credential"
        home = Path(env["CODEX_HOME"])
        home.mkdir()
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "cowboy_fixture"\n'
            '[features]\nremote_control = false\nshell_snapshot = false\n'
            '[model_providers.cowboy_fixture]\nname = "Offline fixture"\n'
            f'base_url = "http://127.0.0.1:{api.server_port}/v1"\n'
            'wire_api = "responses"\nenv_key = "COWBOY_OFFLINE_FIXTURE_KEY"\n'
            'request_max_retries = 0\nstream_max_retries = 0\n'
        )
        executor = client = None
        try:
            binding = {}
            if mode == "remote":
                with socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    port = listener.getsockname()[1]
                url = f"ws://127.0.0.1:{port}"
                target_env = closed_environment(root / "executor-home")
                target_env["COWBOY_PROBE_LOCATION"] = "target"
                executor = subprocess.Popen([str(binary), "exec-server", "--listen", url], cwd=target,
                                            env=target_env, stdin=subprocess.DEVNULL,
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                ready = False
                for _ in range(100):
                    require(executor.poll() is None, "executor exited before admission")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=.05):
                            ready = True
                            break
                    except OSError:
                        time.sleep(.05)
                require(ready, "executor did not listen")
                binding = {"environments": [{"environmentId": "cowboy-fixture-target",
                           "cwd": str(target), "runtimeWorkspaceRoots": [str(target)]}]}
            client = native.app(str(binary), env, runtime)
            if mode == "remote":
                native.add_environment(client, url)
            thread = client.request("thread/start", {"cwd": str(runtime), "approvalPolicy": "never",
                                    "sandbox": "danger-full-access", **binding})["thread"]["id"]
            turn = client.request("turn/start", {"threadId": thread, "input": [{"type": "text",
                                  "text": "Run the isolated fixture.", "text_elements": []}], **binding})
            deadline = time.monotonic() + 60
            children = {}
            while True:
                frame = client.frame(deadline)
                require(not ("id" in frame and "method" in frame), "unexpected approval")
                if frame.get("method") != "turn/completed":
                    continue
                params = frame["params"]
                if params["threadId"] != thread:
                    children[params["threadId"]] = params["turn"]["status"]
                    continue
                require(params["turn"]["id"] == turn["turn"]["id"] and
                        params["turn"]["status"] == "completed", "parent failed")
                break
            require(api.failure is None and counts["child"] == 1, "scripted API failed")
            require(len(children) == 1 and list(children.values()) == ["interrupted"], "child not interrupted")
            require(interrupted is not None, "interrupt not admitted")
            require((effect / "location").read_text() == ("runtime" if mode == "local" else "target"),
                    "command used wrong execution location")
            require(not any((other / name).exists() for name in ("started", "peer-pid", "location", "late")),
                    "wrong-machine side effect")
            retained = alive("child-pid") and alive("descendant-pid")
            late = (effect / "late").exists()
            # This is the observed contract of the supplied pin, not a desired
            # cancellation policy. An upstream change requires reviewing it.
            require(retained == (tool == "codeact") and late == retained, "native cancellation behavior changed")
            require(retained or not (alive("child-pid") or alive("descendant-pid")), "partial process cleanup")
            require(alive("peer-pid"), "interrupt killed independent peer")
            result = {"mode": mode, "tool": tool, "child_status": "interrupted",
                      "retained_after_interrupt": retained, "post_interrupt_effect": late,
                      "observation_seconds": time.monotonic() - interrupted}
            if retained:
                child_thread = next(iter(children))
                listed = client.request("thread/backgroundTerminals/list", {"threadId": child_thread})
                require(listed.get("nextCursor") is None and len(listed["data"]) == 1,
                        "unexpected child terminal ownership or pagination")
                terminal = listed["data"][0]
                require(terminal["cwd"] == str(effect), "terminal belongs to wrong workspace")
                wrong = client.request("thread/backgroundTerminals/terminate",
                                       {"threadId": thread, "processId": terminal["processId"]})
                require(not wrong["terminated"] and alive("child-pid") and alive("descendant-pid"),
                        "wrong thread terminated child process")
                result["wrong_thread_stop_rejected"] = True
                stopped = client.request("thread/backgroundTerminals/terminate",
                                         {"threadId": child_thread, "processId": terminal["processId"]})
                require(stopped["terminated"], "native scoped termination rejected")
                for _ in range(100):
                    if not (alive("child-pid") or alive("descendant-pid")):
                        break
                    time.sleep(.03)
            require(not alive("child-pid") and not alive("descendant-pid"), "process tree survived scoped stop")
            require(alive("peer-pid"), "scoped stop killed independent peer")
            result.update({"tree_stopped": True, "original_peer_survives": True,
                           "scripted_api_requests": len(api.requests)})
            return result
        finally:
            if client is not None:
                client.close()
            if executor is not None:
                native.stop(executor)
            api.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback-only namespace required")
    require(os.getpid() == 1 and Path("/proc/1/comm").read_text().strip() == "python3",
            "run as init in a disposable PID namespace")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    require(args.native_cli.is_absolute() and args.native_cli.is_file(), "native binary missing")
    digest = hashlib.sha256(args.native_cli.read_bytes()).hexdigest()
    require(digest == args.sha256, "native digest mismatch")
    companions = [args.native_cli.parent / "codex-code-mode-host",
                  args.native_cli.parent.parent / "codex-resources/bwrap"]
    require(all(path.is_file() for path in companions), "native companion missing")
    sources = [Path(__file__), Path(native.__file__)]
    hashes = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in companions + sources}
    cases = [run(args.native_cli, mode, tool) for mode in ("local", "remote") for tool in ("direct", "codeact")]
    receipt = {"schema": "cowboy.native-child-stop-audit/v1", "accepted": True,
               "native_sha256": digest, "input_sha256": hashes, "cases": cases, "real_model_requests": 0,
               "production_credentials": False, "production_changes": False,
               "not_checked": ["Cowboy worker/keeper transport", "packaged ACP async task extension",
                               "early terminal registration race", "restricted approvals", "cross-host network"]}
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
