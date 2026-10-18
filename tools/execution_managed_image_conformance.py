#!/usr/bin/env python3
"""Offline native image output through the exact managed ACP package.

The Responses endpoint is loopback and scripted. No real account or model
request is used. Run inside a disposable network/PID namespace, as the other
native execution gates do. This verifies transport and sandbox contracts; it
does not claim live image-service availability or measure billed token usage.
"""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import socket
import ssl
import struct
import subprocess
import tempfile
import zlib

from execution_environment_codex_turn_probe import Api, Handler, final
from execution_environment_probe import Executor, require
from plugin_runtime_conformance import closed_environment


class Acp(Executor):
    def send(self, frame):
        super().send({"jsonrpc": "2.0", **frame})

    def __init__(self, *args, **kwargs):
        self.updates = []
        super().__init__(*args, **kwargs)

    def frame(self, deadline):
        value = super().frame(deadline)
        if "error" in value:
            print("Isolated ACP error:", json.dumps(value["error"])[:1600], flush=True)
        if value.get("method") == "session/update":
            self.updates.append(value["params"]["update"])
        return value


class ImagesHandler(Handler):
    def do_GET(self):
        if self.path.split("?")[0].endswith("/wham/accounts/check"):
            value = {"accounts": [{"id": "offline-fixture", "workspace_backend_origin": self.server.origin,
                                  "account_routing_override": "NO_CONSTRAINT"}]}
        elif self.path.endswith("/wham/config/bundle"):
            value = {}
        else:
            self.send_error(404)
            return
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        self.path = self.path.replace("/backend-api/codex/", "/v1/")
        if self.path == "/v1/responses":
            return super().do_POST()
        if self.path not in ("/v1/images/generations", "/v1/images/edits"):
            self.send_error(404)
            return
        size = int(self.headers.get("Content-Length", 0))
        require(0 < size < 1024 * 1024, "image request exceeds fixture bound")
        self.rfile.read(size)
        self.server.images.append(self.path)
        body = json.dumps({"created": 1, "data": [{"b64_json": self.server.image}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def pixel():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\x00\x12\x34\x56\xff")) + chunk(b"IEND", b"")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["native-cli", "launcher", "receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ["descriptor", "runtime", "target"]:
        parser.add_argument("--" + name, type=Path)
    args = parser.parse_args()
    require([name for _, name in socket.if_nameindex()] == ["lo"], "loopback namespace required")
    require(args.receipt.is_absolute() and not args.receipt.exists(), "new absolute receipt required")
    raw = pixel()
    encoded = base64.b64encode(raw).decode()
    def image_call(identity, edit=False):
        arguments = {"prompt": "A fixture pixel", "transparent_background": False}
        if edit:
            arguments["num_last_images_to_include"] = 1
        elif args.target:
            arguments["referenced_image_paths"] = [str(args.target / "input.png")]
        return {"type": "custom_tool_call", "namespace": "functions", "name": "exec",
                "call_id": identity, "input": "generatedImage(await tools.image_gen__imagegen(" + json.dumps(arguments) + "));"}
    api = Api([image_call("fixture-image"), final("generated"), image_call("fixture-edit", True), final("edited")])
    api.RequestHandlerClass = ImagesHandler
    api.image = encoded
    api.images = []
    api.origin = f"https://127.0.0.1:{api.server_port}"
    client = None
    temporary = tempfile.TemporaryDirectory(prefix="cw-image-")
    try:
        root = Path(temporary.name)
        certificate, key = root / "certificate.pem", root / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                        "-addext", "basicConstraints=critical,CA:FALSE",
                        "-keyout", str(key), "-out", str(certificate)], check=True, capture_output=True)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(certificate, key)
        api.socket = tls.wrap_socket(api.socket, server_side=True)
        workspace = root / "workspace"
        workspace.mkdir()
        if args.runtime:
            workspace = args.runtime
        environment = closed_environment(root / "home")
        home = Path(environment["CODEX_HOME"])
        home.mkdir()
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "fixture"\n'
            f'chatgpt_base_url = "{api.origin}/backend-api"\n'
            '[model_providers.fixture]\nname = "Offline fixture"\nwire_api = "responses"\n'
            f'base_url = "{api.origin}/v1"\nrequires_openai_auth = true\nsupports_websockets = false\n'
            '[features]\nremote_control = false\nshell_snapshot = false\n'
        )
        claims = {"email": "fixture@example.invalid", "https://api.openai.com/auth": {
            "chatgpt_account_id": "offline-fixture", "chatgpt_plan_type": "pro"}}
        token = "eyJhbGciOiJub25lIn0." + base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=") + ".fixture"
        (home / "auth.json").write_text(json.dumps({"auth_mode": "chatgpt", "tokens": {
            "id_token": token, "access_token": token, "refresh_token": "offline-fixture", "account_id": "offline-fixture"},
            "last_refresh": datetime.now(timezone.utc).isoformat()}))
        environment.update({"CODEX_PATH": str(args.native_cli), "DEFAULT_AUTH_REQUEST": '{"methodId":"chat-gpt"}',
                            "SSL_CERT_FILE": str(certificate), "CODEX_CA_CERTIFICATE": str(certificate),
                            "APP_SERVER_LOGS": str(root / "logs"), "RUST_LOG": "info"})
        if args.descriptor:
            environment["COWBOY_EXECUTION_DESCRIPTOR"] = str(args.descriptor)
        manifest = json.loads(Path("plugins/codex/provider.json").read_text())
        profile = manifest["runtime"]["managed_profiles"]["image_v1"]["arguments"]
        preflight = subprocess.run([str(args.native_cli), *profile[1:], "mcp", "list", "--json"],
                                   env=environment, cwd=workspace, capture_output=True, text=True, timeout=30)
        require(preflight.returncode == 0, "isolated native profile rejected: " + preflight.stderr[:1200])
        command = [shutil.which("node"), str(args.launcher), *profile]
        client = Acp(command, 60, environment=environment, cwd=workspace)
        client.request("initialize", {"protocolVersion": 1, "clientCapabilities": {},
            "clientInfo": {"name": "cowboy-managed-image-fixture", "version": "1"}})
        session = client.request("session/new", {"cwd": str(workspace), "mcpServers": []})["sessionId"]
        client.request("session/set_mode", {"sessionId": session, "modeId": "agent-full-access"}, expect_error=True)
        turn_updates = []
        for index in range(2):
            if index == 1:
                client.close()
                client = Acp(command, 60, environment=environment, cwd=workspace)
                client.request("initialize", {"protocolVersion": 1, "clientCapabilities": {},
                    "clientInfo": {"name": "cowboy-managed-image-fixture", "version": "1"}})
                client.request("session/load", {"sessionId": session, "cwd": str(workspace), "mcpServers": []})
                client.request("session/set_mode", {"sessionId": session, "modeId": "agent-full-access"}, expect_error=True)
                require(len(api.requests) == 2, "cold resume consumed a model request")
                client.updates.clear()
            result = client.request("session/prompt", {"sessionId": session,
                "prompt": [{"type": "text", "text": "Generate the fixture image." if index == 0 else "Edit the previous image."}],
                "_meta": {"cowboy.dev/managedCall": {"callId": f"call-{index}", "inputRevision": "fixture", "outputSchema": None}}})
            require(result.get("stopReason") == "end_turn", "image turn failed")
            turn_updates.extend(client.updates)
        generated = [v for v in turn_updates if (v.get("rawOutput") or {}).get("cowboyGeneratedImage")]
        if len(generated) != 2:
            print("Image fixture updates:", [(v.get("sessionUpdate"), v.get("status"), list((v.get("rawOutput") or {}).keys())) for v in client.updates], flush=True)
            print("Image fixture outputs:", [[item for item in v.get("input", []) if item.get("type") in ("function_call_output", "custom_tool_call_output")] for v in api.requests], flush=True)
            for block in api.requests[0].get("input", []):
                for tool in block.get("tools", []):
                    if "image" in tool.get("name", ""):
                        print("Image native tool:", json.dumps(tool)[:8000], flush=True)
        require(len(generated) == 2, "native image events missing")
        for update in generated:
            require(update.get("status") == "completed", "native image not completed")
            require("result" not in update["rawOutput"], "duplicate inline image bytes")
            images = [v["content"] for v in update.get("content", []) if v.get("type") == "content" and v.get("content", {}).get("type") == "image"]
            require(len(images) == 1 and base64.b64decode(images[0]["data"]) == raw, "image bytes changed")
        require(len(api.requests) == 4 and api.failure is None, "unexpected native requests")
        require(api.images == ["/v1/images/edits" if args.target else "/v1/images/generations", "/v1/images/edits"], "native generate/edit requests missing")
        client.request("authenticate", {"methodId":"api-key", "_meta":{"api-key":{"apiKey":"offline-fixture"}}})
        client.request("session/prompt", {"sessionId":session,
            "prompt":[{"type":"text","text":"Generate another image."}],
            "_meta":{"cowboy.dev/managedCall":{"callId":"denied","inputRevision":"fixture","outputSchema":None}}}, expect_error=True)
        require(len(api.requests) == 4, "API-key refusal consumed a model request")
        definitions = [tool for block in api.requests[0].get("input", []) for tool in block.get("tools", [])]
        def names(value):
            if isinstance(value, dict):
                return ([value["name"]] if isinstance(value.get("name"), str) else []) + [name for item in value.values() for name in names(item)]
            if isinstance(value, list):
                return [name for item in value for name in names(item)]
            return []
        tool_names = names(definitions)
        args.receipt.write_text(json.dumps({"schema": 1, "kind": "offline_native_image",
            "native_sha256": hashlib.sha256(args.native_cli.read_bytes()).hexdigest(),
            "adapter_sha256": {name: hashlib.sha256((args.launcher.parent / name).read_bytes()).hexdigest()
                               for name in [args.launcher.name, "node_modules/@agentclientprotocol/codex-acp/dist/index.js", "cowboy-execution.mjs"]},
            "image_sha256": hashlib.sha256(raw).hexdigest(), "model_requests": 4,
            "request_bytes": [len(json.dumps(r).encode()) for r in api.requests],
            "tool_names": tool_names,
            "native_images": generated if args.target else [],
            "checks": ["native_image_availability", "managed_mode_cannot_widen", "native_image_acp_bytes",
                       "continued_native_image_turn", "cold_resume_without_model_request", "single_image_payload", "api_key_refused_without_model_request", "no_live_provider_traffic"]}, indent=2) + "\n")
        client.close()
        client = None
    finally:
        if client is not None:
            client.close()
        api.close()
        log = Path(temporary.name) / "logs" / "app-server.log"
        if log.exists() and not args.receipt.exists():
            print("\n".join(line[:800] for line in log.read_text().splitlines() if "[ERR]" in line and "failed" in line.lower())[-2500:], flush=True)
        temporary.cleanup()


if __name__ == "__main__":
    main()
