"""Run only under the repository's pinned shell inside a private net namespace.

Arguments: Firefox, certutil, native server, assets directory, receipt path.
TLS trust is confined to a disposable Firefox profile. No system VPN is used.
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


def main():
    browser, certutil, executable, assets, receipt = map(lambda p: Path(p).resolve(), sys.argv[1:])
    assert len(sys.argv) == 6
    # Require a single-user mapped namespace and an otherwise empty network.
    mapping = Path('/proc/self/uid_map').read_text().split()
    assert len(mapping) == 3 and mapping[2] == '1', 'private user namespace required'
    assert sorted(name for _, name in socket.if_nameindex()) == ['lo'], 'private network required'
    assert str(browser).startswith('/nix/store/')
    assert all(p.exists() for p in [browser, certutil, executable, assets])
    children = []
    with tempfile.TemporaryDirectory(prefix='cowboy-browser-wg-') as tmp:
        run = Path(tmp)
        os.chmod(run, 0o700)

        def openssl(*args):
            subprocess.run(['openssl', *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                '-subj', '/CN=Cowboy WireGuard disposable fixture CA',
                '-addext', 'basicConstraints=critical,CA:TRUE',
                '-keyout', str(run/'ca.key'), '-out', str(run/'ca.pem'))
        openssl('req', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=localhost',
                '-keyout', str(run/'key.pem'), '-out', str(run/'request.pem'))
        (run/'extensions').write_text('subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n')
        openssl('x509', '-req', '-days', '1', '-in', str(run/'request.pem'),
                '-CA', str(run/'ca.pem'), '-CAkey', str(run/'ca.key'), '-CAcreateserial',
                '-extfile', str(run/'extensions'), '-out', str(run/'cert.pem'))
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(run/'cert.pem', run/'key.pem')

        rust_log = (run/'rust.log').open('w+')
        firefox_log = (run/'firefox.log').open('w+')
        # Do not pass account credentials or provider state into the fixture.
        env = {'PATH': os.environ['PATH']}
        try:
            children.append(subprocess.Popen([str(executable), str(assets), tmp], env=env, stdout=rust_log, stderr=rust_log))
            address = wait_for(lambda: (run/'backend').read_text())
            host, port = address.strip().split(':')
            backend = (host, int(port))

            class Proxy(socketserver.BaseRequestHandler):
                def handle(self):
                    try:
                        self.forward()
                    except (OSError, ssl.SSLError):
                        pass  # Browser speculative connections and closed sockets.

                def forward(self):
                    with tls.wrap_socket(self.request, server_side=True) as client:
                        head = b''
                        while b'\r\n\r\n' not in head:
                            part = client.recv(4096)
                            if not part:
                                return
                            head += part
                            assert len(head) <= 65536
                        headers, pending = head.split(b'\r\n\r\n', 1)
                        lines = headers.split(b'\r\n')
                        upgraded = any(line.lower().startswith(b'upgrade: websocket') for line in lines)
                        if not upgraded:
                            lines = [line for line in lines if not line.lower().startswith(b'connection:')]
                            lines.append(b'Connection: close')
                        with socket.create_connection(backend) as upstream:
                            upstream.sendall(b'\r\n'.join(lines) + b'\r\n\r\n' + pending)
                            while True:
                                ready, _, _ = select.select([client, upstream], [], [], 20)
                                if not ready:
                                    if upgraded:
                                        continue
                                    return
                                for source in ready:
                                    data = source.recv(65536)
                                    if not data:
                                        return
                                    (upstream if source is client else client).sendall(data)

            class Server(socketserver.ThreadingTCPServer):
                allow_reuse_address = True
                daemon_threads = True

            with Server(('127.0.0.1', 0), Proxy) as proxy:
                threading.Thread(target=proxy.serve_forever, daemon=True).start()
                origin = f'https://127.0.0.1:{proxy.server_address[1]}'
                profile = run/'profile'
                profile.mkdir()
                subprocess.run([str(certutil), '-N', '-d', f'sql:{profile}', '--empty-password'], check=True)
                subprocess.run([str(certutil), '-A', '-d', f'sql:{profile}', '-n', 'wireguard-fixture-ca', '-t', 'C,,', '-i', str(run/'ca.pem')], check=True)
                with socket.socket() as reservation:
                    reservation.bind(('127.0.0.1', 0))
                    marionette = reservation.getsockname()[1]
                (profile/'user.js').write_text(f'user_pref("marionette.port", {marionette});\n')
                children.append(subprocess.Popen([str(browser), '--headless', '--no-remote', '--new-instance', '--marionette', '--profile', str(profile), 'about:blank'],
                    env={**env, 'MOZ_HEADLESS':'1', 'MOZ_NO_REMOTE':'1', 'XDG_RUNTIME_DIR':tmp, 'XDG_CACHE_HOME':str(run/'cache'), 'XDG_CONFIG_HOME':str(run/'config')},
                    stdout=firefox_log, stderr=firefox_log))
                with wait_for(lambda: socket.create_connection(('127.0.0.1', marionette))) as control:
                    control.settimeout(30)
                    stream = control.makefile('rb')

                    def receive():
                        length = b''
                        while not length.endswith(b':'):
                            part = stream.read(1)
                            if not part:
                                raise OSError('Marionette disconnected')
                            length += part
                        return json.loads(stream.read(int(length[:-1])))

                    def command(number, name, data):
                        encoded = json.dumps([0, number, name, data]).encode()
                        control.sendall(str(len(encoded)).encode() + b':' + encoded)
                        result = receive()
                        assert result[2] is None, result
                        return result[3]

                    receive()
                    session = command(1, 'WebDriver:NewSession', {'acceptInsecureCerts':False})
                    assert session['capabilities']['acceptInsecureCerts'] is False
                    command(2, 'WebDriver:Navigate', {'url':origin+'/fixture.html'})
                    started = time.monotonic()
                    previous = ''
                    heartbeat = started
                    while time.monotonic() - started < 210 and not (run/'report.json').exists():
                        if (run/'progress.json').exists():
                            progress = (run/'progress.json').read_text()
                            if progress != previous:
                                print(progress, flush=True)
                                previous = progress
                        if time.monotonic() - heartbeat > 25:
                            print(f'Browser experiment running: {time.monotonic() - started:.0f}s', flush=True)
                            heartbeat = time.monotonic()
                        time.sleep(0.2)
                    if not (run/'report.json').exists():
                        diagnostic = command(3, 'WebDriver:ExecuteScript', {'script':'return {url:location.href,body:document.body.innerText}', 'args':[], 'newSandbox':False})
                        raise AssertionError(diagnostic)
                    result = json.loads((run/'report.json').read_text())
                    result.update({
                        'date':'2026-10-03', 'engine':'GotaTun 0.9.2 with experimental WASM clocks',
                        'browser':session['capabilities'].get('browserVersion'),
                        'tls':'certificate verification enabled; disposable CA trusted only in disposable Firefox profile',
                        'isolation':'private user/network namespace; only loopback; no TUN, host routes or production credentials',
                        'wasmBytes':(assets/'cowboy_browser_wireguard_probe_bg.wasm').stat().st_size,
                        'wasmSha256':hashlib.sha256((assets/'cowboy_browser_wireguard_probe_bg.wasm').read_bytes()).hexdigest(),
                        'nativeSha256':hashlib.sha256(executable.read_bytes()).hexdigest(),
                        'sourceFilesSha256':{str(p.relative_to(Path(__file__).parent)):hashlib.sha256(p.read_bytes()).hexdigest()
                            for p in sorted(Path(__file__).parent.rglob('*')) if p.is_file() and p.suffix in ['.rs', '.js', '.py', '.toml', '.lock', '.patch']},
                        'limitations':['IPv4/UDP fixture payloads, not Cowboy HTTP/WebSocket application integration',
                            'loopback measurements include test-harness polling; not LAN throughput benchmarks',
                            'explicit pause/resume path, not a physical-device OS suspension test',
                            'Firefox only; Safari/PWA acceptance outstanding'],
                    })
                    receipt.write_text(json.dumps(result, indent=2)+'\n')
                    print(json.dumps(result, indent=2), flush=True)
                    assert result.get('ok'), result
                proxy.shutdown()
        except BaseException:
            rust_log.flush(); firefox_log.flush()
            print('Rust fixture log:', (run/'rust.log').read_text()[-5000:])
            print('Firefox fixture log:', (run/'firefox.log').read_text()[-5000:])
            raise
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.kill()
                child.wait()
            rust_log.close(); firefox_log.close()


if __name__ == '__main__':
    main()
