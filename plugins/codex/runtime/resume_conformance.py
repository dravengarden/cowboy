"""Exercise a released ACP entrypoint against a copied native rollout, offline."""

import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import tempfile
import time


async def check(adapter, codex, rollout, root, warm):
    home = root / "home"
    codex_home = home / ".codex"
    workspace = root / "workspace"
    workspace.mkdir(exist_ok=True)
    if not warm:
        with rollout.open("rb") as source:
            meta = json.loads(source.readline())
        if meta["type"] != "session_meta":
            raise ValueError("Fixture must start with native session metadata")
        date = re.search(r"rollout-(\d{4})-(\d{2})-(\d{2})", rollout.name)
        if date is None:
            raise ValueError("Fixture needs a native rollout filename")
        target = codex_home / "sessions" / Path(*date.groups()) / rollout.name
        target.parent.mkdir(parents=True)
        shutil.copyfile(rollout, target)
        (codex_home / "config.toml").write_text(
            'model = "gpt-6-astra"\n[features]\nremote_control = false\nshell_snapshot = false\n'
        )
    with rollout.open("rb") as source:
        thread_id = json.loads(source.readline())["payload"]["id"]
    logs = root / ("warm-logs" if warm else "cold-logs")
    logs.mkdir(mode=0o700)
    environment = {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "CODEX_HOME": str(codex_home),
        "CODEX_PATH": str(codex),
        "OPENAI_API_KEY": "cowboy-offline-conformance-not-a-real-key",
        "DEFAULT_AUTH_REQUEST": '{"methodId":"api-key"}',
        "APP_SERVER_LOGS": str(logs),
        "RUST_LOG": "off",
    }
    configuration = [
        "-c", "approval_policy=never",
        "-c", "sandbox_mode=danger-full-access",
        "-c", 'model_auto_compact_token_limit_scope="body_after_prefix"',
    ]
    process = await asyncio.create_subprocess_exec(
        "unshare", "--user", "--map-root-user", "--net", str(adapter), *configuration,
        cwd=workspace, env=environment, start_new_session=True,
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL, limit=1024 * 1024,
    )
    received_bytes = 0
    replay = []

    async def request(request_id, method, params):
        process.stdin.write((json.dumps({
            "jsonrpc": "2.0", "id": request_id, "method": method, "params": params,
        }) + "\n").encode())
        await process.stdin.drain()
        while True:
            nonlocal received_bytes
            line = await process.stdout.readline()
            if not line:
                raise RuntimeError("ACP closed before completing the request")
            received_bytes += len(line)
            message = json.loads(line)
            if message.get("id") == request_id and "method" not in message:
                if "error" in message:
                    raise RuntimeError(f"{method} rejected with code {message['error']['code']}")
                return message["result"]
            if message.get("method") == "session/update":
                kind = message.get("params", {}).get("update", {}).get("sessionUpdate")
                if kind in ("user_message_chunk", "agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update"):
                    replay.append(kind)
            if "method" in message and "id" in message:
                process.stdin.write((json.dumps({
                    "jsonrpc": "2.0", "id": message["id"],
                    "error": {"code": -32601, "message": "No client tools in resume fixture"},
                }) + "\n").encode())
                await process.stdin.drain()

    try:
        initialized = await asyncio.wait_for(request(1, "initialize", {
            "protocolVersion": 1, "clientInfo": {"name": "cowboy-resume-conformance", "version": "1"},
        }), 60)
        if "resume" not in initialized.get("agentCapabilities", {}).get("sessionCapabilities", {}):
            raise AssertionError("Packaged adapter does not advertise resume")
        started = time.monotonic()
        await asyncio.wait_for(request(2, "session/resume", {
            "sessionId": thread_id, "cwd": str(workspace), "mcpServers": [],
        }), 60)
        elapsed = time.monotonic() - started
        children = Path(f"/proc/{process.pid}/task/{process.pid}/children").read_text().split()
        native = []
        for child in children:
            try:
                argv = Path(f"/proc/{child}/cmdline").read_bytes().split(b"\0")[:-1]
            except FileNotFoundError:
                continue
            if argv and argv[0] == os.fsencode(codex):
                native.append(argv)
        expected = [os.fsencode(codex), *map(os.fsencode, configuration), b"app-server"]
        if native != [expected]:
            raise AssertionError("Native CLI is not a direct child with the exact configuration")
        # The debug log lives only inside this private disposable fixture. Parse
        # request objects; never copy history, log bodies, or environment to a receipt.
        requests = []
        for line in (logs / "app-server.log").read_text().splitlines():
            match = re.search(r"\[IN\] (\{.*)", line)
            if match:
                requests.append(json.JSONDecoder().raw_decode(match[1])[0])
        restores = [message for message in requests if message.get("method") == "thread/resume"]
        if len(restores) != 1:
            raise AssertionError("Expected exactly one native resume")
        params = restores[0]["params"]
        if params.get("threadId") != thread_id or params.get("excludeTurns") is not True:
            raise AssertionError("Native identity or excluded-turns contract changed")
        forbidden = {"thread/start", "thread/fork", "turn/start"}
        if any(message.get("method") in forbidden for message in requests) or replay:
            raise AssertionError("Resume created a thread, submitted a turn, or replayed history")
        if received_bytes > 1024 * 1024:
            raise AssertionError("ACP restore exceeded its metadata response budget")
        return {
            "projection": "warm" if warm else "cold",
            "native_thread_id": thread_id,
            "resume_seconds": round(elapsed, 3),
            "acp_received_bytes": received_bytes,
            "exclude_turns": True,
            "exact_direct_native_launch": True,
            "history_replayed": False,
            "new_thread_or_turn": False,
        }
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        await process.wait()


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--rollout", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.receipt.exists():
        raise ValueError("Receipt must be a new path")
    paths = [path.resolve(strict=True) for path in (args.adapter, args.codex, args.rollout)]
    with tempfile.TemporaryDirectory(prefix="cowboy-native-resume-") as temporary:
        root = Path(temporary)
        cold = await check(*paths, root, False)
        warm = await check(*paths, root, True)
    with paths[2].open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    receipt = {
        "schema": "cowboy.codex-resume-conformance/v1",
        "fixture_sha256": digest,
        "fixture_bytes": paths[2].stat().st_size,
        "results": [cold, warm],
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(args.receipt, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "w") as target:
        json.dump(receipt, target, indent=2)
        target.write("\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    asyncio.run(main())
