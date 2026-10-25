# Remote session disaster recovery

Status: proposed design, based on a read-only production audit on 2026-10-24.
This document does not claim implementation, fault-injection acceptance, or
production activation. Source inspected: `bf876f3dd37262722e1355d1b4774ec1424dbff1`.
Production includes separately pinned Controller, Machine, worker, Provider,
and executor releases; the source revision is not their deployment receipt.

## Observed failures

Sources were Hawk's `cowboy.service` journal, OVH's Machine user-service
journal (read as its owning `ubuntu` user through the registered `ovh` alias),
Cowboy MCP session/Machine observations, and existing recovery receipts.
No running session was interrupted or sent a diagnostic prompt.

The available Controller journal from October 18 through the audit contained
57 `runtime incident opened` entries: 20 runtime failures, 19 worker startup
failures, eight interruptions, six transport failures, and four session-restore
timeouts. These are incident entries, not 57 distinct sessions or root causes.
Retention, deleted sessions, and multiple incidents per failure prevent treating
this count as a complete fleet failure rate.

| UTC observation | Evidence | Meaning and limits |
|---|---|---|
| October 22 03:15:04 | Persistence admission closed while `sess-1792637683706` emitted sequences 2621–2628; 03:15:17 reported a restart interruption | Shutdown ordering lost admission. Commit `20b870d3` subsequently fenced runtime consumption before persistence shutdown; this is historical evidence, not proof that the same bug remains in current source. |
| October 22 08:15 and 08:16 | `starting transient worker unit` for `sess-1792495812636` and `sess-1792479320182` | Runtime-worker startup failed independently of Machine connectivity. The latter recovered at 09:03. |
| October 22 14:55–15:04 | ACP `session/new` exceeded 180 seconds, and `initialize` exceeded 60 seconds, across several sessions | A reachable Machine is insufficient to prove native-session readiness. The journal alone does not establish whether each timeout was provider, transport, or load related. |
| October 22 15:05 | `Target recovery refused (EnvironmentLost)` twice for `sess-1791711326382` | A saved recovery can remain fenced. Retrying a prompt is not necessarily enough to repair its target. |
| October 24 01:56–02:31 | OVH reported a failed WebSocket liveness probe, then repeated connection failures | The outage extended well beyond a normal Controller restart window. These records do not identify the underlying network cause. |
| October 24 02:57 and 09:23 | `sess-1792627150213` failed Claude execution binding, then crashed before readiness | Native binding failure survived ordinary worker retry. At 04:42 and 09:28, the broker also reported a declared worker that never reconnected and a failed stop of its missing unit. |
| October 24 09:29:30–34 | Explicit execution maintenance retained history; the same session subsequently reported `session_running` | Scoped lifetime replacement recovered reported runtime state. That status alone is not proof of a successful subsequent remote tool call. |

During the audit, all four registered Machines reported online. Paginated session
observations still included seven crashed records. Two were earlier failed
workspace creations and several were review/test sessions. Do not automatically
wake, reset, or classify every historical crashed record as a current incident.

## Existing guarantees and remaining gaps

Existing mechanisms must be retained: detached worker adoption, fenced Machine
connections, heartbeat isolation, bounded dispatch concurrency, durable
preparation/recovery intents, native conversation resume, preserved unknown
prompts, scoped maintenance, and retained executor digests. See
[connection recovery](execution-connection-recovery.md) and
[supervisor lifetime](architecture/03-supervisor.md).

The following are source findings, not newly reproduced production failures:

1. **Event ACK is not a durable commit.** `src/remote_runtime.rs` applies an
   event to the Hub and emits `Frame::Ack`. The Hub queues `StoreWrite` work;
   `run_store_writer` in `components/server/src/server.rs` commits it later.
   `src/worker.rs::Shared::ack` drops acknowledged outbox entries. Killing the
   Controller between ACK and database commit can therefore remove the replay
   copy before its transcript projection is durable. Fixing graceful shutdown
   does not close this crash window.
2. **Worker replay and command deduplication are process-local.** The worker's
   `outbox` and `seen_commands` are in-memory collections. They help surviving
   workers reconnect, but do not alone provide host-reboot durability. Native
   provider transcripts are separate evidence and cannot be assumed to contain
   every Cowboy receipt, queue mutation, or lifecycle event.
3. **An uncertain recovery start can become a permanent fence.**
   `components/machine/src/machine_cli/execution/recovery.rs` refuses when a
   saved `launch-requested.json` or `started.json` exists but the keeper cannot
   answer. This protects against duplicate effects. It still needs a recovery
   path after independent proof that the exact attempted lifetime is gone;
   deleting its journal or starting under an unrelated ID is unsafe.
