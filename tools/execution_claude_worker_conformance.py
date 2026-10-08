#!/usr/bin/env python3
"""Actual packaged Claude turns through Cowboy's worker endpoint and keeper.

The Rust execution-worker gate owns disposable Machines, target worktree,
transport outage and cleanup. This client supplies only loopback scripted model
responses. It never reads subscription state or sends a real inference request.
"""
import argparse
import base64
import errno
import hashlib
import json
import os
import random
import re
from pathlib import Path
import shutil
import socket
import struct
import threading
import time
import zlib

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool as native_tool
from execution_environment_probe import Executor, ProbeFailure, require
from plugin_runtime_conformance import closed_environment
from matrix_execution_fixture import MatrixFixture
from claude_shell_cases import CASES as SHELL_CASES, normalize as shell_normalize
from claude_file_cases import CASES as FILE_CASES, effects as file_effects, normalize as file_normalize, setup as file_setup
from claude_pdf_cases import CASES as PDF_CASES, normalize as pdf_normalize, notes as pdf_notes, setup as pdf_setup
from claude_mcp_cases import CALLS as MCP_CALLS, expected as mcp_expected, setup as mcp_setup
from claude_skill_cases import (CASES as SKILL_CASES, contents as skill_contents, listing as skill_listing,
                                normalize as skill_normalize, setup as skill_setup)
from claude_lifecycle_cases import (CASES as LIFECYCLE_CASES, STOP as LIFECYCLE_STOP, alive as lifecycle_alive,
                                    left_running_notified as lifecycle_notified, normalize as lifecycle_normalize,
                                    stop_all as lifecycle_stop_all, task_id as lifecycle_task_id)


def tool(name, arguments):
    return native_tool(name, arguments)


def outputs(request):
    for message in request.get("messages", []):
        content = message.get("content", [])
        if isinstance(content, list):
            for block in content:
                if block.get("type") == "tool_result":
                    yield block


def background_id(requests):
    for block in reversed(list(outputs(requests[-1]))):
        for item in block.get("content", []) if isinstance(block.get("content"), list) else [{"type": "text", "text": block.get("content", "")}]:
            if item.get("type") == "text":
                handle = re.search(r"cowboy-task://([a-f0-9-]{36})", item["text"])
                if handle:
                    return handle[1]
    print("background output diagnostic:", [json.dumps(block)[:450] for block in outputs(requests[-1])])
    raise ProbeFailure("background tool did not return a handle")


def stop_background(requests):
    return tool("TaskStop", {"task_id": background_id(requests)})


def read_background(requests):
    return tool("Read", {"file_path": "cowboy-task://" + background_id(requests)})


def text_blocks(message):
    content = message.get("content", [])
    if isinstance(content, str):
        return [content]
    return [block.get("text", "") for block in content if block.get("type") == "text"]


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)


def agent_phases(args, api, client, native, session, context_checked, checks):
    """Native background agents through the packaged Mod and real keeper.

    Parent and child requests interleave in a native-chosen order, so one
    stateful router answers by conversation content instead of position.
    """
    issued = {}
    observations = {}
    state = {"agents": {}, "noted": set(), "read": {}, "finals": 0, "handled": set()}
    loops = {name: f"printf {name}_started >> {name}.txt; while :; do sleep 1; printf tick >> {name}.txt; done"
             for name in ["child-b", "child-c", "peer"]}

    def issue(kind, name, arguments):
        call = tool(name, arguments)
        issued[call[0]["id"]] = kind
        return call

    def router(requests):
        request = requests[-1]
        messages = request.get("messages", [])
        users = [message for message in messages if message.get("role") == "user"]
        prompt = " ".join(text for message in users for text in text_blocks(message))
        results = [block for message in messages if isinstance(message.get("content"), list)
                   for block in message["content"] if block.get("type") == "tool_result"]
        for marker in ["AGENT_CHILD_A", "AGENT_CHILD_B", "AGENT_CHILD_C"]:
            if marker not in prompt:
                continue
            if marker != "AGENT_CHILD_A":
                require(not results, f"{marker} continued after cancellation")
                return issue(marker, "Bash", {"command": loops["child-" + marker[-1].lower()], "timeout": 600000})
            again = "AGENT_CHILD_A_CONTINUE" in prompt
            if len(results) == (1 if again else 0):
                return issue(marker, "Bash", {"command": f"pwd; printf {'again' if again else 'child_once'} >> child-a.txt"})
            return [{"type": "text", "text": "A_AGAIN" if again else "A_DONE"}]
        last = messages[-1]
        latest = " ".join(text_blocks(last))
        # Earlier fixture phases can leave their interrupted tool result in the
        # same user message as a new phase prompt; answer only this router's calls.
        last_results = [block for block in last.get("content", []) if isinstance(last.get("content"), list)
                        and block.get("type") == "tool_result" and block.get("tool_use_id") in issued]
        # Native delivers a notification as its own prompt when idle, or
        # inside a tool result/queued attachment during an active turn.
        notifications = [notification for notification in
                         re.findall(r"<task-notification>[\s\S]*?</task-notification>", "\n".join(strings(last)))
                         if notification not in state["handled"]]
        for block in last_results:
            kind = issued[block["tool_use_id"]]
            encoded = json.dumps(block)
            if kind == "agent":
                state["agents"][len(state["agents"])] = re.search(r"cowboy-agent://([a-zA-Z0-9_-]+)", encoded)[1]
                state["launch"] = encoded
            elif kind == "peer":
                state["peer"] = re.search(r"cowboy-task://([a-f0-9-]{36})", encoded)[1]
            elif isinstance(kind, tuple):
                state["read"][kind[1]] = encoded
                state["noted"].add(kind[1])
            else:
                state.setdefault("results", {})[kind] = encoded
        reads = []
        for notification in notifications:
            state["handled"].add(notification)
            agent = re.search(r"<task-id>([^<]+)</task-id>", notification)[1]
            if "<status>completed</status>" in notification:
                reads.extend(issue(("read", agent), "Read", {"file_path": "cowboy-agent://" + agent}))
            else:
                state["noted"].add(agent)
        if reads:
            return reads
        if last_results:
            state["finals"] += 1
            return [{"type": "text", "text": "PARENT_DONE"}]
        if "AGENT_PHASE_A" in latest:
            return issue("agent", "Agent", {"description": "Target child A", "prompt": "AGENT_CHILD_A run the command."})
        if "CONTINUE_A" in latest:
            return issue("send", "SendMessage", {"to": state["agents"][0], "summary": "Run again",
                                                 "message": "AGENT_CHILD_A_CONTINUE run the command again."})
        if "AGENT_PHASE_B" in latest:
            return (issue("agent", "Agent", {"description": "Target child B", "prompt": "AGENT_CHILD_B loop."})
                    + issue("peer", "Bash", {"command": loops["peer"], "run_in_background": True}))
        if "PING_PARENT" in latest:
            state["finals"] += 1
            return [{"type": "text", "text": "PONG"}]
        if "STOP_CHILD_B" in latest:
            return issue("stop-agent", "TaskStop", {"task_id": state["agents"][1]})
        if "STOP_PEER" in latest:
            return issue("stop-peer", "TaskStop", {"task_id": state["peer"]})
        if "AGENT_PHASE_C" in latest:
            return issue("agent", "Agent", {"description": "Target child C", "prompt": "AGENT_CHILD_C loop."})
        if "READ_AGENTS" in latest:
            return [block for index in range(3)
                    for block in issue(("read", state["agents"][index]), "Read",
                                       {"file_path": "cowboy-agent://" + state["agents"][index]})]
        # A phase prompt can share its message with a queued notification;
        # answer the prompt above, and a notification alone here.
        if notifications:
            state["finals"] += 1
            return [{"type": "text", "text": "NOTED"}]
        raise ProbeFailure("unexpected agent phase request: " + latest[:200])

    api.steps.extend([router] * 80)
    home = str(args.runtime.parent / "claude-home")

    def run(text, done, timeout=90):
        # Each main-thread turn ends with one router text answer and one result
        # frame. Waiting for both keeps late notification turns in this phase.
        start = len(client.messages)
        finals = state["finals"]
        client.send({"type": "user", "message": {"role": "user", "content": text},
                     "parent_tool_use_id": None, "session_id": ""})
        deadline = time.monotonic() + timeout
        try:
            while not (done() and sum(frame.get("type") == "result" for frame in client.messages[start:])
                       >= max(1, state["finals"] - finals)):
                client.until(lambda frame: frame.get("type") == "result",
                             timeout=max(0.1, deadline - time.monotonic()))
        except ProbeFailure:
            print("agent phase diagnostic:", text, api.failure, json.dumps(
                {key: value for key, value in state.items() if key != "notifications"}, default=sorted)[:3000])
            print("agent phase frames:", json.dumps([{key: frame.get(key) for key in
                                                      ["type", "subtype", "task_id", "status", "result", "is_error"]}
                                                     for frame in client.messages[start:]])[:4000])
            if api.requests:
                print("agent phase last request:", json.dumps(api.requests[-1].get("messages", [])[-2:])[:3000])
            raise
        require(api.failure is None, api.failure or "scripted API failed")
        return client.messages[start:]

    def growing(path, expected):
        before = path.read_text()
        time.sleep(1.5)
        require((path.read_text() != before) == expected,
                f"{path.name} {'stopped' if expected else 'kept running'} unexpectedly")

    def stops(path, limit=30):
        # Observe actual target quiescence, not the stop acknowledgement.
        started = time.monotonic()
        while True:
            before = path.read_text()
            time.sleep(1.5)
            if path.read_text() == before:
                return round(time.monotonic() - started - 1.5, 2)
            require(time.monotonic() - started < limit, f"{path.name} kept running after its agent stopped")

    def wait_file(path):
        deadline = time.monotonic() + 30
        while not path.exists():
            require(time.monotonic() < deadline, f"{path.name} did not start")
            time.sleep(0.05)

    frames = run("AGENT_PHASE_A", lambda: len(state["agents"]) == 1 and state["agents"][0] in state["noted"])
    agent_a = state["agents"][0]
    require((args.target / "child-a.txt").read_text() == "child_once", "child command did not run once on target")
    require(not (args.runtime / "child-a.txt").exists(), "child command escaped to runtime")
    require(home not in state["launch"] and "cowboy-agent://" + agent_a in state["launch"],
            "native Agent launch exposed its runtime output file")
    require("A_DONE" in state["read"][agent_a], "handle Read did not return the recorded final answer")
    completion = [frame for frame in frames if frame.get("subtype") == "task_notification"
                  and frame.get("task_id") == agent_a]
    require(len(completion) == 1 and completion[0]["status"] == "completed" and
            completion[0]["output_file"] == "cowboy-agent://" + agent_a,
            "client completion frame kept the runtime output locator")
    # Each turn's system/init describes the runtime process itself (cwd, local
    # messaging socket) to the runtime-side ACP client; it is not agent output.
    agent_frames = [frame for frame in frames if frame.get("subtype") != "init"]
    for frame in agent_frames:
        encoded = json.dumps(frame)
        if home in encoded:
            position = encoded.index(home)
            print("runtime home frame diagnostic:", frame.get("type"), frame.get("subtype"),
                  encoded[max(0, position - 400):position + 200])
    require(home not in json.dumps(agent_frames), "agent phase client frames exposed the runtime home")
    context_checked(api.requests)
    checks.extend(["native_agent_child_tools_route_to_target", "native_agent_launch_and_notification_use_handle",
                   "native_agent_handle_reads_recorded_final_answer", "native_agent_client_frames_use_handle"])

    state["noted"].discard(agent_a)
    run("CONTINUE_A", lambda: agent_a in state["noted"])
    require((args.target / "child-a.txt").read_text() == "child_onceagain", "continued child did not run once")
    require("A_AGAIN" in state["read"][agent_a], "continued agent answer was not recorded")
    context_checked(api.requests)
    checks.append("native_agent_send_message_continues_on_target")

    run("AGENT_PHASE_B", lambda: len(state["agents"]) == 2 and "peer" in state)
    agent_b = state["agents"][1]
    wait_file(args.target / "child-b.txt")
    wait_file(args.target / "peer.txt")
    growing(args.target / "child-b.txt", True)
    # A held child call must not stall the parent: Mods fetches block other
    # native work while pending, so the bridge keeps each observation short.
    ping_sent = time.monotonic()
    run("PING_PARENT", lambda: True)
    observations["parent_turn_during_child_command_seconds"] = round(time.monotonic() - ping_sent, 2)
    require(observations["parent_turn_during_child_command_seconds"] < 5,
            "parent turn stalled behind a running child command")
    stop_sent = time.monotonic()
    run("STOP_CHILD_B", lambda: agent_b in state["noted"])
    stop_turn = round(time.monotonic() - stop_sent, 2)
    require("local_agent" in state["results"]["stop-agent"], "native TaskStop did not stop the agent task")
    observations["taskstop_turn_seconds"] = stop_turn
    require(stop_turn < 8, "native TaskStop stalled behind a running child command")
    observations["taskstop_target_quiescent_after_turn_seconds"] = stops(args.target / "child-b.txt")
    growing(args.target / "peer.txt", True)
    run("STOP_PEER", lambda: "stop-peer" in state.get("results", {}))
    require("Successfully stopped task: " in state["results"]["stop-peer"], "peer job did not stop")
    growing(args.target / "peer.txt", False)
    context_checked(api.requests)
    checks.extend(["native_agent_taskstop_cancels_child_target_command", "native_agent_stop_preserves_peer_job",
                   "parent_turns_and_taskstop_progress_during_child_command"])

    run("AGENT_PHASE_C", lambda: len(state["agents"]) == 3)
    # The child's request may follow the parent's result; count after its
    # target command is observed running.
    wait_file(args.target / "child-c.txt")
    growing(args.target / "child-c.txt", True)
    requests = len(api.requests)
    client.send({"type": "control_request", "request_id": "interrupt-agents", "request": {"subtype": "interrupt"}})
    client.until(lambda frame: frame.get("type") == "control_response" and
                 frame["response"].get("request_id") == "interrupt-agents")
    observations["interrupt_target_quiescent_seconds"] = stops(args.target / "child-c.txt")
    if len(api.requests) != requests:
        print("post-interrupt requests:", json.dumps([request.get("messages", [])[-1:]
                                                      for request in api.requests[requests:]])[:3000])
    require(len(api.requests) == requests, "interrupt cleanup started a model turn")
    checks.append("native_interrupt_stops_idle_background_agent_target_command")

    client.close()
    client = native(session)
    try:
        client.ready()
    except ProbeFailure:
        client.stderr.seek(0)
        print("resumed agent session stderr:", client.stderr.read().decode()[-3000:])
        raise
    state["read"].clear()
    run("READ_AGENTS", lambda: len(state["read"]) == 3)
    require("A_AGAIN" in state["read"][agent_a], "resumed handle lost the final answer")
    for agent in [agent_b, state["agents"][2]]:
        require("was stopped" in state["read"][agent], "resumed handle did not report the stopped agent")
    context_checked(api.requests)
    checks.append("native_agent_outcomes_survive_cold_resume")
    return client, observations


