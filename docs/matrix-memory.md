# Matrix Provider integration

Source contract for standard Codex 3.3.0 and Claude 3.4.2. Upstream native CLI,
ACP and authentication versions are unchanged. Installation and native runtime
acceptance are separate release receipts, not implied by this document.

The host user's `.config/matrix/codex.json` or `claude.json` is an explicit
opt-in. It must be a private regular file owned by that runtime user. Controlled
launchers can select `COWBOY_MATRIX_CONFIG`. This is Matrix enrollment, never
a Provider authentication projection. Isolated variants never load these files.

```json
{
  "schema": 1,
  "provider": "codex",
  "endpoint": "http://127.0.0.1:7331",
  "token": "PROVISION_A_PRIVATE_MATRIX_CLIENT_TOKEN",
  "state_dir": "/home/ubuntu/.local/state/matrix-delivery",
  "projects": [
    {"workspace": "cowboy", "machine": "hawk", "project": "cowboy"},
    {"workspace": "cowboy", "machine": "falcon", "project": "cowboy"},
    {"path": "/home/ubuntu/cowboy", "machine": "ovh", "project": "cowboy"}
  ]
}
```

Use actual registered workspace IDs. Remote sessions need exactly one
workspace/executor mapping. Local sessions need an exact enrolled cwd; unknown
worktrees need their own mapping or a session-aware launcher. There is no
directory-name heuristic or wildcard project grant. The service credential
fixes user, Provider and runtime; the descriptor fixes the execution binding.

Codex's private bridge disables native generation/use, adds authenticated HTTP
MCP and recalls bounded memory before each turn. It captures public completed
messages, command results and file changes. Its existing execution bridge still
owns every remote filesystem/process operation.

Claude's remote Mods facade allows only the four exact Matrix MCP tools
alongside existing target tools. Its private socket projects current recall at
prompt construction, including after compaction. Matrix calls run on OVH;
project tools continue through the target keeper. Local Claude retains its ACP
execution surface with Matrix recall/MCP added. Native auto-memory is disabled.

Observations are redacted and bounded before a durable private queue.
Successful delivery is idempotent. Foreground user evidence supports explicit
writes; a completed turn schedules automatic extraction. The 512-file queue
limit gives a visible delivery failure on overflow. An abrupt process death
can lose the final in-memory segment; Matrix's separate transcript hook adapter
can recover it, but Cowboy does not automatically scan native history.

MCP exposes search/read, evidence-backed create/correct and logical forget.
Forget preserves Git history. Physical erasure requires explicit backup/history
handling. No reasoning is collected. Redirects are refused, remote endpoints
require TLS, and tokens never enter signed artifacts, argv or repository data.

Acceptance includes actual native MCP discovery/calls, cross-Provider recall,
correction/forgetting, resume/compaction, outage delivery, scope isolation and
the existing packaged execution gates. A synthetic extraction or client test
alone does not establish native acceptance or improvement over built-in memory.
