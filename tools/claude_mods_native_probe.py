#!/usr/bin/env python3
"""Measure the native Mods behavior context-mod.js relies on for agents and notifications.

Runs the pinned native CLI with a recording Mod against a scripted loopback
API (no credentials, no inference) and records: classic event input fields,
turn.complete outcomes for agents, next.signal on a held call whose agent is
stopped, how task notifications reach the model when the session is idle or
busy, notification of a plugin-started background command, the Mods API
surface, and whether a pending Mod fetch holds back other native work.
Diff a fresh receipt with tools/claude_mods_native_baseline.json for every
Claude CLI candidate.

Usage: python3 tools/claude_mods_native_probe.py --claude <native cli> --receipt <new path>
"""
import argparse
from http.server import BaseHTTPRequestHandler
import json
from pathlib import Path
import re
import secrets
from socketserver import ThreadingUnixStreamServer
import subprocess
import tempfile
import threading
import time
from unittest.mock import patch

from claude_native_behavior_probe import stable
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from plugin_runtime_conformance import closed_environment

FIXTURE_ENVIRONMENT = {"ANTHROPIC_API_KEY": "offline-fixture-not-a-credential", "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
                       "DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"}

# The recording Mod. Every observation is a POST to the probe's socket; a
# "slow" observation is answered after SLOW seconds.
MOD = r'''let context;
async function observe($, kind, body = {}) {
  // Loaded on first use: session.start does not run before every event.
  context ??= JSON.parse(await $.fs.read(await $.env.get("COWBOY_CLAUDE_CONTEXT")));
  try {
    await $.http.fetch("http://cowboy-execution/" + kind, { socketPath: context.socketPath, method: "POST",
      headers: { Authorization: "Bearer " + context.token }, body: JSON.stringify({ kind, ...body }) });
  } catch {}
}
const fields = (event) => Object.keys(event ?? {}).sort();
// Classic handlers stay synchronous and never touch $ (an async one disables
// the Mod; see the async_classic scenario); tool.call forwards what they saw.
const classics = [];
function classic(name, event, next) {
  classics.push({ name, keys: fields(event), agent_type: event?.agent_type ?? null,
    hook_event_name: event?.hook_event_name ?? null, has_agent_id: typeof event?.agent_id === "string",
    effort: event?.effort ? Object.keys(event.effort).sort() : null });
  return next(event);
}
async function flush($) {
  while (classics.length) await observe($, "classic", classics.shift());
}
export function register(on) {
  // Literal names and handler functions: a handler built by a call (classic("X"))
  // disabled the whole Mod on 2.1.287.
  on("classic.SessionStart", (_$, event, next) => classic("SessionStart", event, next));
  on("classic.UserPromptSubmit", (_$, event, next) => classic("UserPromptSubmit", event, next));
  on("classic.PostToolBatch", (_$, event, next) => classic("PostToolBatch", event, next));
  on("classic.Stop", (_$, event, next) => classic("Stop", event, next));
  on("classic.SubagentStart", (_$, event, next) => classic("SubagentStart", event, next));
  on("turn.complete", async ($, event, next) => {
    await flush($);
    await observe($, "turn", { agent: event.agentId !== undefined, keys: fields(event),
      isAborted: event.isAborted ?? null, reason: event.reason ?? null, answer: event.answer ?? null });
    return next(event);
  });
  on("session.append", { door: "prompt" }, async ($, event, next) => {
    if (event.origin?.kind === "task-notification") await observe($, "append", { door: "prompt",
      content: typeof event.message?.content === "string" ? event.message.content : JSON.stringify(event.message?.content) });
    return next(event);
  });
  on("session.append", { door: "delivery" }, async ($, event, next) => {
    if (event.origin?.kind === "task-notification") await observe($, "append", { door: "delivery",
      content: typeof event.message?.content === "string" ? event.message.content : JSON.stringify(event.message?.content) });
    return next(event);
  });
  on("prompt.attachment", { type: "queued_command" }, async ($, event, next) => {
    await observe($, "queued", { text: typeof event.text === "string" ? event.text : null });
    return next(event);
  });
  on("tool.call", async ($, event, next) => {
    await flush($);
    // tool.call events carry the tool input at the top level.
    const command = typeof event.command === "string" ? event.command : "";
    await observe($, "call", { tool: event.tool, agent: event.agentId !== undefined, command });
    if (command.includes("HOLD")) {
      // A held target call: it ends when native aborts it, or after the
      // probe's delayed "wait" answer (the Mod runtime has no timers).
      const signal = next.signal;
      const timers = { setTimeout: typeof setTimeout };
      const aborted = new Promise((resolve) => {
        if (!signal) return resolve("no signal");
        if (signal.aborted) return resolve("already aborted");
        signal.addEventListener("abort", () => resolve("aborted"), { once: true });
      });
      const reason = await Promise.race([aborted, observe($, "wait").then(() => "not aborted within 10 s")]);
      await observe($, "held", { reason, timers });
      return { result: { stdout: "", stderr: "held call ended", interrupted: true } };
    }
    if (command.includes("PLUGIN_TRIGGER")) {
      const started = await $.tool.call({ tool: "Bash", command: "sleep 1; echo PLUGIN_BG", run_in_background: true });
      await observe($, "plugin_call", { result: JSON.stringify(started).slice(0, 400) });
      return next(event);
    }
    if (command.includes("SLOWOBS")) await observe($, "slow");
    return next(event);
  });
}
'''
SLOW = 4