def permission_phases(args, api, client, context_checked, checks):
    """Native rules and mode gate target tools; asks reach the SDK host.

    The fixture is the host: it switches modes and answers approvals. Every
    case checks actual target bytes, not only the tool result prose.
    """
    issued = {}
    asked = []
    answers = []

    def router(requests):
        last = requests[-1]["messages"][-1]
        if isinstance(last.get("content"), list) and any(
                block.get("type") == "tool_result" and block.get("tool_use_id") in issued
                for block in last["content"]):
            return [{"type": "text", "text": "PERM_DONE"}]
        latest = " ".join(text_blocks(last))
        for marker, call in steps.items():
            if marker in latest:
                calls = call() if callable(call) else call
                for block in calls:
                    issued[block["id"]] = marker
                return calls
        raise ProbeFailure("unexpected permission phase request: " + latest[:200])

    def answer(request):
        asked.append(request)
        return answers.pop(0)

    outside = args.target.parent / "perm-outside.txt"
    absolute = args.target / "perm-absolute.txt"
    (args.target / "perm-keep.txt").write_text("keep\n")
    steps = {
        "PERM_DENY": tool("Bash", {"command": "rm -f perm-keep.txt"}),
        "PERM_AMEND": tool("Write", {"file_path": "perm-amend.txt", "content": "original\n"}),
        "PERM_READ": tool("Read", {"file_path": "perm-keep.txt"}),
        "PERM_ACCEPT_EDITS": lambda: (tool("Edit", {"file_path": "perm-keep.txt", "old_string": "keep",
                                                    "new_string": "edited"})
                                      + tool("Write", {"file_path": str(absolute), "content": "absolute\n"})),
        "PERM_OUTSIDE": tool("Write", {"file_path": str(outside), "content": "outside\n"}),
        "PERM_COLLISION": tool("Write", {"file_path": str(args.runtime / "perm-collision.txt"), "content": "x\n"}),
        "PERM_SYMLINK": tool("Write", {"file_path": "perm-dir-link/escaped.txt", "content": "through link\n"}),
        "PERM_WRITE_LINK": tool("Write", {"file_path": "perm-link.txt", "content": "onto link\n"}),
        "PERM_DONT_ASK": tool("Bash", {"command": "rm -f perm-keep.txt"}),
        "PERM_BYPASS": tool("Bash", {"command": "rm -f perm-amend.txt"}),
    }
    del api.steps[len(api.requests):]
    api.steps.extend([router] * 40)
    client.permission = answer

    def mode(value):
        request_id = "fixture-mode-" + value
        client.send({"type": "control_request", "request_id": request_id,
                     "request": {"subtype": "set_permission_mode", "mode": value}})
        reply = client.until(lambda frame: frame.get("type") == "control_response" and
                             frame["response"].get("request_id") == request_id)
        require(reply["response"]["subtype"] == "success", "native rejected the permission mode")

    def result_of(marker):
        blocks = [block for block in outputs(api.requests[-1]) if issued.get(block.get("tool_use_id")) == marker]
        require(blocks, "permission phase result missing")
        return blocks

    mode("default")
    answers.append({"behavior": "deny", "message": "fixture host denied"})
    client.prompt(text="PERM_DENY", timeout=60)
    require(len(asked) == 1 and asked[0]["tool_name"] == "Bash" and
            asked[0]["input"] == {"command": "rm -f perm-keep.txt"} and
            asked[0]["tool_use_id"] in issued and asked[0]["permission_suggestions"] == [],
            "default mode did not ask the host in native shape")
    require("fixture host denied" in json.dumps(result_of("PERM_DENY")) and
            (args.target / "perm-keep.txt").read_text() == "keep\n", "denied command reached the target")
    answers.append({"behavior": "allow", "updatedInput": {"file_path": "perm-amend.txt", "content": "amended\n"}})
    client.prompt(text="PERM_AMEND", timeout=60)
    require(len(asked) == 2 and (args.target / "perm-amend.txt").read_text() == "amended\n",
            "approved write did not run the host's amended input")
    client.prompt(text="PERM_READ", timeout=60)
    require(len(asked) == 2, "a read-only call inside the workspace asked for approval")
    checks.extend(["native_permission_ask_denial_has_no_target_effect", "native_permission_approval_runs_amended_input",
                   "native_permission_allow_runs_without_prompt"])
    mode("acceptEdits")
    client.prompt(text="PERM_ACCEPT_EDITS", timeout=60)
    require(len(asked) == 2 and (args.target / "perm-keep.txt").read_text() == "edited\n" and
            absolute.read_text() == "absolute\n", "acceptEdits asked for edits inside the target workspace")
    answers.append({"behavior": "allow"})
    client.prompt(text="PERM_OUTSIDE", timeout=60)
    require(len(asked) == 3 and outside.read_text() == "outside\n", "an outside-workspace write did not ask")
    # A target path under the runtime's own workspace is still outside the
    # target workspace; a symlink inside it can lead a write outside.
    context_checked(api.requests)
    checked = len(api.requests)
    link_destination = args.target.parent / "perm-link-destination.txt"
    link_destination.write_text("destination\n")
    (args.target / "perm-link.txt").symlink_to(link_destination)
    outside_directory = args.target.parent / "perm-outside-directory"
    outside_directory.mkdir()
    (args.target / "perm-dir-link").symlink_to(outside_directory, target_is_directory=True)
    for marker in ["PERM_COLLISION", "PERM_SYMLINK"]:
        answers.append({"behavior": "deny", "message": "fixture host denied"})
        client.prompt(text=marker, timeout=60)
    require(len(asked) == 5 and not (args.runtime / "perm-collision.txt").exists() and
            not (outside_directory / "escaped.txt").exists(),
            "acceptEdits auto-approved a write outside the target workspace")
    checks.extend(["accept_edits_maps_target_workspace_paths", "accept_edits_asks_for_runtime_path_collisions_and_escaping_symlinks"])
    mode("dontAsk")
    client.prompt(text="PERM_DONT_ASK", timeout=60)
    require(len(asked) == 5 and (args.target / "perm-keep.txt").exists() and
            any(block.get("is_error") for block in result_of("PERM_DONT_ASK")),
            "dontAsk ran or prompted for a call that needs approval")
    checks.append("dont_ask_denies_without_prompt_or_effect")
    mode("bypassPermissions")
    client.prompt(text="PERM_BYPASS", timeout=60)
    require(len(asked) == 5 and not (args.target / "perm-amend.txt").exists(),
            "bypassPermissions did not run without approval")
    # Natively, Write onto a symbolic link is refused in every mode.
    client.prompt(text="PERM_WRITE_LINK", timeout=60)
    require(len(asked) == 5 and (args.target / "perm-link.txt").is_symlink() and
            link_destination.read_text() == "destination\n" and
            "symbolic link" in json.dumps(result_of("PERM_WRITE_LINK")),
            "Write replaced or followed a symbolic link")
    checks.append("write_onto_symlink_is_refused_in_every_mode")
    require(not (args.runtime / "perm-amend.txt").exists() and not (args.runtime / "perm-keep.txt").exists(),
            "permission phase touched the runtime workspace")
    # The collision case's own scripted input names a runtime path; nothing
    # else may add runtime context to later requests.
    collision = str(args.runtime / "perm-collision.txt")
    for request in api.requests[checked:]:
        encoded = json.dumps(request)
        require(encoded.count(str(args.runtime)) == encoded.count(collision) and
                str(args.runtime.parent / "claude-home") not in encoded, "runtime context reached model")
    checks.append("bypass_permissions_runs_without_prompt")
    client.permission = None


def shell_phases(args, api, client, shell_results, checks):
    """Shared Bash cases in one turn; results are compared with native-local ones."""
    (args.target / "sub").mkdir(exist_ok=True)
    steps = [tool("Bash", arguments) for _, arguments in SHELL_CASES]
    ids = {call[0]["id"]: index for index, call in enumerate(steps)}

    def router(requests):
        # Keyed on the conversation, so a background child's request cannot
        # take a step.
        last = requests[-1]["messages"][-1]
        done = [ids[block["tool_use_id"]] for block in (last.get("content") if isinstance(last.get("content"), list) else [])
                if block.get("type") == "tool_result" and block.get("tool_use_id") in ids]
        if done:
            following = max(done) + 1
            return steps[following] if following < len(steps) else [{"type": "text", "text": "SHELL_DONE"}]
        if "Run the shell parity fixture." in " ".join(text_blocks(last)):
            return steps[0]
        raise ProbeFailure("unexpected shell phase request")

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(steps) + 20))
    client.prompt(text="Run the shell parity fixture.", timeout=120)
    for request in reversed(api.requests):
        for block in outputs(request):
            if block.get("tool_use_id") in ids and SHELL_CASES[ids[block["tool_use_id"]]][0] not in shell_results:
                content = block.get("content")
                if isinstance(content, list):
                    content = "".join(item.get("text", "") for item in content)
                shell_results[SHELL_CASES[ids[block["tool_use_id"]]][0]] = {
                    "is_error": block.get("is_error", False),
                    "content": shell_normalize(content, args.target) if isinstance(content, str) else content,
                }
    require(len(shell_results) == len(SHELL_CASES), "shell parity results missing")
    # Native-local results for the same cases. A Mods-answered error is
    # wrapped in <tool_use_error> tags; it is the one accepted difference.
    native = json.loads((Path(__file__).parent / "claude_shell_native_baseline.json").read_text())["results"]
    differences = {}
    for name, expected in native.items():
        actual = dict(shell_results.get(name, {}))
        if actual.get("is_error") and isinstance(actual.get("content"), str):
            actual["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", actual["content"], flags=re.S)
        if actual != expected:
            differences[name] = {"native": expected, "remote": shell_results.get(name)}
    require(not differences, "Bash results differ from native-local: " + json.dumps(differences)[:3000])
    checks.append("bash_results_match_native_local")


