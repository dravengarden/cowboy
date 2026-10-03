"""Isolated HTTPS + WebCrypto + Rust + WSS conformance. Invoked by just only."""
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import ssl
import subprocess
import sys
import tempfile
import threading
import time


def wait_for(operation, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        try:
            return operation()
        except (OSError, ValueError):
            if time.monotonic() > deadline:
                raise
            time.sleep(0.05)


browser, executable = sys.argv[1:]
assert browser.startswith("/nix/store/") and browser.endswith("/bin/firefox")
assert Path(executable).is_file()
root = Path(__file__).resolve().parent.parent
children = []
with tempfile.TemporaryDirectory(prefix="cowboy-device-conformance-") as tmp:
    temporary = Path(tmp)
    report = []
    completed = threading.Event()
    backend = None
    subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                    "-keyout", f"{tmp}/key.pem", "-out", f"{tmp}/cert.pem", "-days", "1",
                    "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1"],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(f"{tmp}/cert.pem", f"{tmp}/key.pem")

    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                self.forward()
            except (OSError, ssl.SSLError):
                pass  # Browser closes speculative connections and failed upgrades.

        def forward(self):
            global backend
            with tls.wrap_socket(self.request, server_side=True) as client:
                head = b""
                while b"\r\n\r\n" not in head:
                    part = client.recv(4096)
                    if not part:
                        return
                    head += part
                    assert len(head) <= 65536
                headers, pending = head.split(b"\r\n\r\n", 1)
                lines = headers.split(b"\r\n")
                method, target, _ = lines[0].decode().split(" ")
                path = target.split("?", 1)[0]
                fields = dict(line.lower().split(b":", 1) for line in lines[1:] if b":" in line)
                body = None
                mime = "text/javascript"
                if path in ("/fixture", "/peer"):
                    body = b'<!doctype html><body><script src="/device-proof.js"></script>'
                    if path == "/fixture":
                        body += b'<script src="/fixture.js"></script>'
                    mime = "text/html"
                elif path == "/device-proof.js":
                    body = (root / "web/device-proof.js").read_bytes()
                elif path == "/fixture.js":
                    body = (root / "tools/browser-device-fixture.js").read_bytes()
                elif path == "/fixture/restart" and method == "POST":
                    children[0].kill()
                    children[0].wait()
                    (temporary / "backend").unlink()
                    children[0] = subprocess.Popen(rust_command, env=env, stdout=rust_log, stderr=rust_log)
                    address = wait_for(lambda: (temporary / "backend").read_text())
                    host, port = address.split(":")
                    backend = (host, int(port))
                    body = b"restarted"
                elif path == "/report" and method == "POST":
                    size = int(fields.get(b"content-length", b"0"))
                    while len(pending) < size:
                        pending += client.recv(65536)
                    report.append(json.loads(pending[:size]))
                    completed.set()
                    body = b"ok"
                if body is not None:
                    client.sendall(f"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
                    return
                assert backend is not None
                upgrade = b"upgrade" in fields
                forwarded = [line for line in lines[1:] if not line.lower().startswith((b"x-forwarded-", b"connection:"))]
                forwarded += [b"X-Forwarded-Proto: https", b"Connection: Upgrade" if upgrade else b"Connection: close"]
                with socket.create_connection(backend) as upstream:
                    upstream.sendall(b"\r\n".join([lines[0], *forwarded]) + b"\r\n\r\n" + pending)
                    if upgrade:
                        response = b""
                        while b"\r\n\r\n" not in response:
                            chunk = upstream.recv(65536)
                            if not chunk:
                                return
                            response += chunk
                        client.sendall(response)
                        if not response.startswith(b"HTTP/1.1 101 "):
                            # A failed upgrade is ordinary HTTP; do not let the
                            # browser reuse this one-request fixture tunnel.
                            return
                    while True:
                        ready, _, _ = select.select([client, upstream], [], [], 15)
                        if not ready:
                            return
                        for source in ready:
                            data = source.recv(65536)
                            if not data:
                                return
                            (upstream if source is client else client).sendall(data)

    class Server(socketserver.ThreadingTCPServer):
        daemon_threads = True

    with Server(("127.0.0.1", 0), Handler) as server:
        origin = f"https://127.0.0.1:{server.server_address[1]}"
        (temporary / "origin").write_text(origin)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            rust_log = open(temporary / "rust.log", "w+")
            env = {"PATH": os.environ["PATH"], "COWBOY_DEVICE_FIXTURE": tmp}
            rust_command = [executable, "server::secure_transport::tests::browser_conformance_fixture", "--exact", "--ignored", "--nocapture"]
            children.append(subprocess.Popen(rust_command,
                                             env=env, stdout=rust_log, stderr=rust_log))
            address = wait_for(lambda: (temporary / "backend").read_text())
            host, port = address.split(":")
            backend = (host, int(port))
            # A genuine remote HTTP peer is tested by Rust separately. Here TLS
            # terminates in the fixture proxy, just as with production Caddy.
            profile = temporary / "profile"
            profile.mkdir()
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                marionette_port = reservation.getsockname()[1]
            (profile / "user.js").write_text(f'user_pref("marionette.port", {marionette_port});\n')
            children.append(subprocess.Popen([browser, "--headless", "--no-remote", "--new-instance", "--marionette", "--profile", str(profile), "about:blank"],
                                             env={**env, "MOZ_HEADLESS": "1", "MOZ_NO_REMOTE": "1", "XDG_RUNTIME_DIR": tmp,
                                                  "XDG_CACHE_HOME": f"{tmp}/cache", "XDG_CONFIG_HOME": f"{tmp}/config"},
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
            with wait_for(lambda: socket.create_connection(("127.0.0.1", marionette_port))) as control:
                control.settimeout(30)
                stream = control.makefile("rb")

                def receive():
                    length = b""
                    while not length.endswith(b":"):
                        part = stream.read(1)
                        if not part:
                            raise OSError("Marionette disconnected")
                        length += part
                    return json.loads(stream.read(int(length[:-1])))

                def command(number, name, data):
                    encoded = json.dumps([0, number, name, data]).encode()
                    control.sendall(str(len(encoded)).encode() + b":" + encoded)
                    result = receive()
                    assert result[2] is None, result
                    return result[3]

                receive()
                session = command(1, "WebDriver:NewSession", {"acceptInsecureCerts": True})
                command(2, "WebDriver:Navigate", {"url": origin + "/fixture"})
                if not completed.wait(45):
                    diagnostic = command(3, "WebDriver:ExecuteScript", {"script": "return {url:location.href,body:document.body.innerHTML,tests:window.fixtureTests,device:typeof CowboyDeviceProof}", "args": [], "newSandbox": False})
                    raise AssertionError(diagnostic)
                assert report and report[0].get("ok"), report
                print(json.dumps({**report[0], "browser": session.get("capabilities", {}).get("browserVersion"),
                                  "isolation": "private network namespace, disposable browser profile and Rust store, fixture-only TLS trust"}, indent=2))
        finally:
            for child in reversed(children):
                child.kill()
                child.wait()
            server.shutdown()
