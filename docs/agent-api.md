# Agent API: Cowboy as a token-frugal MCP surface

Status: read composition and explicit scoped actions implemented, 2026-10-16.
Activation follows the Controller and Machine compatibility gates below.

## Problem

Cowboy owns live session state; Matrix owns durable historical evidence. Native
agents own execution and conversation state. The MCP surface gives an agent
bounded access to Cowboy's sessions, queue, documents and managed calls. The
`cowboy review` CLI remains a single-command shortcut to the same review
contract. Neither surface creates a second review engine or memory store.

The goal is the smallest resident surface that still lets one tool call do a
multi-step job, as Matrix CodeAct does for memory:

- resident cost (tool descriptions and instructions) under 1.5 KiB, enforced
  by a unit test; the full declaration is a resource read only when needed;
- one read program per composition, returning at most 16 KiB it selected;
- zero idle traffic: no notification stream, no polling, no open connection;
- stable error codes instead of stack traces or transcript dumps.

The four complete tool definitions, including JSON schemas and annotations,
plus initialization instructions measure 2,203 UTF-8 bytes on 2026-10-16.
The discovery test enforces a 4 KiB ceiling for that complete surface, in addition
to the description budget. This is a serialization measurement, not a provider
token count, prompt-cache measurement or successful-task cost comparison.

## Native coverage and the residual gap

Claude Code's hosted programmatic tool calling excludes MCP tools, and neither
Codex nor Claude can compose another product's operations server-side. Cowboy
owns sessions, queues, drafts, transcripts and managed calls, so composition
must run beside that state. The local layer can be deleted when both runtimes
compose MCP operations natively with equivalent budgets; the flat operations
would then remain. Claude and Codex share the same stdio MCP server.

## Transport and authority

Zed browsing defaults use `cowboy.preferences.schema()` and
`cowboy.preferences.get("zed")`. The existing `preferences.update` action accepts
`{scope:"zed", folder_id?, expected, change}`: omit `folder_id` for the calling
session, otherwise select an owned directory. Copy its exact `override` object
into `expected`; use `change:{inherit:true}` to remove the override, or
`change:{directory:null|string}` to choose Automatic or a stable code-context ID.
The schema discovers accepted identities and the read response lists registered
projects. Inheritance is session → containing directory → ancestors → Automatic
(actual session worktree, then associated project). These settings affect the
browser only and grant no filesystem access or worker activation. Existing files
stay open until the user switches contexts or opens the default explicitly.

This scope deliberately reuses folder/mobile-review sync and its DB write-behind
queue. Its receipt says **accepted**, not committed, and expires when the
Controller restarts. `cowboy_receipt({domain:"preferences",operation})` can
therefore return unknown; read current settings before retrying with the same
input, expectation and operation. A changed expectation or operation is a new
user decision. Compare/apply and exact-input replay are serialized with browser
mutations. This additive scope works through the existing Machine action gate;
it does not require a worker generation change.

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

Gateway actions `agent_status`, `agent_read` and `agent_invoke` extend the closed managed-call
`Action` enum. Compatibility is explicit:

- Machine protocol 30 adds `Grant.agent_api`. A Controller sets it only for a
  connection at protocol 30 or later and replaces the grant whenever a
  reconnect changes that answer: an older Machine refuses the field, and an
  upgraded Machine refuses Agent API actions under a grant without it.
- A Machine refuses Agent API actions locally (`unsupported_capability`)
  under a grant without the flag, so an older Controller never receives an
  event it cannot decode.
- Protocol 31 adds `Grant.agent_actions` for explicit actions and receipts.
  Both the Machine and Controller check it; reconnecting at a different floor
  replaces the grant. Protocol-30 peers keep the two read tools. An older
  Controller never receives an unknown action variant.
- Discovery reports that authenticated grant's capabilities. A remote session's
  runtime Machine may have a different protocol from its execution Machine;
  querying the runtime's connection would advertise the wrong surface.
- `cowboy mcp` probes `agent_status` once at startup and advertises no tools
  or resources when it fails. Outside a Cowboy session, or before both
  components are updated, a registered server therefore costs no tokens.

Managed child sessions have no grant and no Agent API.

## Tools