class Observer(ThreadingUnixStreamServer):
    daemon_threads = True


class Recorder(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        if self.headers.get("Authorization") != "Bearer " + self.server.token:
            self.send_response(403)
            self.end_headers()
            return
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
        body["at"] = round(time.monotonic() - self.server.start, 2)
        self.server.events.append(body)
        if body["kind"] == "wait":
            time.sleep(10)
            self.server.events.append({"kind": "wait_answered", "at": round(time.monotonic() - self.server.start, 2)})
        if body["kind"] == "slow":
            time.sleep(SLOW)
            self.server.events.append({"kind": "slow_answered", "at": round(time.monotonic() - self.server.start, 2)})
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"{}")


def texts(request):
    return json.dumps(request.get("messages", []))


def results(message):
    return isinstance(message.get("content"), list) and any(
        block.get("type") == "tool_result" for block in message["content"])


def child(request, marker):
    return marker in json.dumps(request.get("messages", [])[:1])


def agent(prompt):
    return tool("Agent", {"description": "Probe child", "prompt": prompt, "subagent_type": "general-purpose",
                          "run_in_background": True})


def scenario(name, clock=time.monotonic):
    """A router for the scripted API: each request gets the scenario's next step."""
    state = {"agent": None, "ping_at": None}

    def router(requests):
        request = requests[-1]
        last = request["messages"][-1]
        history = texts(request)
        if name in ACCESS:
            return [{"type": "text", "text": "DONE"}] if results(last) else tool("Bash", {"command": "echo hi"})
        if name == "plugin_background":
            if "PLUGIN_BG" in json.dumps(last) and "<task-notification>" in json.dumps(last):
                return [{"type": "text", "text": "NOTED"}]
            if results(last):
                return [{"type": "text", "text": "STARTED"}]
            return tool("Bash", {"command": "echo PLUGIN_TRIGGER"})
        if name == "pending_fetch_parent" and "PING" in json.dumps(last) and state["ping_at"] is None:
            state["ping_at"] = clock()
            return [{"type": "text", "text": "PONG"}]
        marker = {"background_idle": "CHILD_FAST", "background_busy": "CHILD_FAST", "taskstop_held": "CHILD_HOLD",
                  "pending_fetch": "CHILD_STEPS", "pending_fetch_parent": "CHILD_SLOW"}[name]
        if child(request, marker):
            steps = sum(1 for message in request["messages"] if results(message))
            if marker == "CHILD_HOLD":
                return [{"type": "text", "text": "CHILD_DONE"}] if steps else tool("Bash", {"command": "echo HOLD"})
            if marker == "CHILD_STEPS":
                return [{"type": "text", "text": "CHILD_DONE"}] if steps >= 4 else \
                    tool("Bash", {"command": f"echo step{steps}; sleep 0.5"})
            if marker == "CHILD_SLOW":
                return [{"type": "text", "text": "CHILD_DONE"}] if steps else tool("Bash", {"command": "echo SLOWOBS"})
            return [{"type": "text", "text": "CHILD_DONE"}] if steps else tool("Bash", {"command": "echo child"})
        if "<task-notification>" in json.dumps(last):
            return [{"type": "text", "text": "NOTED"}]
        launched = re.search(r"agentId: ([a-z0-9]+)", history)
        if not launched:
            return agent(marker + " run the step.")
        state["agent"] = launched[1]
        if name == "background_busy" and "sleep 4" not in history:
            return tool("Bash", {"command": "sleep 4; echo parent"})
        if name == "taskstop_held" and "TaskStop" not in history:
            time.sleep(1)
            return tool("TaskStop", {"task_id": state["agent"]})
        if name == "pending_fetch" and "SLOWOBS" not in history:
            return tool("Bash", {"command": "echo SLOWOBS"})
        return [{"type": "text", "text": "PARENT_IDLE"}]
    router.state = state
    return router


