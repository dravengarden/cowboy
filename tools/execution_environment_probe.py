#!/usr/bin/env python3
"""Exercise a pinned native executor over stdio, without an Agent or model login.

The command following -- must start `codex exec-server --listen stdio`, locally
or through an already authorized transport. This is an acceptance probe, not a
Cowboy transport, installer, or session-binding implementation.
"""

import argparse
import base64
import datetime
import json
import os
from pathlib import Path
import selectors
import statistics
import subprocess
import time
import uuid


class ProbeFailure(Exception):
    pass


def require(condition, message):
    if not condition:
        raise ProbeFailure(message)


class Executor:
    def __init__(self, command, timeout, *, environment=None, cwd=None):
        self.timeout = timeout
        # Never collect a transport's stderr: it may contain private diagnostics.
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, bufsize=0,
            env=environment, cwd=cwd,
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = bytearray()
        self.request_id = 0
        self.timings = {}

    def send(self, frame):
        encoded = json.dumps(frame, ensure_ascii=True).encode() + b"\n"
        self.process.stdin.write(encoded)
        self.process.stdin.flush()

    def frame(self, deadline):
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise ProbeFailure("executor response timed out")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            require(chunk, "executor closed its output")
            self.buffer.extend(chunk)
            require(len(self.buffer) <= 2 * 1024 * 1024, "executor frame exceeds probe bound")
        line, _, tail = self.buffer.partition(b"\n")
        self.buffer = bytearray(tail)
        try:
            result = json.loads(line)
        except (ValueError, UnicodeError) as error:
            raise ProbeFailure("executor returned an invalid frame") from error
        require(isinstance(result, dict), "executor frame is not an object")
        return result

    def request(self, method, params, expect_error=False):
        self.request_id += 1
        request_id = self.request_id
        started = time.monotonic()
        self.send({"id": request_id, "method": method, "params": params})
        deadline = started + self.timeout
        while True:
            response = self.frame(deadline)
            if "id" not in response:
                # process/read independently retrieves retained, sequenced output.
                continue
            require(response["id"] == request_id, "unexpected executor response identity")
            self.timings.setdefault(method, []).append((time.monotonic() - started) * 1000)
            if expect_error:
                require(isinstance(response.get("error"), dict), "expected a protocol rejection")
                return response["error"].get("code")
            require("result" in response and "error" not in response, f"{method} failed")
            return response["result"]

    def batch(self, method, params, count):
        """Pipeline bounded reads without imposing an extra round trip per read."""
        outstanding = set()
        started = time.monotonic()
        for _ in range(count):
            self.request_id += 1
            outstanding.add(self.request_id)
            self.send({"id": self.request_id, "method": method, "params": params})
        deadline = started + self.timeout
        while outstanding:
            response = self.frame(deadline)
            if "id" not in response:
                continue
            require(response["id"] in outstanding, "unexpected batch response identity")
            require("result" in response and "error" not in response, f"{method} batch failed")
            outstanding.remove(response["id"])
        return round((time.monotonic() - started) * 1000, 3)

    def start(self, argv, cwd, pipe_stdin=False):
        process_id = str(uuid.uuid4())
        result = self.request("process/start", {
            "processId": process_id, "argv": argv, "cwd": cwd,
            "env": {}, "tty": False, "pipeStdin": pipe_stdin, "arg0": None,
            # Commands need only their absolute executables and fixture paths.
            "envPolicy": {
                "inherit": "none", "ignoreDefaultExcludes": False,
                "exclude": [], "set": {}, "includeOnly": [],
            },
        })
        require(result.get("processId") == process_id, "process identity changed")
        return process_id

    def collect(self, process_id):
        deadline = time.monotonic() + self.timeout
        after_seq = None
        output = {"stdout": bytearray(), "stderr": bytearray()}
        while time.monotonic() < deadline:
            result = self.request("process/read", {
                "processId": process_id, "afterSeq": after_seq,
                "maxBytes": 32768, "waitMs": 100,
            })
            for chunk in result["chunks"]:
                require(after_seq is None or chunk["seq"] > after_seq, "process output repeated")
                after_seq = chunk["seq"]
                require(chunk["stream"] in output, "unknown process output stream")
                output[chunk["stream"]].extend(base64.b64decode(chunk["chunk"], validate=True))
            require(sum(map(len, output.values())) <= 65536, "process output exceeds probe bound")
            if result["closed"]:
                require(result["exited"], "closed process lacks an exit observation")
                return result, {key: bytes(value) for key, value in output.items()}
        raise ProbeFailure("process did not settle")

    def close(self):
        self.selector.close()
        try:
            self.process.stdin.close()
            self.process.wait(timeout=5)
        except (BrokenPipeError, subprocess.TimeoutExpired):
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.process.stdout.close()