4. **Readiness is cached for the connection lifetime.** The cache in
   `components/server/src/server/execution/recovery.rs` is bounded in entry
   count, but has no time limit. A keeper can die while its resident Machine
   connection survives. Calls that report `EnvironmentLost` invalidate the
   cache, but preflight can otherwise rely on an old success and deliver a turn
   to an unusable environment before discovering the loss.

## Recovery contract

"Recovered" means an individual session can accept a new turn, resume the same
native conversation, and use the intended execution workspace. A reconnected
socket, live PID, successful enqueue, or `running` status is insufficient.

Under retained durable storage and valid credentials, acknowledged messages and
accepted recovery intentions must survive process crashes and restarts. A
destroyed disk requires verified backup restoration; loss of every durable copy
cannot be repaired by a reconnect loop. Expired credentials, revoked access,
missing source workspaces, and incompatible retained binaries must produce an
actionable blocked state rather than false readiness or infinite spinning.

The central safety rule is: automatically recover availability; observe the
original identity before deciding the fate of an uncertain effect. Never promise
exactly-once arbitrary shell execution. If a command may have executed but its
result cannot be recovered, retain `outcome_unknown`, pause dependent delivery,
and let the user inspect that command. Other sessions must continue operating.

## Proposed implementation

### 1. Commit before acknowledging

Introduce a durable runtime inbox keyed by Service, session, worker incarnation,
and runtime sequence. Accept each frame once in a local database transaction,
persist its disposition and projection work, then advance a contiguous durable
ACK watermark. Replay deduplicates against this persisted key, not only the
Controller's process-local highwater. Recovery reconstructs unapplied projection
work from the inbox. Projection side effects such as queue advancement and
permission resolution need the same identity and replay rules.

Do not hold the Machine receive loop waiting for one database write. Use bounded
per-session ingestion with fair scheduling and batched transactions. Heartbeats
and other sessions retain progress. When storage is unavailable or full, stop
acknowledging durable events, backpressure producers, and expose storage failure.
Do not drop events and continue advertising healthy persistence. Unauthorized
or stale events require a terminal rejection disposition without applying their
content; temporary admission/database failures are not terminal rejection.

Before releasing an event from a worker across a host-reboot boundary, retain it
in a private, bounded Machine-owned spool, or an equivalent durable provider
contract proven to preserve the same information. Coordinate spool admission
with emission: inability to persist cannot silently become ACK eligibility.
Reserve capacity for lifecycle/control records, bind records to their owner and
incarnation, enforce retention and deletion rules, and backpressure instead of
silently evicting unacknowledged entries. This spool owns Cowboy transport
receipts, not a replacement native transcript or agent memory store.

### 2. Reconcile each session from evidence

Extend the existing lifecycle/recovery owners with an observation projection:
`reconciling`, `ready`, `waiting_dependency`, `recovering`, `needs_attention`,
or `dormant`. Keep this separate from native turn status (`busy`, etc.). Derive
observations from durable bindings and current owner receipts; do not create a
second agent task/plan store.

On Controller startup, Machine reconnect, worker exit, target-loss notification,
and a bounded periodic sweep, reconcile only the affected sessions. Serialize
mutation per session; bound fleet and per-Machine work, add retry backoff/jitter,
and allow healthy sessions through while another target is unavailable. Keep
existing admission and memory/capacity limits. Idle history remains dormant;
opening a session must not synthesize a user prompt or start every old worker.

Recovery must verify runtime ownership, current account admission, Provider and
worker compatibility, native conversation identity, target incarnation, executor
digest, and workspace identity. Probe target readiness before new dispatch or
invalidate cached success on a bounded lease plus explicit lifecycle evidence.
A probe is read-only and must never issue a user turn or shell write. Readiness
still cannot eliminate a crash immediately after the check; dispatch and effect
receipts remain necessary.

### 3. Finish interrupted recovery transactions

Use the existing `RecoveryV1` operation identity and binding compare-and-set.
Persist and observe phase receipts for intent, old-lifetime stop, archived old
state, new contract publication, launch admission, readiness, binding commit,
and runtime adoption. An observer timeout never creates a second operation.

After a host reboot, or after authoritative proof that the exact attempted unit
and all descendants are absent, the original recovery may record a successor
attempt. Retain the failed attempt and fence its incarnation. Publish a new
incarnation through the same parent operation, with an atomic phase transition;
never erase `launch-requested.json` to reuse an old identity. A timeout, broken
socket, missing PID-cache entry, or `MainPID=0` alone is not that proof. If loss
cannot be established, report `needs_attention` with the original receipt and
precise next observation.

