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

The 2026-10-18 live check measured 2,226 bytes for the four complete definitions
and initialization instructions: 495 tokens with `o200k_base`, or 484 with
`cl100k_base` (tiktoken 0.12). Domain discovery took 156 tokens and one operation's
input declaration took 91 with `o200k_base`. Five separate reads of the current
session, ten other sessions, usage accounts, Machines and cached quota used 3,403
tokens for tool arguments plus response text. Composing those reads and returning
the selected fields used 805, about 76% less for that task. These counts exclude
Provider wrappers and prompt-cache billing; payload size changes with the live
state. Reuse known declarations at the same revision, compose related reads and
select fields before returning them to the agent.

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
| `accounts` | `list()`, `login_options(account_id,machine_id)`: subscription metadata and local login readiness; no credentials or probes |

`session.events` projects the retained hot log: message chunks coalesce,
tool-call updates fold into their call, thoughts are omitted and text is
truncated per item. `calls.*` uses the exact managed-call dispatcher, minus the
CLI-only `schema` and `next` fields; a program can observe calls but not start
or cancel them. `calls.wait` is clamped to the program's remaining budget. A
failed operation throws an `Error` whose `code` is stable; its bounded `message`
names the operation and, for `invalid_argument`/`not_found`, echoes the
arguments the broker received plus the help topic to consult. A misspelled
namespace or method throws `unknown_operation` listing the valid names instead
of an engine `TypeError`. A help request may carry `timeout_ms`; the MCP server
reports which field was wrong in malformed tool arguments. `help:"actions"`
points to the `accounts` and `preferences` topics instead of embedding them. Paged
list objects from `calls.list`, `machines.list`, `usage.list` and
`control.list` keep their JSON shape but also expose non-enumerable array
methods (`map`, `filter`, `find`, iteration, `length`) over their primary list.

### Completion notices

`calls.start` and `review.start` accept `notify:true`. When that call reaches a
terminal state, the Controller queues one `[Cowboy] Managed call … finished`
prompt into the parent session through the ordinary next-turn queue: an idle
parent starts a turn, a busy parent sees it at its next turn, and a paused queue
holds it. The prompt's call-scoped id (`call-notice:<call_id>`) deduplicates
repeats, records a Cowboy origin rather than a human one, and never resets the
scheduler's unattended-turn guard. A parent that is closing or gone gets none.
The agent should end its turn instead of polling `calls.wait`, then read
`calls.result`. The notice is sent by the Controller runner that observes the
terminal transition; a Controller crash in that instant can lose it, so
`calls.inspect` remains the authority. The option is opt-in: an agent that
already polls would otherwise spend an extra turn. The CLI takes the same choice as
`--notify` on `cowboy review` (which then returns right after admission) and
on `cowboy codex`/`cowboy claude`; Review 1.0.6 guidance starts reviews this
way because a shell command cannot outlive its tool timeout.

Selection `path` values are JSON pointers; a missing leading slash
(`"plugins"`, `"0"`) is read as the same pointer. Each broker operation
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

About joins these Service observations with the current subscription registry:
legacy display names label the Service cards, and every registered additional
account has its own card, including disabled accounts and Providers without a
collector. Renames, additions and removals follow account preference events.
Additional accounts never inherit the Service account's quota, activity,
refresh operation, reset credits or query Machine. Native Claude subscriptions
have their own bounded background collector on a Machine with a current login.
It uses the exact signed collector and native CLI in that account's private
home, without a model prompt, credential export, sidecars or private-API credits
enrichment. Owner, account, enrollment, login generation and connection are
checked around dispatch and publication. Machine protocol 37 is required;
older Machines report an update requirement instead of querying the Service
account. Cached observations are separate from authentication authority and are
discarded when their native binding changes. Refresh attempts coalesce and share
a five-minute budget; failed refreshes retain the last valid quota as stale.
Controller restart rebuilds this in-memory cache. Collection is serial and
independent of session workers, including while all sessions are asleep.

`cowboy.accounts.usage(account_id)` reads the same owner-scoped observation as
`GET /api/provider-accounts/{account_id}/usage`. Explicit
`control.apply("usage.refresh",{account:account_id})` or `POST` to that URL waits
for the bounded, coalesced refresh. CLI equivalents are
`cowboy account usage ACCOUNT_ID` and `cowboy account usage ACCOUNT_ID --refresh`.
Missing quota stays unknown. Unsupported Providers can still show a matching
session's signed `session_rate_limits` projection; context tokens and cost are
not quota. `usage.read("anthropic")` continues to mean the Service account,
even when an additional account becomes the default for new sessions.

About keeps account identity, quota windows and refresh visible. Each card's
Details disclosure contains credits, resets, activity, source age and query
Machine information; it starts collapsed and uses native keyboard focus.

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

### Native image calls

Image tasks use the same Calls lifecycle: discover
`calls.capabilities({capability:"image.generate"})`, then use `calls.start` with
`capability:"image.generate"` or `"image.edit"`. Reuse the capability revision;
load no unrelated catalog. Results return verified artifact paths on the
execution Machine. See [native capabilities](managed-native-capabilities.md) for
input files, continuation, limits and rollout requirements.

### Provider account metadata

Provider subscription identities are distinct from Cowboy product users and
Machine placement. `cowboy_read({help:"accounts"})` loads their small schema on
demand; the four resident MCP tools do not expand into per-setting tools.
`accounts.list()` returns only the authenticated operator's registry. Writes
and receipts revalidate the current parent's ownership and active operator role.
`cowboy_receipt({domain:"accounts",operation})` resolves the same parent-scoped
operation after a lost reply. SQLite and PostgreSQL commit registry revision,
generated account ID and receipt together. Reusing an operation with different
input fails. Concurrent edits return `revision_conflict` and the current registry.

These operations manage metadata only: they never perform login, query a vendor,
copy credentials or start a worker. The HTTP/CLI equivalents are
`GET /api/provider-accounts` and `PUT /api/provider-accounts`; the latter requires
the read response's `dataset`, an `operation`, and the same mutation fields.
The account ID is generated by the Controller; labels are not identities.
Disabling invalidates account admission, including existing session bindings;
it does not stop running processes or perform logout.
A default account must be replaced before it can be disabled. `id:"default"`
restores the legacy default. Machine and subscription remain independent.

