#!/usr/bin/env python3
"""Capture the pinned Grok CLI's `_x.ai/billing` request and projection offline.

The Grok collector reads billing for a named portable account without starting
the CLI (`plugins/grok/collector/index.js`, `billingWithoutCli`). Its request
and `billingProjection` reproduce what the pinned CLI does, case for case, as
recorded in `plugins/grok-billing-cli.fixture.json`. Re-run this for every Grok
pin change and commit the rewritten fixture with the new pin.

For each fixture case, a fresh CLI answers ACP `_x.ai/billing` while a loopback
fake chat proxy (`GROK_CLI_CHAT_PROXY_BASE_URL`) serves only the billing route
with the case's body; then a 401 shows the CLI's unauthorized answer and that
it does not rewrite `auth.json`. The only credential is a fake grok.com login in
the layout `grok login` writes (default issuer::client scope with its
`oidc_issuer`/`oidc_client_id`). No model request is made.

Run in a loopback-only network namespace from the repository root:

    unshare -rn bash -c 'ip link set lo up; exec python3 tools/grok_billing_capture.py "$@"' x \\
      --grok /absolute/pinned/grok --fixture plugins/grok-billing-cli.fixture.json \\
      --write plugins/grok-billing-cli.fixture.json

`--write` may name the fixture itself; it is written only after every case
was answered. The command exits nonzero when a case's projection differs from
the fixture's, so a behavior change is reviewed rather than silently recorded
(pass `--accept-changes` to record it).
"""
import argparse
import base64
import datetime
import hashlib
import http.server
import json
import os
import re
import select
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

# The default grok.com OAuth2 login scope and client the CLI writes.
LOGIN_ISSUER = "https://auth.x.ai"
LOGIN_CLIENT = "b1a00492-073a-47ea-816f-4c329264a828"
FIXTURE_TOKEN = "Bearer <fixture access token>"
ARGUMENTS = [
    "--no-auto-update",
    "--experimental-memory",
    "--rules",
    "Read and follow the closest AGENTS.md project instructions before taking any action.",
    "agent",
    "--always-approve",
    "--no-leader",
    "stdio",
]


class Proxy(http.server.BaseHTTPRequestHandler):
    status = 200
    body = b"{}"
    requests = []

    def answer(self):
        length = int(self.headers.get("content-length") or 0)
        data = self.rfile.read(length) if length else b""
        Proxy.requests.append({
            "method": self.command,
            "path": self.path,
            "headers": {name.lower(): value for name, value in self.headers.items()},
            "body": data.decode("utf8", "replace"),
        })
        # Only the billing route answers, as for a proxy that serves nothing else.
        billing = self.path.startswith("/v1/billing")
        status, body = (Proxy.status, Proxy.body) if billing else (404, b"{}")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = answer

    def log_message(self, *args):
        pass


def token(claims):
    encode = lambda value: base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")
    return encode({"alg": "none", "typ": "JWT"}) + "." + encode(claims) + ".fixture"


def isolated():
    """Only loopback may be reachable: no credential or request leaves the host."""
    try:
        socket.create_connection(("1.1.1.1", 443), timeout=2).close()
    except OSError:
        return True
    return False


