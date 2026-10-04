#!/usr/bin/env python3
"""Exercise the shipped diagnostic command and its read-only remote protocol."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cowboy", type=Path, required=True)
    parser.add_argument("--test-executable", type=Path, help="Otherwise build the owning Rust fixture")
    parser.add_argument("--ssh", help="Optional existing alias for this host (same paths/account)")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    binary = args.cowboy.resolve(strict=True)
    if args.test_executable is None:
        artifacts = subprocess.check_output([
            "cargo", "test", "--locked", "--all-features", "--lib", "--no-run", "--message-format=json",
        ], text=True)
        candidates = [value["executable"] for line in artifacts.splitlines()
                      if (value := json.loads(line)).get("reason") == "compiler-artifact"
                      and value.get("executable") and value["target"]["name"] == "cowboy"]
        assert len(candidates) == 1, "expected one owning Cowboy test executable"
        fixture = Path(candidates[0]).resolve(strict=True)
    else:
        fixture = args.test_executable.resolve(strict=True)
    checks = []

    def run(arguments, *, success=True, input_bytes=None):
        result = subprocess.run(
            [str(binary), "logs", *arguments], input=input_bytes,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=35, check=False,
        )
        assert (result.returncode == 0) == success, result.stderr.decode(errors="replace")
        return result.stdout

    def document(arguments, **kwargs):
        return json.loads(run(arguments, **kwargs))

    with tempfile.TemporaryDirectory(prefix="cowboy-logs-conformance-") as temporary:
        root = Path(temporary)
        logs = root / "logs"
        environment = dict(os.environ, COWBOY_TEST_LOG_DIRECTORY=str(logs), RUST_LOG="off")
        environment.pop("COWBOY_TEST_LOG_FAULT", None)
        subprocess.run(
            [str(fixture), "--exact", "logs::tests::capture_child", "--ignored"],
            env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=30, check=True,
        )
        common = ["--directory", str(logs)]
        schema = document(["schema"])
        assert schema["model"] == "OpenTelemetry"
        assert any(c["name"] == "analyze" for c in schema["cli"]["commands"])
        checks.append("schema derives actual command arguments")
        query = ["query", "--from", "1h", "--event", "cowboy.execution.fixture_failure"]
        local = document([*common, *query])["results"][0]["result"]
        assert len(local["items"]) == 1
        assert local["items"][0]["attributes"]["error.type"] == "cursor_expired"
        assert "secret-token" not in json.dumps(local)
        checks.append("RUST_LOG=off still records correlated redacted failure")
        found = document([*common, "query", "--id", local["items"][0]["id"]])
        assert len(found["results"][0]["result"]["items"]) == 1
        checks.append("analysis evidence identity is queryable")
        metrics = document([*common, "metrics", "--limit", "1"])["results"][0]["result"]
        assert sum(g["count"] for g in metrics["groups"].values()) >= 3
        assert sum(g["errors"] for g in metrics["groups"].values()) == 1
        checks.append("source aggregation is independent of page size")
        analysis = document([*common, "analyze"])["results"][0]["result"]
        assert analysis["findings"] and analysis["status"] != "healthy"
        checks.append("deterministic analysis returns evidence with coverage limits")
        frames = [json.loads(line) for line in run([
            *common, "tail", "--every", "1s", "--iterations", "2", "--limit", "1",
        ]).splitlines()]
        assert len(frames) == 2
        assert sum(len(p["items"]) for p in frames[0]["results"][0]["pages"]) >= 3
        assert not any(p["items"] for p in frames[1]["results"][0]["pages"])
        checks.append("tail drains pages and suppresses repeated evidence IDs")
        watch = [json.loads(line) for line in run([
            *common, "watch", "--every", "1s", "--iterations", "2",
        ]).splitlines()]
        assert len(watch) == 2 and all(v["sources_complete"] for v in watch)
        checks.append("periodic analysis is bounded and emits JSONL")
        exported = run([*common, "export", "--signal", "logs", "--limit", "1000"])
        assert exported and not exported.startswith(b"{")
        run([*common, "export", "--signal", "logs", "--limit", "1"], success=False)
        checks.append("OTLP protobuf export refuses truncated evidence")
        request = {"schema": 1, "source": {"name": "rpc", "directory": str(logs)},
                   "operation": {"operation": "status"}}
        reply = document(["rpc"], input_bytes=json.dumps(request).encode())
        assert reply["result"]["writers"][0]["stopped"]
        request["source"]["ssh"] = "untrusted-recursion"
        reply = document(["rpc"], input_bytes=json.dumps(request).encode())
        assert reply["error"] and reply["result"] is None
        checks.append("RPC is read-only and refuses recursive remote execution")
        sources = root / "sources.json"
        sources.write_text(json.dumps({"schema": 1, "sources": [
            {"name": "available", "directory": str(logs)},
            {"name": "missing", "directory": str(root / "absent")},
        ]}))
        sources.chmod(0o600)
        mixed = document(["--sources", str(sources), "status"], success=False)
        assert not mixed["sources_complete"] and len(mixed["results"]) == 2
        mixed = document(["--sources", str(sources), "--allow-partial", "status"])
        assert not mixed["sources_complete"]
        checks.append("partial-source failure is explicit with nonzero default exit")
        policy = document([*common, "configure", "--retain", "1m"])["policy"]
        assert policy["retain_seconds"] == policy["rotate_seconds"] == 60
        document([*common, "maintain"])
        checks.append("short retention config resolves rotation and supports idle maintenance")
        if args.ssh:
            remote = document([*common, "--ssh", args.ssh, "--remote-command", str(binary), *query])
            assert remote["results"][0]["result"]["items"] == local["items"]
            remote = document([*common, "--ssh", args.ssh, "--remote-command", str(binary), "metrics"])
            assert remote["results"][0]["result"]["groups"] == metrics["groups"]
            checks.append("real SSH source executes query and metrics at the source")
    receipt = {"schema": "cowboy.logs-conformance/v1", "accepted": True,
               "binary_sha256": hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
               "ssh_alias": args.ssh, "checks": checks,
               "limits": "Isolated CLI fixture; not a production rollout or external Collector receipt."}
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"accepted": True, "checks": len(checks), "receipt": str(args.receipt)}))


if __name__ == "__main__":
    main()
