#!/usr/bin/env python3
"""Accept an exact Debug Tauri bundle using only a newly created Simulator."""
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
    if sys.platform != "darwin" or len(sys.argv) != 2:
        raise SystemExit("On a Mac: python3 tools/native_shell_smoke.py <build receipt.json>")
    if command("git", "status", "--porcelain", capture=True):
        raise SystemExit("Native acceptance requires a clean committed worktree")
    receipt_path = Path(sys.argv[1]).resolve(strict=True)
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
            # Wait only for the owned loader/remote document to finish its one
            # navigation. This never enters credentials or initiates login.
            script = """
const tests=[];
const check=(name,value)=>{if(!value)throw Error(name);tests.push(name)};
check("native keyboard shell",window.__cowboyNativeShell===true);
check("Tauri IPC present",typeof window.__TAURI__?.core?.invoke==="function");
check("native tweaks present",typeof window.__cowboySelectionHaptic==="function" &&
  typeof window.__cowboyReadClipboard==="function" &&
  window.__cowboyAuthenticationBrowserBridgeVersion===2);
const host=window.__COWBOY_NATIVE_PLUGIN_HOST;
check("immutable Plugin ABI coexists",host?.version==="1.0.0" && Object.isFrozen(host) &&
  Object.isFrozen(host.capabilities));
let denied=false;try{await host.invoke("unknown",{})}catch{denied=true}
check("unknown Plugin capability rejected",denied);
const passkeys=await host.invoke("webauthn",{action:"capabilities",rp_id:"cowboy.stormbird.xyz"});
check("unentitled shell fails closed",passkeys.ok===true && passkeys.available===false);
denied=false;try{await window.__TAURI__.core.invoke("plugin:opener|open_url",
  {url:"file:///cowboy-conformance-must-not-open"})}catch(error){
  denied=String(error).includes("not allowed") || String(error).includes("Forbidden") ||
    String(error).includes("forbidden");
}
check("opener rejects local files",denied);
return JSON.stringify({tests,origin:location.origin,user_agent:navigator.userAgent});
"""
            deadline = time.monotonic() + 90
            payload = ""
            while True:
                try:
                    status, payload = request(port, simulator, "/aeval", script)
                    report = json.loads(payload) if status == 200 else None
                    if report and len(report.get("tests", [])) == 7:
                        break
                except (OSError, ValueError, urllib.error.URLError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Actual Tauri/WKWebView smoke failed: " + str(payload))
                time.sleep(1)
            tests.extend(report["tests"])
            check("expected shell origin", report["origin"] in
                  ["tauri://localhost", "https://cowboy.stormbird.xyz"])
            check("iPhone WebKit", "iPhone" in report["user_agent"] and "AppleWebKit" in report["user_agent"])
            report.update(ok=True, tests=tests, source_revision=source_revision,
                          acceptance_revision=revision, simulator_runtime=runtime,
                          executable_sha256=build["executable_sha256"],
                          real_login="not_checked", physical_device="not_checked")
            destination = receipt_path.parent / "smoke-receipt.json"
            destination.write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, indent=2), flush=True)
        except Exception:
            if simulator and app_pid and app_pid.isdigit():
                diagnostic = subprocess.run(
                    ["xcrun", "simctl", "spawn", simulator, "log", "show", "--last", "4m",
                     "--style", "compact", "--predicate", "processIdentifier == " + app_pid],
                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    timeout=30, check=False)
                (receipt_path.parent / "smoke-failure.log").write_text(diagnostic.stdout)
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