ACCESS = {
    # A Mod that touches $.session in session.start.
    "start_api_access": ("""  on("session.start", async ($, event, next) => {
    try { typeof $.session.id; } catch {}
    return next(event);
  });
""", ""),
    # A Mod that reads $.session.receive in tool.call before observing.
    "receive_access": ("", "    try { typeof $.session.receive; } catch {}\n"),
    # A Mod registering a handler that a call expression built.
    "factory_handler": ("""  const make = () => (_$, event, next) => next(event);
  on("classic.Stop", make());
""", ""),
    # A Mod with an async classic handler that awaits a Mods call.
    "async_classic": ("""  on("classic.UserPromptSubmit", async ($, event, next) => {
    await $.env.get("COWBOY_CLAUDE_CONTEXT");
    return next(event);
  });
""", ""),
}
ACCESS_MOD = r'''let context;
export function register(on) {
START
  on("tool.call", async ($, event, next) => {
CALL    context ??= JSON.parse(await $.fs.read(await $.env.get("COWBOY_CLAUDE_CONTEXT")));
    await $.http.fetch("http://cowboy-execution/call", { socketPath: context.socketPath, method: "POST",
      headers: { Authorization: "Bearer " + context.token }, body: JSON.stringify({ kind: "call" }) });
    return next(event);
  });
}
'''


def run_scenario(claude, root, name):
    runtime = root / "runtime"
    runtime.mkdir()
    with tempfile.TemporaryDirectory(prefix="cowboy-mods-", dir="/tmp") as sockets:
        socket_path = str(Path(sockets) / "observer.sock")
        server = Observer(socket_path, Recorder)
        server.token, server.events, server.start = secrets.token_hex(16), [], time.monotonic()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        plugin = root / "plugin"
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / "hooks").mkdir()
        (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "mods-probe", "version": "1.0.0"}))
        (plugin / "hooks/hooks.json").write_text(json.dumps({"modules": ["./register.js"]}))
        (plugin / "hooks/register.js").write_text(
            ACCESS_MOD.replace("START\n", ACCESS[name][0]).replace("CALL", ACCESS[name][1]) if name in ACCESS else MOD)
        (root / "context.json").write_text(json.dumps({"socketPath": socket_path, "token": server.token}))
        router = scenario(name, lambda: round(time.monotonic() - server.start, 2))
        api = ScriptedApi([router] * 30, native_titles=True)
        env = closed_environment(root / "home")
        env.update(FIXTURE_ENVIRONMENT, ANTHROPIC_BASE_URL=f"http://127.0.0.1:{api.server_port}",
                   COWBOY_CLAUDE_CONTEXT=str(root / "context.json"))
        original = subprocess.Popen

        def spawn(argv, *args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = "Agent,Bash,TaskStop"
            argv[argv.index("--disallowedTools") + 1] = "Skill"
            return original(argv, *args, **kwargs)
        client = None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(claude), env, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True,
                                extra_arguments=["--plugin-dir", str(plugin)])
            client.ready()
            client.prompt(text="go", timeout=90)
            if name == "pending_fetch_parent":
                # While the child's Mod fetch is pending, the parent gets a new message.
                deadline = time.monotonic() + 20
                while not any(event["kind"] == "slow" for event in server.events) and time.monotonic() < deadline:
                    time.sleep(0.05)
                client.send({"type": "user", "message": {"role": "user", "content": "PING"},
                             "parent_tool_use_id": None, "session_id": ""})
            end = time.monotonic() + (20 if name == "taskstop_held" else 12)
            while time.monotonic() < end:
                try:
                    client.until(lambda frame: False, timeout=max(0.1, end - time.monotonic()))
                except Exception:
                    pass
            notified = [request for request in api.requests if "<task-notification>" in json.dumps(
                request["messages"][-1:])]
            return {"events": server.events, "api_requests": len(api.requests), "ping_at": router.state["ping_at"],
                    "notification_messages": [request["messages"][-1] for request in notified]}
        finally:
            if client:
                client.close()
            api.close()
            server.shutdown()
            server.server_close()


