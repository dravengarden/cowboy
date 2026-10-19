#!/usr/bin/env python3
"""Offline native Claude delivery measurement; no model or production credentials.

Pinned Claude processes use disposable homes and a scripted loopback API. Tool
and generation barriers make message consumption observable at model requests.
This is runtime conformance, not a delegated agent task. Receipts are append-only.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import tempfile
import threading
import time
import uuid

from claude_native_behavior_probe import Session, clean_root
from execution_environment_claude_probe import tool


def wait_for(predicate, seconds=25):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.02)
    raise RuntimeError("delivery fixture barrier timed out")


def measure(binary, root, phase, priority):
    project = root / "project"
    project.mkdir(parents=True)
    generation_entered, generation_release = threading.Event(), threading.Event()
    marker = "DELIVERY_" + uuid.uuid4().hex
    count = 2 if phase == "tool_batch" else 1
    commands = []
    for index in range(count):
        script = (f"printf ready > {shlex.quote(str(project / f'ready-{index}'))}; "
                  f"while [ ! -f {shlex.quote(str(project / 'release'))} ]; do sleep 0.05; done; "
                  f"printf done > {shlex.quote(str(project / f'done-{index}'))}; echo TOOL_DONE_{index}")
        commands += tool("Bash", {"command": script, "timeout": 30000})

    def first(_):
        generation_entered.set()
        if phase == "generation":
            if not generation_release.wait(20):
                raise RuntimeError("generation barrier not released")
        return [{"type": "text", "text": "FIRST_DONE"}] if phase == "generation" else commands

    observations = []
    session = None

    def following(requests):
        observations.append({
            "request_index": len(requests) - 1,
            "marker": marker in json.dumps(requests[-1].get("messages", [])),
            "terminal_results_seen": sum(frame.get("type") == "result" for frame in session.client.messages),
            "tools_finished": [(project / f"done-{i}").exists() for i in range(count)],
        })
        return [{"type": "text", "text": "DONE"}]

    session = Session(binary, root, "Bash", [first] + [following] * 12)
    stopped = threading.Event()
    errors = []

    def pump():
        while not stopped.is_set():
            try:
                session.client.until(lambda frame: frame.get("type") == "result", timeout=2)
            except Exception as error:
                if "timed out" not in str(error) and not stopped.is_set():
                    errors.append(str(error))
                    return

    thread = threading.Thread(target=pump, daemon=True)
    thread.start()
    try:
        def send(text, urgency=None):
            message = {"type": "user", "message": {"role": "user", "content": text},
                       "parent_tool_use_id": None, "session_id": "", "uuid": str(uuid.uuid4())}
            if urgency != "default" and urgency is not None:
                message["priority"] = urgency
            session.client.send(message)

        send("Run the controlled delivery fixture.")
        # Native may serialize Bash calls in a batch. Requiring both barriers
        # here deadlocks a legitimate serial scheduler before any delivery test.
        wait_for(lambda: generation_entered.is_set() if phase == "generation" else
                 (project / "ready-0").exists())
        before = len(session.api.requests)
        send(marker, priority)
        # A bounded pre-release observation, not a proof of all timing races.
        observation_deadline = time.monotonic() + 1
        while time.monotonic() < observation_deadline and len(session.api.requests) == before:
            time.sleep(0.02)
        pre_release = len(session.api.requests)
        generation_release.set()
        (project / "release").touch()
        wait_for(lambda: any(marker in json.dumps(request.get("messages", []))
                             for request in session.api.requests), seconds=30)
        wait_for(lambda: any(frame.get("type") == "result" for frame in session.client.messages))
        requests = session.api.requests
        consumed_at = [i for i, request in enumerate(requests)
                       if marker in json.dumps(request.get("messages", []))]
        marker_messages = requests[consumed_at[0]]["messages"]
        tool_results = [block for message in marker_messages
                        for block in message.get("content", []) if isinstance(block, dict)
                        and block.get("type") == "tool_result"]
        result = {
            "phase": phase, "priority": priority, "requests_before_injection": before,
            "requests_before_release": pre_release, "marker_request_indexes": consumed_at,
            "tool_results_at_first_marker": "TOOL_DONE_" in json.dumps(tool_results),
            "request_observations": observations,
            "first_marker_messages": marker_messages,
            "tools_finished": [(project / f"done-{i}").exists() for i in range(count)],
            "results": [{k: frame.get(k) for k in ["subtype", "is_error", "session_id", "stop_reason"]}
                        for frame in session.client.messages if frame.get("type") == "result"],
            "errors": errors, "api_error": session.api.failure,
        }
        return result
    finally:
        generation_release.set()
        (project / "release").touch()
        stopped.set()
        thread.join(timeout=3)
        session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--phase", action="append", choices=["generation", "tool", "tool_batch"])
    parser.add_argument("--priority", action="append", choices=["default", "now", "next", "later"])
    args = parser.parse_args()
    if not args.receipt.is_absolute() or args.receipt.exists():
        raise SystemExit("new absolute receipt required")
    results = []
    with tempfile.TemporaryDirectory(prefix="cowboy-native-delivery-", dir=clean_root()) as temp:
        for phase in args.phase or ["tool", "tool_batch", "generation"]:
            for priority in args.priority or ["default", "now", "next", "later"]:
                result = measure(args.claude, Path(temp) / f"{phase}-{priority}", phase, priority)
                results.append(result)
                print(json.dumps(result), flush=True)
    args.receipt.write_text(json.dumps({
        "binary_sha256": hashlib.sha256(args.claude.read_bytes()).hexdigest(),
        "results": results,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
