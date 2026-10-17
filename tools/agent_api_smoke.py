#!/usr/bin/env python3
"""Read-only MCP workflow acceptance and tool-payload cost measurement.

Run inside an ordinary Cowboy session with its current gateway context. Writes
only a new local receipt, never conversation content. Optional tiktoken reports
o200k_base/cl100k_base counts; these are not model billing or cache measurements.
"""
import argparse
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import threading
import time


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def size(text):
    result = {"bytes": len(text.encode())}
    try:
        import tiktoken
    except ImportError:
        return result
    for name in ["o200k_base", "cl100k_base"]:
        result[name] = len(tiktoken.get_encoding(name).encode(text, disallowed_special=()))
    return result


class Mcp:
    def __init__(self, binary, environment=None):
        self.process = subprocess.Popen([binary, "mcp"], env=environment,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.sequence = 0
        self.pending = b""
        self.costs = []

    def request(self, method, params, *, allow_error=False):
        self.sequence += 1
        payload = {"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params}
        self.process.stdin.write((encoded(payload) + "\n").encode())
        self.process.stdin.flush()
        start = time.monotonic()
        deadline = start + 80
        while True:
            if b"\n" not in self.pending:
                ready, _, _ = select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))
                if not ready:
                    raise RuntimeError("MCP response timed out")
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk or len(self.pending) + len(chunk) > 1024 * 1024:
                    raise RuntimeError("MCP response ended or exceeded its bound")
                self.pending += chunk
                continue
            line, self.pending = self.pending.split(b"\n", 1)
            reply = json.loads(line)
            if reply.get("id") != self.sequence:
                continue
            if "error" in reply:
                raise RuntimeError("MCP RPC failed")
            result = reply["result"]
            if method == "tools/call":
                self.costs.append({"input": encoded(params), "output": encoded(result),
                                   "elapsed_ms": round((time.monotonic() - start) * 1000)})
                if result.get("isError") and not allow_error:
                    raise RuntimeError("MCP tool failed: " + encoded(result)[:300])
                return json.loads(result["content"][0]["text"])
            return result

    def tool(self, name, args):
        return self.request("tools/call", {"name": name, "arguments": args})

    def read(self, code):
        return self.tool("cowboy_read", {"code": code})

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def costs(rows):
    return {"calls": len(rows), "input": size("".join(row["input"] for row in rows)),
            "output": size("".join(row["output"] for row in rows)),
            "elapsed_ms": sum(row["elapsed_ms"] for row in rows)}


