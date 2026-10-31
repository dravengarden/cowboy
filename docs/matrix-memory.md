# Matrix Provider integration

Source contract for standard Codex 3.3.2 and Claude 3.4.7, extended to every
Agent and Machine by the [Machine relay](#every-machine-and-every-agent).
Upstream native CLI, ACP and authentication versions are unchanged. Installation
and native runtime acceptance are separate release receipts, not implied by
this document.

## Reliability contract

Memory is never required. A Matrix, relay, Controller, disk or configuration
failure may cost a session its memory; it must never stop, fail or noticeably
stall the session.

- No memory configuration or session fault fails a launch: the session starts
  without memory and the reason goes to the operator log, never to the model.
- A user turn waits for memory at most 5 seconds. After a failed exchange, turns
  skip recall for 30 seconds, so an outage costs one wait rather than one per
  turn. Background delivery probes the service and closes this breaker.
- A turn that recalled nothing carries a short unavailable notice.
- Completed turns stay in a private bounded outbox and are delivered later. A
  full or unwritable outbox loses that evidence, not the turn.
- A failing Matrix MCP server is a failed optional tool, never a startup failure.

## Every Machine and every Agent

The `matrix` Plugin is the opt-in on the Machine that runs the Provider. A
Machine with a local binding (below) talks to Matrix directly. Any other Machine
serves a loopback endpoint with Matrix's own routes, named with a per-session
token in the session descriptor. The Machine sends each request over its
authenticated Cowboy connection. The Controller checks that the session belongs
to that Machine and has memory on, derives the Provider, workspace and execution
Machine from its own record, and forwards the request to `memory.home_machines`.
The home Machine maps that identity to a Matrix binding, using its private
credential for the Provider, and calls Matrix on its loopback.

The relay therefore adds no network path: it inherits whatever route the Machine
uses to reach the Controller, including the approved proxy route, and no Machine
but the home Machine holds a Matrix credential. An unmapped workspace is
addressed by its own id on any Machine; Matrix still checks the project against
the Provider's grant.

| Agent | Delivery | Matrix client |
|---|---|---|
| Claude | Plugin adapter | `claude` |
| Codex | Plugin adapter | `codex` |
| Grok | ACP worker | `grok` |
| DeepSeek Harness | ACP worker | `deepseek-harness` |
| Claude DeepSeek | ACP worker | `claude-deepseek` |
| Codex DeepSeek | ACP worker | `codex-deepseek` |

The ACP worker declares Matrix through ACP `mcpServers`, prepends recall to the
prompt, and observes the session's own updates. Each client has its own
credential and private provider scope and shares user, project and machine
scopes with the others. Its home binding is
`$XDG_CONFIG_HOME/cowboy/memory/matrix/<client>.json` on the home Machine.

The relay requires Machine protocol 41 on the runtime and home Machines. Sessions
started before their Machine's upgrade keep their previous behavior.

## Bindings

Matrix is the first-party `matrix` memory-backend Plugin (Codex 3.4.7 and
Claude 3.19.36 onward, see [session extension Plugins](session-extension-plugins.md)).
Installing it on the Machine that runs the Provider and enabling Memory in the
session's Tools settings is the opt-in. The Machine passes a private
session-extension descriptor; the Provider then reads the private binding at
`$XDG_CONFIG_HOME/cowboy/memory/matrix/codex.json` or `claude.json`, and for
compatibility the host user's `.config/matrix/<provider>.json`. With a
descriptor, the Memory tools and Automatic recall switches decide whether the
MCP tools are exposed and whether context is recalled before each turn.
Without one (a Machine older than session extensions) the host enrollment
applies with both on. Every file must be a private regular file owned by that
runtime user; controlled launchers can select `COWBOY_MATRIX_CONFIG` for the
legacy path. This is memory enrollment, never a Provider authentication
projection. Isolated variants never load these files.

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

Claude's remote Mods facade allows only the seven exact Matrix MCP tools
alongside existing target tools. Its input bridge retrieves fresh context before
each user turn, including after compaction; native cached project instructions
are not used for dynamic recall. Matrix calls run on OVH;
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

Matrix 0.3.0 retains the original `memory_search`, `memory_get`, `memory_put`
and `memory_forget` tools, adding `memory_read`, `memory_execute` and
`memory_receipt`. `memory_execute` accepts permissive TS or JS, compiles inside
the service and runs a fresh contained V8 instance. The model-facing SDK remains
strictly typed and runtime inputs remain validated. It can compose scoped reads,
staged atomic writes and lazy shell processing in one call. Shell runs against
private scratch files on the Matrix host; target project commands continue to
use Cowboy's execution tools. The direct query path avoids sandbox startup.

Codex discovers the new MCP tools dynamically. Claude 3.4.7 allows the three
additional exact names; arbitrary Matrix-prefixed names remain refused.
Existing workers retain their signed generation, so the new Claude routing
applies to new sessions. The old Python read-only code path remains compatible.
Matrix's draft protocol support retains older native-client negotiation.
Both packaged native clients and Remote execution passed the
[CodeAct acceptance](releases/matrix-codeact-2026-10-05.md).

The independently versioned [Matrix performance contract][matrix-performance]
and [CodeAct contract][matrix-codeact] own the draft pin, SDK, sandbox limits,
private metrics, benchmark method and weekly feedback review. Matrix product
activation is separate from Cowboy Controller and Machine activation.

[matrix-performance]: https://github.com/dravengarden/matrix/blob/main/docs/performance.md
[matrix-codeact]: https://github.com/dravengarden/matrix/blob/main/docs/codeact-runtime.md