CONTEXT_FILES = {
    "CLAUDE.md": "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL\n@docs/imported.md\n",
    "docs/imported.md": "TARGET_IMPORTED_BY_AT\n",
    "CLAUDE.local.md": "TARGET_CLAUDE_LOCAL_MD\n",
    ".claude/CLAUDE.md": "TARGET_DOT_CLAUDE_MD\n",
    ".claude/rules/always.md": "TARGET_RULE_ALWAYS\n",
    ".claude/rules/scoped.md": "---\npaths:\n  - \"ctx-sub/**\"\n---\nTARGET_RULE_SCOPED\n",
    "ctx-sub/CLAUDE.md": "TARGET_NESTED_CLAUDE_MD\n",
    "ctx-sub/AGENTS.md": "TARGET_NESTED_AGENTS_MUST_NOT_REACH_MODEL\n",
    "ctx-sub/file.txt": "nested file\n",
}


def write_context_fixture(target):
    for name, text in CONTEXT_FILES.items():
        path = target / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    (target.parent / "CLAUDE.md").write_text("TARGET_PARENT_CLAUDE_MD\n")


def context_phases(args, api, client, checks):
    """Target instruction files, environment and Git context in native's form."""
    first = api.requests[0]["messages"][0]
    text = "\n".join(block.get("text", "") for block in first["content"] if isinstance(block, dict))
    target = str(args.target)
    expected = [
        f"Contents of {args.target.parent}/CLAUDE.md (project instructions, checked into the codebase):\n\nTARGET_PARENT_CLAUDE_MD",
        f"Contents of {target}/CLAUDE.md (project instructions, checked into the codebase):\n\nTARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL",
        f"Contents of {target}/docs/imported.md (project instructions, checked into the codebase):\n\nTARGET_IMPORTED_BY_AT",
        f"Contents of {target}/.claude/CLAUDE.md (project instructions, checked into the codebase):\n\nTARGET_DOT_CLAUDE_MD",
        f"Contents of {target}/.claude/rules/always.md (project instructions, checked into the codebase):\n\nTARGET_RULE_ALWAYS",
        f"Contents of {target}/CLAUDE.local.md (user's private project instructions, not checked in):\n\nTARGET_CLAUDE_LOCAL_MD",
    ]
    positions = [text.find(item) for item in expected]
    require("Codebase and user instructions are shown below." in text and all(position >= 0 for position in positions)
            and positions == sorted(positions) and "TARGET_RULE_SCOPED" not in text and
            "TARGET_GUIDANCE_MUST_REACH_MODEL" not in text and "TARGET_NESTED" not in text,
            "target instructions differ from native's: " + text[:4000])
    checks.append("target_instruction_files_render_as_native")
    require(f"# Environment\nYou have been invoked in the following environment: \n - Primary working directory: {target}\n"
            " - Is a git repository: true\n - Platform: linux\n - Shell: " in text and " - OS Version: Linux " in text,
            "environment block differs from native's: " + text[:2000])
    require("# gitStatus\nThis is the git status at the start of the conversation." in text and
            "\n\nCurrent branch: " in text and "\n\nMain branch (you will usually use this for PRs): " in text and
            "\n\nStatus:\n" in text and "\n\nRecent commits:\n" in text,
            "git status block differs from native's: " + text[max(0, text.find("Codebase and user")):][-3000:])
    checks.append("target_environment_and_git_blocks_render_as_native")

    issued = {}
    steps = {"CONTEXT_NESTED": [tool("Read", {"file_path": "ctx-sub/file.txt"}),
                                tool("Read", {"file_path": "ctx-sub/file.txt"})]}

    def router(requests):
        last = requests[-1]["messages"][-1]
        blocks = last.get("content") if isinstance(last.get("content"), list) else []
        done = [issued[block["tool_use_id"]] for block in blocks
                if block.get("type") == "tool_result" and block.get("tool_use_id") in issued]
        index = done[-1] + 1 if done else 0
        if not done and "CONTEXT_NESTED" not in " ".join(text_blocks(last)):
            raise ProbeFailure("unexpected context phase request")
        if index < len(steps["CONTEXT_NESTED"]):
            call = steps["CONTEXT_NESTED"][index]
            issued[call[0]["id"]] = index
            return call
        return [{"type": "text", "text": "CONTEXT_DONE"}]

    del api.steps[len(api.requests):]
    api.steps.extend([router] * 10)
    client.prompt(text="CONTEXT_NESTED", timeout=60)
    messages = {}
    for request in api.requests:
        for message in request.get("messages", []):
            content = message.get("content")
            for block in content if isinstance(content, list) else []:
                if block.get("type") == "tool_result" and block.get("tool_use_id") in issued:
                    messages[issued[block["tool_use_id"]]] = json.dumps(message, ensure_ascii=False)
    print("nested context diagnostic:", messages.get(0, "")[:1500])
    require(f"<system-reminder>\\nContents of {target}/ctx-sub/CLAUDE.md:\\n\\nTARGET_NESTED_CLAUDE_MD" in messages.get(0, "") and
            f"Contents of {target}/.claude/rules/scoped.md:\\n\\nTARGET_RULE_SCOPED" in messages.get(0, "") and
            "TARGET_NESTED_AGENTS" not in messages.get(0, "") and "TARGET_NESTED_CLAUDE_MD" not in messages.get(1, ""),
            "nested instructions differ from native's: " + messages.get(0, "")[:2000])
    checks.append("read_attaches_nested_instructions_once_as_native")


def file_phases(args, api, client, checks):
    """File tool results and on-disk effects compared with native-local ones."""
    inodes = file_setup(args.target)
    steps = [tool(name, arguments) for _, (name, arguments) in FILE_CASES]
    ids = {call[0]["id"]: index for index, call in enumerate(steps)}

    def router(requests):
        last = requests[-1]["messages"][-1]
        done = [ids[block["tool_use_id"]] for block in (last.get("content") if isinstance(last.get("content"), list) else [])
                if block.get("type") == "tool_result" and block.get("tool_use_id") in ids]
        if done:
            following = max(done) + 1
            return steps[following] if following < len(steps) else [{"type": "text", "text": "FILES_DONE"}]
        if "Run the file parity fixture." in " ".join(text_blocks(last)):
            return steps[0]
        raise ProbeFailure("unexpected file phase request")

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(steps) + 20))
    client.prompt(text="Run the file parity fixture.", timeout=120)
    results = {}
    for request in reversed(api.requests):
        for block in outputs(request):
            case = FILE_CASES[ids[block["tool_use_id"]]][0] if block.get("tool_use_id") in ids else None
            if case and case not in results:
                content = block.get("content")
                if isinstance(content, list):
                    content = "".join(item.get("text", "") for item in content)
                results[case] = {"is_error": block.get("is_error", False),
                                 "content": file_normalize(content, args.target) if isinstance(content, str) else content}
    native = json.loads((Path(__file__).parent / "claude_file_native_baseline.json").read_text())
    remote = {"results": results, "effects": file_effects(args.target, inodes)}
    # Stated differences, each documented in the audit: files are rewritten in
    # place (native replaces them, so its inode, hard link and read-only
    # outcomes differ), and a Mods-answered Read keeps the tab of an empty last
    # line that native trims.
    in_place = {"result:edit_readonly", "effect:readonly.txt", "effect:linked-alias.txt"}
    for side in (native["effects"], remote["effects"]):
        for effect in side.values():
            if effect:
                effect.pop("same_inode", None)
                effect.pop("nlink", None)
    for name, result in results.items():
        if name.startswith("read_") and isinstance(result.get("content"), str):
            result["content"] = re.sub(r"\n(\d+)\t$", r"\n\1", result["content"])
    differences = {}
    for name, expected in native["results"].items():
        if "result:" + name in in_place:
            continue
        actual = dict(results.get(name, {}))
        if actual.get("is_error") and isinstance(actual.get("content"), str):
            actual["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", actual["content"], flags=re.S)
            expected = dict(expected)
            expected["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", expected["content"], flags=re.S)
        if actual != expected:
            differences["result:" + name] = {"native": expected, "remote": results.get(name)}
    for name, expected in native["effects"].items():
        if "effect:" + name in in_place:
            continue
        if remote["effects"].get(name) != expected:
            differences["effect:" + name] = {"native": expected, "remote": remote["effects"].get(name)}
    print("file parity diagnostic:", json.dumps(differences, ensure_ascii=False)[:20000])
    require(not differences, "file tools differ from native-local: " + ", ".join(sorted(differences)))
    checks.append("file_tools_match_native_local")


def lifecycle_phases(args, api, client, checks):
    """Bash process lifetimes compared with native-local ones."""
    ids, tasks, raw = {}, [], {}

    def router(requests):
        last = requests[-1]["messages"][-1]
        for block in last.get("content") if isinstance(last.get("content"), list) else []:
            if block.get("type") == "tool_result" and block.get("tool_use_id") in ids:
                content = block.get("content")
                if isinstance(content, list):
                    content = "".join(item.get("text", "") for item in content if item.get("type") == "text")
                raw[ids[block["tool_use_id"]]] = (block.get("is_error", False), content)
                if lifecycle_task_id(content):
                    tasks.append(lifecycle_task_id(content))
        if not raw and "Run the lifecycle parity fixture." not in " ".join(text_blocks(last)):
            raise ProbeFailure("unexpected lifecycle phase request")
        index = len(raw)
        if index >= len(LIFECYCLE_CASES):
            return [{"type": "text", "text": "LIFECYCLE_DONE"}]
        case, (name, arguments) = LIFECYCLE_CASES[index]
        call = tool(name, {"task_id": tasks[-1]} if name == LIFECYCLE_STOP else arguments)
        ids[call[0]["id"]] = case
        return call

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(LIFECYCLE_CASES) + 20))
    try:
        client.prompt(text="Run the lifecycle parity fixture.", timeout=180)
        time.sleep(2)
        processes = lifecycle_alive(args.target)
        notified = lifecycle_notified(api.requests)
    finally:
        lifecycle_stop_all(args.target)
    results = {case: {"is_error": error, "content": lifecycle_normalize(content, args.target, tasks)}
               for case, (error, content) in raw.items()}
    native = json.loads((Path(__file__).parent / "claude_lifecycle_native_baseline.json").read_text())
    differences = {}
    for name, value in native["results"].items():
        actual = dict(results.get(name, {}))
        if actual.get("is_error") and isinstance(actual.get("content"), str):
            actual["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", actual["content"], flags=re.S)
            value = dict(value)
            value["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", value["content"], flags=re.S)
        if actual != value:
            differences["result:" + name] = {"native": value, "remote": results.get(name)}
    if processes != native["alive"]:
        differences["alive"] = {"native": native["alive"], "remote": processes}
    if notified != native["left_running_notified"]:
        differences["left_running_notified"] = {"native": native["left_running_notified"], "remote": notified}
    print("lifecycle parity diagnostic:", json.dumps(differences, ensure_ascii=False)[:20000])
    require(not differences, "Bash process lifetimes differ from native-local: " + ", ".join(sorted(differences)))
    checks.append("bash_process_lifetimes_match_native_local")


def pdf_phases(args, api, client, checks):
    """PDF, image and file-type Read results compared with native-local ones."""
    pdf_setup(args.target)
    steps = [tool("Read", arguments) for _, arguments in PDF_CASES]
    ids = {call[0]["id"]: index for index, call in enumerate(steps)}

    def router(requests):
        last = requests[-1]["messages"][-1]
        done = [ids[block["tool_use_id"]] for block in (last.get("content") if isinstance(last.get("content"), list) else [])
                if block.get("type") == "tool_result" and block.get("tool_use_id") in ids]
        if done:
            following = max(done) + 1
            return steps[following] if following < len(steps) else [{"type": "text", "text": "PDF_DONE"}]
        if "Run the PDF parity fixture." in " ".join(text_blocks(last)):
            return steps[0]
        raise ProbeFailure("unexpected PDF phase request")

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(steps) + 20))
    client.prompt(text="Run the PDF parity fixture.", timeout=180)
    results = {}
    for request in reversed(api.requests):
        for block in outputs(request):
            case = PDF_CASES[ids[block["tool_use_id"]]][0] if block.get("tool_use_id") in ids else None
            if case and case not in results:
                message = next(message for message in request["messages"][::-1]
                               if isinstance(message.get("content"), list) and block in message["content"])
                results[case] = {"is_error": block.get("is_error", False),
                                 "content": pdf_normalize(block.get("content"), args.target),
                                 "notes": pdf_notes(message)}
    native = json.loads((Path(__file__).parent / "claude_pdf_native_baseline.json").read_text())["results"]

    # Stated differences: a Mods-answered error is wrapped in <tool_use_error>,
    # and a Mods-answered Read keeps the tab of an empty last line.
    def comparable(result):
        result = dict(result)
        if isinstance(result.get("content"), str):
            content = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", result["content"], flags=re.S)
            result["content"] = re.sub(r"\n(\d+)\t$", r"\n\1", content)
        return result
    differences = {name: {"native": expected, "remote": results.get(name)}
                   for name, expected in native.items() if comparable(results.get(name, {})) != comparable(expected)}
    print("pdf parity diagnostic:", json.dumps(differences, ensure_ascii=False)[:20000])
    require(not differences, "PDF and file-type Reads differ from native-local: " + ", ".join(sorted(differences)))
    checks.append("pdf_and_file_type_reads_match_native_local")