| Tool | Program | Purpose |
|---|---|---|
| `cowboy_status` | none | Session view and whether managed calls are enabled |
| `cowboy_read` | TS/JS | Read-only composition over the operations below |
| `cowboy_execute` | none | One explicit domain action with a stable operation ID |
| `cowboy_receipt` | none | Inspect a call or document receipt without a sandbox |

`cowboy_read({help:"index"})` discovers namespaces; `help:"read"` and
`help:"actions"` return declarations without starting a program. Help and code
are mutually exclusive. Definitions stay stable for the MCP process; an
unavailable capability is reported instead of silently falling back.

Resource `cowboy://sdk/v1` serves the strict declaration
([`src/agent_api.rs`](../src/agent_api.rs) `SDK`). `cowboy_read` runs CODE
as `async ({cowboy}) => { CODE }`, or a module exporting a default function
with `format: "module"`; `language: "js"` selects JavaScript.

| Namespace | Operations |
|---|---|
| `session` | `info()`, `queue()`, `drafts()`, `events({after?, before?, limit?, chars?, kinds?})` |
| `calls` | `capabilities()`, `inspect(id)`, `wait(id, ms?)`, `result(id, window?)`, `observe(request_id)` |
| `sessions` | `list({status?, limit?})`, `info(id)`: sessions of the same owner, never system sessions |
| `documents` | `list({after?,limit?,include_deleted?})`, `read(id, window?)`: the owner's Draft documents |
| `preferences` | `schema()`, `get("about")`: typed account preferences and compact ordering choices |

`session.events` projects the retained hot log: message chunks coalesce,
tool-call updates fold into their call, thoughts are omitted and text is
truncated per item. `calls.*` uses the exact managed-call dispatcher, minus
the CLI-only `schema` and `next` fields; a program can observe calls but not
start or cancel them. `calls.wait` is clamped to the program's remaining
budget. A failed operation throws an `Error` whose `code` is stable.

Large document bodies and call-result text can be read with
`{offset:0,chars:1000}`. Offsets count Unicode scalars; `range.next` is the next
offset, or null at the end. Paging never clears a retained result's `truncated`
flag. Document pages carry revision clocks: restart a read if the body revision
changes. List pages use the last document id as `after`, at most 200 requested
items. These are live observations, not a cross-page snapshot.

## Explicit effects and recovery

`cowboy_execute({action,arguments,operation})` invokes one existing domain
writer. It does not evaluate code or promise atomicity across domains.

| Action | Contract |
|---|---|
| `review.start` | Focus, design/code choice, files/root and optional provider/preset; shared CLI review instruction, immutable worktree snapshot, read-only child |
| `calls.start` | Self-contained instruction and existing managed-call policy/approval/admission |
| `calls.cancel` | `call_id`; operation must equal that id; requesting cancellation does not prove termination |
| `documents.mutate` | Create, text write, attachment replacement, rename, move, trash or restore; existing owner-scoped store transaction and revision CAS |
| `preferences.update` | `{scope,expected_revision,changes}`; typed partial update of the current user's DB preferences, with atomic revision and receipt |

### User preferences and configuration coverage

About's Usage Provider order and Machine order are account-owned DB state, shared
by mobile, desktop and agents. `cowboy_read({help:"preferences"})` discovers the
contract without loading unrelated declarations. `preferences.get("about")`
returns the revision, values and compact ID/name choices, without usage details
or resource metrics. `usage_order` and `machine_order` are unique ID arrays:
listed items appear first, unlisted items retain their default order, unavailable
IDs stay dormant, and empty arrays restore defaults. An omitted field is unchanged.

`preferences.update` revalidates the live parent account, applies both fields in
one transaction, and persists the exact result alongside the operation receipt.
A revision conflict returns `current`; retry only after reconciling the user's
intent. An uncertain write reuses identical input and operation, or queries
`cowboy_receipt({domain:"preferences",operation})`. Receipts are parent-scoped.
WebSocket invalidations contain only scope/revision and reach only that user.
UI writes bind the dataset observed by the editor; switching accounts cannot
redirect an old proposal into the new account.

