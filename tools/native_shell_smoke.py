#!/usr/bin/env python3
"""Accept an exact Debug Tauri bundle using only a newly created Simulator."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
BUNDLE_ID = "top.thundersparrow.cowboy"
REMOTE_ORIGIN = "https://cowboy.stormbird.xyz"


def probe_script(remote):
    source = (ROOT / "tools/native-shell-probe.js").read_text()
    return source + "\nreturn JSON.stringify(await probeCowboyNativeShell(" + json.dumps(remote) + "));"


def valid_probe(report, remote):
    if not isinstance(report, dict):
        return False
    tests = report.get("tests")
    expected_count = 15 if remote else 7
    return (isinstance(tests, list) and len(tests) == expected_count
            and all(isinstance(test, str) and test for test in tests)
            and len(set(tests)) == expected_count
            and report.get("phase") == ("remote-logged-out" if remote else "shell")
            and report.get("origin") in ([REMOTE_ORIGIN] if remote else ["tauri://localhost", REMOTE_ORIGIN])
            and isinstance(report.get("user_agent"), str))


def wait_for_probe(port, simulator, remote):
    script = probe_script(remote)
    deadline = time.monotonic() + (150 if remote else 90)
    payload = ""
    while True:
        try:
            status, payload = request(port, simulator, "/aeval", script)
            report = json.loads(payload) if status == 200 else None
            if valid_probe(report, remote):
                return report
        except (OSError, ValueError, urllib.error.URLError):
            pass
        if time.monotonic() >= deadline:
            raise RuntimeError("Actual Tauri/WKWebView " + ("remote" if remote else "shell")
                               + " smoke failed: " + str(payload))
        time.sleep(1)


def command(*args, capture=False, env=None):
    result = subprocess.run(args, cwd=ROOT, check=True, text=True,
                            stdout=subprocess.PIPE if capture else None,
                            env=env, timeout=300)
    return result.stdout.strip() if capture else None


def request(port, identity, path, body=None, extra=None):
    headers = {"X-Cowboy-Simulator": identity, **(extra or {})}
    data = body.encode() if body is not None else None
    query = urllib.request.Request("http://127.0.0.1:" + str(port) + path,
                                   data=data, headers=headers)
    try:
        with urllib.request.urlopen(query, timeout=20) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path, help="exact Debug Simulator build receipt.json")
    parser.add_argument("--remote", action="store_true",
                        help="also require the real remote logged-out Cowboy page; never logs in")
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise SystemExit("Actual native acceptance requires a Mac")
    if command("git", "status", "--porcelain", capture=True):
        raise SystemExit("Native acceptance requires a clean committed worktree")
    receipt_path = args.receipt.resolve(strict=True)
    receipt_path.relative_to(ROOT / "dist/native-shell")
    build = json.loads(receipt_path.read_text())
    if build.get("platform") != "ios-sim" or build.get("profile") != "debug":
        raise SystemExit("Acceptance requires an exact Debug Simulator build")
    revision = command("git", "rev-parse", "HEAD", capture=True)
    source_revision = build["source_revision"]
    # A docs/test-only descendant may accept the same source, never a divergent
    # shell or builder. All paths come from this worktree's successful receipt.
    command("git", "merge-base", "--is-ancestor", source_revision, revision)
    command("git", "diff", "--quiet", source_revision, revision, "--",
            "apps/native-shell", "tools/build-native-shell.sh")
    original = Path(build["app"]).resolve(strict=True)
    original.relative_to(receipt_path.parent)
    info = plistlib.loads((original / "Info.plist").read_bytes())
    if info["CFBundleIdentifier"] != BUNDLE_ID:
        raise SystemExit("Unexpected bundle identity")
    binary = original / info["CFBundleExecutable"]
    if hashlib.sha256(binary.read_bytes()).hexdigest() != build["executable_sha256"]:
        raise SystemExit("Native executable differs from its build receipt")
    # Refuse to replace even an unrelated local bridge. The small allocation
    # race fails closed when NWListener cannot bind the exclusively chosen port.
    with socket.socket() as allocation:
        allocation.bind(("127.0.0.1", 0))
        port = allocation.getsockname()[1]
    simulator = None
    app_pid = None
    tests = []
    mode = "remote-logged-out" if args.remote else "shell"
    # A failed rerun must not leave an older success at its receipt location.
    # Retain each attempt's diagnostics separately from immutable build inputs.
    output = Path(tempfile.mkdtemp(prefix="acceptance-" + mode + "-" + revision[:12] + ".",
                                   dir=receipt_path.parent))
    print("Acceptance output: " + str(output), flush=True)

    def check(name, value):
        if not value:
            raise RuntimeError("Native smoke check failed: " + name)
        tests.append(name)

    with tempfile.TemporaryDirectory(prefix="cowboy-tauri-smoke-") as temp:
        try:
            app = Path(temp) / "Cowboy.app"
            # Sign only a disposable copy; the original immutable build stays
            # unsigned with its recorded executable digest.
            shutil.copytree(original, app, symlinks=True)
            command("codesign", "--force", "--deep", "--sign", "-", str(app))
            runtimes = json.loads(command("xcrun", "simctl", "list", "runtimes", "--json", capture=True))
            choices = [r for r in runtimes["runtimes"] if r.get("isAvailable") and ".iOS-" in r["identifier"]]
            runtime = max(choices, key=lambda r: tuple(map(int, r["version"].split("."))))["identifier"]
            simulator = command("xcrun", "simctl", "create", "Cowboy Tauri Smoke " + revision[:8],
                                "com.apple.CoreSimulator.SimDeviceType.iPhone-16", runtime, capture=True)
            print("Smoke Simulator: " + simulator + ", loopback port: " + str(port), flush=True)
            command("xcrun", "simctl", "boot", simulator)
            command("xcrun", "simctl", "bootstatus", simulator, "-b")
            command("xcrun", "simctl", "install", simulator, str(app))
            environment = dict(os.environ, SIMCTL_CHILD_COWBOY_SIM_BRIDGE="1",
                               SIMCTL_CHILD_COWBOY_SIM_DEVPORT=str(port))
            launched = command("xcrun", "simctl", "launch", simulator, BUNDLE_ID, env=environment, capture=True)
            print(launched, flush=True)
            app_pid = launched.rsplit(": ", 1)[-1]
            deadline = time.monotonic() + 90
            while True:
                try:
                    if request(port, simulator, "/ping") == (200, "ok"):
                        break
                except (OSError, urllib.error.URLError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Actual Tauri app did not expose its opt-in Simulator bridge")
                time.sleep(1)
            check("actual Tauri app bridge", True)
            check("wrong Simulator rejected", request(port, "wrong", "/ping")[0] == 403)
            check("browser origin rejected", request(port, simulator, "/ping",
                  extra={"Origin": "https://foreign.invalid"})[0] == 403)
            check("eval requires POST", request(port, simulator, "/eval")[0] == 403)
            # First accept the actual shell, then (only when requested) wait
            # for its own loader navigation. Never force navigation or submit
            # a login: a stuck local loader must fail remote acceptance.
            shell = wait_for_probe(port, simulator, False)
            report = wait_for_probe(port, simulator, True) if args.remote else shell
            tests.extend(report["tests"])
            check("expected shell origin", report["origin"] in
                  ["tauri://localhost", REMOTE_ORIGIN])
            check("iPhone WebKit", "iPhone" in report["user_agent"] and "AppleWebKit" in report["user_agent"])
            report.update(ok=True, tests=tests, source_revision=source_revision,
                          acceptance_revision=revision, simulator_runtime=runtime,
                          executable_sha256=build["executable_sha256"],
                          acceptance_scope=mode, initial_shell_origin=shell["origin"],
                          real_login="not_checked", physical_device="not_checked")
            destination = output / "smoke-receipt.json"
            destination.write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, indent=2), flush=True)
        except Exception as error:
            (output / "failure.json").write_text(json.dumps(dict(
                ok=False, source_revision=source_revision, acceptance_revision=revision,
                acceptance_scope=mode, error=str(error)), indent=2) + "\n")
            if simulator and app_pid and app_pid.isdigit():
                diagnostic = subprocess.run(
                    ["xcrun", "simctl", "spawn", simulator, "log", "show", "--last", "4m",
                     "--style", "compact", "--predicate", "processIdentifier == " + app_pid],
                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    timeout=30, check=False)
                (output / "smoke-failure.log").write_text(diagnostic.stdout)
                print("\n".join(diagnostic.stdout.splitlines()[-50:]), file=sys.stderr, flush=True)
            raise
        finally:
            if simulator:
                for operation in ["shutdown", "delete"]:
                    subprocess.run(["xcrun", "simctl", operation, simulator],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   timeout=60, check=False)


if __name__ == "__main__":
    main()
