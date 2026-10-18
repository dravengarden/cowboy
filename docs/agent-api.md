# Agent API: Cowboy as a token-frugal MCP surface

Status: product control catalog, durable admission receipts and API coverage
guards implemented, 2026-10-18. Activation follows the Controller and Machine
compatibility gates below. Agents prefer MCP for Cowboy state and control; the
CLI remains supported for people, unavailable capabilities and host operations.

## Problem

Cowboy owns live session state; Matrix owns durable historical evidence. Native
agents own execution and conversation state. The MCP surface gives an agent
bounded access to Cowboy's sessions, queue, usage, documents and managed calls.
The `cowboy review` CLI remains a single-command shortcut to the same review
contract. Neither surface creates a second review engine or memory store.

The goal is the smallest resident surface that still lets one tool call do a
multi-step job, as Matrix CodeAct does for memory:

- resident cost (tool descriptions and instructions) under 1.5 KiB, enforced by
  a unit test; the full declaration is a resource read only when needed;
- one read program per composition, returning at most 16 KiB it selected;
- zero idle traffic: no notification stream, no polling, no open connection;
- stable error codes instead of stack traces or transcript dumps.

The four complete tool definitions, including JSON schemas and annotations, plus
initialization instructions are bounded to 4 KiB. The 2026-10-17 baseline was
2,184 UTF-8 bytes; additions to the operation catalog do not add tool schemas. The discovery test
enforces a 4 KiB ceiling for that complete surface, in addition to the
description budget. This is a serialization measurement, not a provider token
count, prompt-cache measurement or successful-task cost comparison.

## Native coverage and the residual gap

Claude Code's hosted programmatic tool calling excludes MCP tools, and neither
Codex nor Claude can compose another product's operations server-side. Cowboy
owns sessions, queues, drafts, transcripts and managed calls, so composition
must run beside that state. The local layer can be deleted when both runtimes
compose MCP operations natively with equivalent budgets; the flat operations
would then remain. Claude and Codex share the same stdio MCP server.

## Transport and authority

Zed browsing defaults use `cowboy.preferences.schema()` and
`cowboy.preferences.get("zed")`. The existing `preferences.update` action
accepts `{scope:"zed", folder_id?, expected, change}`: omit `folder_id` for the
calling session, otherwise select an owned directory. Copy its exact `override`
object into `expected`; use `change:{inherit:true}` to remove the override, or
`change:{directory:null|string}` to choose Automatic or a stable code-context
ID. The schema discovers accepted identities and the read response lists
registered projects. Inheritance is session → containing directory → ancestors →
Automatic (actual session worktree, then associated project). These settings
affect the browser only and grant no filesystem access or worker activation.
Existing files stay open until the user switches contexts or opens the default
explicitly.

This scope deliberately reuses folder/mobile-review sync and its DB write-behind
queue. Its receipt says **accepted**, not committed. The 4,096 most recently
used receipts are retained until eviction or Controller restart. An evicted
operation still in the sync dedupe window returns `receipt_expired` instead of
acknowledging an unverified replay.
`cowboy_receipt({domain:"preferences",operation})` can therefore return unknown;
read current settings before retrying with the same input, expectation and
operation. A changed expectation or operation is a new user decision.
Compare/apply and exact-input replay are serialized with browser mutations. This
additive scope works through the existing Machine action gate; it does not
require a worker generation change.

```text
agent ── stdio MCP ──> cowboy mcp ── COWBOY_CALL_CONTEXT socket ──> Machine gateway
      ── authenticated Machine channel ──> Controller: authorize, then act
```

`cowboy mcp` reuses the managed-call gateway every ordinary session already has.
It adds no credential: the Controller accepts an action only on the grant
installed for that Machine connection and after re-deriving authority from the
live parent worker. Each `tools/call` is one gateway exchange; the server holds
nothing open between calls, so an idle server costs nothing even through the
Claude remote MCP relay.

Gateway actions `agent_status`, `agent_read` and `agent_invoke` extend the
closed managed-call `Action` enum. Compatibility is explicit:

