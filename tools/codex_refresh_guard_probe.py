#!/usr/bin/env python3
"""Check offline that a refresh-free Codex usage read can reach no refresh endpoint.

The Codex app-server redeems the refresh token of an expired access token on its
own (`proactive_expiry`), whatever `account/read` asks. A named portable
account's usage read (`token_refresh: false`) therefore points the app-server's
`CODEX_REFRESH_TOKEN_URL_OVERRIDE` at a port nothing listens on
(`plugins/codex/collector/index.js`, `REFRESH_REFUSED_URL`). Re-run this for every
native Codex pin change.

With a fake ChatGPT login whose access token has expired, the collector's two
calls (`account/read` without refresh, then `account/rateLimits/read`) run twice:
once with the override at a loopback fake token endpoint, which must receive a
refresh (the override is honored and the app-server still refreshes on its own),
and once with the collector's refusal URL, which must leave the fake endpoint
untouched and `auth.json` unchanged. No credential or model request is used.

Run in a loopback-only network namespace from the repository root:

    unshare -rn bash -c 'ip link set lo up; exec python3 tools/codex_refresh_guard_probe.py "$@"' x \\
      --codex /absolute/pinned/codex
"""
import argparse
import base64
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


def refused_url():
    source = Path(__file__).resolve().parent.parent / "plugins/codex/collector/index.js"
    match = re.search(r'export const REFRESH_REFUSED_URL = "([^"]+)";', source.read_text())
    if not match:
        raise SystemExit("REFRESH_REFUSED_URL not found in the Codex collector")
    return match.group(1)


class TokenEndpoint(http.server.BaseHTTPRequestHandler):
    hits = 0

    def answer(self):
        length = int(self.headers.get("content-length") or 0)
        self.rfile.read(length)
        TokenEndpoint.hits += 1
        body = b'{"error":"invalid_grant"}'
        self.send_response(401)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = answer

    def log_message(self, *args):
        pass


def token(claims):
    encode = lambda value: base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")
    return encode({"alg": "none"}) + "." + encode(claims) + ".fixture"


def read(codex, override):
    TokenEndpoint.hits = 0
    with tempfile.TemporaryDirectory(prefix="cowboy-codex-refresh-") as temporary:
        root = Path(temporary)
        home = root / "home" / ".codex"
        home.mkdir(parents=True)
        expired = int(time.time()) - 3600
        claims = {"chatgpt_account_id": "account-fixture", "chatgpt_plan_type": "pro",
                  "chatgpt_user_id": "user-fixture"}
        auth = home / "auth.json"
        auth.write_text(json.dumps({"auth_mode": "chatgpt", "OPENAI_API_KEY": None, "tokens": {
            "id_token": token({"email": "fixture@example.invalid", "exp": expired,
                               "https://api.openai.com/auth": claims}),
            "access_token": token({"exp": expired, "https://api.openai.com/auth": claims}),
            "refresh_token": "fixture-refresh", "account_id": "account-fixture"},
            "last_refresh": "2026-10-01T00:00:00Z"}))
        before = hashlib.sha256(auth.read_bytes()).hexdigest()
        environment = {"HOME": str(root / "home"), "CODEX_HOME": str(home), "PATH": "/usr/bin:/bin",
                       "TMPDIR": str(root), "CODEX_REFRESH_TOKEN_URL_OVERRIDE": override}
        # The collector's own launch (see JsonRpcLines.start).
        child = subprocess.Popen([codex, "app-server", "--stdio", "-c", "features.memories=false",
                                  "-c", "analytics.enabled=false"], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=environment, cwd=root)
        replies, pending = {}, b""

        def send(message):
            child.stdin.write((json.dumps(message) + "\n").encode())
            child.stdin.flush()

        def wait(request_id, timeout=40):
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
                    if message.get("id") is not None:
                        replies[message["id"]] = message
            return replies.get(request_id)

        try:
            send({"id": 1, "method": "initialize", "params": {
                "clientInfo": {"name": "cowboy-usage", "title": "Cowboy", "version": "1"},
                "capabilities": {"experimentalApi": True}}})
            if wait(1) is None:
                raise SystemExit("the app-server did not answer initialize")
            send({"method": "initialized", "params": {}})
            send({"id": 2, "method": "account/read", "params": {"refreshToken": False}})
            wait(2)
            send({"id": 3, "method": "account/rateLimits/read", "params": {}})
            wait(3)
        finally:
            child.kill()
            child.wait()
        return {"token_requests": TokenEndpoint.hits,
                "auth_json_changed": hashlib.sha256(auth.read_bytes()).hexdigest() != before}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--codex", type=Path, required=True)
    args = parser.parse_args()
    if not args.codex.is_absolute():
        parser.error("--codex must be absolute")
    try:
        socket.create_connection(("1.1.1.1", 443), timeout=2).close()
        parser.error("run in a loopback-only network namespace (see the module documentation)")
    except OSError:
        pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), TokenEndpoint)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    refreshable = read(str(args.codex), f"http://127.0.0.1:{server.server_port}/oauth/token")
    refresh_free = read(str(args.codex), refused_url())
    receipt = {"codex": subprocess.run([str(args.codex), "--version"], capture_output=True, text=True,
                                       env={"PATH": "/usr/bin:/bin", "HOME": tempfile.gettempdir()},
                                       timeout=60).stdout.strip(),
               "refreshable": refreshable, "refresh_free": refresh_free}
    accepted = refreshable["token_requests"] > 0 and refresh_free == {"token_requests": 0, "auth_json_changed": False}
    print(json.dumps({**receipt, "accepted": accepted}))
    return 0 if accepted else 1


if __name__ == "__main__":
    sys.exit(main())