def summarize(name, observed):
    """The behavior a scenario establishes, without timings or raw transcripts."""
    events = observed["events"]
    kinds = lambda kind: [event for event in events if event["kind"] == kind]  # noqa: E731
    summary = {"classic": sorted({json.dumps({key: event[key] for key in
                                              ("name", "keys", "agent_type", "hook_event_name", "has_agent_id", "effort")})
                                  for event in kinds("classic")}),
               "turns": sorted({json.dumps({key: event[key] for key in ("agent", "keys", "isAborted", "reason")})
                                for event in kinds("turn")}),
               "appends": [{"door": event["door"], "content": event["content"]} for event in kinds("append")],
               "queued": [event["text"] for event in kinds("queued")],
               "notification_messages": observed["notification_messages"]}
    summary["classic"] = [json.loads(item) for item in summary["classic"]]
    summary["turns"] = [json.loads(item) for item in summary["turns"]]
    if name in ACCESS:
        return {"mod_active": bool(kinds("call"))}
    if name == "taskstop_held":
        # The child's held call has a pending fetch ("wait", answered after 10 s).
        stop = [event["at"] for event in kinds("call") if event["tool"] == "TaskStop"]
        answered = kinds("wait_answered")
        summary["stop_processed_during_pending_child_fetch"] = bool(stop and answered and stop[0] < answered[0]["at"])
        # Whether the held handler reported anything (abort or timeout) after the stop.
        summary["held_handler_observed_after_stop"] = bool(kinds("held"))
    if name == "plugin_background":
        summary["plugin_call"] = [event["result"] for event in kinds("plugin_call")]
        summary["plugin_command_notified"] = any("PLUGIN_BG" in json.dumps(message)
                                                 for message in observed["notification_messages"])
    if name == "pending_fetch_parent":
        answered = kinds("slow_answered")
        return {"parent_turn_ran_during_pending_child_fetch": None if not answered or observed["ping_at"] is None
                else observed["ping_at"] < answered[0]["at"]}
    if name == "pending_fetch":
        slow = kinds("slow")
        window = (slow[0]["at"], kinds("slow_answered")[0]["at"]) if slow and kinds("slow_answered") else None
        child_calls = [event["at"] for event in kinds("call") if event["agent"]]
        # How many fall inside the window is timing; whether any do is the behavior.
        summary["child_progressed_during_pending_fetch"] = None if window is None else \
            any(window[0] < at < window[1] for at in child_calls)
        for key in ("appends", "queued", "notification_messages"):
            summary.pop(key)
    return summary


SCENARIOS = ["background_idle", "background_busy", "taskstop_held", "plugin_background", "pending_fetch",
             "pending_fetch_parent",
             "start_api_access", "receive_access", "async_classic", "factory_handler"]


def run(claude, scenarios=SCENARIOS):
    receipt = {}
    for name in scenarios:
        with tempfile.TemporaryDirectory(prefix="cowboy-native-mods-") as temp:
            try:
                value = summarize(name, run_scenario(claude, Path(temp), name))
            except Exception as error:  # A failed scenario is a recorded observation, not a crash.
                value = {"error": str(error)[:300]}
            receipt[name] = json.loads(stable(json.dumps(value), temp))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--scenario", choices=SCENARIOS, action="append")
    args = parser.parse_args()
    if args.receipt.exists():
        parser.error("use a new receipt path")
    args.receipt.write_text(json.dumps(run(args.claude, args.scenario or SCENARIOS), indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