Recovering an environment does not replay the interrupted tool invocation or
prompt. Preserve the native conversation and worktree; pause uncertain messages.
For a prompt proved never delivered, finish delivery under its original message
identity. For an acknowledged live turn, reattach rather than submit it again.
For a dead turn, report interruption and make the session ready for subsequent
user work when safe.

### 4. Make failure and recovery inspectable

Extend existing diagnostics and session inspection, including typed Cowboy MCP
discovery, with content-free evidence: blocked dependency, original operation,
binding/incarnation, last durable sequence, last observation, retry time,
attempt count, and whether user action is required. Show these in the session UI
instead of a permanent generic spinner. Derive automatic behavior from one
shared recovery classification; UI text must not become a second state machine.

Record recovery duration, oldest unacknowledged event age, queue wait age, spool
occupancy, unknown effects, and unresolved session counts per Machine. A fleet
health check must report these alongside transport health. No prompt bodies,
credentials, or tool outputs belong in these metrics.

## Fault acceptance matrix

All destructive experiments run in isolated test services with synthetic
sessions. Do not use the production fleet as a chaos-test target.

| Failure injection | Required observation after restoration |
|---|---|
| Controller SIGTERM, repeatedly while streaming | Same surviving worker and native conversation; contiguous durable history; one prompt submission |
| Controller SIGKILL between inbox commit and ACK | Replay deduplicates, projection completes, no duplicate echo or queue advancement |
| Controller SIGKILL before inbox commit | Worker retains and replays the unacknowledged event |
| Database stalls, returns errors, or disk fills | No false durable ACK; bounded memory/spool; visible storage block; progress resumes after repair |
| Runtime Machine restart without worker maintenance | Surviving workers adopted; no duplicate native session |
| Runtime host reboot or worker OOM | Durable event/command evidence survives; same native conversation resumed where supported; uncertain effects retained |
| Target host reboot while idle | Next admitted turn recovers exact workspace and executor binding before dispatch |
| Target loss during a mutating command; reply lost | Original outcome observed or explicitly unknown; command is not replayed |
| Crash at every recovery phase, especially after launch marker and before ready receipt | Same parent operation converges or reports a precise evidence block; no second live owner |
| Multi-minute partition, stale connection replies, repeated reconnects | Old authority fenced; healthy sessions remain responsive; no timeout interpreted as process death |
| 40 sessions recovering together, one permanently unavailable target | Capacity bounds respected; independent sessions recover; no global serial wait |
| Missing workspace, changed executor digest, expired/revoked account | Specific repair action; no fallback workspace, account, local execution, or new conversation |
| Client/PWA offline during admission and then reconnect | Original message ID reconciles to queue, delivered, or unknown; no duplicate send |
| Backup restore after host replacement | Verified database/artifact consistency and identity recovery; no execution before owner fencing and retained bindings are validated |

Each scenario checks session identity, native conversation identity, workspace
identity, original command count, durable history, queue/draft ownership, and
unrelated-session progress. Repeat across Codex and Claude, SQLite and PostgreSQL,
current and retained compatible worker generations. Include randomized crash
points and deterministic receipts; a happy-path unit test is insufficient.

## Delivery order and ownership

First close the Controller ACK/commit gap with a deterministic crash-window test
and durable deduplication. Then add host-reboot spool guarantees, resumable
recovery attempts and readiness revalidation. Finally expose the unified health
projection and run the full multi-session fault matrix. The first change must
not claim the later guarantees are already implemented.

Codex owns native execution, conversation resume and tool semantics. Cowboy's
residual responsibility is its cross-Machine transport, receipts, process
ownership and workspace binding. Use native APIs and the existing Provider
adapter surfaces; do not implement a substitute executor or task runtime.
Claude is adapter-backed for native execution and conversation recovery, and
shares the same transport/lifecycle guarantees. Unsupported native recovery
retains an explicit blocked/interrupted session; it never silently starts a new
conversation or executes locally. Delete a local recovery adapter when the
native owner supplies equivalent attested recovery and fault acceptance.

Add new schema migrations; never edit deployed migration bytes. Introduce
readers and negotiated capabilities before writers. Old peers must advertise
their weaker durability boundary rather than silently satisfy the new claim.
Keep Controller, Machine host, worker pool and Provider rollout separate, with
exact artifact receipts and rollback-reader compatibility. Production acceptance
requires a disposable session to resume its native context and complete a
read-only remote tool call after each supported maintenance boundary; Machine
online and `session_running` alone do not pass.