def startup_probe(binary):
    """A late private fixture gateway must work in the same MCP process."""
    environment = dict(os.environ)
    environment.pop("COWBOY_CALL_CONTEXT", None)
    outside = Mcp(binary, environment)
    try:
        assert not outside.request("tools/list", {})["tools"]
    finally:
        outside.close()
    with tempfile.TemporaryDirectory(prefix="cowboy-mcp-") as directory:
        root = Path(directory)
        context = root / "context.json"
        environment["COWBOY_CALL_CONTEXT"] = str(context)
        pending = Mcp(binary, environment)
        listener = socket.socket(socket.AF_UNIX)
        listener.settimeout(5)
        try:
            init = pending.request("initialize", {"protocolVersion": "2025-11-25"})
            assert init["instructions"]
            assert len(pending.request("tools/list", {})["tools"]) == 4
            arguments = {"name": "cowboy_status", "arguments": {}}
            denied = pending.request("tools/call", arguments, allow_error=True)
            assert denied["error"]["code"] == "context_unavailable"
            listener.bind(str(root / "gateway.sock"))
            listener.listen()
            context.write_text(encoded({"schema": 1, "socket": str(root / "gateway.sock"),
                                        "capability": "a" * 64}))
            context.chmod(0o600)
            observed = []

            def gateway():
                with listener.accept()[0] as connection:
                    connection.settimeout(5)
                    request = json.loads(connection.makefile("rb").readline(65536))
                    observed.append(request["action"])
                    reply = {"schema": 1, "state": "ok", "value": {"late_gateway": True}}
                    connection.sendall((encoded(reply) + "\n").encode())

            worker = threading.Thread(target=gateway, daemon=True)
            worker.start()
            assert pending.request("tools/call", arguments) == {"late_gateway": True}
            worker.join(timeout=5)
            assert not worker.is_alive() and observed == [{"kind": "agent_status"}]
        finally:
            listener.close()
            pending.close()
    return {"outside_session_empty": True, "pending_grant_discoverable": True,
            "missing_context_refused": True, "late_gateway_same_process": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cowboy", default="cowboy")
    parser.add_argument("--startup-only", action="store_true",
                        help="test a private late gateway fixture without contacting a real session")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.receipt.exists():
        parser.error("receipt must be a new path")
    startup = startup_probe(args.cowboy)
    if args.startup_only:
        with args.receipt.open("x", encoding="utf8") as target:
            json.dump({"schema": "cowboy.agent-api-startup/v1", "checks": startup}, target, indent=2)
        print(json.dumps(startup))
        return
    mcp = Mcp(args.cowboy)
    try:
        init = mcp.request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "cowboy-acceptance", "version": "1"}})
        tools = mcp.request("tools/list", {})["tools"]
        assert [tool["name"] for tool in tools] == ["cowboy_status", "cowboy_read", "cowboy_execute", "cowboy_receipt"]
        status = mcp.tool("cowboy_status", {})
        index = mcp.tool("cowboy_read", {"help": "index"})
        assert status["revision"] == index["revision"]
        assert "sessions.event" in index["read"] and "configuration.read" in index["read"]
        help_costs = {}
        for topic in ["sessions", "usage", "machines", "configuration"]:
            value = mcp.tool("cowboy_read", {"help": topic})
            assert value["revision"] == index["revision"] and value["types"]
            help_costs[topic] = costs(mcp.costs[-1:])

        start = len(mcp.costs)
        mcp.read("return await cowboy.session.info();")
        usage = mcp.read("return await cowboy.session.usage();")
        if usage["account"]:
            mcp.read("return await cowboy.usage.read(" + encoded(usage["account"]) + ");")
        mcp.read("return await cowboy.machines.list({limit:20});")
        mcp.read("return await cowboy.calls.list({limit:5});")
        separate = costs(mcp.costs[start:])
        start = len(mcp.costs)
        combined = mcp.read("""const s=await cowboy.session.info(); const u=await cowboy.session.usage();
const q=u.account?await cowboy.usage.read(u.account):null;
const m=await cowboy.machines.list({limit:20}); const c=await cowboy.calls.list({limit:5});
return {session:{id:s.id,status:s.status,queued:s.queued,drafts:s.drafts},
usage:{account:u.account,context:u.context,quota:q?.page.data,observed_at_ms:q?.observed_at_ms,refreshing:q?.refreshing},
machines:m.machines.map(x=>({id:x.id,status:x.status,workers:x.active_sessions,capacity:x.capacity,resources:x.resources})),
calls:c.calls};""")
        composed = costs(mcp.costs[start:])
        assert combined["session"]["id"] == status["session"]["id"]
        acceptance = mcp.read("""const s=await cowboy.sessions.list({limit:50});
const other=s.filter(x=>x.id!==""" + encoded(status["session"]["id"]) + """).sort((a,b)=>b.events-a.events)[0];
let event=false,search=false;
if(other){const h=await cowboy.sessions.history(other.id,{limit:3,chars:100});
if(h.items.length){const d=await cowboy.sessions.event(other.id,h.items[0].seq);event=!!d.revision;}
const found=await cowboy.sessions.search(other.id,{query:'cowboy',limit:1,chars:80}); search=Array.isArray(found.matches);}
const c=await cowboy.configuration.read('session'); const t=await cowboy.configuration.read('tools');
const u=await cowboy.usage.diagnostics();
return {other:!!other,event,search,configuration:!!c.revision&&!!t.revision,diagnostics:!!u.page};""")
        assert acceptance["configuration"] and acceptance["diagnostics"]
        if acceptance["other"]:
            assert acceptance["event"] and acceptance["search"]
        receipt = {"schema": "cowboy.agent-api-acceptance/v1", "api": index["api"], "revision": index["revision"],
                   "resident": size(encoded(tools) + init.get("instructions", "")), "help": help_costs,
                   "workflow": {"separate_reads": separate, "composed_selection": composed}, "checks": acceptance,
                   "startup": startup,
                   "not_checked": ["model_billing", "prompt_cache_hits", "provider_registration", "mutations"]}
        with args.receipt.open("x", encoding="utf8") as target:
            target.write(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt, indent=2))
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
