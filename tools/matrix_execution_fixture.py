"""Optional independent Matrix fixture for the owned native execution gates."""
import atexit
import json
import os
from pathlib import Path
import sys
import threading


class MatrixFixture:
    def __init__(self, inputs, root, descriptor_path, provider, environment):
        self.enabled = bool(inputs.get("matrix_source"))
        if not self.enabled:
            return
        sys.path.insert(0, str(Path(inputs["matrix_source"]).resolve() / "src"))
        from matrix_memory.cli import initialize
        from matrix_memory.model import Principal
        from matrix_memory.service import Application, Server
        from matrix_memory.store import Store, load

        instance = root / "matrix-instance"
        initialize(instance, "fixture", ["fixture"], 7331)
        self.store = Store(instance)
        access = load(instance / ".matrix/access.json")
        self.codeact = bool(os.environ.get("MATRIX_TEST_RUNTIME"))
        if self.codeact:
            (instance / ".matrix/runtime.json").write_bytes(Path(os.environ["MATRIX_TEST_RUNTIME"]).read_bytes())
        self.server = Server(("127.0.0.1", 0), Application(self.store, access))
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        atexit.register(self.server.server_close)
        atexit.register(self.server.shutdown)
        descriptor = json.loads(descriptor_path.read_text())["binding"]
        client = next(c for c in access["clients"] if c["provider"] == provider)
        binding = Principal.from_config(client).bind({"project": "fixture", "machine": descriptor["environment"]["machine_id"], "session": "source-session"})
        text = "This fixture release requires memory marker MATRIX_REMOTE_PROOF."
        observation = self.store.observe(binding, {"turn": "seed", "learn": False, "events": [{"id": "source", "role": "user", "text": text}]})
        record = self.store.put(binding, {"operation": "seed", "scope": "project", "topic": "fixture-memory", "kind": "fact", "title": "Fixture memory", "body": text, "expected_revision": 0, "sources": [{"event": observation["events"][0], "quote": text}]})
        self.id = record["id"]
        self.tool = "memory_get"
        self.arguments = {"id": self.id}
        if self.codeact:
            self.tool = "memory_execute"
            payload = {"scope": "project", "topic": "codeact-proof", "kind": "fact", "title": "CodeAct proof", "body": text, "expected_revision": 0, "sources": [{"event": observation["events"][0], "quote": text}]}
            self.arguments = {"operation": "remote-native-codeact", "code": "const found=await memory.search({query:'MATRIX_REMOTE_PROOF'}); const read=await memory.read({refs:found.hits.map(h=>h.ref),snapshot:found.snapshot}); await memory.put(" + json.dumps(payload) + "); return await shell.exec({command:'jq',args:['-r','.items[0].record.body'],stdin:JSON.stringify(read)});"}
        path = root / "matrix-client.json"
        path.write_text(json.dumps({"schema": 1, "provider": provider, "endpoint": f"http://127.0.0.1:{self.server.server_port}", "token": client["token"], "state_dir": str(root / "matrix-delivery"), "projects": [{"workspace": descriptor["workspace"]["id"], "machine": descriptor["environment"]["machine_id"], "project": "fixture"}]}))
        path.chmod(0o600)
        environment["COWBOY_MATRIX_CONFIG"] = str(path)

    def accept(self, requests):
        if not self.enabled:
            return []
        from execution_environment_probe import require
        require(requests and "MATRIX_REMOTE_PROOF" in json.dumps(requests[0]),
                "Automatic Matrix recall missing before the first native tool call")
        require(bool(self.store.status()["jobs"]), "Native remote turn was not captured")
        checks = ["matrix_memory_uses_runtime_service_with_bound_target_scope", "matrix_remote_turns_are_durably_captured"]
        if self.codeact:
            require(self.store.sequence == 2, "Remote CodeAct batch was not committed once")
            require("MATRIX_REMOTE_PROOF" in json.dumps(requests[1:]), "Remote CodeAct returned no scoped data")
            checks.append("matrix_remote_native_codeact_reads_shell_and_atomic_commit")
        return checks