- Machine protocol 30 adds `Grant.agent_api`. A Controller sets it only for a
  connection at protocol 30 or later and replaces the grant whenever a reconnect
  changes that answer: an older Machine refuses the field, and an upgraded
  Machine refuses Agent API actions under a grant without it.
- A Machine refuses Agent API actions locally (`unsupported_capability`) under a
  grant without the flag, so an older Controller never receives an event it
  cannot decode.
- Protocol 31 adds `Grant.agent_actions` for explicit actions and receipts. Both
  the Machine and Controller check it; reconnecting at a different floor
  replaces the grant. Protocol-30 peers keep the two read tools. An older
  Controller never receives an unknown action variant.
- Protocol 32 makes action names and receipt domains extensible behind the
  existing envelope. The Controller remains the closed authority/operation
  dispatcher. Older Machines retain their original operations and all compatible
  reads; `status.control.writes` is false for the new catalog until the execution
  Machine reaches 32. An updated MCP executable is also required: its status
  includes `client.action_catalog:true`; an older executable still validates its
  original action allowlist. Future catalog additions need only a Controller
  change, not another protocol bump or resident tool.
- Discovery reports that authenticated grant's capabilities. A remote session's
  runtime Machine may have a different protocol from its execution Machine;
  querying the runtime's connection would advertise the wrong surface.
- `cowboy mcp` probes `agent_status` once at startup. Outside a Cowboy session,
  with invalid wiring or an explicitly unsupported peer, it advertises no tools
  or resources. A bound parent whose gateway is starting or reconnecting retains
  the four compact tool definitions: worker readiness can depend on MCP
  initialization, so waiting for the grant here would create a startup cycle.
  Definitions confer no authority; every call re-reads the context and requires
  the current Controller grant. No background polling is added.

Managed child sessions have no grant and no Agent API.

## Tools

| Tool             | Program | Purpose                                               |
| ---------------- | ------- | ----------------------------------------------------- |
| `cowboy_status`  | none    | Session view and whether managed calls are enabled    |
| `cowboy_read`    | TS/JS   | Read-only composition over the operations below       |
| `cowboy_execute` | none    | One explicit domain action with a stable operation ID |
| `cowboy_receipt` | none    | Inspect a call or document receipt without a sandbox  |

`cowboy_read({help:"index"})` discovers namespaces; `help:"read"` and
`help:"actions"` return declarations from the current Controller via the same
bounded read broker (`cowboy.api.help(topic)`). Help and code are mutually
exclusive. No bundled-SDK fallback can falsely advertise a newer capability.
`cowboy_status` and help return a content revision for reusing known
declarations. The four tool definitions stay stable for the MCP process; an
unavailable capability is reported instead of silently falling back. Use
`help:"session"`, `"sessions"`, `"usage"`, `"calls"`, `"machines"`,
`"configuration"`, `"documents"` or `"preferences"` for just that domain. Read
topics include their own type dependencies and share one declaration source with
the full SDK. Each of the seven read topics is tested to cost less than half of
full read discovery.

Resource `cowboy://sdk/v1` also obtains the current Controller's strict
declaration ([`src/agent_api.rs`](../src/agent_api.rs) `SDK`). `cowboy_read`
runs CODE as `async ({cowboy}) => { CODE }`, or a module exporting a default
function with `format: "module"`; `language: "js"` selects JavaScript.

