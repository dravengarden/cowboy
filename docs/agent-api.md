# Agent API: Cowboy as a token-frugal MCP surface

Status: Stage 0 (read-only) implemented, 2026-10-15. Later stages are design.

## Problem

An agent inside a Cowboy session reaches Cowboy only through shell commands:
`cowboy codex`, `cowboy call capabilities|wait|result`. One review is four or
five Bash round trips, each returning a JSON envelope, and the system prompt
carries a paragraph of CLI instructions on every turn. The agent cannot see its
own queue, drafts, retained transcript or sibling sessions at all.

The goal is the smallest resident surface that still lets one tool call do a
multi-step job, as Matrix CodeAct does for memory:

- resident cost (tool descriptions and instructions) under 1.5 KiB, enforced
  by a unit test; the full declaration is a resource read only when needed;
- one program per job, returning at most 16 KiB that the program selected;
- zero idle traffic: no notification stream, no polling, no open connection;
- stable error codes instead of stack traces or transcript dumps.

## Native coverage and the residual gap

Claude Code's hosted programmatic tool calling excludes MCP tools, and neither
Codex nor Claude can compose another product's operations server-side. Cowboy
owns sessions, queues, drafts, transcripts and managed calls, so composition
must run beside that state. The local layer can be deleted when both runtimes
compose MCP operations natively with equivalent budgets; the flat operations
would then remain. Claude and Codex share the same stdio MCP server.

## Transport and authority

```text
agent ── stdio MCP ──> cowboy mcp ── COWBOY_CALL_CONTEXT socket ──> Machine gateway
      ── authenticated Machine channel ──> Controller: authorize, then act
```

`cowboy mcp` reuses the managed-call gateway every ordinary session already
has. It adds no credential: the Controller accepts an action only on the
grant installed for that Machine connection and after re-deriving authority
from the live parent worker. Each `tools/call` is one gateway exchange; the
server holds nothing open between calls, so an idle server costs nothing even
through the Claude remote MCP relay.

Gateway actions `agent_status` and `agent_read` extend the closed managed-call
`Action` enum. Compatibility is explicit:

- Machine protocol 30 adds `Grant.agent_api`. A Controller sets it only for a
  connection at protocol 30 or later and replaces a flagged grant when the
  Machine reconnects lower, because an older Machine refuses the field.
- A Machine refuses Agent API actions locally (`unsupported_capability`)
  under a grant without the flag, so an older Controller never receives an
  event it cannot decode.
- `cowboy mcp` probes `agent_status` once at startup and advertises no tools
  or resources when it fails. Outside a Cowboy session, or before both
  components are updated, a registered server therefore costs no tokens.

Managed child sessions have no grant and no Agent API.

## Tools

| Tool | Program | Purpose |
|---|---|---|
| `cowboy_status` | none | Session view and whether managed calls are enabled |
| `cowboy_read` | TS/JS | Read-only composition over the operations below |

Resource `cowboy://sdk/v1` serves the strict declaration
([`src/agent_api.rs`](../src/agent_api.rs) `SDK`). `cowboy_read` runs CODE
as `async ({cowboy}) => { CODE }`, or a module exporting a default function
with `format: "module"`; `language: "js"` selects JavaScript.

| Namespace | Operations |
|---|---|
| `session` | `info()`, `queue()`, `drafts()`, `events({after?, before?, limit?, chars?, kinds?})` |
| `calls` | `capabilities()`, `inspect(id)`, `wait(id, ms?)`, `result(id)`, `observe(request_id)` |
| `sessions` | `list({status?, limit?})`, `info(id)`: sessions of the same owner, never system sessions |
| `documents` | `list()`, `read(id)`: the owner's Draft documents |

`session.events` projects the retained hot log: message chunks coalesce,
tool-call updates fold into their call, thoughts are omitted and text is
truncated per item. `calls.*` uses the exact managed-call dispatcher, minus
the CLI-only `schema` and `next` fields; a program can observe calls but not
start or cancel them. `calls.wait` is clamped to the program's remaining
budget. A failed operation throws an `Error` whose `code` is stable.

## Contained runtime

Programs run on the Controller host in a fresh process per call:

- Bubblewrap with every namespace unshared (no network), capabilities dropped,
  a cleared environment, a read-only view of `/nix/store` (and `/usr`, `/lib`
  where present), private `/proc`, `/dev` and `/tmp`, and a read-only
  directory holding only the runner and the program;
- `prlimit`: 4 s CPU, 512 MiB data, 64 descriptors, 1 MiB files, no core;
- Bun as the TS/JS engine; it strips TypeScript without type-checking;
- a wall-clock deadline (default 20 s, at most 60 s) that kills the process
  tree; at most two concurrent programs, excess refused with `capacity`.

The runtime reaches Cowboy only through line-delimited frames on its standard
streams. The broker validates every operation and its arguments, allows 16
calls, bounds each reply to 256 KiB and all traffic to 1 MiB. Results over
16 KiB fail with `result_too_large` instead of being truncated into malformed
JSON; console output is returned up to 4 KiB. The Nix build pins Bubblewrap,
util-linux `prlimit` and Bun; a missing runtime yields `runtime_unavailable`,
never an unsandboxed fallback. Compared with Matrix CodeAct this omits the
seccomp filter and cgroup memory accounting; both are Stage 1 hardening gates
before any program may mutate state.

## Stages

0. **Read-only (implemented).** Protocol 30, `cowboy_status`, `cowboy_read`,
   the operations above, `cowboy mcp`. Activation needs a Controller release
   and a Machine maintenance release; the worker generation is unchanged.
1. **Writes.** `cowboy_execute(code, operation)` and `cowboy_receipt`, with
   Matrix's staged, atomic, operation-identified semantics: `calls.start` and
   `cancel` (keeping the existing approval flow), queue add/edit/remove with
   `cmid` idempotency, Draft document writes with `body_revision` CAS. Queue
   and documents default on; seccomp and cgroup limits land first.
2. **Delivery.** Provider Plugins register the server through the session
   extension descriptor (`docs/session-extension-plugins.md`) and replace the
   CLI paragraph in the system prompt with one line. Acceptance compares one
   review flow's tokens and round trips with the CLI baseline.
3. **Cross-session actions.** `sessions.prompt` and `sessions.new` (off by
   default, approved like calls), project listing and `cowboy logs` queries.

The CLI remains for Providers without MCP and for people.