def subagent_background_phases(args, api, client, checks):
    """A background agent's background command notifies that agent, as natively."""
    background = tool("Bash", {"command": "sleep 2; echo sub-bg-out", "run_in_background": True})
    waiting = tool("Bash", {"command": "sleep 6; echo sub-waited"})
    seen = []

    def router(requests):
        request = requests[-1]
        first = json.dumps(request["messages"][0])
        last = json.dumps(request["messages"][-1], ensure_ascii=False)
        if "SUBBG_CHILD" in first:
            seen.append(last)
            if len(request["messages"]) == 1:
                return background
            if background[0]["id"] in last:
                return waiting
            return [{"type": "text", "text": "SUBBG_CHILD_DONE"}]
        if "SUBBG_PARENT" in last and len(request["messages"][-1].get("content", [])) and \
                not any(block.get("type") == "tool_result" for block in request["messages"][-1]["content"]
                        if isinstance(block, dict)):
            return tool("Agent", {"description": "background child", "prompt": "SUBBG_CHILD run it",
                                  "subagent_type": "general-purpose", "run_in_background": True})
        return [{"type": "text", "text": "SUBBG_NOTED"}]

    del api.steps[len(api.requests):]
    api.steps.extend([router] * 30)
    client.prompt(text="SUBBG_PARENT", timeout=60)
    deadline = time.monotonic() + 60
    while not any("sub-bg-out" in item or "<task-notification>" in item for item in seen[2:]):
        require(time.monotonic() < deadline, "the agent's background command never notified it: " + json.dumps(seen)[-2000:])
        try:
            client.until(lambda frame: frame.get("type") == "result", timeout=2)
        except ProbeFailure:
            pass
    note = next(item for item in seen[2:] if "<task-notification>" in item)
    require("cowboy-task://" in note and "sleep 2; echo sub-bg-out" in note and
            str(args.runtime) not in note and "task-wait.mjs" not in note,
            "the agent's notification differs from native's: " + note[:1500])
    checks.append("agent_background_command_notifies_the_agent_as_natively")


def mcp_phases(args, api, client, checks):
    """The target's MCP servers load and run on the target as natively."""
    home = Path(os.environ["HOME"])
    calls = {name: tool(f"mcp__{name}__where", {}) for name in MCP_CALLS}
    order = list(calls.values())
    position = {call[0]["id"]: index for index, call in enumerate(order)}

    def router(requests):
        last = requests[-1]["messages"][-1]
        done = [position[block["tool_use_id"]] for block in (last.get("content") if isinstance(last.get("content"), list) else [])
                if block.get("type") == "tool_result" and block.get("tool_use_id") in position]
        if done:
            following = max(done) + 1
            return order[following] if following < len(order) else [{"type": "text", "text": "MCP_DONE"}]
        if "Run the MCP fixture." in " ".join(text_blocks(last)):
            return order[0]
        raise ProbeFailure("unexpected MCP phase request")

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(order) + 10))
    client.prompt(text="Run the MCP fixture.", timeout=180)
    names = {definition["name"] for definition in api.requests[-1].get("tools", [])}
    require({f"mcp__{name}__where" for name in MCP_CALLS} <= names and "mcp__loopsrv__where" not in names,
            "target MCP tools differ: " + ",".join(sorted(name for name in names if name.startswith("mcp__"))))
    results = {}
    for message in api.requests[-1]["messages"]:
        for block in message["content"] if isinstance(message.get("content"), list) else []:
            for name, call in calls.items():
                if block.get("type") == "tool_result" and block.get("tool_use_id") == call[0]["id"]:
                    content = block.get("content")
                    text = content[0]["text"] if isinstance(content, list) else content
                    try:
                        results[name] = json.loads(text)
                    except (TypeError, ValueError):
                        results[name] = {"error": text, "is_error": block.get("is_error")}
    print("mcp diagnostic:", json.dumps(results, ensure_ascii=False)[:4000])
    for name in MCP_CALLS:
        require(results.get(name) == mcp_expected(args.target, home, name),
                f"target MCP server {name} differs: " + json.dumps(results.get(name))[:1000])
    checks.append("target_mcp_servers_run_on_target_in_native_scopes_and_precedence")
    encoded = json.dumps(api.requests[-1]["messages"], ensure_ascii=False)
    require("# MCP Server Instructions" in encoded and "projsrv instructions" in encoded and
            "dup-local instructions" in encoded and "dup-user instructions" not in encoded,
            "target MCP instructions did not reach the model as natively")
    checks.append("target_mcp_instructions_reach_the_model")


def skill_phases(args, api, client, checks):
    """Target skills and commands: native's Skill results, listing and typed commands."""
    user = Path(os.environ["HOME"]) / ".claude"
    steps = [tool("Skill", arguments) for _, arguments in SKILL_CASES]
    ids = {call[0]["id"]: index for index, call in enumerate(steps)}
    extra = {"bundled": tool("Skill", {"skill": "simplify"}), "unavailable": tool("Skill", {"skill": "dataviz"})}
    extra_ids = {call[0]["id"]: name for name, call in extra.items()}
    order = [*steps, extra["bundled"], extra["unavailable"]]
    position = {call[0]["id"]: index for index, call in enumerate(order)}

    def router(requests):
        last = requests[-1]["messages"][-1]
        done = [position[block["tool_use_id"]] for block in (last.get("content") if isinstance(last.get("content"), list) else [])
                if block.get("type") == "tool_result" and block.get("tool_use_id") in position]
        if done:
            following = max(done) + 1
            return order[following] if following < len(order) else [{"type": "text", "text": "SKILL_DONE"}]
        if "Run the skill parity fixture." in " ".join(text_blocks(last)):
            return order[0]
        raise ProbeFailure("unexpected skill phase request")

    del api.steps[len(api.requests):]
    api.steps.extend([router] * (len(order) + 20))
    client.prompt(text="Run the skill parity fixture.", timeout=300)
    results, others = {}, {}
    for request in reversed(api.requests):
        for message in request["messages"]:
            for block in message["content"] if isinstance(message.get("content"), list) else []:
                if block.get("type") != "tool_result":
                    continue
                if block.get("tool_use_id") in ids:
                    case = SKILL_CASES[ids[block["tool_use_id"]]][0]
                    results.setdefault(case, {"is_error": block.get("is_error", False),
                                              "content": skill_normalize(block.get("content"), args.target, user),
                                              "follows": skill_contents(message, args.target, user)})
                elif block.get("tool_use_id") in extra_ids:
                    others.setdefault(extra_ids[block["tool_use_id"]], block)
    native = json.loads((Path(__file__).parent / "claude_skill_native_baseline.json").read_text())

    # Stated difference: a Mods-answered error is wrapped in <tool_use_error>.
    def comparable(result):
        result = dict(result)
        if isinstance(result.get("content"), str):
            result["content"] = re.sub(r"^<tool_use_error>(.*)</tool_use_error>$", r"\1", result["content"], flags=re.S)
        return result
    differences = {name: {"native": expected, "remote": results.get(name)}
                   for name, expected in native["results"].items()
                   if comparable(results.get(name, {})) != comparable(expected)}
    print("skill parity diagnostic:", json.dumps(differences, ensure_ascii=False)[:20000])
    require(not differences, "Skill results differ from native-local: " + ", ".join(sorted(differences)))
    checks.append("target_skill_results_match_native_local")
    shown = next((block["text"] for block in api.requests[0]["messages"][0]["content"]
                  if "The following skills are available" in block.get("text", "")), "")
    print("skill listing diagnostic:", json.dumps(skill_listing(shown)))
    require(skill_listing(shown) == native["listing"], "skill listing differs from native-local: " + shown[:3000])
    require(not others["bundled"].get("is_error") and "Launching skill: simplify" in json.dumps(others["bundled"]) and
            others["unavailable"].get("is_error") and "unavailable in this execution session" in json.dumps(others["unavailable"]),
            "bundled skills are not offered as stated: " + json.dumps(others)[:2000])
    checks.append("target_skill_listing_matches_native_local_with_stated_bundled_skills")

    # A command the user types runs under its own name; @-mentions of this
    # machine's files never reach the model.
    del api.steps[len(api.requests):]
    api.steps.extend([[{"type": "text", "text": "TYPED_DONE"}], [{"type": "text", "text": "MENTION_DONE"}]])
    client.prompt(text="/grp:inner typed-arg", timeout=120)
    typed = [skill_normalize(block["text"], args.target, user) for block in api.requests[-1]["messages"][-1]["content"]
             if block.get("type") == "text" and not block["text"].startswith("<system-reminder>")]
    print("typed command diagnostic:", json.dumps(typed, ensure_ascii=False))
    require(typed == native["typed"], "typed command differs from native-local: " + json.dumps(typed)[:2000])
    client.prompt(text="Compare @CLAUDE.md and @. please", timeout=120)
    mentioned = json.dumps(api.requests[-1]["messages"][-1], ensure_ascii=False)
    require("RUNTIME_CLAUDE_GUIDANCE" not in mentioned and "Called the Read tool" not in mentioned and
            "Called the Bash tool" not in mentioned and str(args.runtime) not in mentioned,
            "an @-mention attached a runtime file: " + mentioned[:2000])
    encoded = json.dumps(api.requests, ensure_ascii=False)
    exposed = next((name for name in ["cowboy-target", "target-skills", ".cowboy-claude-"] if name in encoded), None)
    if exposed:
        position = encoded.index(exposed)
        print("private runtime diagnostic:", encoded[max(0, position - 300):position + 300])
    require(exposed is None, "the private skill plugin or runtime stage reached the model")
    checks.append("typed_target_command_and_runtime_mentions_as_native")