The authenticated CLI can use `cowboy api` against `GET /api/preferences/about`
and `PUT /api/preferences`, supplying the returned dataset ID as the body field
`dataset` (or HTTP header `x-cowboy-dataset`) and an `operation` alongside the update fields. UI, CLI and MCP
reach the same typed domain writer. Display changes are immediate; no Service
or Device restart is needed for subsequent preference edits. Codex and Claude
share this surface. Initial feature rollout requires the updated Controller,
Machine gateway and MCP binary: older gateways retain their closed action list
and refuse `preferences.update`. The authenticated HTTP/CLI path remains the
fallback until those components are updated; restart MCP discovery when adopting
its new help and receipt domain. Retain accepted worker generations during a
resident Machine update.

Future Cowboy settings should expose AI-readable schema and an explicit MCP
action by default, with a documented reason for any exception. Keep storage
ownership: UI-edited user state belongs in DB; Service/Device configuration
remains in the unified configuration framework and retains its validation,
permission and activation contract. Extend domain operations behind the four
existing tools, discover schemas on demand, and return only selected fields or
compact receipts. Do not expose arbitrary file/DB writes as a configuration API.

Call start uses the operation as the durable request id. Document operations
are additionally namespaced by the parent session; their receipt commits in
the same SQL transaction as the mutation. Reusing an id with changed input is
refused. A receipt query is scoped to the live parent and domain. `unknown` is
not proof that the write failed: retry identical input under the same id or
continue observing, never mint a new id to resolve uncertainty.

Document writes recheck the active product account, operator role and current
parent owner. They use `body_revision` for `write_text`/`write`,
`metadata_revision` for rename/move, full `revision` for trash/restore and zero
for create. `write_text` retains attachments; `write` replaces them explicitly.
Conflict replies return current metadata for an intentional retry. Normal
product notifications and recovery history are preserved.

Review remains optional and normally skipped. The reply contract asks for
actionable findings with location, trigger, impact and fix, with no fixed count
or one-line cap. An incomplete, failed or truncated review is not a clean
review. The CLI resolves its current Git root (or explicit `--root`), and an
unresolved admission prints the original request id for `cowboy call observe`.

## Contained runtime

Programs run on the Controller host in a fresh process per call:

- Bubblewrap with every namespace unshared (no network), capabilities dropped,
  a cleared environment, a read-only view of `/nix/store` (and `/usr`, `/lib`
  where present), private `/proc`, a read-only `/dev`, a 1 MiB `/tmp`, and a
  read-only directory holding only the runner and the program;
- `prlimit`: 4 s CPU, 512 MiB data, 64 descriptors, 1 MiB files, no core;
- Bun as the TS/JS engine; it strips TypeScript without type-checking;
- a wall-clock deadline (default 20 s, at most 60 s) that kills the process
  tree; at most two concurrent programs, excess refused with `capacity`.

The runtime reaches Cowboy only through line-delimited frames on its standard
streams. The broker validates every operation and its arguments, allows 16
calls, bounds each reply to 256 KiB and all traffic to 1 MiB. Results over
16 KiB fail with `result_too_large` instead of being truncated into malformed
JSON; console output is returned up to 4 KiB, and a thrown error returns
only a bounded identifier code and a message of at most 2000 characters. The Nix build pins Bubblewrap,
util-linux `prlimit` and Bun; a missing runtime yields `runtime_unavailable`,
never an unsandboxed fallback. Compared with Matrix CodeAct this omits the
seccomp filter and cgroup memory accounting. They remain hardening gates before
any generated program may mutate state. Explicit actions bypass the program
runtime and reach only the closed, authorized domain dispatcher.

## Stages

0. **Read-only (implemented).** Protocol 30, `cowboy_status`, `cowboy_read`,
   the operations above, `cowboy mcp`. Activation needs a Controller release
   and a Machine maintenance release with an accepted worker pin for the wire floor.
1. **Explicit writes (implemented).** Protocol 31, four tools, calls and
   documents with their own receipts. Matrix's in-memory staged batch is not
   copied over remote call launches. Queue mutation still needs a durable
   receipt tied atomically to dispatch/CAS; queue reads remain available.
2. **Automatic delivery.** Provider Plugins register the server through the session
   extension descriptor (`docs/session-extension-plugins.md`) and replace the
   CLI paragraph in the system prompt with one line. Acceptance compares one
   review flow's tokens and round trips with the CLI baseline.
3. **Cross-session actions.** `sessions.prompt` and `sessions.new` (off by
   default, approved like calls), project listing and `cowboy logs` queries.

The CLI remains for Providers without MCP and for people.
