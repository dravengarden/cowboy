#!/usr/bin/env python3
"""Disposable GotaTun/reference-WireGuard interop; never configures the host.

Run from the pinned shell under unshare --user --map-root-user --mount --net.
Pass absolute paths to gotatun and a bin directory containing wg, wireguard-go,
ip and ping. The private mount namespace owns /run and the two netns handles.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gotatun", type=Path)
    parser.add_argument("network_tools", type=Path)
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    mapping = Path("/proc/self/uid_map").read_text().split()
    if len(mapping) != 3 or mapping[0] != "0" or mapping[2] != "1" or mapping[1] == "0":
        raise SystemExit("Refusing to run outside a fresh unprivileged user namespace")
    ip = str(args.network_tools / "ip")
    wg = str(args.network_tools / "wg")
    ping = str(args.network_tools / "ping")

    def run(command, **kwargs):
        return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout.strip()

    interfaces = json.loads(run([ip, "-j", "link"]))
    if [entry["ifname"] for entry in interfaces] != ["lo"]:
        raise SystemExit("Refusing to run in a populated network namespace")
    run(["mount", "--make-rprivate", "/"])
    run(["mount", "-t", "tmpfs", "tmpfs", "/run"])
    checks = []
    processes = []
    with tempfile.TemporaryDirectory(prefix="cowboy-wireguard-probe-") as temporary:
        root = Path(temporary)
        os.chmod(root, 0o700)

        def net(namespace, command, **kwargs):
            return run([ip, "netns", "exec", namespace, *command], **kwargs)

        def start(namespace, command):
            log = (root / f"process-{len(processes)}.log").open("w")
            process = subprocess.Popen([ip, "netns", "exec", namespace, *command], stdout=log, stderr=log)
            processes.append((process, log))
            return process

        try:
            for namespace in ["cowboy-client", "cowboy-server"]:
                run([ip, "netns", "add", namespace])
                net(namespace, [ip, "link", "set", "lo", "up"])
            run([ip, "link", "add", "underlay-client", "type", "veth", "peer", "name", "underlay-server"])
            for side, address in [("client", "192.0.2.1/24"), ("server", "192.0.2.2/24")]:
                namespace = "cowboy-" + side
                interface = "underlay-" + side
                run([ip, "link", "set", interface, "netns", namespace])
                net(namespace, [ip, "address", "add", address, "dev", interface])
                net(namespace, [ip, "link", "set", interface, "up"])

            hosts = root / "hosts"
            hosts.write_text("127.0.0.1 localhost\n192.0.2.2 gateway.cowboy-probe.invalid\n"
                             "10.77.0.2 service.cowboy-probe.invalid wrong.cowboy-probe.invalid\n")
            # Bind over hosts only inside this process's private mount namespace.
            run(["mount", "--bind", str(hosts), "/etc/hosts"])
            start("cowboy-server", [str(args.gotatun), "--foreground", "wg-server"])
            start("cowboy-client", [str(args.network_tools / "wireguard-go"), "-f", "wg-client"])
            deadline = time.monotonic() + 10
            while not all(Path(f"/run/wireguard/wg-{side}.sock").exists() for side in ["client", "server"]):
                if time.monotonic() > deadline or any(process.poll() is not None for process, _ in processes):
                    raise RuntimeError("WireGuard process did not create its private UAPI socket")
                time.sleep(0.05)

            public = {}
            for side in ["client", "server", "wrong"]:
                private = run([wg, "genkey"])
                key_file = root / f"{side}.key"
                key_file.write_text(private + "\n")
                os.chmod(key_file, 0o600)
                public[side] = run([wg, "pubkey"], input=private + "\n")
            for side, address, peer in [("client", "10.77.0.1", "server"), ("server", "10.77.0.2", "client")]:
                namespace = "cowboy-" + side
                interface = "wg-" + side
                peer_address = "10.77.0.2" if side == "client" else "10.77.0.1"
                net(namespace, [wg, "set", interface, "private-key", str(root / f"{side}.key"),
                                "listen-port", "51820", "peer", public[peer], "allowed-ips", peer_address + "/32"])
                net(namespace, [ip, "address", "add", address + "/32", "dev", interface])
                net(namespace, [ip, "link", "set", interface, "mtu", "1280", "up"])
                net(namespace, [ip, "route", "add", peer_address + "/32", "dev", interface])

            def endpoint(key, address):
                net("cowboy-client", [wg, "set", "wg-client", "peer", key, "allowed-ips", "10.77.0.2/32", "endpoint", address])

            def reaches_server(source=None):
                command = [ping, "-n", "-c", "1", "-W", "2"]
                if source:
                    command += ["-I", source]
                command += ["10.77.0.2"]
                return subprocess.run([ip, "netns", "exec", "cowboy-client", *command], capture_output=True).returncode == 0

            endpoint(public["server"], "192.0.2.2:51820")
            assert reaches_server(), "IP endpoint did not establish a tunnel"
            checks.append("reference WireGuard client interoperates with GotaTun using an IP endpoint")
            endpoint(public["server"], "gateway.cowboy-probe.invalid:51820")
            assert reaches_server(), "hostname endpoint failed"
            checks.append("hostname endpoint resolves without changing the pinned peer key")

            # Validate the existing HTTPS architecture over the tunnel with a
            # disposable, explicitly trusted certificate. No insecure TLS flag.
            key = root / "tls.key"
            cert = root / "tls.crt"
            run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj",
                 "/CN=service.cowboy-probe.invalid", "-addext", "subjectAltName=DNS:service.cowboy-probe.invalid",
                 "-keyout", str(key), "-out", str(cert)])
            server = root / "https.py"
            server.write_text('''import http.server,ssl,sys,socket,threading
udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind(("10.77.0.2",10444))
def record_udp():
 while True:
  data,peer=udp.recvfrom(2048)
  with open(sys.argv[3],"ab") as output:output.write(data+b"\\n")
threading.Thread(target=record_udp,daemon=True).start()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  self.send_response(200);self.end_headers();self.wfile.write(b"cowboy-wireguard-https")
 def log_message(self,*args):pass
server=http.server.HTTPServer(("10.77.0.2",10443),Handler)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(sys.argv[1],sys.argv[2])
server.socket=context.wrap_socket(server.socket,server_side=True);server.serve_forever()
''')
            datagrams = root / "received.datagrams"
            start("cowboy-server", [sys.executable, str(server), str(cert), str(key), str(datagrams)])
            fetch = ("import ssl,urllib.request,sys; opener=urllib.request.build_opener("
                     "urllib.request.ProxyHandler({}),urllib.request.HTTPSHandler("
                     "context=ssl.create_default_context(cafile=sys.argv[2])));"
                     "print(opener.open(sys.argv[1],timeout=3).read().decode())")
            deadline = time.monotonic() + 5
            while True:
                try:
                    response = net("cowboy-client", [sys.executable, "-c", fetch, "https://service.cowboy-probe.invalid:10443/", str(cert)])
                    break
                except subprocess.CalledProcessError:
                    if time.monotonic() > deadline:
                        raise
                    time.sleep(0.1)
            assert response == "cowboy-wireguard-https"
            checks.append("certificate-verified HTTPS works over WireGuard")
            try:
                net("cowboy-client", [sys.executable, "-c", fetch, "https://wrong.cowboy-probe.invalid:10443/", str(cert)])
            except subprocess.CalledProcessError as error:
                assert "CERTIFICATE_VERIFY_FAILED" in error.stderr
            else:
                raise AssertionError("Wrong TLS hostname was accepted")
            checks.append("TLS still rejects an incorrect certificate hostname inside the tunnel")

            send_datagram = ("import socket,sys; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);"
                             "s.bind((sys.argv[1],0));s.sendto(sys.argv[2].encode(),('10.77.0.2',10444))")
            net("cowboy-client", [sys.executable, "-c", send_datagram, "10.77.0.1", "authorized"])
            deadline = time.monotonic() + 3
            while not datagrams.exists():
                if time.monotonic() > deadline:
                    raise RuntimeError("Authorized UDP control packet did not reach the server")
                time.sleep(0.05)
            assert datagrams.read_bytes() == b"authorized\n"
            net("cowboy-client", [ip, "address", "add", "10.77.0.9/32", "dev", "wg-client"])
            net("cowboy-client", [sys.executable, "-c", send_datagram, "10.77.0.9", "unauthorized"])
            time.sleep(0.5)
            assert datagrams.read_bytes() == b"authorized\n", "Peer sent an unauthorized inner source IP"
            net("cowboy-client", [ip, "address", "del", "10.77.0.9/32", "dev", "wg-client"])
            checks.append("GotaTun rejects a peer source outside its AllowedIPs")
            net("cowboy-client", [wg, "set", "wg-client", "peer", public["server"], "remove"])
            endpoint(public["wrong"], "192.0.2.2:51820")
            assert not reaches_server(), "Incorrect server public key established a tunnel"
            checks.append("correct endpoint with a wrong server public key cannot communicate")
            net("cowboy-client", [wg, "set", "wg-client", "peer", public["wrong"], "remove"])
            endpoint(public["server"], "gateway.cowboy-probe.invalid:51820")
            assert reaches_server(), "Restored peer did not recover"
            checks.append("restoring the trusted peer recovers the tunnel")
            net("cowboy-server", [wg, "set", "wg-server", "peer", public["client"], "remove"])
            assert not reaches_server(), "Revoked device still communicated"
            checks.append("removing the device peer stops its established traffic")

            receipt = {
                "schema": 1, "gotatun": run([str(args.gotatun), "--version"]),
                "gotatun_sha256": hashlib.sha256(args.gotatun.read_bytes()).hexdigest(),
                "reference": run([str(args.network_tools / "wireguard-go"), "--version"]),
                "checks": checks, "host_network_modified": False, "production_activation": False,
                "scope": "isolated Linux userspace interoperability; no Cowboy enrollment or mobile VPN implementation",
            }
            args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
            print(json.dumps(receipt, indent=2))
        finally:
            for process, log in reversed(processes):
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                log.close()
            for namespace in ["cowboy-client", "cowboy-server"]:
                subprocess.run([ip, "netns", "del", namespace], capture_output=True)


if __name__ == "__main__":
    main()