def notification_phases(args, api, client, checks):
    """Target commands left running notify the model as native background tasks do."""
    handle = re.compile(r"cowboy-task://([A-Za-z0-9-]+)")
    issued = {}
    asked = []

    def job_of(requests):
        found = handle.search(json.dumps(requests[-1]["messages"][-1]))
        require(found, "background result named no handle")
        return found[1]

    plans = {
        "NOTIFY_IDLE": [tool("Bash", {"command": "echo bg-out; sleep 2; exit 3", "run_in_background": True})],
        "NOTIFY_BUSY": [tool("Bash", {"command": "sleep 1; echo busy-out", "run_in_background": True}),
                        tool("Bash", {"command": "sleep 5; echo fg"})],
        "NOTIFY_STOP": [tool("Bash", {"command": "sleep 30", "run_in_background": True}),
                        lambda requests: tool("TaskStop", {"task_id": job_of(requests)})],
        "NOTIFY_TIMEOUT": [tool("Bash", {"command": "echo t1; sleep 3; echo t2", "timeout": 1000})],
        "NOTIFY_ASK": [tool("Bash", {"command": "touch notify-asked.txt; sleep 1", "run_in_background": True})],
        "NOTIFY_DEADLINE": [tool("Bash", {"command": "while :; do echo beat >> deadline-beat.txt; sleep 0.2; done",
                                          "run_in_background": True, "timeout": 3000})],
    }

    def router(requests):
        last = requests[-1]["messages"][-1]
        blocks = last.get("content") if isinstance(last.get("content"), list) else []
        done = [issued[block["tool_use_id"]] for block in blocks
                if block.get("type") == "tool_result" and block.get("tool_use_id") in issued]
        if done:
            marker, index = done[-1]
        else:
            latest = " ".join(text_blocks(last))
            marker = next((name for name in plans if name in latest), None)
            if marker is None:
                require("<task-notification>" in json.dumps(last), "unexpected notification phase request")
                return [{"type": "text", "text": "NOTED"}]
            index = -1
        if index + 1 < len(plans[marker]):
            step = plans[marker][index + 1]
            call = step(requests) if callable(step) else step
            issued[call[0]["id"]] = (marker, index + 1)
            return call
        return [{"type": "text", "text": marker + "_DONE"}]

    del api.steps[len(api.requests):]
    api.steps.extend([router] * 60)

    def pump_until(condition, seconds, message):
        deadline = time.monotonic() + seconds
        while not condition():
            remaining = deadline - time.monotonic()
            require(remaining > 0, message)
            try:
                client.until(lambda frame: frame.get("type") == "result", timeout=min(remaining, 2))
            except ProbeFailure:
                pass

    def notifications(job, since=0):
        found = []
        for request in api.requests[since:]:
            for message in request.get("messages", []):
                encoded = json.dumps(message, ensure_ascii=False)
                if f"<task-id>{job}</task-id>" in encoded:
                    found.append(encoded)
        return found

    def job_for(marker):
        for request in api.requests:
            for block in outputs(request):
                if issued.get(block.get("tool_use_id"), (None, None))[0] == marker:
                    found = handle.search(json.dumps(block))
                    if found:
                        return found[1], block
        raise ProbeFailure("notification phase result missing")

    def original_id(marker):
        return next(identity for identity, (name, index) in issued.items() if name == marker and index == 0)

    runtime_markers = [str(args.runtime), "task-wait.mjs"]

    # Idle: the command ends after the turn; native starts a turn for it.
    client.prompt(text="NOTIFY_IDLE", timeout=60)
    job, block = job_for("NOTIFY_IDLE")
    require("You will be notified when it completes." in json.dumps(block), "background result made no promise")
    start = len(api.requests)
    pump_until(lambda: notifications(job, start), 30, "idle completion did not start a turn")
    note = notifications(job, start)[-1]
    require("NOT USER INPUT" in note and f"<tool-use-id>{original_id('NOTIFY_IDLE')}</tool-use-id>" in note and
            f"<output-file>cowboy-task://{job}</output-file>" in note and "<status>failed</status>" in note and
            'Background command \\"echo bg-out; sleep 2; exit 3\\" failed with exit code 3' in note and
            not any(marker in note for marker in runtime_markers),
            "idle notification differs from native's: " + note[:1500])
    checks.append("background_completion_starts_a_native_notification_turn")

    # Busy: the notification is delivered into the running turn, no extra turn.
    client.prompt(text="NOTIFY_BUSY", timeout=60)
    busy, _ = job_for("NOTIFY_BUSY")
    delivered = notifications(busy)
    require(delivered and "<status>completed</status>" in delivered[0] and
            'Background command \\"sleep 1; echo busy-out\\" completed (exit code 0)' in delivered[0],
            "busy completion was not delivered into the running turn")
    checks.append("background_completion_is_delivered_into_a_running_turn")

    # Stopped: natively nothing is sent.
    client.prompt(text="NOTIFY_STOP", timeout=60)
    stopped, _ = job_for("NOTIFY_STOP")
    start = len(api.requests)
    time.sleep(4)
    pump_until(lambda: True, 1, "")
    require(not notifications(stopped, start), "a stopped background command sent a notification")
    checks.append("stopped_background_command_sends_no_notification")

    # Timeout: moved to the background as natively, then notified.
    client.prompt(text="NOTIFY_TIMEOUT", timeout=60)
    moved, block = job_for("NOTIFY_TIMEOUT")
    require("did not complete within its 1s timeout and was moved to the background" in json.dumps(block),
            "timed-out command was not moved to the background")
    start = len(api.requests)
    pump_until(lambda: notifications(moved, start), 30, "timed-out command sent no completion notification")
    require("completed (exit code 0)" in notifications(moved, start)[-1], "timeout notification status differs")
    checks.append("timed_out_command_moves_to_background_and_notifies")

    # Deadline: native stops a background command at its timeout and says so;
    # the target command stops with it.
    client.prompt(text="NOTIFY_DEADLINE", timeout=60)
    limited, _ = job_for("NOTIFY_DEADLINE")
    start = len(api.requests)
    pump_until(lambda: notifications(limited, start), 30, "a background deadline sent no notification")
    note = notifications(limited, start)[-1]
    require("<status>killed</status>" in note and
            'Background command \\"while :; do echo beat >> deadline-beat.txt; sleep 0.2; done\\" was stopped after reaching its background time limit' in note and
            f"<output-file>cowboy-task://{limited}</output-file>" in note and
            not any(marker in note for marker in runtime_markers),
            "deadline notification differs from native's: " + note[:1500])
    beat = args.target / "deadline-beat.txt"
    before = beat.stat().st_size
    time.sleep(2)
    require(beat.stat().st_size == before, "the target command kept running past its background deadline")
    checks.append("background_deadline_stops_the_target_command_as_natively")

    # The native task behind it never asks the user a second time.
    def answer(request):
        asked.append(request)
        return {"behavior": "allow", "updatedInput": request["input"]}
    client.permission = answer
    request_id = "fixture-notify-mode-default"
    client.send({"type": "control_request", "request_id": request_id,
                 "request": {"subtype": "set_permission_mode", "mode": "default"}})
    client.until(lambda frame: frame.get("type") == "control_response" and
                 frame["response"].get("request_id") == request_id)
    client.prompt(text="NOTIFY_ASK", timeout=60)
    asked_job, _ = job_for("NOTIFY_ASK")
    start = len(api.requests)
    pump_until(lambda: notifications(asked_job, start), 30, "background command in default mode sent no notification")
    require(len(asked) == 1 and asked[0]["input"].get("command") == "touch notify-asked.txt; sleep 1",
            f"the notification task asked the user: {[item.get('input') for item in asked]}")
    request_id = "fixture-notify-mode-bypass"
    client.send({"type": "control_request", "request_id": request_id,
                 "request": {"subtype": "set_permission_mode", "mode": "bypassPermissions"}})
    client.until(lambda frame: frame.get("type") == "control_response" and
                 frame["response"].get("request_id") == request_id)
    client.permission = None
    checks.append("notification_task_needs_no_second_approval")


def hook_phases(args, api, client, native, session, context_checked, checks):
    """Target project hooks: lifecycle and native-tool hooks run by native
    through the proxy, facade tool hooks by the adapter, all on the target."""
    hooks_dir = args.target / ".claude"
    hooks_dir.mkdir(exist_ok=True)
    marker = lambda name: f'printf "%s\\n" {name} >> "$CLAUDE_PROJECT_DIR/hook-events.txt"'
    # The hook reads its transcript_path on the target and requires content.
    transcript_readable = ("python3 -c 'import json,os,sys; p=json.load(sys.stdin).get(\"transcript_path\"); "
                           "sys.exit(0 if p and os.path.getsize(p) > 0 else 1)'")
    (hooks_dir / "settings.json").write_text(json.dumps({"hooks": {
        "SessionStart": [{"hooks": [{"type": "command",
                                      "command": marker("SessionStart") + '; printf "%s" "$CLAUDE_PROJECT_DIR" > hook-project-dir.txt'
                                                 + "; echo 'export COWBOY_HOOK_ENV=from-session-start' >> \"$CLAUDE_ENV_FILE\""},
                                     # Exec form: no shell, placeholder substituted as a plain string.
                                     {"type": "command", "command": "sh", "args": [
                                         "-c", 'printf "%s\\n" ExecForm >> "$1/hook-events.txt"', "sh", "${CLAUDE_PROJECT_DIR}"]}]}],
        "UserPromptSubmit": [{"hooks": [{"type": "command", "command": marker("UserPromptSubmit")}]}],
        "Stop": [{"hooks": [{"type": "command", "command": (
            transcript_readable + " && " + marker("StopTranscript") + "; " + marker("Stop"))}]}],
        "PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command",
              "command": "if grep -q FORBIDDEN; then echo blocked-by-target-hook >&2; exit 2; fi"}]},
            {"matcher": "Agent", "hooks": [{"type": "command",
              "command": "if grep -q HOOK_CHILD_MUST_NOT_RUN; then " + marker("Agent") +
                         "; echo agent-blocked-by-target-hook >&2; exit 2; fi"}]},
            # A subagent's facade tool hook input names its agent.
            {"matcher": "Read", "hooks": [{"type": "command",
              "command": 'cat >> "$CLAUDE_PROJECT_DIR/hook-read-inputs.jsonl"; echo >> "$CLAUDE_PROJECT_DIR/hook-read-inputs.jsonl"'}]},
        ],
        # Races the host prompt: the slow host would deny; the hook approves an amended call.
        "PermissionRequest": [{"matcher": "Bash", "hooks": [{"type": "command", "command":
            "if grep -q PERMREQ_PROBE; then " + marker("PermissionRequest") + "; printf '%s' "
            """'{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","""
            """"updatedInput":{"command":"printf hook-approved > permreq.txt"}}}}'; fi"""}]}],
        "PostToolUseFailure": [{"matcher": "Read", "hooks": [{"type": "command", "command": marker("ReadFailed") +
            """; printf '%s' '{"hookSpecificOutput":{"hookEventName":"PostToolUseFailure","additionalContext":"TARGET_FAILURE_CONTEXT"}}'"""}]},
            {"matcher": "Bash", "hooks": [{"type": "command", "command": marker("BashFailed") +
             """; printf '%s' '{"hookSpecificOutput":{"hookEventName":"PostToolUseFailure","additionalContext":"TARGET_BASH_FAILURE"}}'"""}]}],
        "PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": marker("BashPost")}]},
                        {"matcher": "Write", "hooks": [{"type": "command", "command": transcript_readable + " && " +
            marker("PostTranscript") + "; " + marker("PostWrite") +
            """; printf '%s' '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"TARGET_POST_CONTEXT"}}'"""},
            {"type": "command", "async": True, "command":
             """cat > /dev/null; printf '%s' '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"ASYNC_POST_CONTEXT"}}'"""}]}],
    }}))
    issued = {}
    steps = {
        "HOOK_BLOCK": tool("Bash", {"command": "printf FORBIDDEN >> forbidden.txt"}),
        "HOOK_WRITE": tool("Write", {"file_path": "hooked.txt", "content": "hooked\n"}),
        "HOOK_AGENT": tool("Agent", {"description": "Blocked child", "prompt": "HOOK_CHILD_MUST_NOT_RUN"}),
        "HOOK_ENV": tool("Bash", {"command": 'printf "%s" "$COWBOY_HOOK_ENV" > hook-env.txt'}),
        "HOOK_FAIL": tool("Read", {"file_path": "hook-missing-file.txt"}),
        "HOOK_BASH_FAIL": tool("Bash", {"command": "echo partial-output; exit 3"}),
        "HOOK_PERMREQ": tool("Bash", {"command": "printf PERMREQ_PROBE > permreq.txt"}),
        "HOOK_SPAWN": tool("Agent", {"description": "Hooked child", "prompt": "CHILD_READS_WITH_HOOK",
                                     "subagent_type": "general-purpose"}),
    }

    def router(requests):
        last = requests[-1]["messages"][-1]
        if isinstance(last.get("content"), list) and any(
                block.get("type") == "tool_result" and block.get("tool_use_id") in issued for block in last["content"]):
            return [{"type": "text", "text": "HOOK_DONE"}]
        latest = " ".join(text_blocks(last))
        if "CHILD_READS_WITH_HOOK" in latest:
            call = tool("Read", {"file_path": "hooked.txt"})
            issued[call[0]["id"]] = "HOOK_CHILD_READ"
            return call
        for name, call in steps.items():
            if name in latest:
                issued[call[0]["id"]] = name
                return call
        raise ProbeFailure("unexpected hook phase request: " + latest[:200])

    def result_of(name):
        for request in reversed(api.requests):
            blocks = [block for block in outputs(request) if issued.get(block.get("tool_use_id")) == name]
            if blocks:
                return json.dumps(blocks) + json.dumps(request["messages"][-1])
        raise ProbeFailure("hook phase result missing")

    def message_of(name):
        # The user message carrying the result and its reminders, from any
        # request (the latest may be a background child's own conversation).
        for request in reversed(api.requests):
            for message in request.get("messages", []):
                content = message.get("content")
                if isinstance(content, list) and any(
                        block.get("type") == "tool_result" and issued.get(block.get("tool_use_id")) == name
                        for block in content):
                    return json.dumps(message)
        raise ProbeFailure("hook phase result missing")

    client.close()
    del api.steps[len(api.requests):]
    api.steps.extend([router] * 20)
    client = native(session)
    client.ready()
    phase_start = len(api.requests)
    hook_inputs = Path.home() / ".cache/cowboy/hook-input"
    inputs_before = set(hook_inputs.iterdir() if hook_inputs.exists() else [])
    events = args.target / "hook-events.txt"
    asked = []

    def slow_host(request):
        # Stays undecided long enough for the target hook to answer first.
        asked.append(request)
        time.sleep(5)
        return {"behavior": "deny", "message": "HOST_DENIED_TOO_LATE"}

    def mode(value):
        request_id = f"fixture-hook-mode-{value}-{len(api.requests)}"
        client.send({"type": "control_request", "request_id": request_id,
                     "request": {"subtype": "set_permission_mode", "mode": value}})
        reply = client.until(lambda frame: frame.get("type") == "control_response" and
                             frame["response"].get("request_id") == request_id)
        require(reply["response"]["subtype"] == "success", "native rejected the permission mode")

    for name in steps:
        if name == "HOOK_PERMREQ":
            client.permission = slow_host
            mode("default")
        client.prompt(text=name, timeout=60)
        if name == "HOOK_PERMREQ":
            mode("bypassPermissions")
            client.permission = None
    require(len(asked) == 1 and (args.target / "permreq.txt").read_text() == "hook-approved" and
            "HOST_DENIED_TOO_LATE" not in result_of("HOOK_PERMREQ"),
            "target PermissionRequest hook did not answer the pending prompt")
    read_inputs = [json.loads(line) for line in (args.target / "hook-read-inputs.jsonl").read_text().splitlines()
                   if line.strip()]
    require(any(item.get("agent_type") == "general-purpose" and item.get("agent_id") and
                item.get("tool_input") == {"file_path": "hooked.txt"} for item in read_inputs),
            f"a subagent's facade tool hook input lacked its agent: {read_inputs[-1:]}")
    require(not (args.target / "forbidden.txt").exists() and
            "PreToolUse:Bash hook error: [if grep -q FORBIDDEN" in result_of("HOOK_BLOCK") and
            "blocked-by-target-hook" in result_of("HOOK_BLOCK"), "target PreToolUse hook did not block before the effect")
    require((args.target / "hooked.txt").read_text() == "hooked\n" and
            # The last request may be a background child's, without this history.
            "PostToolUse:Write hook additional context: TARGET_POST_CONTEXT" in message_of("HOOK_WRITE"),
            "target PostToolUse context did not reach the model: " + result_of("HOOK_WRITE")[:600] +
            " events: " + (events.read_text() if events.exists() else "none") +
            " hook notes: " + json.dumps(sorted({m for m in re.findall(r"[^\\\"]{0,80}hook[^\\\"]{0,160}",
                                                                   json.dumps(api.requests[phase_start:]))}))[:3000])
    deadline = time.monotonic() + 10
    while "Stop" not in (events.read_text() if events.exists() else "") and time.monotonic() < deadline:
        time.sleep(0.1)
    recorded = events.read_text().split()
    # Agent runs natively: native applies the proxied target hook's block.
    require("agent-blocked-by-target-hook" in result_of("HOOK_AGENT") and
            "HOOK_CHILD_MUST_NOT_RUN" not in json.dumps([request.get("messages", [])[:1] for request in api.requests]),
            "native Agent tool hook did not block on the target")
    require(any("PostToolUse:Write async hook additional context: ASYNC_POST_CONTEXT" in json.dumps(request)
                for request in api.requests[phase_start:]), "async target hook context did not reach a later request")
    require((args.target / "hook-env.txt").read_text() == "from-session-start",
            "SessionStart CLAUDE_ENV_FILE exports did not reach target Bash")
    require("PostToolUseFailure:Read hook additional context: TARGET_FAILURE_CONTEXT" in result_of("HOOK_FAIL"),
            "target PostToolUseFailure feedback did not reach the model")
    # As natively, a non-zero Bash exit is a tool error: failure hooks run, PostToolUse does not.
    failed_bash = result_of("HOOK_BASH_FAIL")
    require('"is_error": true' in failed_bash and "Exit code 3\\npartial-output" in failed_bash and
            "PostToolUseFailure:Bash hook additional context: TARGET_BASH_FAILURE" in failed_bash,
            "a non-zero target Bash exit was not a native tool error with failure hook feedback")
    require(recorded.count("BashFailed") == 1 and recorded.count("BashPost") == 2,
            f"Bash success/failure hooks ran out of turn: {recorded}")
    for name in ["SessionStart", "ExecForm", "UserPromptSubmit", "Agent", "PostWrite", "PostTranscript", "ReadFailed",
                 "PermissionRequest", "Stop", "StopTranscript"]:
        require(name in recorded, f"target {name} hook did not run on the target")
    # The fixture executor shares this host's HOME; only new entries count.
    # A background child's hook may still be running: its copies must go when it ends.
    deadline = time.monotonic() + 15
    while True:
        leftovers = set(hook_inputs.iterdir() if hook_inputs.exists() else []) - inputs_before
        if not leftovers or time.monotonic() > deadline:
            break
        time.sleep(0.2)
    require(not leftovers, f"hook input or transcript copies were left on the target: {sorted(leftovers)[:3]}")
    require(hook_inputs.stat().st_mode & 0o777 == 0o700, "target hook input directory is not private")
    require((args.target / "hook-project-dir.txt").read_text() == str(args.target),
            "hook CLAUDE_PROJECT_DIR is not the target project")
    require(not (args.runtime / "hook-events.txt").exists() and not (args.runtime / "hook-project-dir.txt").exists(),
            "a project hook ran on the runtime")
    # History still holds the permission phase's scripted collision path.
    collision = str(args.runtime / "perm-collision.txt")
    for request in api.requests[phase_start:]:
        encoded = json.dumps(request)
        stripped = encoded.replace(collision, "")
        for leak in [str(args.runtime), str(args.runtime.parent / "claude-home")]:
            if leak in stripped:
                position = stripped.index(leak)
                print("hook phase leak diagnostic:", stripped[max(0, position - 600):position + 300])
        if "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL" not in encoded:
            print("hook phase guidance diagnostic:", encoded[:600])
        require(encoded.count(str(args.runtime)) == encoded.count(collision) and
                str(args.runtime.parent / "claude-home") not in encoded and
                "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL" in encoded, "runtime context reached model")
    checks.extend(["project_lifecycle_hooks_run_on_target", "native_tool_hooks_run_on_target",
                   "session_start_env_file_reaches_target_bash", "facade_tool_failure_hooks_run_on_target",
                   "nonzero_bash_exit_is_native_tool_error", "permission_request_hook_answers_pending_prompt",
                   "subagent_tool_hooks_name_the_agent",
                   "facade_pre_tool_hook_blocks_before_target_effect", "facade_post_tool_hook_context_reaches_model"])
    return client


