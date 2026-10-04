"""Disposable browser proof for the execution gate's trusted loopback proxy.

This exercises current cookie/device binding. The isolated fixture speaks on
the Controller side of the TLS terminator; external TLS is a separate gate.
It never reads an installed browser, account credential or production key.
"""
import base64
import http.client
import json
import os
import subprocess


def encoded(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


class FixtureDevice:
    def __init__(self, root, environment, port, origin):
        self.private = root / "browser-device.pem"
        self.environment = environment
        self.port = port
        self.origin = origin
        self.run("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
                 "-out", str(self.private))
        self.private.chmod(0o600)
        public = self.run("pkey", "-in", str(self.private), "-pubout", "-outform", "DER")
        assert len(public) == 91 and public[-65] == 4
        self.key = encoded(public[-65:])

    def run(self, *arguments, data=None):
        return subprocess.run(["openssl", *arguments], input=data, env=self.environment,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=10, check=True).stdout

    def proof(self, method, target):
        client = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            client.request("GET", "/api/auth/browser/challenge", headers={
                "Origin": self.origin, "X-Forwarded-Proto": "https"})
            response = client.getresponse()
            assert response.status == 200, "browser challenge refused"
            challenge = json.loads(response.read(4096))
        finally:
            client.close()
        proof = {"key": self.key, "epoch": challenge["epoch"], "origin": self.origin,
                 "time": challenge["server_time_ms"], "nonce": encoded(os.urandom(32))}
        message = "\n".join(["cowboy-browser-proof-v1", proof["epoch"], self.origin,
                             self.key, method, target, str(proof["time"]), proof["nonce"]])
        signature = self.run("dgst", "-sha256", "-sign", str(self.private), data=message.encode())
        # OpenSSL returns the two P-256 integers as DER; the protocol uses r||s.
        assert signature[0] == 0x30 and signature[1] == len(signature) - 2
        cursor, raw = 2, b""
        for _ in range(2):
            assert signature[cursor] == 2
            size = signature[cursor + 1]
            integer = signature[cursor + 2:cursor + 2 + size]
            assert len(integer) == size and 1 <= size <= 33
            raw += int.from_bytes(integer).to_bytes(32)
            cursor += 2 + size
        assert cursor == len(signature)
        proof["signature"] = encoded(raw)
        return encoded(json.dumps(proof).encode())
