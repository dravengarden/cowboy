"""Isolated HTTPS + WebCrypto + Rust + WSS conformance. Invoked by just only.

Arguments: pinned Firefox, Rust test binary, pinned NSS certutil.
Only a disposable browser profile trusts the fixture CA; TLS validation stays on.
"""
import hashlib
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


browser, executable, certutil = sys.argv[1:]
assert browser.startswith("/nix/store/") and browser.endswith("/bin/firefox")
assert certutil.startswith("/nix/store/") and certutil.endswith("/bin/certutil")
assert all(Path(path).is_file() for path in (browser, executable, certutil))
mapping = Path("/proc/self/uid_map").read_text().split()
assert len(mapping) == 3 and mapping[2] == "1", "private user namespace required"
assert sorted(name for _, name in socket.if_nameindex()) == ["lo"], "private network required"
root = Path(__file__).resolve().parent.parent
children = []
with tempfile.TemporaryDirectory(prefix="cowboy-device-conformance-") as tmp:
    temporary = Path(tmp)
    report = []
    completed = threading.Event()
    backend = None
    def openssl(*args):
        subprocess.run(["openssl", *args], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
            "-subj", "/CN=Cowboy device disposable fixture CA",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-keyout", f"{tmp}/ca.key", "-out", f"{tmp}/ca.pem")
    openssl("req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=127.0.0.1",
            "-keyout", f"{tmp}/key.pem", "-out", f"{tmp}/request.pem")
    # Deliberately exclude localhost so it is a real wrong-name negative case.
    (temporary / "extensions").write_text(
        "subjectAltName=IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\n"
        "extendedKeyUsage=serverAuth\n")
    openssl("x509", "-req", "-days", "1", "-in", f"{tmp}/request.pem",
            "-CA", f"{tmp}/ca.pem", "-CAkey", f"{tmp}/ca.key", "-CAcreateserial",
            "-extfile", f"{tmp}/extensions", "-out", f"{tmp}/cert.pem")
    openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
            "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
            "-keyout", f"{tmp}/untrusted.key", "-out", f"{tmp}/untrusted.pem")
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(f"{tmp}/cert.pem", f"{tmp}/key.pem")
    untrusted_tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    untrusted_tls.load_cert_chain(f"{tmp}/untrusted.pem", f"{tmp}/untrusted.key")
    tls_checks = []

    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                self.forward()
            except (OSError, ssl.SSLError):
                pass  # Browser closes speculative connections and failed upgrades.

        def forward(self):
            global backend
            with self.server.tls.wrap_socket(self.request, server_side=True) as client:
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
                with self.server.count_lock:
                    self.server.application_requests += 1
                    self.server.request_hosts.add(fields.get(b"host", b"").strip().decode("ascii"))
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

        def __init__(self, context):
            super().__init__(("127.0.0.1", 0), Handler)
            self.tls = context
            self.application_requests = 0
            self.request_hosts = set()
            self.count_lock = threading.Lock()

    with Server(tls) as server, Server(untrusted_tls) as untrusted_server:
        origin = f"https://127.0.0.1:{server.server_address[1]}"
        wrong_name = f"https://localhost:{server.server_address[1]}"
        untrusted_origin = f"https://127.0.0.1:{untrusted_server.server_address[1]}"
        (temporary / "origin").write_text(origin)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        threading.Thread(target=untrusted_server.serve_forever, daemon=True).start()
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
            subprocess.run([certutil, "-N", "-d", f"sql:{profile}", "--empty-password"], check=True)
            subprocess.run([certutil, "-A", "-d", f"sql:{profile}", "-n", "device-fixture-ca",
                            "-t", "C,,", "-i", f"{tmp}/ca.pem"], check=True)
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

                def command(number, name, data, expected_error=None):
                    encoded = json.dumps([0, number, name, data]).encode()
                    control.sendall(str(len(encoded)).encode() + b":" + encoded)
                    result = receive()
                    if expected_error is not None:
                        assert isinstance(result[2], dict) and result[2].get("error") == expected_error, result
                        return result[2]
                    assert result[2] is None, result
                    return result[3]

                receive()
                session = command(1, "WebDriver:NewSession", {"acceptInsecureCerts": False})
                assert session["capabilities"]["acceptInsecureCerts"] is False
                tls_checks.append("browser certificate validation remains enabled")
                command(2, "WebDriver:Navigate", {"url": untrusted_origin + "/peer"}, "insecure certificate")
                assert untrusted_server.application_requests == 0
                tls_checks.append("untrusted HTTPS certificate rejected before application dispatch")
                command(3, "WebDriver:Navigate", {"url": wrong_name + "/peer"}, "insecure certificate")
                assert server.application_requests == 0
                tls_checks.append("trusted certificate with wrong HTTPS hostname rejected before application dispatch")
                command(4, "WebDriver:Navigate", {"url": origin + "/peer"})
                verified = command(5, "WebDriver:ExecuteAsyncScript", {
                    "script": """
const [untrusted, wrongName, done] = arguments;
async function rejected(url) {
  return await new Promise((resolve) => {
    const socket = new WebSocket(url.replace('https:', 'wss:') + '/ws');
    const timer = setTimeout(() => { socket.close(); resolve(false); }, 5000);
    socket.onopen = () => { clearTimeout(timer); socket.close(); resolve(false); };
    socket.onerror = () => { clearTimeout(timer); resolve(true); };
  });
}
(async () => done({ secureContext: isSecureContext,
  untrusted: await rejected(untrusted), wrongName: await rejected(wrongName) }))();
""",
                    "args": [untrusted_origin, wrong_name], "newSandbox": False,
                })
                assert verified["value"] == {"secureContext": True, "untrusted": True, "wrongName": True}, verified
                assert untrusted_server.application_requests == 0
                tls_checks.append("untrusted WSS certificate rejected before application dispatch")
                # The wrong-name socket must not become an ordinary HTTP auth failure.
                assert f"localhost:{server.server_address[1]}" not in server.request_hosts
                tls_checks.append("trusted certificate with wrong WSS hostname rejected before application dispatch")
                tls_checks.append("trusted HTTPS establishes a secure browser context")
                command(6, "WebDriver:Navigate", {"url": origin + "/fixture"})
                if not completed.wait(45):
                    diagnostic = command(7, "WebDriver:ExecuteScript", {"script": "return {url:location.href,body:document.body.innerHTML,tests:window.fixtureTests,device:typeof CowboyDeviceProof}", "args": [], "newSandbox": False})
                    raise AssertionError(diagnostic)
                assert report and report[0].get("ok"), report
                sources = ["tools/browser-device-conformance.py", "tools/browser-device-fixture.js",
                           "web/device-proof.js", "src/browser_device.rs", "src/server/secure_transport.rs",
                           "src/store/browser_devices.rs"]
                with open(executable, "rb") as binary:
                    binary_digest = hashlib.file_digest(binary, "sha256").hexdigest()
                print(json.dumps({**report[0], "tests": tls_checks + report[0]["tests"],
                                  "browser": session.get("capabilities", {}).get("browserVersion"),
                                  "acceptInsecureCerts": False,
                                  "browserPath": browser, "certutilPath": certutil,
                                  "testBinarySha256": binary_digest,
                                  "sourceFilesSha256": {path: hashlib.sha256((root / path).read_bytes()).hexdigest() for path in sources},
                                  "isolation": "private network namespace, disposable browser profile and Rust store; only the fixture CA is trusted"}, indent=2))
        finally:
            for child in reversed(children):
                child.kill()
                child.wait()
            server.shutdown()
            untrusted_server.shutdown()