# Acceptance phases after the base turn, in their run order.
PHASES = ["agents", "permissions", "shell", "notifications", "lifecycle", "context", "files", "pdf", "skills",
          "mcp", "agent_background", "hooks"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["native-cli", "descriptor", "runtime", "target", "receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new receipt required")
    inputs = json.loads(Path(os.environ["COWBOY_TEST_EXECUTION_INPUT"]).read_text())
    selected = set(inputs.get("phases") or PHASES)
    require(selected <= set(PHASES), "unknown acceptance phase: " + ", ".join(sorted(selected - set(PHASES))))
    binary = Path(inputs["claude_cli"])
    require(hashlib.sha256(binary.read_bytes()).hexdigest() == inputs["claude_sha256"], "Claude artifact changed")
    launcher = Path(inputs["adapter_launcher"])
    wrapper = launcher.parent.parent / "bin/cowboy-configured-cli"
    require(wrapper.is_file(), "packaged Claude execution launcher missing")
    checks = ["machine_owned_worktree", "machine_restart_reattaches_same_keeper_and_binding"]
    (args.runtime / "CLAUDE.md").write_text("RUNTIME_CLAUDE_GUIDANCE_MUST_NOT_REACH_MODEL")
    write_context_fixture(args.target)
    # The target's home is the harness's fresh one (see the justfile).
    skill_setup(args.target, Path(os.environ["HOME"]) / ".claude")
    mcp_setup(args.target, Path(os.environ["HOME"]))
    def png_chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    pixel = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0)) +
             png_chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\0\0" * 2) * 2)) + png_chunk(b"IEND", b""))
    (args.target / "pixel.png").write_bytes(pixel)
    random_text = "".join(random.Random(7).choices("abcdefghijklmnopqrstuvwxyz0123456789", k=58000))
    (args.target / "large.txt").write_text(random_text)
    (args.target / "range-large.txt").write_text(("x" * 127 + "\n") * 8192)
    (args.target / "book.ipynb").write_text(json.dumps({
        "nbformat": 4, "nbformat_minor": 5, "metadata": {"preserve": True},
        "cells": [{"id": "cell", "cell_type": "code", "metadata": {}, "source": ["before"],
                   "outputs": [], "execution_count": None}],
    }))
    quoted = "quoted '\" $() 中文 🐎.txt"
    content = "target after '\" $() 中文 🐎\r\n"
    def conflict(_requests):
        (args.target / quoted).write_text("external change\n")
        return tool("Edit", {"file_path": quoted, "old_string": "hello", "new_string": "lost"})
    api = ScriptedApi([
        tool("Read", {"file_path": "fixture.txt"}),
        tool("Edit", {"file_path": "fixture.txt", "old_string": "target before\n", "new_string": content}),
        tool("Bash", {"command": "pwd; cat fixture.txt; printf once >> once.txt"}),
        tool("Write", {"file_path": quoted, "content": "hello\r\n"}),
        tool("Read", {"file_path": quoted}), conflict,
        tool("Read", {"file_path": "pixel.png"}),
        tool("Read", {"file_path": "large.txt"}),
        tool("Read", {"file_path": "range-large.txt", "offset": 101, "limit": 10}),
        tool("Read", {"file_path": "book.ipynb"}),
        tool("NotebookEdit", {"notebook_path": "book.ipynb", "cell_id": "cell", "new_source": "print('target')\n"}),
        tool("Bash", {"command": "printf background_started >> jobs.txt; while :; do sleep 1; printf tick >> jobs.txt; done", "run_in_background": True}),
        read_background,
        stop_background,
    ], native_titles=True)
    environment = closed_environment(args.runtime.parent / "claude-home")
    environment.update({
        "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
        "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
        "COWBOY_PRIVATE_CLAUDE_EXECUTABLE": str(binary),
        "CLAUDE_CODE_EXECUTABLE": str(binary),
        "COWBOY_EXECUTION_DESCRIPTOR": str(args.descriptor),
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
    })
    fixture = WorkspaceFixture(args.target)
    memory = MatrixFixture(inputs, args.runtime.parent, args.descriptor, "claude", environment)
    if memory.enabled:
        api.steps.insert(0, tool("mcp__matrix__" + memory.tool, memory.arguments))
    def native(resume=None):
        # The packaged ACP adapter disallows only AskUserQuestion without form
        # elicitation; the bound launcher owns every other tool restriction.
        return Claude(str(wrapper), environment, args.runtime, fixture, resume=resume, bound_native=True,
                      disallowed="AskUserQuestion")
    def context_checked(requests):
        for request in [*api.token_requests, *api.title_requests]:
            require(str(args.runtime) not in json.dumps(request) and
                    str(args.runtime.parent / "claude-home") not in json.dumps(request),
                    "auxiliary request leaked runtime context")
        for index, request in enumerate(requests):
            names = {definition["name"] for definition in request.get("tools", [])}
            target_mcp = {f"mcp__{name}__where" for name in MCP_CALLS}
            require(not any(name.startswith("mcp__") and name not in target_mcp and not (memory.enabled and name in {"mcp__matrix__memory_search", "mcp__matrix__memory_get", "mcp__matrix__memory_put", "mcp__matrix__memory_forget", "mcp__matrix__memory_read", "mcp__matrix__memory_execute", "mcp__matrix__memory_receipt"}) for name in names), "Unowned MCP tool definitions remain advertised")
            # Native's default set (2.1.287): no Glob, Grep or TodoWrite; the
            # task list, web and review tools run where the session runs.
            require(not names or ({"Read", "Edit", "Write", "Bash", "NotebookEdit"} <= names
                                  and not names & {"Glob", "Grep", "TodoWrite"}),
                    "native execution tools differ from native's default set: " + ",".join(sorted(names)))
            encoded = json.dumps(request)
            exposed = next((path for path in [str(args.runtime), str(args.runtime.parent / "claude-home")]
                            if path in encoded), None)
            if exposed:
                position = encoded.index(exposed)
                print("runtime context diagnostic:", encoded[max(0, position-180):position+500])
            require(str(args.runtime) not in encoded and str(args.runtime.parent / "claude-home") not in encoded and
                    "RUNTIME_CLAUDE_GUIDANCE" not in encoded and
                    "RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL" not in encoded, "runtime context reached model")
            if "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL" not in encoded:
                print("target guidance diagnostic:", index, json.dumps({"system": request.get("system"), "tools": sorted(names), "messages": request.get("messages", [])[:1]})[:3500])
            # Native Claude Code does not read AGENTS.md; CLAUDE.md is the guidance.
            require(str(args.target) in encoded and "TARGET_CLAUDE_GUIDANCE_MUST_REACH_MODEL" in encoded and
                    "TARGET_GUIDANCE_MUST_REACH_MODEL" not in encoded, "target guidance missing or AGENTS.md read")
        require(not fixture.calls, "test client unexpectedly supplied target tools")
        require(api.failure is None, api.failure or "scripted API failed")
    client = None
    try:
        client = native()
        client.ready()
        require(not api.requests, "readiness called the model")
        client.send({"type": "control_request", "request_id": "forbidden-mcp", "request": {
            "subtype": "mcp_set_servers", "servers": {"local": {"type": "stdio", "command": "false"}},
        }})
        denied = client.until(lambda frame: frame.get("type") == "control_response" and
                              frame["response"].get("request_id") == "forbidden-mcp")
        require(denied["response"]["subtype"] == "error", "execution override was accepted")
        checks.append("native_control_cannot_replace_bound_execution")
        try:
            result = client.prompt(timeout=150)
        except ProbeFailure:
            client.stderr.seek(0)
            print("native stderr diagnostic:", client.stderr.read()[-4000:].decode("utf-8", "replace"))
            print("first turn diagnostic:", len(api.requests), json.dumps(api.requests[-1]["messages"][-2:] if api.requests else None)[-3000:])
            raise
        session = result["session_id"]
        context_checked(api.requests)
        main_tools = {definition["name"] for definition in api.requests[-1].get("tools", [])}
        require({"TaskStop", "TaskCreate", "TaskGet", "TaskList", "TaskUpdate", "WebFetch", "WebSearch",
                 "Agent"} <= main_tools,
                "native's runtime tools are not advertised: " + ",".join(sorted(main_tools)))
        checks.append("native_default_tool_set_without_runtime_only_search_tools")
        # Target tools keep native's own descriptions.
        expected = json.loads((Path(__file__).parent / "claude_native_behavior_baseline.json").read_text())["tool_descriptions"]
        described = {definition["name"]: definition.get("description") for definition in api.requests[-1].get("tools", [])}
        differing = sorted(name for name, text in expected.items() if described.get(name) != text)
        for name in differing:
            print("tool description diagnostic:", name, json.dumps(described.get(name))[:1500])
        require(not differing, "tool descriptions differ from native's: " + ", ".join(differing))
        checks.append("target_tools_carry_native_descriptions")
        require((args.target / "fixture.txt").read_bytes() == content.encode(), "target edit bytes changed")
        require((args.target / "once.txt").read_text() == "once", "command did not execute exactly once")
        require((args.target / quoted).read_text() == "external change\n", "stale edit overwrote external change")
        book = json.loads((args.target / "book.ipynb").read_text())
        require(book["metadata"] == {"preserve": True} and book["cells"][0]["source"] == ["print('target')\n"],
                "native notebook edit lost target metadata or content")
        errors = [block for block in outputs(api.requests[-1]) if block.get("is_error")]
        require(len(errors) == 1, f"expected only the edit conflict; received {len(errors)} tool errors: "
                + json.dumps(errors)[:3000])
        require((args.runtime / "fixture.txt").read_text() == "runtime remains untouched\n" and
                not (args.runtime / "once.txt").exists(), "runtime filesystem was modified")
        jobs = (args.target / "jobs.txt").read_text()
        require(jobs.count("background_started") == 1, "background start replayed")
        stopped = list(outputs(api.requests[-1]))[-1]
        require("Successfully stopped task: " in json.dumps(stopped), "background cancellation did not settle")
        time.sleep(1.2)
        require((args.target / "jobs.txt").read_text() == jobs, "background descendants survived cancellation")
        images = [item for block in outputs(api.requests[-1]) if isinstance(block.get("content"), list) for item in block["content"]
                  if item.get("type") == "image"]
        require(images and base64.b64decode(images[0]["source"]["data"]) == pixel, "target image bytes changed")
        require(any(random_text in json.dumps(request) for request in api.requests),
                "large tool result spilled or was truncated before reaching the model")
        checks.extend(["module_readiness_uses_no_model_request", "native_tools_use_target_files_and_processes_without_mcp",
                       "runtime_context_and_guidance_are_replaced", "unicode_quotes_crlf_bytes_preserved",
                       "stale_edit_is_refused", "target_image_enters_model_context", "background_cancel_settles",
                       "bounded_large_read_stays_in_target_tool_result",
                       "native_notebook_edit_preserves_metadata", "native_read_observes_retained_task_output",
                       "lost_start_receipt_and_transport_outage_do_not_replay"])
        # The task list runs natively where the session runs; WebFetch of the
        # machine's own name would reach the runtime, not the target.
        api.steps.extend([[], tool("TaskCreate", {"subject": "Parity task", "description": "Track parity"}),
                          tool("WebFetch", {"url": "http://localhost:9/", "prompt": "Summarize"})])
        client.prompt(timeout=90)
        runtime_results = list(outputs(api.requests[-1]))[-2:]
        require(len(runtime_results) == 2 and not runtime_results[0].get("is_error")
                and runtime_results[1].get("is_error")
                and "cannot reach the target's localhost" in json.dumps(runtime_results[1]),
                "native runtime tools did not run as expected: " + json.dumps(runtime_results)[:800])
        context_checked(api.requests)
        checks.append("native_task_list_runs_and_target_loopback_fetch_is_refused")
        api.steps.extend([[], tool("Bash", {"command": "printf retained_started >> retained.txt; while :; do sleep 1; printf tick >> retained.txt; done", "run_in_background": True})])
        client.prompt(timeout=90)
        client.close(); client = None
        api.steps.extend([[], read_background, stop_background, tool("Edit", {"file_path": "fixture.txt", "old_string": "target after", "new_string": "target resumed"}),
                          tool("Bash", {"command": "cat fixture.txt; cat once.txt"})])
        previous = len(api.requests)
        client = native(session)
        client.ready()
        require(len(api.requests) == previous, "resume readiness called model")
        client.prompt(timeout=90)
        context_checked(api.requests)
        require((args.target / "fixture.txt").read_bytes() == content.replace("target after", "target resumed").encode(), "cold resume lost read stamps")
        require((args.target / "once.txt").read_text() == "once", "cold resume replayed a command")
        retained = (args.target / "retained.txt").read_text()
        time.sleep(1.2)
        require((args.target / "retained.txt").read_text() == retained, "resumed process handle did not stop target job")
        checks.extend(["cold_native_resume_preserves_context_reads_and_effects", "background_process_handle_survives_native_resume"])
        # The Rust relay discards the real completed write reply and changes
        # the target independently. A replay would erase that later change.
        api.steps.extend([[], tool("Write", {"file_path": "lost-write-receipt.txt", "content": "native-write-before-loss\n"}),
                          tool("Read", {"file_path": "lost-write-receipt.txt"})])
        client.prompt(timeout=90)
        write_results = list(outputs(api.requests[-1]))[-2:]
        require(len(write_results) == 2 and not any(block.get("is_error") for block in write_results),
                "retained write result did not reach native Claude")
        require("external-write-after-commit" in json.dumps(write_results[-1]),
                "native Read did not observe the independent post-write change")
        require((args.target / "lost-write-receipt.txt").read_text() == "external-write-after-commit\n",
                "lost write completion replayed over a later target mutation")
        require(not (args.runtime / "lost-write-receipt.txt").exists(), "write escaped to runtime")
        checks.append("lost_file_write_completion_preserves_later_external_change")
        (args.target / "claude-reconnect-request").write_text("close the endpoint")
        deadline = time.monotonic() + 15
        while not (args.target / "claude-reconnect-closed").exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        require((args.target / "claude-reconnect-closed").exists(), "endpoint close fixture did not run")
        time.sleep(0.2)
        api.steps.extend([[], tool("Read", {"file_path": "fixture.txt"}),
                          tool("Bash", {"command": "printf recovered >> reconnect-once.txt"})])
        client.prompt(timeout=90)
        resumed_results = list(outputs(api.requests[-1]))[-2:]
        require(len(resumed_results) == 2 and not any(block.get("is_error") for block in resumed_results),
                "new tools did not recover after execution endpoint close")
        require((args.target / "reconnect-once.txt").read_text() == "recovered" and
                not (args.runtime / "reconnect-once.txt").exists(), "reconnect replayed or escaped the target")
        checks.append("closed_execution_endpoint_recovers_in_same_native_session_without_replay")
        api.steps.extend([[], [{"type": "text", "text": "<summary>Continue the target fixture. The target project instructions remain authoritative.</summary>"}],
                          tool("Read", {"file_path": "fixture.txt"})])
        client.prompt(text="/compact", timeout=90)
        client.prompt(timeout=90)
        context_checked(api.requests)
        checks.append("real_compaction_and_next_turn_preserve_target_context")
        client.close(); client = None
        client = native(session)
        client.ready()
        api.steps.extend([[], tool("Read", {"file_path": "fixture.txt"})])
        client.prompt(timeout=90)
        context_checked(api.requests)
        checks.append("cold_resume_after_compaction_has_no_runtime_file_locators")
        # Real target pipe writes split a UTF-8 character around another stream.
        # Native result validation must preserve the character, not merely the
        # adapter's unit-test representation of the chunks.
        api.steps.extend([[], tool("Bash", {"command":
            "python3 -c 'import os,time; os.write(1,bytes([228])); time.sleep(0.2); "
            "os.write(2,b\"stream-marker\"); time.sleep(0.2); os.write(1,bytes([184,173]))'"})])
        client.prompt(timeout=90)
        stream_result = json.dumps(list(outputs(api.requests[-1]))[-1], ensure_ascii=False)
        # Native merges both streams into one in write order, so a character
        # split around a stderr write decodes as native-local does (measured).
        require("\ufffdstream-marker\ufffd\ufffd" in stream_result,
                "Bash streams were not merged in write order: " + stream_result[:300])
        checks.append("native_bash_merges_streams_in_write_order")
        # Native validation must accept replacing a binary original, and target
        # path expansion must use the executor's home rather than Claude's.
        (args.target / "replace-image.png").write_bytes(pixel)
        (args.target / "bad-utf8.data-file").write_bytes(b"\xff\xfe\0")
        linked = args.target / "link-source.txt"
        linked.write_text("linked before\n")
        linked.chmod(0o751)
        (args.target / "file-link.txt").symlink_to(linked)
        (args.target / "home-relative.txt").write_text("target home before\n")
        def target_home_path(_requests):
            home = (args.target / "observed-home.txt").read_text()
            require(Path(home).is_absolute(), "target HOME is not absolute")
            relative = os.path.relpath(args.target / "home-relative.txt", home)
            require((args.runtime.parent / "claude-home" / relative).resolve() !=
                    (args.target / "home-relative.txt").resolve(), "home fixture cannot distinguish runtime and target")
            return tool("Read", {"file_path": "~/" + relative})
        image_write = tool("Write", {"file_path": "replace-image.png", "content": "image replaced with text\n"})
        binary_read = tool("Read", {"file_path": "bad-utf8.data-file"})
        api.steps.extend([[],
            tool("Read", {"file_path": "replace-image.png"}), image_write,
            binary_read,
            tool("Read", {"file_path": "file-link.txt"}),
            tool("Edit", {"file_path": "file-link.txt", "old_string": "linked before", "new_string": "linked after"}),
            tool("Bash", {"command": "printf '%s' \"$HOME\" > observed-home.txt"}), target_home_path,
            tool("Edit", {"file_path": str(args.target / "home-relative.txt"),
                          "old_string": "target home before", "new_string": "target home after"})])
        client.prompt(timeout=90)
        result_blocks = list(outputs(api.requests[-1]))
        image_result = next(block for block in result_blocks if block.get("tool_use_id") == image_write[0]["id"])
        require(not image_result.get("is_error"), "successful image replacement was reported as a failed Write")
        binary_result = next(block for block in result_blocks if block.get("tool_use_id") == binary_read[0]["id"])
        # Natively a file that is not valid UTF-8 still reads, its bytes
        # decoded with replacement characters (measured on 2.1.287).
        require(not binary_result.get("is_error") and "\\ufffd" in json.dumps(binary_result),
                "binary Read did not decode as native does: " + json.dumps(binary_result)[:300])
        require((args.target / "replace-image.png").read_text() == "image replaced with text\n",
                "image Write did not reach target")
        require((args.target / "file-link.txt").is_symlink() and linked.read_text() == "linked after\n"
                and linked.stat().st_mode & 0o777 == 0o751, "target symlink or executable mode was lost")
        require((args.target / "home-relative.txt").read_text() == "target home after\n",
                "tilde Read did not authorize the same absolute target file")
        context_checked(api.requests)
        checks.extend(["native_image_to_text_write_reports_success", "target_symlink_write_preserves_link_and_mode",
                       "tilde_read_uses_target_home_and_shares_absolute_path_stamp", "invalid_utf8_read_decodes_as_native"])
        for kind in ["symlink", "hardlink"]:
            source = args.target / f"race-{kind}-source.txt"
            alias = args.target / f"race-{kind}-alias.txt"
            source.write_text("before\n")
            if kind == "symlink":
                alias.symlink_to(source)
            else:
                os.link(source, alias)
            first = tool("Edit", {"file_path": source.name, "old_string": "before", "new_string": "first"})
            second = tool("Edit", {"file_path": alias.name, "old_string": "before", "new_string": "second"})
            api.steps.extend([[], tool("Read", {"file_path": source.name}),
                              tool("Read", {"file_path": alias.name}), first + second])
            client.prompt(timeout=90)
            blocks = list(outputs(api.requests[-1]))
            first_result = next(block for block in blocks if block.get("tool_use_id") == first[0]["id"])
            second_result = next(block for block in blocks if block.get("tool_use_id") == second[0]["id"])
            require(not first_result.get("is_error") and second_result.get("is_error")
                    and "modified since read" in json.dumps(second_result), "aliased edits silently overwrote each other")
            require(source.read_text() == "first\n" and alias.read_text() == "first\n"
                    and source.stat().st_ino == alias.stat().st_ino, "aliased edit changed file identity or lost content")
            checks.append(f"native_{kind}_alias_edit_rejects_stale_read")
        context_checked(api.requests)
        # Each search reads a FIFO. Its writer supplies content only after BOTH
        # readers have opened their pipes: sequential native/facade dispatch
        # cannot pass. This exercises native read-only scheduling and the target route.
        parallel = []
        pipes = [args.target / f"parallel-{name}.fifo" for name in ["a", "b"]]
        for pipe in pipes:
            os.mkfifo(pipe)
            parallel.extend(tool("Bash", {"command": f"grep parallel_read_complete {pipe}"}))
        overlap = []
        def feed_pipes():
            opened = {}
            deadline = time.monotonic() + 8
            try:
                while len(opened) < len(pipes) and time.monotonic() < deadline:
                    for pipe in pipes:
                        if pipe in opened:
                            continue
                        try:
                            opened[pipe] = os.open(pipe, os.O_WRONLY | os.O_NONBLOCK)
                        except OSError as error:
                            if error.errno != errno.ENXIO:
                                raise
                    time.sleep(0.01)
                if len(opened) == len(pipes):
                    overlap.append(True)
                    for descriptor in opened.values():
                        os.write(descriptor, b"parallel_read_complete\n")
            finally:
                for descriptor in opened.values():
                    os.close(descriptor)
        writer = threading.Thread(target=feed_pipes, daemon=True)
        writer.start()
        api.steps.extend([[], parallel])
        client.prompt(timeout=90)
        writer.join(timeout=10)
        require(overlap == [True], "independent native searches did not overlap")
        results = [json.dumps(block) for block in outputs(api.requests[-1])
                   if block.get("tool_use_id") in {call["id"] for call in parallel}]
        require(len(results) == 2 and all("parallel_read_complete" in result for result in results),
                "parallel searches did not return their target contents")
        context_checked(api.requests)
        checks.append("independent_native_searches_execute_concurrently")
        api.steps.extend([[], tool("Bash", {"command": "printf foreground_started >> foreground.txt; while :; do sleep 1; printf tick >> foreground.txt; done", "timeout": 600000})])
        messages_before = len(client.messages)
        client.send({"type": "user", "message": {"role": "user", "content": "Run the foreground cancellation fixture."},
                     "parent_tool_use_id": None, "session_id": ""})
        deadline = time.monotonic() + 20
        while not (args.target / "foreground.txt").exists():
            require(time.monotonic() < deadline, "foreground command did not start")
            time.sleep(0.05)
        client.send({"type": "control_request", "request_id": "cancel-foreground", "request": {"subtype": "interrupt"}})
        client.until(lambda frame: frame.get("type") == "control_response" and
                     frame["response"].get("request_id") == "cancel-foreground")
        if not any(frame.get("type") == "result" for frame in client.messages[messages_before:]):
            client.until(lambda frame: frame.get("type") == "result")
        foreground = (args.target / "foreground.txt").read_text()
        time.sleep(1.2)
        require((args.target / "foreground.txt").read_text() == foreground, "foreground descendants survived interruption")
        context_checked(api.requests)
        checks.append("native_interrupt_stops_foreground_target_process")
        # The base turn above always runs; an incremental run names the
        # phases its change affects (tools/claude_remote_check_map.json).
        agent_observations = []
        if "agents" in selected:
            client, agent_observations = agent_phases(args, api, client, native, session, context_checked, checks)
        if "permissions" in selected:
            permission_phases(args, api, client, context_checked, checks)
        shell_results = {}
        for name, phase in [("shell", lambda: shell_phases(args, api, client, shell_results, checks)),
                            ("notifications", lambda: notification_phases(args, api, client, checks)),
                            ("lifecycle", lambda: lifecycle_phases(args, api, client, checks)),
                            ("context", lambda: context_phases(args, api, client, checks)),
                            ("files", lambda: file_phases(args, api, client, checks)),
                            ("pdf", lambda: pdf_phases(args, api, client, checks)),
                            ("skills", lambda: skill_phases(args, api, client, checks)),
                            ("mcp", lambda: mcp_phases(args, api, client, checks)),
                            ("agent_background", lambda: subagent_background_phases(args, api, client, checks))]:
            if name in selected:
                phase()
        if "hooks" in selected:
            client = hook_phases(args, api, client, native, session, context_checked, checks)
        client.close(); client = None
        native_requests = len(api.requests)
        title_requests = len(api.title_requests)
        memory.accept(api.requests)
        api.close()

        # Drive the actual bundled ACP adapter, not just its private CLI shim.
        api = ScriptedApi([tool("Bash", {"command": "cat fixture.txt; printf acp_once >> acp-once.txt"})], native_titles=True)
        environment["ANTHROPIC_BASE_URL"] = f"http://127.0.0.1:{api.server_port}"
        class Acp(Executor):
            def send(self, message):
                super().send({"jsonrpc": "2.0", **message})

            def frame(self, deadline):
                while True:
                    frame = super().frame(deadline)
                    if frame.get("error"):
                        print("ACP fixture rejection:", json.dumps(frame["error"]))
                    require(not ("id" in frame and "method" in frame), "unexpected ACP client operation")
                    if "method" not in frame:
                        return frame
        def acp():
            instance = Acp([str(launcher.parent.parent / "bin/claude-agent-acp")], 120,
                           environment=environment, cwd=args.runtime)
            instance.request("initialize", {"protocolVersion": 1, "clientCapabilities": {},
                                             "clientInfo": {"name": "cowboy-fixture", "version": "1"}})
            return instance
        client = acp()
        created = client.request("session/new", {"cwd": str(args.runtime), "mcpServers": []})
        acp_session = created["sessionId"]
        client.request("session/set_mode", {"sessionId": acp_session, "modeId": "bypassPermissions"})
        client.request("session/set_config_option", {"sessionId": acp_session, "configId": "effort", "value": "high"})
        prompt = {"sessionId": acp_session, "prompt": [{"type": "text", "text": "Run the fixture."}]}
        client.request("session/prompt", prompt)
        context_checked(api.requests)
        require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP did not execute on target")
        client.close(); client = None
        api.steps.extend([[], tool("Bash", {"command": "cat acp-once.txt"})])
        client = acp()
        client.request("session/load", {"sessionId": acp_session, "cwd": str(args.runtime), "mcpServers": []})
        client.request("session/set_mode", {"sessionId": acp_session, "modeId": "bypassPermissions"})
        client.request("session/prompt", prompt)
        context_checked(api.requests)
        require((args.target / "acp-once.txt").read_text() == "acp_once", "packaged ACP replayed an effect")
        checks.extend(["packaged_acp_native_spawn_routes_target_tools", "packaged_acp_cold_load_preserves_binding_history_and_effects"])
        checks.append("hot_effort_configuration_preserves_bound_execution")
        client.close(); client = None
        before = len(api.requests)
        # Negative candidates are disposable copies; never modify supplied
        # packaged input bytes or production Plugin state.
        broken = args.runtime.parent / "broken-claude-package"
        (broken / "app").mkdir(parents=True)
        shutil.copytree(launcher.parent.parent / "bin", broken / "bin")
        (broken / "runtime").symlink_to(launcher.parent.parent / "runtime", target_is_directory=True)
        (broken / "app/node_modules").symlink_to(launcher.parent / "node_modules", target_is_directory=True)
        for name in ["cowboy-launch.mjs", "connection.mjs", "tools.mjs", "read-range.mjs", "mod-bridge.mjs", "memory.mjs", "matrix-client.mjs", "hook-proxy.mjs", "task-wait.mjs", "instructions.mjs", "skills.mjs", "mcp.mjs", "mcp-proxy.mjs"]:
            shutil.copyfile(launcher.parent / name, broken / "app" / name)
        (broken / "app/context-mod.js").write_text("export function register() { throw new Error('fixture broken module'); }\n")
        for extra in [(), ("--bare",)]:
            failed = Claude(str(broken / "bin/cowboy-configured-cli"), environment, args.runtime, fixture,
                            extra_arguments=extra, bound_native=True)
            try:
                try:
                    failed.ready()
                except ProbeFailure:
                    pass
                else:
                    raise ProbeFailure("invalid execution module or bare mode was accepted")
                failed.process.wait(timeout=10)
                require(len(api.requests) == before, "failed readiness reached model")
                failed.stderr.seek(0)
                diagnostic = failed.stderr.read().decode()
                require(("--bare" if extra else "execution module is missing or disabled") in diagnostic,
                        "readiness failed for an unrelated reason: " + diagnostic[-2000:])
            finally:
                failed.close()
        checks.extend(["broken_module_refused_before_model_request", "bare_mode_refused_without_changing_authentication"])
        shutil.copyfile(launcher.parent / "context-mod.js", broken / "app/context-mod.js")
        source = (launcher.parent / "tools.mjs").read_text()
        dispatch = "async dispatch(name, args, call) {"
        require(source.count(dispatch) == 1, "fault injection point changed; update the negative candidates")
        for label, injected in [("malformed_result", 'return { result: {} };'), ("bridge_error", "throw new Error('fixture lost result');")]:
            (broken / "app/tools.mjs").write_text(source.replace(dispatch, dispatch + "\n" + injected))
            api.steps.extend([[], tool("Bash", {"command": "printf forbidden_local_effect > forbidden-local.txt"})])
            failed = Claude(str(broken / "bin/cowboy-configured-cli"), environment, args.runtime, fixture, bound_native=True)
            try:
                failed.ready()
                failed.prompt(timeout=90)
                require(not (args.runtime / "forbidden-local.txt").exists() and not (args.target / "forbidden-local.txt").exists(),
                        label + " ran a native tool body")
                require(any(block.get("is_error") for block in outputs(api.requests[-1])), label + " did not return a tool error")
            finally:
                failed.close()
            checks.append(label + "_denies_without_native_fallback")
        checks.extend(memory.accept(api.requests))
        receipt = {
            "schema": "cowboy.claude-execution-worker-conformance/v1", "accepted": False, "checks": checks,
            "scope": "full" if selected == set(PHASES) else "incremental", "phases": sorted(selected),
            "claude_version": inputs["claude_version"], "claude_sha256": inputs["claude_sha256"],
            "executor_sha256": inputs["sha256"], "executor_version": inputs["version"],
            "packaged_launcher_sha256": hashlib.sha256(launcher.read_bytes()).hexdigest(),
            "scripted_api_requests": native_requests + len(api.requests),
            "native_title_requests": title_requests + len(api.title_requests),
            "agent_observations": agent_observations,
            "shell_results": shell_results,
            "production_credentials": False, "production_activation": False,
            "not_checked": ["real_subscription_inference", "cross_host_latency", "project_hook_settings_reload",
                            "agent_permission_modes", "grandchild_agents", "teammates_and_agent_worktrees",
                            "agent_partial_output_stream", "native_runtime_crash_with_live_agent"],
        }
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"accepted {len(checks)} packaged Claude worker checks")
    finally:
        if client:
            if isinstance(client, Claude) and client.process.poll() is not None and not client.stderr.closed:
                client.stderr.seek(0)
                print(client.stderr.read().decode()[-2000:])
            client.close()
        api.close()


if __name__ == "__main__":
    main()