| Namespace       | Operations                                                                                                                                                                 |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session`       | `info()`, `usage()`, `queue(query?)`, `drafts(query?)`, `message(id, window?)`, `events(query?)`, `history(query?)`, `event(seq, selection?)`, `search(query)`             |
| `calls`         | `list({before?,limit?})`, `capabilities()`, `inspect(id)`, `wait(id, ms?)`, `result(id, window?)`, `observe(request_id)`                                                   |
| `sessions`      | `list({after?,status?,limit?,query?,provider?,machine?,workspace?})` and the current-session reads with a leading session ID: same owner, never system or closing sessions |
| `usage`         | `list({after?,limit?})`, `read(account, {section?,path?,offset?,limit?,describe?})`, `diagnostics(selection?)`: cached account observations and collector placement        |
| `machines`      | `list({after?,limit?})`, `read(id, {section?,...selection})`: resources, installed Plugins, workspaces and capabilities                                                    |
| `configuration` | `schema("service"\|"device", selection?)`, `read("service"\|"session"\|"tools", {session_id?,...selection})`: effective values and their owning schema                     |
| `documents`     | `list({after?,limit?,include_deleted?})`, `read(id, window?)`: the owner's Draft documents                                                                                 |
| `preferences`   | `schema()`, `get("about")`: typed account preferences and compact ordering choices                                                                                         |

`session.events` projects the retained hot log: message chunks coalesce,
tool-call updates fold into their call, thoughts are omitted and text is
truncated per item. `calls.*` uses the exact managed-call dispatcher, minus the
CLI-only `schema` and `next` fields; a program can observe calls but not start
or cancel them. `calls.wait` is clamped to the program's remaining budget. A
failed operation throws an `Error` whose `code` is stable. Each broker operation
rechecks the live parent and, for owned sessions, the active product account. A
long-running read cannot continue using an owner snapshot after the parent
closes or its account is disabled.

Large document bodies and call-result text can be read with
`{offset:0,chars:1000}`. An unwindowed read returns a small body whole, or
`text_window_required` above 1,000 characters. It never silently turns a full
read into a prefix: an older caller might otherwise append to that prefix and
overwrite the document's remainder, or mistake partial findings for a complete
review. An explicit window defaults to 1,000 characters; at most 4,000 may be
requested per page. Offsets count Unicode scalars; `range.next` is the next
offset, or null at the end. Paging never clears a retained result's `truncated`
flag. Document pages carry revision clocks: restart a read if the body revision
changes. Document and Session lists are ordered by ID and use the last returned
ID as `after`. Sessions return 50 items by default (maximum 200). Documents
require an explicit `limit` when more than 50 entries remain, returning
`page_required` otherwise; small unpaged lists remain complete. Use `{limit:50}`
(maximum 200), then fetch until a page is shorter than that limit. Document
pagination and tombstone filtering run in the owning database, loading only that
page's metadata. These are live observations, not a cross-page snapshot.

Queue/draft lists retain their queue order. Use `{limit:20,chars:400}` for
previews (maximum 100 messages and 4,000 characters each). Without an explicit
`limit`, more than 20 remaining messages returns `page_required`; without
explicit `chars`, a message over 400 characters returns `text_window_required`.
Small legacy reads remain complete. Use the last ID as `after`, and
`session.message(id,{offset,chars})` to continue a message's text. A cursor
removed by dispatch or editing returns `invalid_cursor`; restart the list. Text
windows have no snapshot guarantee when a queued message is edited.

Transcript pages include `next:{after}` or `next:{before}`, or null at the
retained end. Carry it into the next query together with the original filters
and limits. Empty filtered pages still advance through the scanned raw window.
Never resume from a tool item's `to`: its updates can span other messages. A
tool may appear again as a partial update on a later page; merge by tool ID and
sequence, retaining the newest fields when reading backwards. Page boundaries
are chosen from chronological message/tool-update groups before folding tools
inside the selected interval. This preserves a tool's final update even when its
start lies on an earlier page, while keeping streaming chunks together.
`reached_start:false` at the end means older history is no longer retained. Zero
list limits are rejected.

### Other sessions and archived history

The plural `sessions` methods use the same bounds and projections as `session`.
An explicit target must have the caller's owner, including the anonymous local
owner when applicable. System, closing, foreign and missing targets all return
`not_found`. Queue and live-log ownership checks happen under the lock copying
the data; archived reads recheck target visibility and the active parent after
storage I/O. Reading does not start, resume or hydrate a worker.

`events` reads the retained live window. `history` reads saved history through
the existing byte-bounded storage reader, scanning at most 64 raw events per
call. It accepts `before`, `limit`, `chars` and `kinds`, pages backwards through
`next.before`, and includes `source:"durable"`. Without persistence it returns
the bounded in-memory history with `source:"memory"`; `reached_start:false`
still identifies missing older history. Saved history may lag the live window.
Streamed messages and tools can span raw pages; combine their sequences when
assembling a conversation. Both surfaces omit thoughts and retain the same
per-item text limits.

`session.usage()` and `sessions.usage(id)` return that session's reported
context/cost payload and its observation time, plus the account key declared by
its exact Provider version and digest. Missing usage and unknown account
bindings remain null. The Provider ID (for example `codex`) is not assumed to be
the account key (for example `openai`). Session context consumption is not
account quota or a total billed-token counter.

### Account usage and quota

`usage.list()` lists the active Plugin-defined account keys, labels, status,
source, observation/refresh metadata and available sections. It returns 20
accounts by default (maximum 100), in account-ID order; use `next` as `after`.
It does not include account payloads or activity history. Account observations
are Service-wide, matching the product Usage API: on an authenticated Service
the parent must belong to an active product user. An unowned legacy session does
not gain this access.

`usage.read(account)` defaults to the `quota` section: the Provider's cached
`rate_limits` payload. Select `section:"account"` for its reported account/plan
or balance information, or `section:"activity"` for its cached activity. Cowboy
preserves Provider fields and units instead of adding another parser. For
example, `usedPercent` means used, `windowDurationMins` is minutes, and
`resetsAt` is epoch seconds; `observed_at_ms` is milliseconds. Daily activity is
not evidence of consumption within the current plan window. A missing section is
`available:false` with null data, never an invented zero or 100%. Quota inferred
only from a live session remains available through that session's
`usage().usage.raw`, separately from the cached account observation.

Use a JSON pointer such as `path:"/rateLimits/primary"` or
`path:"/last24Hours/summary"` to select before copying. `page.data` contains at
most 50 object entries or array elements by default (maximum 200); object keys
sort lexically, array order is preserved. Resume using `page.range.next` as
`offset`, retaining the same section/path. These are live cached observations,
not snapshots: compare observation times and restart if consistency matters.
`describe:true` returns field types and sizes instead of their contents, useful
for new Plugins or large activity records. A selected page above 32 KiB returns
`selection_too_large`; inspect its shape and select a deeper path. The final
program result still has the independent 16 KiB limit.

These operations project the existing cache without cloning unrelated account
activity, starting a collector, refreshing a quota, or spending reset credits.
`refreshing`, `refresh.stale`, observation timestamps and retained errors stay
visible. Normal UsageService collection keeps its existing schedule. An explicit
refresh still uses the authenticated Usage HTTP/CLI surface.

For example, one read program can resolve the current account and select only
its reported quota data:

```ts
const current = await cowboy.session.usage();
if (!current.account) return { account: null, usage: current.usage };
const quota = await cowboy.usage.read(current.account);
return {
  account: current.account,
  observed_at_ms: quota.observed_at_ms,
  refresh: quota.refresh,
  available: quota.available,
  quota: quota.page,
};
```

### Coverage boundary

`sessions.list` combines exact Provider/runtime-Machine/workspace filters with
case-insensitive literal metadata search (`query`, at most 256 bytes).
Pagination still uses stable ID order. Session views name the runtime placement
and the separate execution placement; neither is inferred from the other.

`session.event(seq)` describes a raw saved or live event behind a summary.
Select a JSON pointer to read its message blocks, tool inputs/outputs or
attachment references. Strings use Unicode scalar `offset`/`chars`; objects and
arrays use `offset`/`limit`. Details carry a content `revision`; send it as
`expected` on continuation so a changed event returns `revision_conflict`. This
recovers every retained block across a summary's `seq..to` span without putting
full output or binary attachment bodies in a list response.

`session.search({query,before?,limit?,chars?})` scans at most 64 saved records
per call and returns the first literal, case-sensitive string match per event,
with its exact JSON pointer, Unicode offset, content revision and a short
excerpt. It searches individual stored string values, including tool output; it
does not join strings across events. Follow `next_before` even on an empty
result page. The plural `sessions` methods use the same owner checks before and
after storage I/O. No search or detail read revives a worker.

Machine lists include timestamped resource observations and worker capacity,
separate from historical session counts. Installed Plugin details, workspaces,
configuration and collector diagnostics use bounded JSON selection: omitted
paths describe shape, explicit paths fetch values, and oversized selections
request a deeper path. Service values come from the live unified config handle;
session options and Tools defaults/overrides reuse the same owners as the UI.

### Product control catalog

`cowboy.control.list()` returns domain counts and explicit owner boundaries.
`list("sessions")` lists only that domain; `describe("sessions.configure")`
returns its input and current availability. Discovery accounts for the live
product role and execution Machine protocol. It never grants authority.
Use `cowboy_read({help:"control"})` once, then reuse the API revision.

The catalog covers ordinary session creation, remote placement, configuration,
wake/hibernate/reload, queue/draft edits and scheduling, shared folders/order,
usage refresh/reset/schedule/executor, registered projects and placement policy,
Plugin inventory/lifecycle/commands, diagnostics and Service configuration.
[The coverage ledger](../contracts/agent-api-coverage.json) maps every product HTTP handler,
WebSocket command and merged protocol router to its MCP operation or concrete
owner exception. Tests reject unclassified additions and stale entries.

Read through `control.read(name,input,selection)`. Its default is a shape
description, not a full payload. Select a JSON pointer, a short string window
or an array page; the same bounds apply to document history, log detail and
Plugin results. The adapter allows up to 16 MiB of internal handler data, then
selects before the 256 KiB broker and 16 KiB final program limits. Oversized
selection is explicit; no silently truncated JSON is presented as complete.

Writes use one stable identity:

```json
{"action":"control.apply","operation":"rename-session-1","arguments":{"name":"sessions.rename","input":{"session_id":"owned-id","title":"Review API"}}}
```

The adapter invokes existing product handlers/typed command writers. It accepts
no arbitrary URL, headers, cookies, SQL, shell command or host delegation.
An active product Operator can write; Service configuration and shared Tools
defaults require Owner. Session targets remain same-owner, non-system and
non-closing even for an Owner; nested sync targets and draft moves follow the
same boundary. Reads recheck the original permission observation after I/O.
Self-cancellation, deletion, reset, reload and force-send are refused so the
caller does not interrupt the operation it is trying to observe. Human approval,
credentials/login, Device-local configuration, binary maintenance and recovery
writer promotion keep their owning UI/CLI. Native file/Git/agent execution stays
with native tools; browser leases and telemetry ingestion keep their protocols.

Before any effect, a SQL admission row binds owner, parent, operation and exact
input hash. Duplicate identical calls return the saved result; changed input is
a conflict. Slow submissions return a pending receipt after 500 ms, before the
gateway transport deadline. The admitted task survives caller disconnect. Its result is saved
independently of the domain effect: a crash between those events leaves
`pending/outcome:unknown`, **never permission to repeat under a new ID**.
`cowboy_receipt({domain:"control",operation})` observes this ledger. Results over
8 KiB return a compact pointer to `control.read("operations.receipt",
{operation},selection)`; use it to continue the original result.
Controller-managed Plugin install/uninstall also retain their original domain
operation identity and use the existing live authority continuation.

An `accepted` receipt means the domain accepted submission. Queue persistence,
remote cancellation/deletion, Plugin installation and worker readiness must be
observed through their domain state. This is not an exactly-once transaction
across external systems. Completed and uncertain admissions survive Controller
restart and PostgreSQL/SQLite copy; rows are not evicted by a small in-memory
cache.

`configuration.service` returns the stored file revision and overrides.
`configuration.service_set` accepts that revision plus declared dotted keys;
null removes an override. It uses the same interprocess lock, validation,
history, atomic rename and fsync as `cowboy config install`. A stale revision
is refused; unrelated overrides remain, while TOML formatting/comments may be
normalized. The existing Service watcher owns live activation; restart fields
wait for restart. Use `configuration.read("service")` to observe effective
values. Account preferences, session/Tools settings and folders retain their
existing DB ownership.

Usage reads consume cached observations. `usage.refresh` enters the existing
UsageService request path, with its persisted per-account cooldown and in-flight
deduplication; it returns immediately and never starts a second polling loop.
Credit resets retain the explicit confirmation/credit identity contract.

The stdio server negotiates MCP through `2025-11-25` and keeps the earlier
versions. It bounds input frames to 1 MiB and concurrent requests to 16, applies
backpressure, and drains an oversized frame before accepting another request.
Malformed JSON and invalid requests receive bounded JSON-RPC errors; valid
notifications remain silent. Framing follows the
[MCP stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

## Explicit effects and recovery

`cowboy_execute({action,arguments,operation})` invokes one existing domain
writer. It does not evaluate code or promise atomicity across domains.

| Action               | Contract                                                                                                                                        |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `review.start`       | Focus, design/code choice, files/root and optional provider/preset; shared CLI review instruction, immutable worktree snapshot, read-only child |
| `calls.start`        | Self-contained instruction and existing managed-call policy/approval/admission                                                                  |
| `calls.cancel`       | `call_id`; operation must equal that id; requesting cancellation does not prove termination                                                     |
| `documents.mutate`   | Create, text write, attachment replacement, rename, move, trash or restore; existing owner-scoped store transaction and revision CAS            |
| `preferences.update` | `{scope,expected_revision,changes}`; typed partial update of the current user's DB preferences, with atomic revision and receipt                |
| `control.apply` | `{name,input}` from the on-demand catalog; durable admission and compact original-operation receipts |

### User preferences and configuration coverage

About's Usage Provider order and Machine order are account-owned DB state,
shared by mobile, desktop and agents. `cowboy_read({help:"preferences"})`
discovers the contract without loading unrelated declarations.
`preferences.get("about")` returns the revision, values and compact ID/name
choices, without usage details or resource metrics. `usage_order` and
`machine_order` are unique ID arrays: listed items appear first, unlisted items
retain their default order, unavailable IDs stay dormant, and empty arrays
restore defaults. An omitted field is unchanged.

`preferences.update` revalidates the live parent account, applies both fields in
one transaction, and persists the exact result alongside the operation receipt.
A revision conflict returns `current`; retry only after reconciling the user's
intent. An uncertain write reuses identical input and operation, or queries
`cowboy_receipt({domain:"preferences",operation})`. Receipts are parent-scoped.
WebSocket invalidations contain only scope/revision and reach only that user. UI
writes bind the dataset observed by the editor; switching accounts cannot
redirect an old proposal into the new account.

The authenticated CLI can use `cowboy api` against `GET /api/preferences/about`
and `PUT /api/preferences`, supplying the returned dataset ID as the body field
`dataset` (or HTTP header `x-cowboy-dataset`) and an `operation` alongside the
update fields. UI, CLI and MCP reach the same typed domain writer. Display
changes are immediate; no Service or Device restart is needed for subsequent
preference edits. Codex and Claude share this surface. Initial feature rollout
requires the updated Controller, Machine gateway and MCP binary: older gateways
retain their closed action list and refuse `preferences.update`. The
authenticated HTTP/CLI path remains the fallback until those components are
updated; restart MCP discovery when adopting its new help and receipt domain.
Retain accepted worker generations during a resident Machine update.

Future Cowboy settings should expose AI-readable schema and an explicit MCP
action by default, with a documented reason for any exception. Keep storage
ownership: UI-edited user state belongs in DB; Service/Device configuration
remains in the unified configuration framework and retains its validation,
permission and activation contract. Extend domain operations behind the four
existing tools, discover schemas on demand, and return only selected fields or
compact receipts. Do not expose arbitrary file/DB writes as a configuration API.

Call start uses the operation as the durable request id. Document operations are
additionally namespaced by the parent session; their receipt commits in the same
SQL transaction as the mutation. Reusing an id with changed input is refused. A
receipt query is scoped to the live parent and domain. `unknown` is not proof
that the write failed: retry identical input under the same id or continue
observing, never mint a new id to resolve uncertainty.

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

- Bubblewrap with every namespace unshared (no network), capabilities dropped, a
  cleared environment, a read-only view of `/nix/store` (and `/usr`, `/lib`
  where present), private `/proc`, a read-only `/dev`, a 1 MiB `/tmp`, and a
  read-only directory holding only the runner and the program;
- `prlimit`: 4 s CPU, 512 MiB data, 64 descriptors, 1 MiB files, no core;
- Bun as the TS/JS engine; it strips TypeScript without type-checking;
- a wall-clock deadline (default 20 s, at most 60 s) that kills the process
  tree; at most two concurrent programs, excess refused with `capacity`.

The runtime reaches Cowboy only through line-delimited frames on its standard
streams. The broker validates every operation and its arguments, allows 16
calls, bounds each reply to 256 KiB and all traffic to 1 MiB. Results over 16
KiB fail with `result_too_large` instead of being truncated into malformed JSON;
console output is returned up to 4 KiB, and a thrown error returns only a
bounded identifier code and a message of at most 2000 characters. The Nix build
pins Bubblewrap, util-linux `prlimit` and Bun; a missing runtime yields
`runtime_unavailable`, never an unsandboxed fallback. Compared with Matrix
CodeAct this omits the seccomp filter and cgroup memory accounting. They remain
hardening gates before any generated program may mutate state. Explicit actions
bypass the program runtime and reach only the closed, authorized domain
dispatcher.

## Stages

0. **Read-only (implemented).** Protocol 30, `cowboy_status`, `cowboy_read`, the
   operations above, `cowboy mcp`. Activation needs a Controller release and a
   Machine maintenance release with an accepted worker pin for the wire floor.
1. **Explicit writes (implemented).** Protocol 31, four tools, calls and
   documents with their own receipts. Matrix's in-memory staged batch is not
   copied over remote call launches. Product catalog effects use a separate
   durable admission ledger with explicit uncertain outcomes (stage 3).
2. **Automatic delivery (implemented).** Ordinary
   Codex and Claude sessions register the session Cowboy MCP without changing
   user configuration or copying gateway credentials. Local sessions inherit
   `COWBOY_CALL_CONTEXT`; Codex Remote uses native MCP `environment_id`, and
   Claude Remote uses its existing target MCP relay. Codex asks native for
   configured server names once and uses `cowboy`, or a free `cowboy_session`
   suffix when that name exists; it disables the old reserved `cowboy` entry for
   this process so native table merging cannot retain its URL or wiring.
   The pinned native CLI restarts its `mcp-stdio-N` process counter after a cold
   resume; the target executor retains those IDs. A bounded authenticated
   loopback relay namespaces only these temporary MCP handles per native process,
   retaining the namespace across transport reconnects. Other process handles,
   effect requests and recovery remain native-owned. Remove this compatibility
   layer when the native MCP launcher allocates IDs unique across cold resumes.
   Claude's native session override replaces the reserved name. Managed children
   omit registration and retain their existing read-only isolation. Every
   operation still requires the live core grant. Publishing these Plugin
   versions is separate from installing them on a Machine; existing sessions
   retain their pinned generation. Review 1.0.2 prefers MCP with CLI fallback;
   it retains optional, at-most-once review and on-demand action help.
3. **Product control (implemented).** Protocol 32; dynamic domain catalog,
   same-owner session actions, configuration, projects, lifecycle, usage and
   diagnostics, using existing writers and durable admission receipts. Read
   composition cannot invoke these writes.

The CLI remains supported. Agents use it when live MCP capabilities are absent
or an operation belongs to an explicit host/native/identity boundary.
