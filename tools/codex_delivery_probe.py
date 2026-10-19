#!/usr/bin/env python3
"""Pinned native Codex turn/steer consumption against an isolated scripted API."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import tempfile
import threading
import time
import uuid

from execution_environment_codex_turn_probe import Api, app, final
from plugin_runtime_conformance import closed_environment
from native_delivery_probe import wait_for


def measure(binary, root, phase):
    root.mkdir(parents=True)
    project = root / "project"
    project.mkdir()
    entered, release = threading.Event(), threading.Event()
    marker = "STEER_" + uuid.uuid4().hex
    def first(_):
        entered.set()
        if phase == "generation":
            if not release.wait(20):
                raise RuntimeError("generation fixture not released")
            return final("initial-final")
        script = (f"printf ready > {shlex.quote(str(project / 'ready'))}; "
                  f"while [ ! -f {shlex.quote(str(project / 'release'))} ]; do sleep 0.05; done; "
                  "echo TOOL_DONE")
        return {"type": "function_call", "call_id": "barrier-tool", "name": "exec_command",
                "arguments": json.dumps({"cmd": script, "yield_time_ms": 10000, "max_output_tokens": 1000})}
    api = Api([first] + [final(f"final-{i}") for i in range(8)])
    environment = closed_environment(root / "home")
    environment["COWBOY_OFFLINE_FIXTURE_KEY"] = "not-a-production-credential"
    home = Path(environment["CODEX_HOME"])
    home.mkdir()
    (home / "config.toml").write_text(
        'model = "gpt-6-astra"\nmodel_provider = "cowboy_fixture"\n'
        '[features]\nremote_control = false\nshell_snapshot = false\n'
        '[model_providers.cowboy_fixture]\nname = "Offline fixture"\n'
        f'base_url = "http://127.0.0.1:{api.server_port}/v1"\n'
        'wire_api = "responses"\nenv_key = "COWBOY_OFFLINE_FIXTURE_KEY"\n'
        'request_max_retries = 0\nstream_max_retries = 0\n')
    client = app(str(binary), environment, project)
    frames = []
    frame = client.frame
    def record(deadline):
        value = frame(deadline)
        frames.append(value)
        return value
    client.frame = record
    try:
        thread_id = client.request("thread/start", {"cwd": str(project), "approvalPolicy": "never",
                                                     "sandbox": "danger-full-access"})["thread"]["id"]
        turn_id = client.request("turn/start", {"threadId": thread_id,
            "input": [{"type": "text", "text": "Run the offline barrier fixture.", "text_elements": []}]})["turn"]["id"]
        wait_for(lambda: entered.is_set() if phase == "generation" else (project / "ready").exists())
        accepted = client.request("turn/steer", {"threadId": thread_id, "expectedTurnId": turn_id,
            "input": [{"type": "text", "text": marker, "text_elements": []}]})
        mismatch = client.request("turn/steer", {"threadId": thread_id, "expectedTurnId": "not-this-turn",
            "input": [{"type": "text", "text": "MUST_NOT_APPEAR", "text_elements": []}]}, expect_error=True)
        release.set()
        (project / "release").touch()
        deadline = time.monotonic() + 40
        while True:
            completed = next((x for x in frames if x.get("method") == "turn/completed"
                              and x["params"].get("threadId") == thread_id
                              and x["params"]["turn"]["id"] == turn_id), None)
            if completed:
                break
            client.frame(deadline)
        consumed = [i for i, request in enumerate(api.requests) if marker in json.dumps(request.get("input"))]
        assert consumed, "acknowledged steering was not consumed before turn completion"
        assert "MUST_NOT_APPEAR" not in json.dumps(api.requests), "mismatched steer reached model"
        idle = client.request("turn/steer", {"threadId": thread_id, "expectedTurnId": turn_id,
            "input": [{"type": "text", "text": "MUST_NOT_APPEAR", "text_elements": []}]}, expect_error=True)
        return {"phase": phase, "accepted": accepted, "target_turn": turn_id,
                "mismatch_error": mismatch, "idle_error": idle, "consumed_request_indexes": consumed,
                "turn_status": completed["params"]["turn"]["status"], "api_error": api.failure,
                "model_inputs": [request.get("input") for request in api.requests],
                "turn_events": [x for x in frames if x.get("method") in ["turn/started", "turn/completed"]]}
    finally:
        release.set()
        (project / "release").touch()
        client.close()
        api.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    args = parser.parse_args()
    if not args.receipt.is_absolute() or args.receipt.exists():
        raise SystemExit("new absolute receipt required")
    with tempfile.TemporaryDirectory(prefix="cowboy-codex-delivery-") as temp:
        results = [measure(args.codex, Path(temp) / phase, phase) for phase in ["tool", "generation"]]
    args.receipt.write_text(json.dumps({"binary_sha256": hashlib.sha256(args.codex.read_bytes()).hexdigest(),
                                       "results": results}, indent=2) + "\n")
    print(json.dumps([{k: v for k, v in result.items() if k != "model_inputs"} for result in results]))


if __name__ == "__main__":
    main()
