#!/usr/bin/env python3
"""Offline native thread/environment acceptance; no model call or real login.

Run inside a fresh network namespace with only loopback enabled. Both processes
use fresh, closed homes, and distinct runtime and execution directories. This
tests the pinned native interface, not Cowboy transport, cold restore or billing.
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

from execution_environment_probe import Executor, ProbeFailure, require


def stop(process):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def native_probe(binary, version, checks):
    with tempfile.TemporaryDirectory(prefix="cowboy-native-environment-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        target = root / "target"
        for name in ["runtime", "target", "agent-home/.codex", "executor-home/.codex"]:
            (root / name).mkdir(parents=True)
        (runtime / "AGENTS.md").write_text("RUNTIME_GUIDANCE_MUST_NOT_BE_PROJECT_GUIDANCE")
        (target / "AGENTS.md").write_text("TARGET_GUIDANCE_IS_PROJECT_GUIDANCE")
        for home in ["agent-home", "executor-home"]:
            (root / home / ".codex/config.toml").write_text(
                '[features]\nremote_control = false\nshell_snapshot = false\n'
            )
        environment = {
            "PATH": "/run/current-system/sw/bin",
            "HOME": str(root / "agent-home"),
            "CODEX_HOME": str(root / "agent-home/.codex"),
            "NO_COLOR": "1",
        }
        executor_environment = dict(
            environment, HOME=str(root / "executor-home"),
            CODEX_HOME=str(root / "executor-home/.codex"),
        )
        observed = subprocess.run(
            [binary, "--version"], env=environment, cwd=runtime, check=True,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10,
        ).stdout.decode().strip()
        require(observed == f"codex-cli {version}", "native CLI version differs")
        checks.append("exact_native_version")
        with socket.socket() as allocated:
            allocated.bind(("127.0.0.1", 0))
            port = allocated.getsockname()[1]
        url = f"ws://127.0.0.1:{port}"
        executor = subprocess.Popen(
            [binary, "exec-server", "--listen", url], cwd=target,
            env=executor_environment, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        client = None
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.05):
                        break
                except OSError:
                    require(executor.poll() is None, "executor stopped before readiness")
                    time.sleep(0.05)
            else:
                raise ProbeFailure("executor readiness timed out")
            client = Executor(
                [binary, "app-server"], 40, environment=environment, cwd=runtime,
            )
            client.request("initialize", {
                "clientInfo": {"name": "cowboy-native-environment-probe", "version": "1"},
                "capabilities": {"experimentalApi": True},
            })
            client.send({"method": "initialized", "params": {}})
            client.request("environment/add", {
                "environmentId": "cowboy-test-target", "execServerUrl": url,
                "connectTimeoutMs": 1000,
            })
            info = client.request("environment/info", {"environmentId": "cowboy-test-target"})
            require(info["cwd"] == target.as_uri(), "environment cwd is not the target")
            checks.append("separate_execution_cwd")
            selection = [{
                "environmentId": "cowboy-test-target", "cwd": str(target),
                "runtimeWorkspaceRoots": [str(target)],
            }]
            started = client.request("thread/start", {
                "cwd": str(runtime), "ephemeral": True, "approvalPolicy": "never",
                "sandbox": "danger-full-access", "environments": selection,
            })
            require(started["cwd"] == str(runtime), "native runtime cwd moved")
            require(started["thread"]["environments"] == selection, "thread selection changed")
            checks.append("native_thread_retains_runtime_and_execution_locations")
            require(started["instructionSources"] == [str(target / "AGENTS.md")],
                    "project guidance was not resolved on the target")
            checks.append("target_guidance_without_runtime_guidance")
            client.request("thread/start", {
                "cwd": str(runtime), "ephemeral": True,
                "environments": [dict(selection[0], environmentId="absent-environment")],
            }, expect_error=True)
            checks.append("unknown_environment_refused_without_local_fallback")
            require(not (root / "agent-home/.codex/environments.toml").exists(),
                    "native binding mutated global environment configuration")
            checks.append("no_global_environment_configuration_written")
        finally:
            if client is not None:
                client.close()
            stop(executor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require(args.native_cli.is_absolute(), "native CLI path must be absolute")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "receipt must be a new absolute path")
    require(os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net"),
            "run inside a fresh network namespace")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "network namespace must have only loopback")
    with args.native_cli.open("rb") as executable:
        digest = hashlib.file_digest(executable, "sha256").hexdigest()
    require(digest == args.sha256, "native CLI digest differs")
    checks = []
    native_probe(str(args.native_cli), args.version, checks)
    receipt = {
        "schema": "cowboy.native-environment-probe/v1", "accepted": True,
        "native_cli": {"version": args.version, "sha256": digest},
        "checks": checks,
        "topology": "isolated_loopback_distinct_homes_and_workspaces",
        "model_requests": 0, "production_credentials": False,
        "proves_cowboy_routing": False, "proves_native_resume": False,
        "proves_subscription": False, "proves_token_savings": False,
    }
    descriptor = os.open(args.receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"accepted": True, "checks": len(checks), "model_requests": 0}))


if __name__ == "__main__":
    main()
