"""Target MCP server fixture for the packaged remote acceptance.

The native-local behavior it checks (2.1.287, SDK mode) was measured with the
same server: user, project and local servers all load without approval; a
local server overrides a project one, which overrides a user one;
`${VAR}`/`${VAR:-default}` expand from Claude Code's environment; a stdio
server starts in the working directory; server instructions reach the model
in an `# MCP Server Instructions` reminder.
"""
import json
from pathlib import Path

SERVER = r'''
import json, os, sys
name = sys.argv[1]
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("id") is None:
        continue
    method = msg.get("method")
    if method == "initialize":
        result = {"protocolVersion": msg["params"].get("protocolVersion", "2025-06-18"),
                  "capabilities": {"tools": {}}, "serverInfo": {"name": name, "version": "1"},
                  "instructions": name + " instructions"}
    elif method == "tools/list":
        result = {"tools": [{"name": "where", "description": name + " where",
                             "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": json.dumps({
            "server": name, "cwd": os.getcwd(), "argv": sys.argv[2:],
            "env": os.environ.get("MCP_FIXTURE"), "project": os.environ.get("CLAUDE_PROJECT_DIR"),
            "claudecode": os.environ.get("CLAUDECODE")})}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "unknown"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
'''


def server(target, name):
    return {"command": "python3", "args": [str(target / ".mcp-fixture-server.py"), name, "${HOME}", "${UNSET_FIXTURE:-dflt}"],
            "env": {"MCP_FIXTURE": "${HOME}-env"}}


def setup(target, home):
    """User, local and project servers, a shadowed name and a loopback URL."""
    (target / ".mcp-fixture-server.py").write_text(SERVER)
    (target / ".mcp.json").write_text(json.dumps({"mcpServers": {
        "projsrv": server(target, "projsrv"),
        "dupsrv": server(target, "dup-project"),
        "loopsrv": {"type": "http", "url": "http://127.0.0.1:9/mcp"},
    }}))
    config = home / ".claude.json"
    existing = json.loads(config.read_text()) if config.exists() else {}
    existing.setdefault("mcpServers", {})["usersrv"] = server(target, "usersrv")
    existing["mcpServers"]["dupsrv"] = server(target, "dup-user")
    existing.setdefault("projects", {})[str(target)] = {"mcpServers": {
        "localsrv": server(target, "localsrv"), "dupsrv": server(target, "dup-local")}}
    config.write_text(json.dumps(existing))


CALLS = ["projsrv", "usersrv", "localsrv", "dupsrv"]
EXPECTED_SERVER = {"projsrv": "projsrv", "usersrv": "usersrv", "localsrv": "localsrv", "dupsrv": "dup-local"}


def expected(target: Path, home: Path, name):
    return {"server": EXPECTED_SERVER[name], "cwd": str(target), "argv": [str(home), "dflt"],
            "env": f"{home}-env", "project": str(target), "claudecode": "1"}
