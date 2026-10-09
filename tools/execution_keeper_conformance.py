#!/usr/bin/env python3
"""Exercise Cowboy's real detached keeper against an exact native executor.

Uses disposable directories and loopback-only networking. No installed Plugin
is changed, no Agent/model is started and no production grants are created.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import uuid

from execution_environment_probe import require
from plugin_runtime_conformance import closed_environment, stop_group


def identity():
    return uuid.uuid4().hex


class Keeper:
    def __init__(self, path, contract):
        self.path = path
        binding = contract["binding"]
        self.envelope = {"schema": 1, "capability": contract["capability"], "scope": {
            "binding_id": binding["id"], "revision": binding["revision"],
            "incarnation": binding["environment"]["incarnation"],
        }}

    def request(self, command, *, discard=False, override=None):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(25)
            connection.connect(str(self.path))
            packet = dict(self.envelope, command=command)
            if override:
                packet.update(override)
            connection.sendall(json.dumps(packet).encode() + b"\n")
            if discard:
                return None
            with connection.makefile("rb") as output:
                line = output.readline(4 * 1024 * 1024 + 1)
                require(line.endswith(b"\n") and len(line) <= 4 * 1024 * 1024,
                        "keeper returned an invalid frame")
                return json.loads(line)

    def invocation(self, method, params):
        return {"operation_id": identity(), "method": method, "params": params}

    def invoke(self, invocation, *, discard=False):
        return self.request({"kind": "invoke", "invocation": invocation, "wait_ms": 20_000}, discard=discard)

    def result(self, method, params):
        reply = self.invoke(self.invocation(method, params))
        require(reply.get("kind") == "operation", "keeper refused a fixture operation")
        outcome = reply["outcome"]
        require(outcome.get("state") == "completed", "fixture operation did not complete")
        require("error" not in outcome["reply"], f"native {method} failed")
        return outcome["reply"]["result"]


def process_arguments(target, process_id, script):
    return {"processId": process_id, "argv": ["/run/current-system/sw/bin/bash", "-c", script],
            "cwd": target.as_uri(), "env": {}, "tty": False, "pipeStdin": False, "arg0": None,
            "envPolicy": {"inherit": "all", "ignoreDefaultExcludes": False,
                          "exclude": [], "set": {}, "includeOnly": []}}


def probe(args):
    checks = []
    with tempfile.TemporaryDirectory(prefix="cowboy-keeper-") as directory:
        root = Path(directory)
        state, target = root / "state", root / "target"
        state.mkdir(mode=0o700)
        target.mkdir()
        # Preserve the upstream metadata layout without borrowing an installed
        # Plugin as a running target component. The full release owns its pin.
        executor_root = root / "executor"
        (executor_root / "bin").mkdir(parents=True)
        binary = executor_root / "bin" / "codex"
        shutil.copyfile(args.native_cli, binary)
        binary.chmod(0o555)
        shutil.copyfile(args.native_cli.parent.parent / "codex-package.json", executor_root / "codex-package.json")
        home = root / "target-home"
        home.mkdir(mode=0o700)
        contract = {
            "schema": 1, "session_id": "keeper-fixture",
            "binding": {
                "schema": 1, "id": "fixture-binding", "revision": 1,
                "runtime": {"machine_id": "ovh", "cwd": str(root / "runtime")},
                "environment": {"machine_id": "hawk", "id": "fixture-environment",
                                "incarnation": identity(), "executor_digest": f"sha256:{args.sha256}", "protocol": 1},
                "workspace": {"id": "fixture", "worktree_id": "fixture-worktree", "source_path": str(target), "cwd": str(target)},
                "access": "project",
            },
            "executor": {"command": str(binary), "sha256": args.sha256, "version": args.version},
            "capability": identity() + identity(),
            "environment": {"HOME": str(home), "PATH": "/run/current-system/sw/bin",
                            "SHELL": "/run/current-system/sw/bin/bash"},
        }
        contract_path = root / "contract.json"
        with contract_path.open("x") as output:
            json.dump(contract, output)
        contract_path.chmod(0o600)
        env = closed_environment(root / "runtime-home")
        env["OPENAI_API_KEY"] = "must-not-enter-target-executor"
        command = [str(args.keeper), "--contract", str(contract_path), "--state-dir", str(state)]
        logs = tempfile.TemporaryFile()
        daemon = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                                  stdout=logs, stderr=logs, start_new_session=True)
        keeper = Keeper(state / "keeper.sock", contract)
        try:
            for _ in range(100):
                require(daemon.poll() is None, "keeper exited during startup")
                try:
                    ready = keeper.request({"kind": "describe"})
                    break
                except (OSError, TimeoutError):
                    time.sleep(0.05)
            else:
                raise AssertionError("keeper startup timed out")
            require(ready["kind"] == "ready", "keeper did not become ready")
            require(ready["initialization"]["environmentInfo"]["cwd"] == target.as_uri(), "wrong executor cwd")
            checks.append("exact_pinned_executor_and_target_context")
            content = "quotes '\" $() ${HOME} 中文 🐎\r\n".encode() + bytes(range(256))
            file = target / "quoted ' 中文.bin"
            write = keeper.invocation("fs/writeFile", {"path": file.as_uri(), "dataBase64": base64.b64encode(content).decode()})
            first = keeper.invoke(write)
            require(file.read_bytes() == content, "file bytes changed")
            read = keeper.result("fs/readFile", {"path": file.as_uri()})
            require(base64.b64decode(read["dataBase64"]) == content, "read bytes changed")
            file.write_bytes(b"later independent edit")
            require(keeper.invoke(write) == first and file.read_bytes() == b"later independent edit", "duplicate invocation replayed an edit")
            changed = dict(write, params=dict(write["params"], dataBase64="YQ=="))
            require(keeper.invoke(changed) == {"kind": "refused", "reason": "operation_conflict"}, "changed arguments reused an operation")
            checks.extend(["file_bytes_preserved", "repeated_write_does_not_overwrite_later_edit", "changed_operation_arguments_refused"])
            missing = keeper.request({"kind": "observe", "operation_id": identity(), "wait_ms": 0})
            require(missing == {"kind": "operation", "outcome": {"state": "missing"}}, "unknown operation fabricated an effect")
            require(keeper.request({"kind": "describe"}, override={"capability": "0" * 64}) == {"kind": "refused", "reason": "unauthorized"}, "wrong capability accepted")
            bad_scope = dict(keeper.envelope["scope"], revision=2)
            require(keeper.request({"kind": "describe"}, override={"scope": bad_scope}) == {"kind": "refused", "reason": "unauthorized"}, "changed binding accepted")
            checks.append("capability_binding_and_missing_operation_enforced")
            process_id = identity()
            start = keeper.invocation("process/start", process_arguments(target, process_id,
                "test -z \"${OPENAI_API_KEY:-}\" || exit 39; "
                "printf '%s' \"$COWBOY_EXECUTION_FILE_HELPER\" > helper-path; "
                "PATH=/nonexistent \"$COWBOY_EXECUTION_FILE_HELPER\" realpath -- \"$PWD\" > helper-realpath || exit 40; "
                "printf once >> starts; printf '%s' \"$$\" > process.pid; exec /run/current-system/sw/bin/sleep 180"))
            # Disconnect before reading the result, then recover only the same
            # operation. No IPC client remains attached during the 35s interval.
            keeper.invoke(start, discard=True)
            for _ in range(100):
                if (target / "starts").exists():
                    break
                time.sleep(0.05)
            require((target / "starts").read_text() == "once", "target command did not execute once")
            require(Path((target / "helper-path").read_text()).resolve() == args.keeper.resolve() and
                    (target / "helper-realpath").read_text() == str(target),
                    "target file helper was not the owned keeper or required ambient executables")
            checks.append("owned_file_helper_works_without_python_or_path_tools")
            time.sleep(35)
            observed = keeper.request({"kind": "observe", "operation_id": start["operation_id"], "wait_ms": 1000})
            require(observed["outcome"]["state"] == "completed", "lost receipt was not retained")
            require(keeper.invoke(start) == observed, "duplicate process start changed outcome")
            state_reply = keeper.result("process/read", {"processId": process_id, "waitMs": 1, "maxBytes": 1024})
            require(not state_reply["exited"] and (target / "starts").read_text() == "once", "disconnected task stopped or replayed")
            checks.extend(["lost_start_receipt_recovers_without_reexecution", "job_survives_35_seconds_without_control_clients", "runtime_auth_environment_not_inherited"])
            keeper.result("process/terminate", {"processId": process_id})
            for _ in range(50):
                settled = keeper.result("process/read", {"processId": process_id, "waitMs": 100, "maxBytes": 1024})
                if settled["closed"]:
                    break
            require(settled["closed"] and settled["exited"], "remote cancellation did not settle")
            checks.append("original_process_can_be_cancelled_after_disconnect")
            # Path equality is insufficient: replacing the root ends admission.
            retained = root / "retained-target"
            target.rename(retained)
            target.mkdir()
            require(keeper.invoke(keeper.invocation("fs/readFile", {"path": file.as_uri()})) == {"kind": "refused", "reason": "workspace_changed"}, "replaced workspace admitted an operation")
            checks.append("workspace_inode_replacement_refuses_new_effects")
            stop_group(daemon)
            restarted = subprocess.run(command, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, timeout=10, check=False)
            require(restarted.returncode != 0, "dead incarnation was automatically restarted")
            require((retained / "starts").read_text() == "once", "restart replayed a command")
            checks.append("same_incarnation_restart_refused_without_replay")
        finally:
            stop_group(daemon)
            logs.close()
    return checks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--keeper", type=Path, required=True)
    parser.add_argument("--native-cli", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require(args.keeper.is_absolute() and args.native_cli.is_absolute(), "executables must be absolute")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "receipt must be a new absolute path")
    require(os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net") and
            [name for _, name in socket.if_nameindex()] == ["lo"], "isolated loopback required")
    with args.native_cli.open("rb") as binary:
        require(hashlib.file_digest(binary, "sha256").hexdigest() == args.sha256, "executor digest differs")
    with args.keeper.open("rb") as binary:
        keeper_digest = hashlib.file_digest(binary, "sha256").hexdigest()
    checks = probe(args)
    receipt = {"schema": "cowboy.execution-keeper-conformance/v1", "accepted": True,
               "keeper_sha256": keeper_digest, "executor": {"version": args.version, "sha256": args.sha256},
               "checks": checks, "disconnected_ms": 35_000, "cleanup": True,
               "real_model_requests": 0, "production_credentials": False,
               "proves_enrolled_transport": False, "proves_provider_integration": False}
    with args.receipt.open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"accepted": True, "checks": len(checks)}))


if __name__ == "__main__":
    main()
