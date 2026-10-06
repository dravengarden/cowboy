#!/usr/bin/env python3
"""Research native child tool/context inheritance; never enables production agents.

Uses pinned native binaries, a disposable copy of the shipping context Mod and
a research-only Read bridge to a resident keeper. Not a production adapter.
"""
import argparse
import base64
from contextlib import ExitStack
import hashlib
from http.server import BaseHTTPRequestHandler
import json
from pathlib import Path
import secrets
import socket
from socketserver import ThreadingUnixStreamServer
import subprocess
import tempfile
import threading
from unittest.mock import patch

from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from execution_environment_probe import require
from plugin_runtime_conformance import closed_environment
from claude_native_task_stop_probe import resident


class Bridge(ThreadingUnixStreamServer):
    daemon_threads = True


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        require(self.headers.get("Authorization") == "Bearer " + self.server.token, "bridge token differs")
        length = int(self.headers.get("Content-Length", "0"))
        require(0 < length < 65536, "invalid fixture request size")
        data = json.loads(self.rfile.read(length))
        if self.path == "/ready":
            self.server.ready += 1
            result = {"ready": True}
        elif self.path == "/observe":
            self.server.events.append(data)
            result = {}
        else:
            require(self.path == "/tool" and data["tool"] == "Read", "unexpected target operation")
            require(data["input"]["file_path"] == "fixture.txt", "unexpected target path")
            self.server.calls.append(data)
            native = self.server.keeper.result("fs/readFile", {
                "path": (self.server.target / "fixture.txt").as_uri(),
            })
            content = base64.b64decode(native["dataBase64"]).decode()
            result = {"result": {"type": "text", "file": {
                "filePath": str(self.server.target / "fixture.txt"), "content": content,
                "numLines": 1, "startLine": 1, "totalLines": 1,
            }}}
        encoded = json.dumps(result).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def probe(args, source):
    with tempfile.TemporaryDirectory(prefix="cowboy-agent-research-") as temp, \
            tempfile.TemporaryDirectory(prefix="cowboy-claude-mod-", dir="/tmp") as sockets, ExitStack() as lifecycle:
        root = Path(temp)
        runtime, target = root / "runtime", root / "target"
        runtime.mkdir(); target.mkdir()
        (runtime / "fixture.txt").write_text("RUNTIME_FILE_MUST_NOT_REACH_CHILD")
        (runtime / "CLAUDE.md").write_text("RUNTIME_GUIDANCE_MUST_NOT_REACH_CHILD")
        (target / "fixture.txt").write_text("TARGET_CHILD_READ_PROOF")
        (root / "target-home").mkdir()
        _, keeper, _, _ = lifecycle.enter_context(resident(
            args, root, target, runtime, closed_environment(root / "keeper-home")))
        server = Bridge(str(Path(sockets) / "bridge.sock"), Handler)
        server.token = secrets.token_hex(32)
        server.ready = 0
        server.target, server.events, server.calls = target, [], []
        server.keeper = keeper
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        plugin = root / "plugin"
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / "hooks").mkdir()
        (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "cowboy-agent-research", "version": "1.0.0"}))
        (plugin / "hooks/hooks.json").write_text(json.dumps({"modules": ["./register.js"]}))
        module = source.read_text()
        # This research spliced Agent support into the 3.4.12 Mod. Plugin 3.5.0
        # admits Agent natively (and owns turn.complete, which a second handler
        # would make the pinned loader reject); its acceptance is
        # execution-worker-conformance. Run this probe against the 3.4.12 Mod.
        require('event.tool === "Agent"' not in module,
                "shipping Mod already admits Agent; use execution-worker-conformance")
        if args.read_task_output:
            module = "const researchTasks = new Map();\n" + module
        needle = '["TodoWrite", "AskUserQuestion"].includes(event.tool)'
        require(module.count(needle) == 1, "shipping passthrough changed; reassess experiment")
        module = module.replace(needle, '["TodoWrite", "AskUserQuestion", "Agent"].includes(event.tool)')
        hook = 'on("tool.call", async ($, event, next) => {'
        require(module.count(hook) == 1, "shipping tool hook changed")
        module = module.replace(hook, hook + '''
    await $.http.fetch("http://cowboy-execution/observe", {
      socketPath: context.socketPath, method: "POST",
      headers: { Authorization: "Bearer " + context.bridgeToken },
      body: JSON.stringify({ tool: event.tool, agentId: event.agentId ?? null }),
    });
''')
        native_passthrough = 'if (["TodoWrite", "AskUserQuestion", "Agent"].includes(event.tool)) {'
        module = module.replace(native_passthrough, '''if (event.tool === "Agent") {
      const result = await next(event);
      await $.http.fetch("http://cowboy-execution/observe", {
        socketPath: context.socketPath, method: "POST",
        headers: { Authorization: "Bearer " + context.bridgeToken },
        body: JSON.stringify({ kind: "agent-result", keys: Object.keys(result),
          resultKeys: Object.keys(result.result ?? {}),
          textType: typeof result.text, agentId: result.result?.agentId ?? null }),
      });
      return result;
    }
    ''' + native_passthrough)
        if args.project_task_output:
            end = 'return result;\n    }\n    ' + native_passthrough
            require(module.count(end) == 1, "research Agent result boundary changed")
            module = module.replace(end, '''const task = result.result;
      if (!task?.isAsync || typeof task.outputFile !== "string" ||
          typeof result.text !== "string" || !/^[a-zA-Z0-9_-]+$/.test(task.agentId)) {
        throw new Error("Unexpected native Agent result shape");
      }
      const handle = "cowboy-agent://" + task.agentId;
      return { ...result, result: { ...task, outputFile: handle },
        text: result.text.replaceAll(task.outputFile, handle) };
    }
    ''' + native_passthrough)
        if args.read_task_output:
            module = module.replace('const handle = "cowboy-agent://" + task.agentId;', '''const handle = "cowboy-agent://" + task.agentId;
      researchTasks.set(handle, { path: task.outputFile, owner: event.agentId ?? null });''')
            module = module.replace(hook, hook + '''
    if (event.tool === "Read" && event.file_path?.startsWith("cowboy-agent://")) {
      const task = researchTasks.get(event.file_path);
      if (!task || task.owner !== (event.agentId ?? null)) return { deny: "Unknown task owner" };
      const result = await next({ ...event, file_path: task.path });
      await $.http.fetch("http://cowboy-execution/observe", {
        socketPath: context.socketPath, method: "POST",
        headers: { Authorization: "Bearer " + context.bridgeToken },
        body: JSON.stringify({ kind: "task-read", keys: Object.keys(result),
          resultKeys: Object.keys(result.result ?? {}) }),
      });
      return result;
    }
''')
        if args.native_task_output:
            require(module.count('next({ ...event, file_path: task.path })') == 1, "missing native Read boundary")
            module = module.replace('event.tool === "Read" && event.file_path?.startsWith("cowboy-agent://")',
                                    'event.tool === "TaskOutput"')
            module = module.replace('researchTasks.get(event.file_path)',
                                    'researchTasks.get("cowboy-agent://" + event.task_id)')
            module = module.replace('next({ ...event, file_path: task.path })', 'next(event)')
        if args.completion_output:
            require(module.count('const result = await next({ ...event, file_path: task.path });') == 1,
                    "missing native Read result boundary")
            module = "const researchAnswers = new Map();\n" + module
            module = module.replace('export function register(on) {', '''export function register(on) {
  on("turn.complete", async ($, event, next) => {
    if (event.agentId) researchAnswers.set(event.agentId, {
      answer: event.answer, isAborted: event.isAborted, turnId: event.turnId, reason: event.reason,
    });
    return next(event);
  });''')
            module = module.replace('const result = await next({ ...event, file_path: task.path });', '''const answer = researchAnswers.get(event.file_path.slice("cowboy-agent://".length));
      if (!answer || answer.isAborted || answer.reason !== "answer" || typeof answer.answer !== "string") {
        return { deny: "Completed task answer unavailable" };
      }
      const result = { result: { type: "text", file: {
        filePath: event.file_path, content: answer.answer, startLine: 1,
        numLines: answer.answer.split("\\n").length, totalLines: answer.answer.split("\\n").length,
      } } };''')
        (plugin / "hooks/register.js").write_text(module)
        context = root / "context.json"
        context.write_text(json.dumps({"schema": 1, "nonce": secrets.token_hex(16),
            "socketPath": str(Path(sockets) / "bridge.sock"), "bridgeToken": server.token,
            "environment": f"Execution directory: {target}",
            "instructions": "TARGET_GUIDANCE_MUST_REACH_CHILD", "git": "Target fixture",
            "descriptions": {"Read": "Read a file from the bound target project."}}))
        child_prompt = "CHILD_PROBE Read fixture.txt and return its content."
        read_requested = False
        output_call_id = None
        task_output_result = None
        def is_child(request):
            return any(child_prompt in block.get("text", "")
                       for message in request.get("messages", []) if message.get("role") == "user"
                       for block in message.get("content", []) if isinstance(block, dict) and block.get("type") == "text")
        def response(requests):
            nonlocal read_requested, output_call_id, task_output_result
            request = requests[-1]
            results = [block for message in request.get("messages", []) if isinstance(message.get("content"), list)
                       for block in message["content"] if block.get("type") == "tool_result"]
            if is_child(request):
                if any("TARGET_CHILD_READ_PROOF" in json.dumps(block) for block in results):
                    return [{"type": "text", "text": "CHILD_FINISHED"}]
                require(not results, "child Read failed; inspect native result")
                return tool("Read", {"file_path": "fixture.txt"})
            if args.read_task_output and "READ_COMPLETED_TASK" in json.dumps(request):
                if not read_requested:
                    read_requested = True
                    tasks = [e for e in server.events if e.get("kind") == "agent-result"]
                    require(len(tasks) == 1, "missing task registration")
                    if args.native_task_output:
                        call = tool("TaskOutput", {"task_id": tasks[0]["agentId"], "block": False, "timeout": 1000})
                    else:
                        call = tool("Read", {"file_path": "cowboy-agent://" + tasks[0]["agentId"]})
                    output_call_id = call[0]["id"]
                    return call
                matches = [block for block in results if block.get("tool_use_id") == output_call_id]
                require(len(matches) == 1, "matching task output tool result missing")
                task_output_result = matches[0]
                return [{"type": "text", "text": "TASK_READ_FINISHED"}]
            if results:
                return [{"type": "text", "text": "PARENT_WAITING"}]
            return tool("Agent", {"subagent_type": "general-purpose", "description": "Probe target read", "prompt": child_prompt})
        api = ScriptedApi([response] * 8, native_titles=True)
        environment = closed_environment(root / "home")
        environment.update({"COWBOY_CLAUDE_CONTEXT": str(context),
            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}",
            "ANTHROPIC_API_KEY": "offline-fixture-not-a-credential",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "DISABLE_AUTOUPDATER": "1",
            "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"})
        original_spawn = subprocess.Popen
        def spawn(argv, *spawn_args, **kwargs):
            argv = list(argv)
            argv[argv.index("--tools") + 1] = "Agent,Read,TaskOutput" if args.native_task_output else "Agent,Read"
            argv[argv.index("--disallowedTools") + 1] = "Skill"
            return original_spawn(argv, *spawn_args, **kwargs)
        client = None
        try:
            with patch.object(subprocess, "Popen", spawn):
                client = Claude(str(args.claude), environment, runtime, WorkspaceFixture(target), aliases=False,
                                bound_native=True, extra_arguments=["--plugin-dir", str(plugin)])
            client.ready()
            client.prompt(text="/cost")
            require(not api.requests, "readiness called model")
            require(server.ready > 0, "context Mod did not acknowledge fixture bridge")
            client.prompt(timeout=90)
            tasks = [e for e in server.events if e.get("kind") == "agent-result"]
            require(len(tasks) == 1 and tasks[0].get("agentId"), "native Agent identity missing")
            def completed(frame):
                return (frame.get("type") == "system" and frame.get("subtype") == "task_notification"
                        and frame.get("task_id") == tasks[0]["agentId"])
            completion = next((frame for frame in client.messages if completed(frame)), None)
            if completion is None:
                completion = client.until(completed, timeout=30)
            require(completion.get("status") == "completed", "native child did not complete")
            if args.read_task_output:
                client.prompt(text="READ_COMPLETED_TASK", timeout=40)
                def read_finished(frame):
                    return frame.get("type") == "assistant" and "TASK_READ_FINISHED" in json.dumps(frame)
                if not any(read_finished(frame) for frame in client.messages):
                    client.until(read_finished, timeout=40)
                require(read_requested, "task output was not requested")
                if args.native_task_output:
                    require(task_output_result.get("is_error") is True and
                            "No such tool available: TaskOutput" in json.dumps(task_output_result),
                            "native TaskOutput availability changed; reassess research")
                else:
                    require(any(e.get("kind") == "task-read" for e in server.events),
                            "registered task Read did not run")
                    require("CHILD_FINISHED" in json.dumps(task_output_result), "native task Read lost child output")
            require(api.failure is None, api.failure or "scripted API failed")
            children = [request for request in api.requests if is_child(request)]
            require(len(children) == 2, "native child did not execute exactly two requests")
            require(len(server.calls) == 1, "child Read did not reach fixture bridge exactly once")
            child_events = [e for e in server.events if e.get("tool") == "Read"]
            require(len(child_events) == 1 and child_events[0]["agentId"], "Read was not identified as a child call")
            require("TARGET_CHILD_READ_PROOF" in json.dumps(children[-1]), "target result missing from child context")
            require("CHILD_FINISHED" in json.dumps(completion), "native completion omitted child result")
            encoded = json.dumps(api.requests)
            require(all("TARGET_GUIDANCE_MUST_REACH_CHILD" in json.dumps(x) and str(target) in json.dumps(x)
                        for x in children), "child lost target context")
            require(((args.read_task_output and not args.completion_output) or str(runtime) not in encoded) and "RUNTIME_FILE_MUST_NOT_REACH_CHILD" not in encoded
                    and "RUNTIME_GUIDANCE_MUST_NOT_REACH_CHILD" not in encoded,
                    "runtime workspace leaked into model context")
            if args.project_task_output:
                if not args.read_task_output:
                    require(str(root / "home") not in encoded, "Agent locator projection left a runtime path in model context")
                require("cowboy-agent://" in encoded, "native Agent result did not retain the projected handle")
            return {"child_tool_intercepted": True, "child_agent_id_present": True,
                "model_task_locator_projection": args.project_task_output,
                "native_registered_task_read": args.read_task_output and not args.native_task_output and read_requested,
                "native_task_output_tool": args.native_task_output,
                "native_task_output_unavailable": args.native_task_output and task_output_result.get("is_error") is True,
                "native_completion_answer_read": args.completion_output,
                "task_output_contains_runtime_cwd": str(runtime) in json.dumps(task_output_result),
                "task_output_contains_runtime_home": str(root / "home") in json.dumps(task_output_result),
                "target_read_calls": len(server.calls), "native_child_completion_notification": True,
                "child_requests_have_target_guidance": all("TARGET_GUIDANCE_MUST_REACH_CHILD" in json.dumps(x) for x in children),
                "child_requests_have_target_directory": all(str(target) in json.dumps(x) for x in children),
                "runtime_path_leaked": str(runtime) in encoded,
                "runtime_home_path_leaked": str(root / "home") in encoded,
                "runtime_file_leaked": "RUNTIME_FILE_MUST_NOT_REACH_CHILD" in encoded,
                "runtime_guidance_leaked": "RUNTIME_GUIDANCE_MUST_NOT_REACH_CHILD" in encoded,
                "scripted_api_requests": len(api.requests), "tool_events": server.events,
                "native_completion_exposes_runtime_path": str(runtime) in json.dumps(completion),
                "native_completion_exposes_runtime_home": str(root / "home") in json.dumps(completion),
                "native_completion_fields": sorted(completion),
                "child_request_context_keys": [list(x) for x in children]}
        except Exception as error:
            if api.failure:
                raise RuntimeError("scripted API failed: " + api.failure) from error
            raise
        finally:
            if client: client.close()
            api.close()
            server.shutdown(); server.server_close(); thread.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--claude-sha256", required=True)
    parser.add_argument("--keeper", type=Path, required=True)
    parser.add_argument("--executor", type=Path, required=True)
    parser.add_argument("--executor-sha256", required=True)
    parser.add_argument("--project-task-output", action="store_true",
                        help="Research only: project the native Agent tool's output locator, without enabling handle reads")
    parser.add_argument("--read-task-output", action="store_true",
                        help="Research exact owner-registered handle translation through native Read")
    parser.add_argument("--native-task-output", action="store_true",
                        help="Use owner-registered native TaskOutput instead of reading the raw task file")
    parser.add_argument("--completion-output", action="store_true",
                        help="Read only the final answer observed from native turn.complete, not the raw transcript")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require(not args.read_task_output or args.project_task_output, "task Read requires locator projection")
    require(not args.native_task_output or args.read_task_output, "native TaskOutput requires output research")
    require(not args.completion_output or (args.read_task_output and not args.native_task_output),
            "completion output requires Read and excludes TaskOutput")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    require(args.claude.is_absolute() and hashlib.sha256(args.claude.read_bytes()).hexdigest() == args.claude_sha256,
            "pinned native binary digest differs")
    require(args.executor.is_absolute() and hashlib.sha256(args.executor.read_bytes()).hexdigest() == args.executor_sha256,
            "pinned executor digest differs")
    require(args.keeper.is_absolute() and args.keeper.is_file(), "absolute keeper executable required")
    source = Path(__file__).resolve().parent.parent / "plugins/claude-code/runtime/context-mod.js"
    result = {"schema": "cowboy.claude-native-agent-research/v1", "claude_sha256": args.claude_sha256,
        "mod_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "keeper_sha256": hashlib.sha256(args.keeper.read_bytes()).hexdigest(),
        "executor_sha256": args.executor_sha256,
        "topology": "same-host native Claude and independent resident keeper, shared filesystem",
        "production_credentials": False, "real_model_requests": 0,
        "not_checked": ["enrolled_worker_transport", "cross_host", "durable_handle_reads", "background_agent_cancellation_and_resume", "cancellation", "cold_resume", "permissions", "grandchildren", "worktrees", "implicit_project_discovery"]}
    result.update(probe(args, source))
    with args.receipt.open("x") as output:
        json.dump(result, output, indent=2); output.write("\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