Authentication research checked 2026-10-19: Claude Code's official
[multiple-account instructions](https://code.claude.com/docs/en/authentication#log-in-with-multiple-accounts)
document separate `CLAUDE_CONFIG_DIR` directories for simultaneous claude.ai
logins. They explicitly exclude isolation of two Console sign-ins without API
keys, whose credentials live elsewhere. This supports configuration-directory
isolation for personal subscriptions; it is not a guarantee against account
enforcement or authorization to evade limits. Cowboy still requires a verified
identity recipe for the exact signed Provider release before named login, and
keeps session admission gated until runtime acceptance is complete.

The `claude-subscription` method in published Claude Plugin 3.20.1 declares a
fresh-login JSON recipe for `.claude.json`: `oauthAccount.accountUuid` and
`oauthAccount.organizationUuid`, with authority `claude-ai`. It requires both
stable fields and never uses the email or label as identity. The logout recipe
uses the same pinned executable's `auth logout`. These declarations target the
pinned 2.1.302 CLI; inspecting its login writer is not a successful live login
acceptance. Do not enable admission or claim deployment from the fixture tests.
It is appended after the legacy `claude-account` and `api-key` methods, retaining
their selection order. Its required bundle key satisfies the existing SDK
contract; the named Claude native-profile path never exports that bundle.

The
[2026-10-20 release receipt](releases/claude-native-account-foundation-2026-10-20.json)
records Controller and Machine activation, the OVH Plugin installation, exact
artifact verification and the compatible Controller recovery floor. It does not
record a real subscription login. At that acceptance point, the named account
registry was empty and both native login and session rollout settings remained
disabled. The remaining live acceptance requires user-selected subscriptions,
independent official logins on each recipient Machine, session
start/restart/stop and account revocation without affecting another account.
Named Codex, DeepSeek and Grok runtime admission and macOS native-account
containment remain outside this Claude Linux release's acceptance.

Native Claude methods are excluded from the legacy default account's portable
credential contract. Its digest retains every legacy method, required key,
projection and refresh rule in their original typed serialization. Adding a
native recipe therefore preserves existing encrypted vault entries, signed
replicas and writable projections through refresh, restart and rollback. It
does not authorize a native method through default-account login or credential
import. Native accounts still match the complete signed authentication contract,
method, identity and exact package generation. Other Providers retain their
existing portable contract; this compatibility rule follows the native-owned
Claude boundary and is not a general relaxation of authentication checks.

Native subscription login has a separate operator-only HTTP entry:
`POST /api/provider-accounts/{account_id}/login`, with `machine_id`,
`provider_version`, `generation_digest`, and `method`. The owner and Provider
come from the authenticated user's account registry, never from the body.
It requires a matching installed release, signed native identity recipe, and
Machine protocol 36. The response supplies `provider_id` and `request_id` for
the existing owner-checked authentication events, code submission and cancel
routes. It neither rotates accounts nor falls back to default authentication.
This interactive login is separate from metadata mutation.

Login and receipt responses include a relative `continue_url`. Resolve it
against the Cowboy origin and give it to the user to continue the same login
on Desktop or Mobile. The fragment contains only account and request IDs, not
authorization links or codes. Product login is still required. The owner-checked
`GET /api/provider-accounts/{account_id}/logins/{request_id}` resolves the
original durable operation across CLI and MCP namespaces; `DELETE` requests
cancellation of that same operation. Neither route starts another native login.
The page verifies the current account/dataset before reading the challenge,
stores only recovery metadata and submits the code directly to Cowboy. Keep
using the original parent-scoped MCP operation for MCP receipt/cancel calls.

A fresh, explicitly requested native login can retire an older failed attempt
only when its scoped Machine metadata proves both leader and process-tree exit,
unsuccessful login, no bound identity and no logout in progress. The Machine
persists a revocation tombstone before removing that generation's private home;
the original request can never be dispatched again. Successful or uncertain
attempts continue to require reconciliation. This does not rotate subscriptions
or make cancellation acknowledgement itself proof of process exit.

Settings exposes the same flow on Desktop and Mobile. Select a Machine and
start sign-in explicitly; the authorization page belongs to the Provider.
The UI retains only the original operation ID and exact account/Machine/release
selection in tab storage before sending. Links and codes are never persisted.
Reopening performs no login mutation. Status checks and retries use the original
operation; cancellation remains visibly pending until there is confirming
evidence. A successful login is not session admission while its runtime gate is
closed. Browser conformance covers both surfaces and themes, lost responses,
remount recovery and keyboard traversal; it is not physical-iOS or real-account
acceptance.

Permanent Cowboy access revocation uses `accounts.revoke` with
`{account_id, expected_revision}` and an explicit operation ID. It fences the
account and captures its cleanup plan atomically; it does not cancel Provider
billing. Reuse the identical input/operation after an uncertain response. An
already revoking account refuses a different operation. Read
`cowboy.accounts.revocation_status(account_id)` for the original plan's state and
exact missing evidence. `accounts.reconcile` with `{account_id}` resumes that
plan from any same-owner session; the original parent-scoped `{}` form remains
supported. Discovery never supplies cleanup authority: each stage revalidates
its original owner, account, enrollment and connection.

Revocation uses the existing product-operator authority, equally for CLI and
MCP. It is an explicit effect, unavailable to generated read programs, and
requires current user intent; retrieved instructions cannot authorize it.
There is no separate browser-confirmation token for an already authorized
operator. This is an operator trust boundary, not protection against a
compromised operator session. The account writer cannot enable the separate
administrative native-login or session-admission rollout policies.

HTTP equivalents are `GET` / `POST /api/provider-accounts/{account_id}/revocation`
and `POST /api/provider-accounts/{account_id}/revocation/reconcile`. Start requires
`{operation, dataset, expected_revision}`; reconcile requires `{dataset}` from
the current registry read. CLI equivalents are `cowboy account revoke`,
`cowboy account revocation` and `cowboy account reconcile`; their `--help` lists
required flags. The UI retains uncertain revocation metadata, offers status and
retry after remount, and lists offline/unconfirmed targets. Only confirmed
completion clears the pending status. The account remains permanently disabled;
metadata enablement cannot undo revocation.

`cowboy_execute({action:"accounts.login_start",operation,arguments:{account_id,
machine_id,provider_version,generation_digest,method}})` starts an explicitly
requested native login. `cowboy_receipt({domain:"logins",operation})` observes
the same parent-scoped operation. Both recheck the current owner/operator role.
SQLite and PostgreSQL reserve the immutable request ID and exact input before
dispatch. Only the fresh claimant may send a native login command. Concurrent
retries share one reservation; a changed input is rejected. A replay never starts
login again, including after process loss or expiration. Receipts contain no
login URL, code or credential. Use the existing owner-checked HTTP/CLI flow for
interactive challenges and code submission.

`cowboy_execute({action:"accounts.login_cancel",operation,arguments:{}})` cancels
the original login operation from the same parent session. It first persists
cancel intent in the existing DB receipt, then uses the HTTP cancellation writer.
It accepts no alternate request ID, account or Machine. A cancellation racing
with initial dispatch is rechecked after executor registration. Replaying the
original login never regains dispatch authority. `cancellation_requested` proves
enqueue, not native process exit or credential revocation; receipts retain
`cancel_requested:true` across Controller restart even when live status is unknown.
If the Machine no longer has a live login task, cancellation acknowledges the
intent idempotently. This still supplies no process-exit or profile-cleanup
evidence: retained native profiles and candidate receipts continue to fence
unsafe reuse. In the UI, an expired/disconnected attempt offers **Start again**.
It cancels the original request, clears only the tab's pointer after acknowledgement,
and reloads Machine readiness. A separate **Start sign-in** click creates a fresh
operation; opening or refreshing the page never automatically restarts login.
Native cancellation also recovers a disconnected in-memory tracker through the
current enrollment-checked Machine connection; rejected or uncertain cancellation
retains the tracker. Native errors and rejected login commands appear immediately
in the form instead of waiting for the executor deadline. Restarting a handoff
replaces its URL; a failed receipt read remains retryable on that same request.

Receipt `status` is a live observation: `active`, `completed`, `inactive`,
`cancelling`, or `unknown`; a fresh attempt refused before enqueue may return
`not_started`. Definite pre-enqueue refusals persist their reason before reporting
`not_started`, so replay and restart preserve that result without regaining
dispatch authority. An uncertain receipt write reports `outcome_unknown` instead.
Cancellation intent is preserved independently of a dispatch refusal.
`dispatch_reserved:true` proves the durable fence exists, not that
the Machine received anything or that authentication succeeded. A crash between
reservation and enqueue intentionally leaves the outcome unknown and cannot
automatically reacquire dispatch authority. Inspect the original request before
explicitly authorizing a new operation. Missing receipts also report `unknown`
with `dispatch_reserved:false`, never proof of failure. No credential quota
rotation, account fallback or automated native-login retry is performed.

`accounts.login_options(account_id,machine_id)` discovers native-login readiness
through the existing read tool. HTTP and CLI clients use
`GET /api/provider-accounts/{account_id}/login?machine_id=...` (including
`cowboy api` with that path). The response contains the installed version and
digest, method IDs, availability and stable blocking reason codes. Ownership is
checked before Machine lookup. It uses only local registry, enrollment, signed
catalog and connection state: no vendor probes, vault creation or native process.
Readiness is advisory; starting login rechecks all conditions. Login readiness
does not imply named session runtime admission, reported separately.
`native_profile` is either null or the generation, exact Provider version/digest
and method of the selected Machine's current bound native login. Discovery is
scoped by owner, account and Machine and selects the latest reserved
generation, skipping newer replacements that were cancelled or refused before
completing; a pending or unknown login never falls back to an earlier success.
The response rechecks current binding, enrollment and cancellation, but is still
an advisory snapshot: scheduling must repeat admission around its actual effect.
It contains no identity fields, credentials or reusable authorization token.
The native profile must also match the active trusted Provider release (or a
compatible later release, below), authentication contract and native identity
recipe. Its auth generation comes
from that Machine's native login, independently of the default account's Service
authentication or replicated credentials.

A login follows a later Provider release in place, without a new sign-in, when
that release keeps the login's authentication contract fingerprint *and* an
identical signed `authentication` section (methods, native identity recipes,
credential layout). The fingerprint alone is insufficient because it excludes
native identity recipes. The Controller compares against the Catalog's
signed-in release; the Machine compares again against the retained signed-in
release before every launch and usage read, and native usage runs the active
compatible collector. The native home, identity and credentials are never
copied or rewritten; new sessions and dormant re-pins use the compatible release
while existing sessions keep their own.

A Provider upgrade that does not satisfy this rule makes this readiness
projection null; it never substitutes the default account. Account usage then
reports `native_login_outdated`
("sign in again") instead of `native_login_required`, and the login list marks
that completed login `outdated` with its signed-in and installed Provider
versions. Existing sessions keep their exact retained generation; a new login
on the updated release is required for new sessions and usage. The New Session
and next-agent subscription picker reads the same cached observations (no
Provider request) and disables an account whose login is outdated or missing
with that reason; creation still rechecks the exact Machine and reports either
refusal as a readable "sign in (again) from Settings → Subscriptions" message.

When the effective login on a Machine is an unfinished newer sign-in (waiting,
unknown or still verifying), usage, the picker and creation report
`native_login_pending` ("finish or cancel it") instead of
`native_login_required`, because cancelling it restores the previous working
login without a fresh sign-in. The per-Machine login list shows the effective
login: a newer attempt that was cancelled or refused before completing never
hides the login that still admits sessions.

The internal native-session writer checks this binding and inserts the session
in one transaction under the account admission lock. Concurrent revocation
either captures the committed session in its cleanup plan or denies insertion;
registration must not enqueue an asynchronous session insert after releasing
that lock. Duplicate session IDs cannot overwrite an existing row. The internal
registration path commits the row first, then rechecks native binding under the
same account lock before projecting it into Hub. Projection never queues another
session insert and refuses to overwrite an existing in-memory session. If
revocation wins between commit and projection, the row remains in its cleanup
plan without being published. Both ordinary remote sessions and split-execution
session creation now use this shared account-aware registration path. Named
registration requires the account protocol and rechecks the same live Machine
connection, runtime policy and Plugin lifecycle fence before projection. The
public named-account gate remains closed until worker-start/recovery admission
and exact-generation runtime acceptance are complete.

The synchronous Supervisor compatibility registration path rejects named
accounts before publishing a session or queuing persistence or worker commands.
This includes legacy local creation and eager-start callers. Named callers must
prepare metadata and use the database-backed registration above; a later worker
launch refusal is not a substitute for admission before registration.

Existing sessions retain their exact database-persisted login generation when a
replacement login is reserved. Discovery reads that session's owner, account,
Machine and generation; runtime admission locks and rechecks its exact stored
Provider version/digest/auth generation before any side effect. A missing or
changed row refuses admission. Cancellation, revocation, account authority
changes and Machine enrollment changes still invalidate the original binding.
New-session discovery and registration require that same effective
generation: an abandoned replacement (cancelled or dispatch-refused, never
completed, so it can never bind) leaves the previous working login usable
without signing in again, while a pending or unknown replacement never falls
back to an older login. This preserves existing sessions during fresh login; it does not switch
them to the replacement or permit automatic Provider upgrades.

The internal session-transition journal records an explicit same-account,
same-Machine migration proposal under owner and enrollment locks. It requires a
currently verified newer target login and the exact original persisted session
binding. One unfinished proposal may exist per session; concurrent identical
operation IDs replay the same immutable intent, while changed inputs or competing
operations are rejected. Recording an intent changes no session binding and
starts or stops no process. Historical replay after target cancellation is only
receipt recovery, never dispatch authority. A separate migration stop journal
accepts scoped non-nil worker tree-exit evidence only after the caller confirms
quiescence and queries the currently authenticated Machine. The writer rechecks
target login, original session binding and live connection under admission
locks; a failed connection check rolls back the evidence. Identical historical
replay needs no live connection, but cannot authorize a binding change. Missing
tree evidence and conflicting observations are rejected. Migration worker-exit
queries additionally carry the exact namespaced stop command ID. Machine stop
markers now retain this ID in schema two; schema-one markers remain readable and
continue fencing launches, but cannot satisfy a migration query. A replacement
stop invalidates the prior command's query even if the same worker tree is still
exited. Controller evidence recording and binding commit both require the exact
migration command identity. No stop marker is removed by this change.
The internal stop
reconciler now checks the current target and original binding before enqueueing
a namespaced idempotent stop on the exact runtime connection. It waits at most
30 seconds for acknowledgement, then separately queries worker tree exit on the
authenticated Machine control connection and rechecks both connections before
recording evidence. Offline, timeout and lost-connection outcomes remain
unconfirmed. Repeating after recorded evidence is read-only. The internal binding
commit now requires current target-login admission, the exact original session
row and scoped persisted tree-exit evidence in one transaction. It updates the
Provider version/digest/auth generation and target behavior, clears the old native
session ID, sets the session to `exited`, and writes an immutable commit receipt
atomically. Callers must obtain behavior from the exact trusted target package.
Duplicate commits are historical reads; conflicting behavior is rejected. This
does not project Hub state, release the Machine fence or mark migration finished.
The internal Controller commit reconciler now obtains stop evidence, resolves
behavior through the exact trusted Provider catalog and installed contract, and
rechecks its live Machine connection, Plugin lifecycle fence and original Hub
binding inside the commit transaction. A failed final check rolls back both the
row change and receipt. An existing commit receipt is a historical read and
performs no stop or live validation. Machine binding preparation now persists
an immutable operation-scoped record after checking the exact current stop
command, exited worker launch, native login receipt, identity and target release.
Retries revalidate these conditions even when a record already exists; a newer
stop or revoked login refuses preparation. This record is evidence only and
never clears a fence or authorizes a launch. Before committing the DB binding,
the Controller now requests this preparation on the current authenticated Machine
connection. Only the exact typed receipt confirms preparation; generic ACKs,
missing or mismatched receipts, timeouts and connection replacement cannot confirm
it. The Machine independently resolves the installed target package and rechecks
local evidence. Preparation receipts stay out of diagnostic event history.
The internal Hub projection reconciler resumes committed operations under current
native-login and stored-session admission. It compares the original binding,
accepts an exact already-projected target idempotently, and rejects active,
closing or conflicting sessions. Projection updates only binding fields and
clears the old native session ID; it preserves user state, queues and history,
does not enqueue a second DB write, and never starts a worker.
Named worker launch journals now also retain the exact Provider/version/digest
and native auth generation. Both broker adoption and the serialized pre-spawn
journal check reject changed bindings after restart; legacy journals without
binding metadata remain readable as exit evidence but cannot authorize a named
replacement. Replaying the same binding still requires previous tree exit.
The internal Machine readmission primitive now persists the target worker
binding before releasing a schema-three stop marker. An interruption between
these writes remains fenced and permits only the exact pending operation to
resume. Released markers retain the admitted generation and original launch;
a later stop preserves that generation floor, so replaying the old operation
cannot release it again. Damaged and linked markers remain fenced and are never
repaired by stop writes. The Broker's internal readmission operation now holds
the session lifecycle gate, rejects deletion, pending worker cleanup and changed
session context, and resolves the exact installed package and behavior before
local release. It replaces the declaration and clears stop/cancellation state
without spawning. Worker registration rejects an exited account journal until
a subsequent real spawn records a new live launch. The runtime wire now carries
an explicit readmission command with a namespaced operation ID; the Broker
acknowledges only after local admission completes. The bounded waiter distinguishes
admitted, refused and unconfirmed, and never queues Ensure or starts a worker.
Outbound writes recheck the current named-session authority and exact native
receipt, including replay; old protocol peers refuse the command. The Controller
migration reconciler now invokes this transport under current native/session
admission after Hub projection. It rechecks both Machine connections and the
target Hub binding after the scoped ACK, then marks the migration finished under
the account and stored-session locks. Wrong admission evidence or a failed final
connection check cannot finish it; timeout stays unconfirmed. Completed operations
are historical reads and never redispatch. The reconciler obtains its dormant
declaration directly from Supervisor rather than accepting a caller-built launch:
the exact committed native generation, owner presence, exited status and cleared
native thread are required. It reuses normal execution-environment, runtime
restriction and configuration projection checks. Supervisor may now queue an
explicit start for a structurally complete registered native binding (owner,
enrolled Machine, Claude Provider, exact release, nonzero auth generation and
behavior). This cached declaration is not authority: RemoteRuntime still checks
the current durable binding and Machine identity before sending or projecting
events, and older protocols refuse named commands. Public named-login and creation entry points still
await end-to-end release acceptance; these stages do not open those gates.
MCP now exposes `accounts.transition_plan` and `accounts.transition_reconcile`
through the existing execute tool. Planning reserves the exact original binding
and target login operation; it neither stops nor starts workers. Reconciliation
uses that immutable intent, checks current admission at every effect boundary,
and has a two-second cooperative wait budget. Repeat the same operation after
`unconfirmed`; errors never roll back already committed stages. Both the migration
and referenced login operation are scoped to the authenticated owner and calling
parent session. Reusing a migration ID with different input is refused. These
actions require an already registered named session and admitted native login;
they do not enable public named login or creation.
The read-only `cowboy.accounts.transition_status(operation)` MCP method exposes
owner- and parent-scoped durable progress: `planned`, `stop_confirmed`, `binding_committed`,
or `completed`; missing and foreign-owner operations both return `unknown`.
It reads one database snapshot without vendor probes, native identity details,
or runtime effects. Its `evidence: historical` and `worker_state: not_observed`
fields explicitly distinguish migration receipts from current worker health.

Outbound runtime frames now recheck native account admission at the transport
boundary, including reconnect declarations and execution recovery. The check
matches session owner, Machine, account and exact Provider/auth generations to
the current native binding, including the runtime connection's enrollment-key
fingerprint. A same-name Machine's new enrollment cannot authorize its old
connection. The account lock covers the bounded frame write;
write failure after dispatch begins closes the transport rather than claiming
the request was never sent. Stop, cancel and account-quiesce commands bypass
this authorization check so revoked accounts can still be cleaned up. A queued
frame is not proof that a remote process started or stopped.

Inbound named-worker snapshots now check the current native binding under the
same account lock before updating worker caches, recovery declarations, pending
ensure acknowledgements or Hub state. Bootstrap, reconnect, Welcome batches,
individual snapshots and reset replacements share this admission. Missing
launch metadata, an old enrollment connection, mismatched account/generation,
or revoked binding cannot adopt a worker. Rejection does not prove process exit;
account-stop receipts remain the cleanup evidence.
The Machine broker also preserves the exact named Provider/auth binding when
adopting Controller declarations or worker snapshots. An unchanged account ID
does not permit changing Provider, version, artifact digest or auth generation.
A worker without launch evidence cannot be adopted as a named account. Default
account upgrade compatibility is unchanged; named reauthentication requires an
explicit lifecycle transition rather than rewriting a surviving worker's claim.
Direct worker startup applies signed Provider environment removal rules to both
ambient variables and Machine-configured worker variables before applying the
Provider's credential projections. A configured credential with a scrubbed
prefix cannot survive merely because that name was absent from the broker's
ambient environment. Explicit session projections still follow this step.
Machine-side native-home admission checks the login profile's Provider release,
not only its auth contract. Both worker startup and Provider host invocation
carry their verified release digest into that check. A different release may
use the profile only when the retained signed-in release has identical signed
authentication (the carry-over rule above); the Machine retains every
native-profile release for that check. Rejection preserves the original home.
Explicit Reload, automatic idle updates and dormant re-pinning may move an
existing named session to an installed release on its existing login only
under that same rule: the Controller resolves the target exactly as new-session
admission resolves the session's own login generation, requires an unchanged
native session contract, and hands the Hub a proof bound to that session, login
generation and exact release. No new login is needed. A release whose signed
authentication changed still requires a new login and a verified login
transition; surviving-worker release reconciliation never changes a named
session's release. Rejection leaves the session metadata and runtime status
intact.

Native bindings use the account's last authority-change revision, independently
of metadata CAS on the whole registry. Creating or binding another account,
renaming an account or changing defaults does not invalidate existing native
bindings. Enablement transitions, revocation and initial identity binding advance
that account's authority revision; disabling then re-enabling cannot revive an
old login operation. Legacy account records without an authority revision retain
whole-registry matching rather than silently gaining broader admission.

After Controller restart, cancellation of a native login can use its durable
owner-scoped receipt even when the executor map is empty. Recovery targets only
the original login request on the original enrolled Machine, requires the named
account protocol, and waits for a separately correlated cancellation response.
Enrollment and connection currency are checked again after the response. This
reports cancellation requested, never process exit, profile deletion or logout;
refused or uncertain cancellation remains retryable without starting a login.

Named-worker events require a cached launch identity and the same worker epoch.
Each event repeats native binding admission before telemetry, configuration
replay, prompt completion, automatic permission responses or Hub projection;
the cached launch and epoch are rechecked after DB admission. Thus an old worker
cannot borrow its replacement's identity, and an admitted snapshot does not
authorize events after account revocation. Rejected events are consumed and
ACKed without projection; this ACK is neither authorization nor process-exit
evidence. Public named-account creation is separately controlled by
`accounts.native_claude_sessions`; completing a login does not enable it.
The [OVH Otter acceptance record](releases/claude-otter-session-acceptance-2026-10-20.json)
records native start, hibernation/resume and confirmed deletion while existing
Lynx workers retained their identities. Revocation and crash recovery checks used
isolated synthetic profiles; neither real account was logged out or revoked.
Other installations still require their own lifecycle and runtime acceptance.

Native login operation status survives loss of the in-memory executor. A recorded
completion without binding is `binding_pending`; a bound receipt whose current
admission cannot be established is `binding_unavailable`. `completed` requires
fresh binding admission, and durable cancellation takes precedence. These are
login observations, not runtime readiness or a session launch grant. Retrying
the same start operation recovers the original completion/binding stages just
like querying its receipt; it never dispatches another native login.
The owner-scoped login events endpoint uses this durable status for native
logins, including after the executor map is lost. It returns a synthetic
`SignedIn` event only after current binding admission; it does not set Service
vault authentication or launch authority. Pending Machine history cannot
overwrite that completion. Inactive, cancelled or unavailable bindings expose
no stale challenge events. Requests without a matching owner-scoped native
receipt keep the existing not-found behavior.

The authenticated CLI exposes the same native login flow:

```sh
cowboy account list
cowboy account update --operation CHANGE_ID --dataset DATASET --expected-revision REVISION < metadata-change.json
cowboy account login-options ACCOUNT_ID --machine MACHINE_ID
cowboy account login ACCOUNT_ID --machine MACHINE_ID \
  --provider-version VERSION --generation-digest DIGEST --method METHOD --operation OPERATION_ID
cowboy account receipt OPERATION_ID
cowboy account events PROVIDER_ID REQUEST_ID
cowboy account submit-code PROVIDER_ID REQUEST_ID < private-code-file
cowboy account cancel OPERATION_ID
```

Use the exact installed version, digest and method returned by discovery. Commands
retain a stable login operation ID. After uncertainty, query the receipt or reuse
the identical start arguments and operation; never create a new ID as a retry.
HTTP clients must supply `operation` in the named login body;
`GET /api/provider-accounts/login-operations/{operation}` only observes it. HTTP
operations are owner-scoped and separate from MCP parent-session operations.
Named HTTP starts without `operation` are rejected before dispatch.
`DELETE /api/provider-accounts/login-operations/{operation}` persists cancellation
before attempting Machine cleanup. CLI cancellation uses this same route, including
after an uncertain start. Cancellation intent is not proof of process exit or
revocation of a previously committed subscription.
Commands
reuse `cowboy login` credentials and accept `--origin` for multiple Services.
Code submission reads one bounded value from stdin and has no code argument.
Events may contain private sign-in links; they should not be published as logs.
Metadata input uses the same `AccountChange` union as MCP (`create`, `rename`,
`set_enabled`, `set_default`), rejects credentials and internal identity binding,
and is bounded to 16 KiB. Disabling an account is not credential revocation.

The Accounts & sign-in page presents existing Service credentials and additional
accounts together. Existing display aliases (for example `Claude · Lynx`) label
their credential card; additional accounts have their own Machine sign-in rows.
`Manage` exposes rename, default, enable/disable and removal. Removal uses the
durable account revocation workflow: it retains the disabled record and cleanup
history, and does not cancel the Provider subscription or billing. Login methods
come from the selected Machine's trusted installed release and readiness checks;
unavailable methods remain visible with their reason. Adding metadata does not
make an unsupported Provider login or gated session runtime available.

`GET /api/provider-accounts/{account_id}/logins` and
`cowboy.accounts.logins(account_id,{after_machine?,limit?})` discover the latest
native login reservation on each Machine, including attempts started by another
device, CLI or MCP session. Pages are ordered by Machine ID (default 20, maximum
50); pass `next_after_machine` as `after_machine` for the next page. Discovery
reads local metadata only, never probes a Provider, starts recovery or falls back
to an older completed generation while a replacement is pending. Results contain
no credentials, native identity, authorization code or OAuth challenge URL.

`cowboy.accounts.login_status(account_id,request_id)` uses the same owner-checked
continuation as the GUI and may reconcile the existing completion. An agent can
cancel a discovered attempt with `accounts.login_cancel_request` and arguments
`{account_id,request_id}`; this resolves and cancels the original operation, even
when it originated in another parent session. Observe the original reference
with `login_status` after uncertainty; no new login is dispatched. CLI equivalents
are `cowboy account logins ACCOUNT [--after-machine ID] [--limit N]`,
`cowboy account login-status ACCOUNT REQUEST` and
`cowboy account cancel-request ACCOUNT REQUEST`. Existing metadata, preferences,
login-start, revocation and reconciliation commands remain the write surfaces.
OAuth challenges and secret submission continue through the authenticated UI or
bounded CLI stdin; raw secrets are not added to MCP arguments or receipts.
Legacy Service authentication retains its signed Provider UI and authenticated
`cowboy api` routes; the named-account MCP commands do not change those shared
credentials. These are on-demand account helpers within the existing four tools.

After a durable cancellation acknowledgement the UI may discard its local tab
pointer. The cancelled receipt stays discoverable, and Machine cleanup fences
remain authoritative. Closing that view neither asserts process exit nor starts
another attempt automatically.

For an already recorded revocation intent, MCP can explicitly retry
`cowboy_execute({action:"accounts.reconcile",arguments:{},operation:ORIGINAL_ID})`
from the same parent session. This does not create a revocation intent or enable
the gated account logout flow. Each remote target has a two-second cooperative
wait budget and each stage uses at most four concurrent requests. A slow target
does not prevent later targets from being attempted. Login cancellation and session
stopping progress independently; local grant cleanup waits for all planned
sessions to have confirmed stop evidence. Partial failures remain attached to
their targets. Account receipts report `revoking` and `completion_state:"unconfirmed"`;
the status and reconciliation responses expose the separate `completion` object
with `account_completed` and the exact `missing` evidence list. This keeps the
historical receipt state distinct from the cleanup inventory.
Timed-out targets remain pending and can be retried with the same ID.
Neither a cancellation request nor a local wipe record proves native
logout, remote credential cleanup, or whole-account completion.
Local grant cleanup records its evidence in the original account operation,
under the same owner lock as grant admission. A committed grant requires an
authenticated newer vault tombstone; a cancelled promotion requires confirmed
discard. Failed or superseded cleanup does not create evidence. Repeated passes
reuse recorded local evidence without running the cleanup callback again.
If vault cleanup succeeded but the database commit was lost, revocation recovery
may authenticate an already empty newer vault and finish the record; it must not
erase or accept a newer live login. The ordinary discard API still distinguishes
whether this invocation actually performed a write.
After confirmed session stops, the pass reconciles native logout for each planned
login before attempting signed remote wipes for locally revoked committed grants.
`native_profiles` reports `profile_cleaned`, `command_succeeded`, `command_failed`,
`unconfirmed`, or `not_applicable` per login. Only `profile_cleaned` or
`not_applicable` permits remote wipes; command success alone is insufficient.
Failure or uncertainty blocks remote wipes with
`waiting_for_native_logout`; missing generation on an old native receipt is
uncertain, never evidence that native logout is unnecessary. Delivery requires the original recipient fingerprint,
a currently enrolled Machine and the named-account protocol. `remote_grants`
reports `wipe_recorded`, `unconfirmed`, or `pending` per target. `wipe_recorded`
requires the exact sealed wipe acknowledgement followed by a durable receipt in
the original account operation. First recording validates the immutable grant
target, committed grant generation, original recipient fingerprint, current
enrollment, disabled/revoking account, and live connection under database locks.
Identical historical evidence survives restart and enrollment changes without
authorizing another dispatch; conflicting evidence cannot replace it. Recorded
targets are skipped on later passes. A lost acknowledgement or uncertain commit
retries the same tombstone generation idempotently. This is historical Machine
cleanup evidence, not whole-account completion or upstream logout. The additive
receipt field defaults to empty when reading old operations.
Native-only profiles without portable grants still require their
separate logout lifecycle; this delivery stage does not cover them.

The Controller also resumes existing revocation intents from the database after
restart. Every `accounts.revocation_retry_interval` (Service config, default one
minute, allowed 30 seconds–24 hours), it examines up to 32 journal entries and
reconciles at most four operations concurrently. Each page advances past failed
operations; reaching the end starts the next scan from the beginning. Waiting
after each page avoids catch-up bursts. Reconnected Machines are retried on a
subsequent scan, without triggering vendor probes or another login. Shutdown
cancels the current pass; durable steps remain available for the next process.
This recovery includes native command outcome reconciliation but still reports
whole-account native logout as pending: command exit and local profile cleanup do
not establish upstream revocation. It does not open named session
admission.

Machine generation retention also keeps the exact package digest recorded by
each unfinished native profile, even when no session references it. A revoked
profile with a remaining home still retains that package for cleanup. Only a
revoked profile whose home has been removed releases this pin; its operation
metadata remains. Invalid or unreadable profile metadata stops retirement rather
than allowing it to delete an executable needed for recovery or logout.

The native authentication contract can declare an optional
`native_logout: {kind:"command_v1", arguments:[...]}` on a native identity login
method. This is a bounded, noninteractive command recipe for the same pinned
executable and isolated environment, not a separate arbitrary executable or a
Controller hardcoded Provider command. It requires a command-based native login
and identity recipe; absent fields preserve legacy manifest serialization.
The declaration alone does not enable dispatch or establish logout completion.
The internal Machine runner reserves a logout operation durably before spawn,
then runs that profile's pinned command with its own home and environment. It
does not rerun login preflight or expose command output. Its two-minute deadline
uses the authentication process-group cleanup path. A lost result remains
unknown; the same operation only observes its reservation, never spawns again.
Recorded leader success/failure is historical evidence, not upstream token
revocation or whole-account completion. Pending logout blocks runtime admission
and home deletion, while outcome persistence can finish after a concurrent wipe
has installed its revocation fence. Public revocation creation uses the explicit
owner action above; reconciliation retains the Controller's scoped authorization
and receipt path. This does not open named native login or session admission.
After recorded native success, the runner durably marks that exact generation
revoked, removes only its private home and syncs the directory. It retains profile
metadata and the original logout operation. Failed deletion remains retryable:
replay observes the original success and repeats cleanup without spawning logout.
The typed observation carries `profile_cleaned`, defaulting to false for older
messages and receipts. The Controller accepts only a scoped, reauthorized
false-to-true cleanup transition on a successful command receipt, never a
downgrade or a change of outcome. Even when an account-wide wipe fence remains,
the original logout can be observed and its exact generation cleaned; the fence
still blocks new login/logout execution.
Machine protocol 36 advertises account-bound control support; protocols through 35
peers retain their default-account connection but cannot receive named commands.
This wire capability does not open the independent product login/create gates.
The Service's `accounts.native_claude_login` setting controls explicit named
Claude logins and defaults to `false`. It is validated/discoverable through the
unified config CLI and applies live. Turning it on only permits requesting a
login; Machine enrollment, protocol, signed recipe, account ownership and
identity verification remain mandatory. Turning it off refuses new requests
without discarding receipts or preventing cancellation/recovery. Named session
creation is controlled independently by `accounts.native_claude_sessions`, also
defaulting to `false` and applying live. It permits only `claude-code`; all other
named Providers remain refused, without default-account fallback. Account
discovery returns `session_providers`, used by desktop/mobile selection and
default controls. Missing capability metadata refuses named selection. Enabling
creation does not log in, bypass the exact native DB binding, or choose a Machine.
Disabling it blocks new creation but does not stop existing sessions.
Changing these service-wide rollout policies
uses the administrative config CLI; the session-scoped account MCP writer cannot
change it or gain that administrative authority.
The named-account Machine protocol has a dedicated native logout
request and typed observation. The transport validates the full echoed account,
Provider, generation, original login request and logout operation against its
pending request, then rechecks the exact connection. Generic command ACKs do not
complete it. Missing or mismatched observations remain unconfirmed and private
observations are excluded from the Machine diagnostics history. The Machine
rejects default-account requests and requests whose login is still registered.
This internal transport is not a public owner-authorized logout endpoint; named
session admission remains controlled by the Service rollout setting.
The Controller store can retain a confirmed native command outcome in the
original login receipt. First recording requires membership in the immutable
owner-scoped revocation plan, matching dispatch enrollment and generation,
confirmed stop evidence for every planned session, and a current connection.
Identical recorded outcomes can later be read after enrollment changes; this
historical read grants no dispatch authority. Conflicting outcomes and unknown
command results are rejected. The additional receipt field is optional for
reading old data; deploying writers still requires compatible account-lifecycle
readers and is not permission to enable the public logout endpoint.
The internal dispatch admission uses the same plan, stop and enrollment checks
under database locks. Its synchronous callback enqueues on the exact live
connection before releasing those locks and returns a future to await afterward.
An already recorded failure or confirmed cleanup suppresses dispatch. A recorded
success without cleanup evidence permits an authorized retry of the same
operation to finish cleanup. An uncertain transport or
database commit must reuse the original operation, whose Machine reservation
prevents another native command spawn. Dropping the response future removes its
local waiter, not the remote process. Explicit reconciliation and restart recovery
use this admission API with a two-second observation budget per target. A later
pass queries the same Machine operation; it does not launch a new native command.

Events are fetched once, without background polling. A service or transport error
does not trigger a new login automatically. A repeated start with identical input
observes the durable reservation; it never dispatches again after expiry or restart.
A reserved operation whose executor cannot be observed reports an unknown outcome.
Cancellation preserves intent even when the executor is no longer observable.
Secret-input login rechecks registration, cancellation and expiry while holding
the login registry lock before publishing a credential candidate. Failed event
delivery is an error, not a pending successful promotion.
Named secret-input requests also reserve a private temporary scope before
waiting for input. The secret stays in the existing in-memory candidate path;
the reservation never writes it to disk. Success, failure and task cancellation
clean up that scope and retain a never-spawned stop receipt. Cleanup or journal
failure prevents candidate publication, concurrent requests for the same account
remain fenced, and stopped request IDs cannot be reused. Legacy default-account
secret input keeps its existing lifecycle.
Machine-owned temporary login cleanup retains a scoped local stop receipt before
deleting the candidate, then records deletion after syncing the candidate root.
It distinguishes a task that never spawned its command from a confirmed leader
exit. Named portable command logins additionally require pre-exec containment;
after observing the subtree empty they persist a separate exit marker before
allowing cleanup or candidate publication. The retained stop receipt projects
this as `process_tree_exited`. Startup can finish cleanup from that marker even
when the login failed or the cleanup journal write was interrupted. A malformed
marker or legacy completion marker cannot establish tree exit. Default-account
commands retain their legacy runner. Unknown exit cannot produce this evidence; a missing directory is not exit
evidence. Startup resumes cleanup from these receipts, retains unfinished evidence,
and refuses reuse of the same login request. Atomic-write scratch files are not
receipts, and linked candidate roots cannot authorize deletion outside the scope.
Reconnect finalization and startup recovery of a valid completion marker also
persist stop evidence before removing that marker. A journal failure retains the
candidate for retry. A legacy missing home remains an idempotent finalize result,
not new exit evidence; previous-boot cleanup alone cannot establish whether a
command was spawned.
Revocation reconciliation queries these receipts through a typed, connection-bound
Machine request and returns `temporary_logins` observations. It checks the original
owner, account, login request and enrollment; missing receipts or mismatches stay
unconfirmed. Confirmed candidate removal is persisted in the original owner's
revocation receipt as `login_stops`, with the original login operation and Machine
enrollment fingerprint. First recording revalidates cancellation, the immutable
plan, disabled/revoking account and current connection under database locks.
Identical historical replay survives restart without another Machine query;
conflicting evidence is rejected. Valid tree evidence resolves the corresponding
completion-inventory item only alongside matching cleanup evidence. These records
do not authorize subsequent cleanup or complete whole-account revocation.
Fresh vault cleanup and remote wipe sealing additionally require every planned
session's persisted stop evidence and every login's persisted candidate cleanup
or successful native profile cleanup. These checks run inside the owner-locked
Store transaction before the cleanup callback; a cancellation ACK or an offline
Machine is insufficient. Already recorded local deletion replays without another
deletion. This ordering fence is not proof of complete process-tree termination.
Recovery reports a matching persisted temporary-login cleanup as
`cleanup_recorded` and skips another cancellation request for that login.
Missing session, temporary-login or native-profile cleanup evidence keeps the
local grant stage `pending`; scope mismatches and storage failures remain
`failed`. Neither state exposes raw diagnostic errors or implies vendor logout.
Reconciliation includes a read-only `completion` evidence inventory: `missing`
names exact session/login/grant targets still lacking stop, cleanup, process-tree
exit, local wipe or remote wipe evidence. A local wipe never substitutes for a
committed grant's remote wipe; cancelled undelivered grants do not require remote
delivery. Existing leader-exit/native-command receipts do not prove descendant
exit. The internal completion writer rechecks the complete original plan under
the same owner transaction lock as account, login and grant admission. Only
complete scoped evidence can persist `revocation_completed`; concurrent retries
and restart recovery reuse the original receipt. Completed operations leave
automatic recovery discovery and reconciliation returns without remote effects.
The account remains disabled and fenced against re-enabling. This is local
Cowboy cleanup completion, not upstream token revocation or subscription
cancellation. The diagnostic inventory reports `account_completed` from this
receipt and `completion_admission: internal_only`; public lifecycle entry is
still gated. Reading the inventory never commits completion.
Native logout dispatch and fresh local/remote grant wipe admission also require
scoped session process-tree exit evidence inside the owner-locked transaction.
Grant cleanup requires tree-exit evidence for every started login process and
both native login/logout trees. Missing evidence remains pending; mismatched
evidence fails. Replaying an identical historical wipe receipt performs no new
deletion and does not retroactively upgrade its evidence.
The current session-stop ACK is shared by systemd and Direct workers; the latter
confirms its owned leader without proving detached descendants exited. Thus even
a valid historical session-stop receipt alone leaves `session_process_tree_exit`
in the completion inventory. Direct account-stop snapshots report background-task
count as unknown, not zero. Only a separately admitted typed containment receipt
resolves this gap; old ACKs cannot be upgraded implicitly.
New named-account Direct workers on Linux now share the login runner's pre-exec
cgroup containment. The child reaper kills and observes the entire owned subtree
before clearing a separate in-memory tree-exit fence. A missing leader PID alone
cannot clear that fence: failed or aborted cleanup blocks confirmed stopping and
replacement launch. Unsupported containment fails before starting the worker;
default-account workers retain their existing behavior. This local lifecycle
check now writes a separate Machine-owned launch journal before spawn. Each
launch has a fresh identity; its reaper can record tree exit after both
child wait and containment confirmation succeed. A synchronous spawn failure
records that nothing executed, allowing a fresh launch after restart; an exited
child, including an unsuccessful systemd-run invocation, cannot use that shortcut.
Unknown or corrupt records block replacement and confirmed stopping in both
Direct and systemd modes, even after PID-map loss.
Late reapers cannot confirm another launch. A typed, connection-bound query reads
this journal only while a matching durable account-stop fence remains. Controller
reconciliation reports `worker_trees` and appends the proof to the original stop
receipt under owner/enrollment locks. First admission rechecks the plan, account,
session, current enrollment and connection; identical replay is historical, while
another launch or a downgrade conflicts. Generic command ACKs never satisfy the
query. For systemd workers, the Machine observes the launch token, invocation ID,
boot ID and existing cgroup, then persists that binding. Stop recovery checks the
same incarnation before signalling it. Exit requires an inactive/failed unit
without a queued job, plus an empty or collected recorded cgroup, followed by
another unit check. A collected unit alone or a successful stop ACK is insufficient.
Broker restart can reuse the durable binding. New Linux launches record their
kernel boot ID before spawn. Launch/stop recovery can persist exit after observing
a distinct valid boot ID on the same Machine; it preserves both boot IDs and the
original launch. Queries remain read-only. Old journals without a boot stamp
and corrupt evidence remain unconfirmed. New Direct launches also persist the
exclusive cgroup path, filesystem device/inode and owner process incarnation
before spawn. Explicit stop/recycle can recover that exact tree after PID-map
loss, including a same-boot Machine crash. Direct account-stop commands and
startup reconciliation use that journal when the session declaration is absent;
live workers retain their graceful Stop/TERM/KILL sequence. Descriptor-relative
traversal refuses links, foreign filesystems and replacement cgroups. Same-boot
path-absence evidence also requires the recorded cgroupfs root identity and a
visible parent; a changed namespace view or missing ancestor stays unconfirmed.
Recovery kills the original
subtree, removes its empty descendant groups and removes the original group
before recording exit; this closes a delayed pre-exec membership race. Late
recovery still cannot complete a newer launch. Named systemd workers use a
startup pipe:
after argument validation and before Provider initialization, the worker requires
its exact launch token. The broker sends it only after durably binding the unit
and checking the launch is still admitted. EOF, incomplete tokens, capture timeout
and cancellation refuse startup. This prevents a valid worker's immediate Provider
failure from outrunning unit capture; failure before entering the worker handshake
still remains unconfirmed without independent evidence. Retained older workers
cannot be assumed to support this handshake; named admission stays gated until
the exact worker generation is accepted. Journals must not be transplanted between
Machines. This is lifecycle ownership,
not protection against root deliberately migrating processes or replacing units.
Every named launch, including systemd, first
invalidates the previous launch's exit proof; changing spawn modes cannot reuse
old Direct evidence. Named-session admission remains gated pending native
acceptance; revocation reports completion only when every planned proof exists.
New native login/logout executions require delegated Linux cgroup-v2 containment
before the Provider executable starts. Failure to join prevents execution. The
runner kills the owned subtree and requires `cgroup.events` to report
`populated 0` and the exact group to be removed before returning a confirmed
result; missing observations and timeouts retain an unconfirmed outcome. Future
cancellation signals the subtree
without creating a completion receipt. This is process lifecycle containment,
not a sandbox against root or deliberate same-user cgroup migration. Machine
metadata persists login and logout containment outcomes separately, before
publication. `process_trees_exited` in a native logout observation is true only
when both outcomes are durably confirmed. The Controller stores it under the
original scoped login/revocation receipt and uses it in the completion inventory
only alongside valid cleanup evidence. Missing fields in historical metadata or
receipts default to false; replay cannot upgrade or downgrade this evidence.
Native profile completion, recovery and runtime-home admission also require
the login process-tree exit proof. A successful leader exit alone cannot make
an old profile usable after restart; callers must reconcile the original
operation rather than silently upgrading its evidence.
Native reservations now retain the owning Machine process's boot ID, PID,
PID namespace and kernel start time, then bind the exact cgroup before dispatch.
Changed or unknown PID views cannot prove that a same-boot owner exited.
Reconciliation of
the original logout can recover an orphaned login or logout only after that
owner has exited. A reservation interrupted before containment binding proves
that it never reached spawn; legacy metadata without the reservation does not.
This recovery records only tree termination: it cannot infer native login or
logout success, bind an identity, or delete a home whose logout result remains
unknown. A live owner, including another in-flight retry, is left untouched.
Private-home cleanup requires both trees stopped and confirmed native logout
success, including when a legacy leader-only receipt says the command succeeded.
These additions to private profile metadata require the matching Machine reader,
including the explicit recovery release. The worker bundle must independently
support the account arguments and startup gate; accepting that aligned bundle is
a separate maintenance gate, not a consequence of Controller/Web deployment.
Native logout evidence does not establish exit for other commands or active
sessions; the internal completion transaction still checks the whole plan.
A persisted `CleanupRequired` refusal with no dispatch enrollment/revision,
native binding or completion evidence establishes `not_dispatched`. Such a
cancelled planned login needs no Machine exit/cleanup query or native logout.
An absent dispatch stamp by itself, a reserved native generation, or a
`StartRejected` result after a possible enqueue does not establish this state.
Receipt observations include `historical_grant`: `null` when no promotion journal
exists, otherwise its persisted `prepared`, `committed` (with generation), or
`cancelled` phase. This survives executor loss and is read under the same owner
and exact login request/account/Provider/Machine scope. It does not change the
live executor `status`, prove worker exit, or authorize a session. A failed
journal read returns an error rather than silently reporting no grant.

`cowboy session new PROVIDER --subscription ACCOUNT_ID` explicitly selects a
subscription independently of `--machine` and `--runtime-machine`. Omitting it
uses the server's defaults; `--subscription default` explicitly selects legacy
authentication. Server admission remains authoritative and refuses named sessions
until runtime acceptance is complete; the CLI never silently substitutes default.

Current rollout: metadata management and persisted session account identities
are present, but named-account runtime admission is not enabled. The read result
reports this explicitly. Creating a session with an unavailable named account
fails rather than silently falling back to `default`. First-party native login
acceptance and account-aware revocation must be completed before this
gate opens. Existing sessions deserialize and migrate to `default`.

### User preferences and configuration coverage

About's Usage account order and Machine order are user-owned DB state,
shared by mobile, desktop and agents. `cowboy_read({help:"preferences"})`
discovers the contract without loading unrelated declarations.
`preferences.get("about")` returns the revision, values and compact ID/name
choices, without usage details or resource metrics. `usage_order` and
`machine_order` are unique ID arrays: listed items appear first, unlisted items
retain their default order, unavailable IDs stay dormant, and empty arrays
restore defaults. Service cards retain their Plugin account key; additional
cards use their registry account ID. An old Provider order keeps its additional
accounts beside it until an explicit account order is saved. Discovery returns
registered account labels, including legacy display aliases, without running a
collector. An omitted field is unchanged.

Legacy subscription names use `preferences.get("subscriptions")` and the same
`preferences.update` action with `scope:"subscriptions"`, `expected_revision`,
and `changes:{legacy_labels:{"claude-code":"Claude · Lynx"}}`. Read and preserve
the other entries when replacing this complete Provider-to-label map; `{}`
restores the generic labels. Labels are trimmed, nonempty, at most 128 UTF-8
bytes and contain no control characters. The map holds at most 128 Provider
IDs. `accounts.list().display_preferences` projects this same state for the
mobile/desktop subscription manager and New Session picker. Its revision is
independent of the account registry: renaming the legacy `default` identity
never changes credentials, native admission, existing sessions or which account
is selected by default. Existing Controllers can ignore these preference rows
on rollback; the authority registry's stored format is unchanged.

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

## Send delivery

`queue.submit` uses `delivery: "next_turn"` by default: idle sessions dispatch,
busy sessions queue, and a manually paused queue remains held. `front: true`
changes queue position without interrupting. `delivery: "interrupt"` explicitly
cancels current work and places this message first. Legacy `force: true`
remains supported; contradictory choices are rejected.

`delivery: "current_turn"` supplements an active native turn only when the
running worker reports `supports_current_turn` through a Machine connection
negotiated at protocol 34 or later. The connection's immutable protocol gate
also refuses a capable worker behind an older host. Native Codex uses `turn/steer`;
Claude uses its ACP steering extension with `idleBehavior: "promptRequired"`.
Neither path falls back to a detached new prompt. Gemini, Grok, isolated
variants and retained workers without that contract keep queue/interrupt
available. Claude may preempt generation through native priority `now`, so this
is not a cross-provider promise to wait for the next tool call.

MCP uses its existing stable operation and control receipt. `injected` confirms
native receipt, not completion of the requested work. `not_delivered` permits
an explicit new choice; `pending` or `unknown` requires observing the original
receipt, never a new operation or an automatic queue/interrupt fallback.
Self-steering through MCP is refused because it can preempt the caller's tool.

The CLI exposes `cowboy session prompt SESSION TEXT --delivery current_turn
--operation ID` (also `next_turn` and `interrupt`, plus independent `--front`).
It prints the operation before submission and reports the actual response.
`cowboy session delivery SESSION ID` reads that original receipt.
HTTP accepts the same fields at `POST /api/sessions/{id}/prompt`; current-turn
delivery requires an operation ID. Its receipt is available at
`GET /api/sessions/{id}/delivery/{operation}` or MCP `queue.delivery_receipt`.
Reusing a current-turn operation with different content is rejected.

Web's busy Send opens a scoped delivery picker; idle, unpaused, empty Queue sends
directly. Queue and Session Settings share `queue.policy_set` with a required
revision (compare-and-swap); `queue.options` reads that preference, Pause, hold,
exact target and live capabilities without waking a worker. The preference
preselects a mode and never skips the busy picker. Unconfigured sessions default
to Steer; an explicitly saved preference is retained. Existing queued messages keep
their Next turn scheduling. MCP may explicitly request `session_default`;
resolution precedes the prohibition on steering the caller itself.

`next_checkpoint` requires Machine protocol 35 and a worker advertising that
native control. Claude 3.20.0 extends ACP's existing priority selection: consume
after foreground tools finish, or at turn end if there is no further tool
boundary. This is deliberately not named "Next tool call". Codex exposes Steer
without an artificial second checkpoint mode. Preferences store this mode as
`tool_boundary` for compatibility with the initial policy schema.

For target-bound Interrupt, supply `target:{worker_epoch,turn_id}` from
`queue.options`. The captured turn is cancelled and one replacement is admitted;
Pause and other rows are preserved. `submitted` means dispatch admission, not
completion. Uncertain cancellation or settlement leaves a durable queue hold.
After inspecting the transcript and original receipt, `queue.release_hold`
accepts the matching operation on an idle worker. It never resends the message.
Legacy `force` keeps its old semantics. CLI captures the exact target for native
delivery; `cowboy session queue-policy SESSION` reads options and accepts
`--mode MODE --revision N` to update them. `release-delivery-hold` is the same
explicit recovery action.

Web persists identity and payload in the local outbox before clearing only the
matching draft revision. The Current turn Queue lane owns pending/unknown
delivery; late native acknowledgement cannot clear a newer draft. Reload,
reconnect and Check receipt only observe the original identity, never POST into
a later turn. Current-turn sends serialize per session; Next turn retains the
server queue's ordering. Interrupt confirmation uses fresh Mod+Enter or an
explicit pointer action, excluding bare/repeated Enter and IME composition.

This extension owns cross-session authorization, queue intent, worker/turn
fencing and durable receipts. Native runtimes continue to own turn execution.
The Codex ACP compatibility patch can be removed when upstream exposes the
same opt-in no-new-turn outcome; Claude follows through its native extension.