def call_billing(grok, environment, root):
    child = subprocess.Popen([grok, *ARGUMENTS], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, env=environment, cwd=root)
    replies, pending = {}, b""

    def send(message):
        child.stdin.write((json.dumps({"jsonrpc": "2.0", **message}) + "\n").encode())
        child.stdin.flush()

    def wait(request_id, timeout=60):
        nonlocal pending
        deadline = time.time() + timeout
        while time.time() < deadline and request_id not in replies:
            ready, _, _ = select.select([child.stdout], [], [], 0.5)
            if not ready:
                continue
            chunk = os.read(child.stdout.fileno(), 1 << 16)
            if not chunk:
                break
            pending += chunk
            *lines, pending = pending.split(b"\n")
            for line in lines:
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                if message.get("id") is not None and ("result" in message or "error" in message):
                    replies[message["id"]] = message
        return replies.get(request_id)

    try:
        send({"id": 1, "method": "initialize", "params": {
            "protocolVersion": 1,
            "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
            "clientInfo": {"name": "cowboy-usage", "title": "Cowboy", "version": "1"}}})
        if wait(1) is None:
            raise SystemExit("the CLI did not answer initialize")
        send({"id": 2, "method": "_x.ai/billing", "params": {}})
        reply = wait(2)
    finally:
        child.kill()
        child.wait()
    if reply is None:
        raise SystemExit("the CLI did not answer _x.ai/billing")
    return {"result": reply["result"]} if "result" in reply else {"error": reply["error"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--write", type=Path, required=True)
    parser.add_argument("--accept-changes", action="store_true")
    args = parser.parse_args()
    if not args.grok.is_absolute():
        parser.error("--grok must be absolute")
    if not isolated():
        parser.error("run in a loopback-only network namespace (see the module documentation)")
    fixture = json.loads(args.fixture.read_text())

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="cowboy-grok-billing-") as temporary:
        root = Path(temporary)
        home = root / "home"
        (home / ".grok").mkdir(parents=True)
        (root / "tmp").mkdir()
        expiry = int(time.time()) + 30 * 86400
        access = token({"sub": "user-fixture", "exp": expiry})
        stamp = datetime.datetime.fromtimestamp(expiry, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000000000Z")
        auth = home / ".grok" / "auth.json"
        auth.write_text(json.dumps({f"{LOGIN_ISSUER}::{LOGIN_CLIENT}": {
            "key": access, "auth_mode": "oidc", "create_time": "2026-11-22T00:00:00.000000000Z",
            "user_id": "user-fixture", "email": "fixture@example.invalid", "coding_data_retention_opt_out": True,
            "refresh_token": "fixture-refresh", "expires_at": stamp,
            "oidc_issuer": LOGIN_ISSUER, "oidc_client_id": LOGIN_CLIENT}}))
        environment = {"HOME": str(home), "PATH": "/usr/bin:/bin", "TMPDIR": str(root / "tmp"), "LANG": "C.UTF-8",
                       "GROK_FOLDER_TRUST": "0",
                       "GROK_CLI_CHAT_PROXY_BASE_URL": f"http://127.0.0.1:{server.server_port}/v1"}
        version = subprocess.run([str(args.grok), "--version"], env=environment, capture_output=True,
                                 text=True, timeout=60).stdout.strip()
        match = re.fullmatch(r"grok (\d+\.\d+\.\d+) \([0-9a-f]+\)", version)
        if not match:
            raise SystemExit(f"unexpected --version output: {version[:80]}")
        cli_version = match.group(1)

        cases, changed, request = [], [], None
        for case in fixture["cases"]:
            Proxy.status, Proxy.body, Proxy.requests = 200, json.dumps(case["body"]).encode(), []
            answer = call_billing(str(args.grok), environment, root)
            billing = [entry for entry in Proxy.requests if entry["path"].startswith("/v1/billing")]
            if len(billing) != 1:
                raise SystemExit(f"{case['name']}: expected one billing request, got {len(billing)}")
            if request is None:
                headers = dict(billing[0]["headers"])
                headers.pop("host", None)
                if headers.get("authorization") != f"Bearer {access}":
                    raise SystemExit("the billing request does not carry the stored access token")
                headers["authorization"] = FIXTURE_TOKEN
                request = {"method": billing[0]["method"], "path": billing[0]["path"],
                           "base_url": "https://cli-chat-proxy.grok.com/v1", "headers": headers,
                           "body": billing[0]["body"]}
            if answer != case["cli"]:
                changed.append(case["name"])
            cases.append({**case, "cli": answer})

        before = hashlib.sha256(auth.read_bytes()).hexdigest()
        Proxy.status, Proxy.body, Proxy.requests = 401, b'{"error":"unauthorized"}', []
        unauthorized = call_billing(str(args.grok), environment, root)
        attempts = len([entry for entry in Proxy.requests if entry["path"].startswith("/v1/billing")])
        rewritten = hashlib.sha256(auth.read_bytes()).hexdigest() != before

    if changed and not args.accept_changes:
        raise SystemExit(f"projection changed for: {', '.join(changed)} (review, then pass --accept-changes)")
    description = re.sub(r"Grok CLI \d+\.\d+\.\d+", f"Grok CLI {cli_version}", fixture["description"])
    output = {**fixture, "description": description, "cli_version": cli_version, "request": request,
              "unauthorized": {"server_status": 401, "cli": unauthorized, "billing_requests": attempts,
                               "auth_json_rewritten": rewritten},
              "cases": cases}
    args.write.write_text(json.dumps(output, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({"cli_version": cli_version, "cases": len(cases), "changed": changed,
                      "unauthorized_requests": attempts, "auth_json_rewritten": rewritten}))


if __name__ == "__main__":
    sys.exit(main())