def probe(executor, expected_version, shell, expected_host, receipt):
    initialized = executor.request("initialize", {"clientName": "cowboy-execution-acceptance"})
    environment = initialized["environmentInfo"]
    require(environment["executorVersion"] == expected_version, "unexpected executor version")
    require(environment.get("platformOs") == "linux", "this probe currently requires Linux")
    executor.send({"method": "initialized", "params": {}})
    receipt["executor"] = {
        "version": environment["executorVersion"],
        "provider_id": environment.get("providerId"),
        "platform": environment["platformOs"],
    }
    temp_dir = environment.get("tempDir")
    require(isinstance(temp_dir, str) and temp_dir.startswith("file:///"), "missing target temp URI")
    root = temp_dir.rstrip("/") + "/cowboy-execution-probe-" + uuid.uuid4().hex
    fixture = root + "/quoted%20%27%20%24%20%E4%B8%AD%E6%96%87.bin"
    created = False
    processes = set()
    try:
        executor.request("fs/createDirectory", {"path": root, "recursive": False})
        created = True
        if expected_host:
            process_id = executor.start(["/run/current-system/sw/bin/uname", "-n"], root)
            processes.add(process_id)
            result, output = executor.collect(process_id)
            require(result["exitCode"] == 0 and output["stdout"].strip().decode() == expected_host,
                    "unexpected executor hostname")
            processes.remove(process_id)
            receipt["executor"]["observed_hostname"] = expected_host
            receipt["checks"].append("target_hostname")
        content = b"quotes: '\" `$() ${HOME} \\\n" + "中文 🐎\r\n".encode() + bytes(range(256))
        executor.request("fs/writeFile", {
            "path": fixture, "dataBase64": base64.b64encode(content).decode(),
        })
        read = executor.request("fs/readFile", {"path": fixture})
        require(base64.b64decode(read["dataBase64"], validate=True) == content, "file bytes changed")
        receipt["checks"].append("file_roundtrip_unicode_quotes_binary")

        # Structured argv and cwd, with no interpolation of a project path.
        process_id = executor.start([shell, "-c", "printf '%s\\n' \"$1\"; printf 'err' >&2; exit 37",
                                     "probe", "'\" `$() 中文"], root)
        processes.add(process_id)
        result, output = executor.collect(process_id)
        require(result["exitCode"] == 37, "nonzero process exit was lost")
        require(output == {"stdout": "'\" `$() 中文\n".encode(), "stderr": b"err"},
                "argv or output bytes changed")
        processes.remove(process_id)
        receipt["checks"].append("argv_streams_and_exit_status")

        # The command sees the very file written over the filesystem interface.
        process_id = executor.start([shell, "-c", "for p in *; do exec /run/current-system/sw/bin/cat -- \"$p\"; done"], root)
        processes.add(process_id)
        result, output = executor.collect(process_id)
        require(result["exitCode"] == 0 and output["stdout"] == content,
                "filesystem and process working directories disagree")
        processes.remove(process_id)
        receipt["checks"].append("file_and_process_share_workspace")

        process_id = executor.start([shell, "-c", "IFS= read -r line; printf '%s' \"$line\""], root, True)
        processes.add(process_id)
        written = executor.request("process/write", {
            "processId": process_id, "chunk": base64.b64encode(b"stdin ' $()\n").decode(),
            "writeId": str(uuid.uuid4()),
        })
        require(written["status"] == "accepted", "process stdin was not accepted")
        result, output = executor.collect(process_id)
        require(result["exitCode"] == 0 and output["stdout"] == b"stdin ' $()", "stdin bytes changed")
        processes.remove(process_id)
        receipt["checks"].append("process_stdin")

        process_id = executor.start([shell, "-c", "exec /run/current-system/sw/bin/sleep 120"], root)
        processes.add(process_id)
        state = executor.request("process/read", {"processId": process_id, "waitMs": 1, "maxBytes": 1024})
        require(not state["exited"], "background process exited before cancellation")
        executor.request("process/terminate", {"processId": process_id})
        result, _ = executor.collect(process_id)
        require(result["closed"] and result["exited"], "cancellation did not settle")
        processes.remove(process_id)
        receipt["checks"].append("background_process_cancel_and_settle")

        error_code = executor.request("fs/readFile", {"path": root + "/missing"}, expect_error=True)
        require(isinstance(error_code, int), "missing file lacked a typed protocol error")
        require(executor.request("fs/readFile", {"path": fixture})["dataBase64"] == read["dataBase64"],
                "connection failed after an operation error")
        receipt["checks"].append("error_isolation_on_persistent_connection")

        samples = []
        for _ in range(10):
            started = time.monotonic()
            executor.request("fs/getMetadata", {"path": fixture})
            samples.append(round((time.monotonic() - started) * 1000, 3))
        receipt["metadata_roundtrip_ms"] = {
            "samples": samples, "median": round(statistics.median(samples), 3),
        }
        receipt["metadata_batch_10_ms"] = executor.batch("fs/getMetadata", {"path": fixture}, 10)
        receipt["checks"].append("pipelined_reads")
    finally:
        for process_id in processes:
            executor.request("process/terminate", {"processId": process_id})
            executor.collect(process_id)
        if created:
            executor.request("fs/remove", {"path": root, "recursive": True, "force": False})
            receipt["fixture_removed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--expected-version", default="0.159.3")
    parser.add_argument("--expected-host", help="also require this target's uname -n")
    parser.add_argument("--shell", default="/run/current-system/sw/bin/bash")
    parser.add_argument("--timeout", type=float, default=20)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("an executor command is required after --")
    if not 0 < args.timeout <= 60:
        parser.error("--timeout must be between 0 and 60 seconds")
    receipt = {
        "schema": 1, "kind": "native_executor_protocol_probe",
        "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "checks": [], "fixture_removed": False, "status": "failed",
        "claims": {
            "model_inference": False, "subscription_acceptance": False,
            "cowboy_session_integration": False, "reconnect_acceptance": False,
            "token_savings_measured": False,
        },
    }
    executor = None
    try:
        executor = Executor(command, args.timeout)
        probe(executor, args.expected_version, args.shell, args.expected_host, receipt)
        receipt["status"] = "passed"
    except Exception as error:
        # Exclude server errors, arbitrary response bodies and command arguments.
        receipt["failure"] = str(error) if isinstance(error, ProbeFailure) else type(error).__name__
    finally:
        if executor is not None:
            executor.close()
        args.receipt.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"status": receipt["status"], "checks": receipt["checks"],
                      "receipt": str(args.receipt), "failure": receipt.get("failure")}))
    return 0 if receipt["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
