"""Exact packaged native clients against a disposable Matrix and scripted models.

The caller supplies the independent Matrix source explicitly. No production
configuration, credentials, history, memories or network endpoints are used.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

from execution_environment_probe import Executor, require
from execution_environment_codex_turn_probe import Api, final
from execution_environment_claude_probe import Claude, ScriptedApi, WorkspaceFixture, tool
from execution_worker_conformance import complete_turn
from plugin_runtime_conformance import closed_environment

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--matrix-source", type=Path, required=True)
parser.add_argument("--native-root", type=Path, required=True)
parser.add_argument("--receipt", type=Path, required=True)
parser.add_argument("--code-mode", action="store_true", help="Exercise Matrix's bounded Python retrieval through both native clients")
parser.add_argument("--codeact", action="store_true", help="Exercise TS, JS, shell and transactional writes through both native clients")
parser.add_argument("--runtime-config", type=Path)
args = parser.parse_args()
require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
require(not args.receipt.exists(), "new receipt required")
sys.path.insert(0, str(args.matrix_source.resolve() / "src"))
from matrix_memory.cli import initialize
from matrix_memory.model import Principal
from matrix_memory.service import Application, Server
from matrix_memory.store import Store, load

checks = []
with tempfile.TemporaryDirectory(prefix="matrix-native-") as directory:
    root = Path(directory)
    runtime = root / "workspace"
    runtime.mkdir()
    instance = root / "instance"
    initialize(instance, "fixture", ["fixture"], 7331)
    store = Store(instance)
    access = load(instance / ".matrix/access.json")
    if args.codeact:
        require(args.runtime_config is not None, "CodeAct requires an exact contained runtime")
        (instance / ".matrix/runtime.json").write_bytes(args.runtime_config.read_bytes())
    server = Server(("127.0.0.1", 0), Application(store, access))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    clients = {c["provider"]: c for c in access["clients"]}
    binding = Principal.from_config(clients["codex"]).bind({"project": "fixture", "machine": "fixture-host", "session": "source"})
    source = store.observe(binding, {"turn": "seed", "learn": False, "events": [{"id": "u1", "role": "user", "text": "The release protocol requires receipt marker MATRIX-SHARED-619."}]})
    note = {"scope": "project", "topic": "release-marker", "kind": "fact", "title": "Release protocol", "body": "The release protocol requires receipt marker MATRIX-SHARED-619.", "sources": [{"event": source["events"][0], "quote": "MATRIX-SHARED-619"}]}
    receipt = store.put(binding, {**note, "operation": "seed", "expected_revision": 0})
    memory_tool = "memory_search" if args.code_mode else "memory_get"
    memory_arguments = {"code": "for m in search('release')['results'][:1]: emit({'id':m['id'],'body':get(m['id'])['body']})"} if args.code_mode else {"id": receipt["id"]}
    if args.codeact:
        memory_tool = "memory_execute"
        memory_arguments = {"operation": "native-codeact", "code": "const s = await memory.search({query:'release'}); const r = await memory.read({refs:s.hits.map(h=>h.ref),snapshot:s.snapshot}); await memory.put(" + json.dumps({**note, "topic": "codeact-proof", "expected_revision": 0}) + "); return await shell.exec({command:'jq',args:['-r','.items[0].record.body'],stdin:JSON.stringify(r)});"}
    def config(provider):
        path = root / (provider + "-matrix.json")
        path.write_text(json.dumps({"schema": 1, "provider": provider, "endpoint": f"http://127.0.0.1:{server.server_port}", "token": clients[provider]["token"], "state_dir": str(root / "delivery"), "projects": [{"path": str(runtime), "machine": "fixture-host", "project": "fixture"}]}))
        path.chmod(0o600)
        return str(path)

    native = args.native_root.resolve()
    for provider, adapter in [("codex", "codex-adapter/bin/codex-acp"), ("claude", "claude-adapter/bin/claude-agent-acp")]:
        environment = closed_environment(root / (provider + "-probe-home"))
        environment["COWBOY_MATRIX_CONFIG"] = config(provider)
        result = subprocess.run([str(native / adapter), "--version"], env=environment, cwd=root,
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        require(result.returncode == 0, "Artifact version probe requires session bindings when Matrix is enabled")
    checks.append("enrolled_artifact_probes_need_no_native_session_binding")
    codex = native / "codex/package/vendor/x86_64-unknown-linux-musl/bin/codex"
    launcher = native / "codex-adapter/app/cowboy-launch.mjs"
    api = Api([{"type": "custom_tool_call", "call_id": "memory-read", "name": "exec", "namespace": "functions", "input": "text(await tools.mcp__matrix__" + memory_tool + "(" + json.dumps(memory_arguments) + "));"}, final("memory-done"), final("corrected"), final("forgotten")])
    environment = closed_environment(root / "codex-home")
    environment.update({"COWBOY_OFFLINE_FIXTURE_KEY": "fake-fixture", "COWBOY_PRIVATE_CODEX_EXECUTABLE": str(codex), "COWBOY_PRIVATE_CODEX_ARGUMENTS": "[]", "COWBOY_MATRIX_CONFIG": config("codex")})
    home = Path(environment["CODEX_HOME"])
    home.mkdir(parents=True)
    (home / "config.toml").write_text('model = "gpt-6-astra"\nmodel_provider = "fixture"\n[model_providers.fixture]\nname = "Fixture"\n' + f'base_url = "http://127.0.0.1:{api.server_port}/v1"\nwire_api = "responses"\nenv_key = "COWBOY_OFFLINE_FIXTURE_KEY"\nrequest_max_retries = 0\nstream_max_retries = 0\n')
    client = Executor([shutil.which("node"), str(launcher), "--cowboy-private-cli", "app-server"], 60, environment=environment, cwd=runtime)
    try:
        client.request("initialize", {"clientInfo": {"name": "matrix-acceptance", "version": "1"}})
        client.send({"method": "initialized", "params": {}})
        thread = client.request("thread/start", {"cwd": str(runtime), "approvalPolicy": "never", "sandbox": "danger-full-access"})["thread"]["id"]
        status = client.request("mcpServerStatus/list", {})
        require(any(server["name"] == "matrix" and len(server["tools"]) == 7 for server in status["data"]), "Native Matrix tools missing")
        complete_turn(client, thread, "Find the release protocol marker.")
        encoded = json.dumps(api.requests)
        require("MATRIX-SHARED-619" in json.dumps(api.requests[0]), "Codex automatic recall missing")
        require("mcp__matrix__" + memory_tool in encoded and "MATRIX-SHARED-619" in encoded, "Codex MCP/recall missing")
        require(any("MATRIX-SHARED-619" in str(item.get("output")) for request in api.requests for item in request.get("input", []) if item.get("type") in ("function_call_output", "custom_tool_call_output")), "Codex MCP did not return Matrix data")
        checks.append("packaged_codex_native_http_mcp_and_recall")
        if args.codeact:
            require(store.sequence == 2, "Codex CodeAct mutation was not committed exactly once")
            checks.append("packaged_codex_ts_compile_batched_read_lazy_shell_and_atomic_commit")
        # Automatic capture includes the actual public final message, no helper history scan.
        deadline = time.monotonic() + 3
        while not store.status()["jobs"] and time.monotonic() < deadline:
            time.sleep(0.05)
        require(bool(store.status()["jobs"]), "native turn was not queued")
        checks.append("packaged_codex_completed_turn_capture")
    finally:
        client.close()
        api.close()

    if args.codeact:
        memory_arguments = {**memory_arguments, "language": "js", "operation": "claude-native-codeact", "code": memory_arguments["code"].replace('"topic": "codeact-proof"', '"topic": "claude-codeact-proof"')}
    api = ScriptedApi([tool("mcp__matrix__" + memory_tool, memory_arguments)])
    environment = closed_environment(root / "claude-home")
    environment.update({"ANTHROPIC_BASE_URL": f"http://127.0.0.1:{api.server_port}", "ANTHROPIC_API_KEY": "fake-fixture", "COWBOY_PRIVATE_CLAUDE_EXECUTABLE": str(native / "claude/package/claude"), "COWBOY_MATRIX_CONFIG": config("claude"), "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"})
    client = Claude(str(native / "claude-adapter/bin/cowboy-configured-cli"), environment, runtime, WorkspaceFixture(runtime), aliases=False, bound_native=True)
    try:
        client.ready()
        result = client.prompt(text="Find the release protocol marker.")
        require(not result.get("is_error"), "Claude native turn failed")
        require("MATRIX-SHARED-619" in json.dumps(api.requests[0]), "Claude automatic recall missing")
        encoded = json.dumps(api.requests)
        require("mcp__matrix__" + memory_tool in encoded and "MATRIX-SHARED-619" in encoded, "Claude MCP/recall missing")
        outputs = [b for r in api.requests for m in r.get("messages", []) for b in (m.get("content") if isinstance(m.get("content"), list) else []) if b.get("type") == "tool_result"]
        require(any("MATRIX-SHARED-619" in json.dumps(b) for b in outputs), "Claude MCP did not return Matrix data")
        checks.append("packaged_claude_native_http_mcp_and_shared_recall")
        if args.codeact:
            require(store.sequence == 3, "Claude CodeAct mutation was not committed exactly once")
            checks.append("packaged_claude_js_batched_read_lazy_shell_and_atomic_commit")
    finally:
        client.close()
        api.close()
        server.shutdown()
        server.server_close()

report = {"schema": "cowboy.matrix-native-conformance/v1", "accepted": True, "retrieval_mode": "matrix-ts-v1" if args.codeact else "matrix-python-v1" if args.code_mode else "memory_get", "checks": checks, "not_checked": ["remote execution placement", "live model quality", "native-memory baseline"]}
args.receipt.parent.mkdir(parents=True, exist_ok=True)
args.receipt.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
