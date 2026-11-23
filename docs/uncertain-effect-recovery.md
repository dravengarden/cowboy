# Uncertain effect recovery across Controller restarts

Status: design (2026-11-23, revised after design review round 1). L3a and L1
are implemented first, then L4 without `target`. L2 and L3b are designed here
and implemented later.

## Problem

Runtime proxies on OVH reach the Hawk execution keeper through their worker's
execution endpoint, the OVH Machine, the Controller relay and the Hawk Machine.
A Controller restart (component activation) breaks the Machine WebSockets.
Twice on 2026-11-23 an Antigravity turn was fenced with
`cowboy.uncertain_effect` and its session parked until the owner acknowledged
it. Example: `sess-1795343291454`, sequence 6, `process/terminate` of an
`EditFile` publish helper, around the 03:05:55 Controller restart.

## What actually happened

Evidence from the Controller journal (VictoriaLogs) and the OVH journal:

| UTC          | Event                                                                                   |
| ------------ | --------------------------------------------------------------------------------------- |
| 03:05:55.67  | `cowboy.service` stopping; OVH Machine: `Connection reset without closing handshake`    |
| 03:05:57–13  | OVH Machine reconnect attempts at 1.5, 4, 8.6, 17 s (exponential backoff 1, 2, 4, 8, 16) |
| 03:06:14     | `cowboy.service` started                                                                |
| 03:06:32.64  | OVH Machine authenticated again (next backoff slot): **37 s** without a Controller      |
| 03:06:36.5   | Held worker invocations complete: `call_observed` 19–41 s for many sessions, no losses   |
| 03:06:39.8   | `sess-1795343291454`: uncertain `process/terminate` recorded                            |
| 03:06:40.4   | Its prompt finished `cancelled` (the proxy ends a turn with an unknown effect)          |

No `response_timeout`, `call_failed` or `execution.disconnected` event exists
for that worker (PID 624848). Every transport call was answered. The
uncertainty was reported by the facade itself (`cowboy/uncertainEffect`).

Root cause, reproduced with the fake endpoint once it forgets closed processes
like the pinned executor:

1. The pinned Codex exec-server (0.159.3, `local_process.rs`) removes a process
   **30 s after it closes** (`EXITED_PROCESS_RETENTION`). A later
   `process/read` answers the remote error `unknown process id <id>`;
   `process/terminate` answers `running: false`.
2. The publish helper finished while the Controller was down. The facade's
   reads were held by the worker (which retains their identity) and reached the
   executor 37 s later, after the helper had been forgotten.
3. `runUntracked` treated the remote error as a failure, called `terminate`,
   whose confirming read failed the same way, so `terminate()` rejected,
   `catch(() => false)` turned that into "not closed", and the facade raised
   `UncertainEffect("process/terminate")`.

The edit had in fact been applied. In the reproduction the file holds the new
bytes and the turn is still fenced. The transport never lost anything; the
facade misread "process forgotten" as "termination unconfirmed".

## What the transport already guarantees

The task statement assumed a keeper journal had to be built. Most of it exists:

- The worker gives each native request an `Invocation.operation_id` and keeps
  the request pending across Machine and Controller reconnects. It re-sends the
  same `RuntimeRequest` on `Replay` and every 30 s (`worker_execution.rs`,
  `frames`). `Refused{Unavailable|Capacity}` keeps it pending; it never
  re-sends under a new identity.
- The Hawk keeper (`execution_host/ledger.rs`) admits an operation once per id
  (digest-checked; a different body under the same id is `OperationConflict`),
  answers repeats from the retained outcome, and keeps tombstones for 15 min,
  beyond the worker's 180 s bound. `Observe{operation_id}` reports
  `Pending | Completed | Unknown | ResultExpired | Missing`.
- So, between the worker and the keeper, a Controller restart costs latency,
  not correctness, as long as the worker's 180 s wait is not exceeded.

What remains is above and beside that path:

| Class | Where                                                                                                                                                       | Who records it                       | Layer    |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ | -------- |
| F1    | Facade: `unknown process id` for a process it started (the outage outlasted the executor's 30 s retention); misread as "termination unconfirmed"               | facade report                        | L1       |
| F2    | Facade: its own wall-clock bounds (publish 30 s, terminate 5 s) elapse while requests are held                                                               | facade report                        | L3 (window), unchanged otherwise |
| F3    | The worker's native socket closes with requests outstanding; the client rejects them "result unknown"                                                       | worker ledger `undelivered` + facade | L2       |
| F4    | Worker-level loss: the 180 s bound, or a final refusal after admission                                                                                      | worker ledger                        | L2       |
| F5    | Keeper lost (`Unknown`, `EnvironmentLost`)                                                                                                                  | worker ledger                        | genuine  |
| F6    | The outage itself: a 19 s Controller restart plus up to 16 s of reconnect backoff, longer than the executor's 30 s retention                                  | (cause of F1/F2)                     | L3       |
| F7    | Existing worker bug: after a lost reply, a resend refused for another reason (for example `WorkspaceChanged`) is reported "not sent", although the first send may have been admitted | worker (understates)                 | L2       |

F7 can understate uncertainty, so it is a pre-existing safety bug, not a
source of false parks. It is fixed in the worker with L2. `invoke_classified`
must treat every send after the first as "possibly admitted".

## Principles

- Never replay a mutation, and never signal a process to settle its outcome.
- Settle an outcome only from authoritative evidence that is already in hand.
  A wall-clock deadline, a failed read or a missing reply is never evidence. A
  lost reply keeps today's immediate family fence and park.
- What cannot be proven stays `UncertainEffect`, with the existing durable
  park and acknowledgement.
- The worker ledger stays the trusted recorder. The confined proxy can add
  uncertainty, never remove it. Removing a worker-recorded uncertainty is L2
  and happens in the worker.

## L1: stop misreading a forgotten process (Plugin release)

Scope: `components/provider-runtime/packages/antigravity-acp/facade.mjs`.
Ships in the Antigravity Plugin. The shared `execution-connection.mjs` and the
Claude Plugin are unchanged. L1 targets exactly F1, the class that produced
both incidents. Every transport call in F1 was answered, so nothing is lost and
there is nothing to wait for. A design that reconnected and waited inside the
facade (round 1 of this document) was dropped: it could not preserve the
immediate `onLost` family fence, and its evidence (file digests after
signalling a helper, re-reads of possibly evicted output, replacement payload
writes) did not prove history.

### Evidence rule E1: a forgotten process ended

Preconditions, all required:

- the facade generated the `processId` (a fresh UUID with the
  `cowboy-agy-` or readiness prefix, never reused);
- the facade received that process's successful `process/start` reply on the
  same Connection, whose reconnects are pinned to the same executor session
  (`initialize` with `resumeSessionId` refuses a different one);
- a later `process/read` of that id answered the executor's own remote error
  whose message is exactly `unknown process id <that id>`. A transport loss,
  a worker or ledger refusal (`code -32000` with Cowboy's text), or any other
  message does not qualify.

Then the process has ended and will never run again. The pinned executor
(0.159.3, `local_process.rs`) removes an entry only 30 s after the process
closed, or when `process/terminate` meets it still `Starting`. The latter
cancels the start, and it cannot happen after a received start reply, when the
entry is `Running`. E1 says nothing about the exit status, and nothing about
output the facade has not read. Those are lost.

### What changes

| Situation                                                       | Today                                                     | With L1                                                                                                                                                                                              |
| --------------------------------------------------------------- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Any reader sees E1                                              | remote error thrown; callers misread it                   | the process is marked `closed`, `ended: "forgotten"`, `exitCode` kept if an exit was already observed, otherwise `null`. No further read, terminate or signal is sent for it                         |
| Publish helper ends by E1, and its `applied`/`conflict` report was already read | `UncertainEffect("process/terminate")`                    | the report decides, as for a normal exit                                                                                                                                                              |
| Publish helper ends by E1 without a complete report             | `UncertainEffect("process/terminate")`                    | the existing post-error check: read the file; if its digest equals `expect`, `applied`. This is the same evidence `publish` already accepts after a helper error. Anything else: `UncertainEffect("file publish")` and the file's `observed` digest is dropped, so `EditFile` refuses until the file is read again |
| `RunCommand` (or a continuation) ends by E1                     | "refused the request: unknown process id", output lost   | `Exit code: unknown (the command's process ended while the Machine connection was interrupted; its remaining output is unavailable)` plus the output read so far. Not an uncertain effect: the command was admitted and is no longer running. The narrower fact is stated, not success |
| `StopCommand` / expiry / `terminate()` confirming read sees E1  | `terminate()` rejects, caller treats it as unconfirmed   | confirmed closed                                                                                                                                                                                      |
| Lost replies, timeouts, refusals                                | fence + park                                              | unchanged                                                                                                                                                                                             |

The helper is never signalled on the E1 path: E1 means it had already ended. A
helper that is still running at its deadline is terminated exactly as today,
and its publish stays uncertain, as today.

The `applied` decision rests on the digest check `publish` already makes after
a helper error. It shows only the file's current bytes. If another writer put
identical bytes there, the agent's intended state still holds, which is all the
tool result claims. No new evidence class is introduced. A digest equal to the
base is never read as "not applied".

### Tests (fake endpoint)

The fake endpoint gains `retainClosedMs` (forgets a closed process with the
executor's exact message) and answers `process/terminate` with `running`.
Tests:

- the incident: a publish whose helper is forgotten before its reads reach the
  target, with the result applied: `Edited`, nothing uncertain, no
  `process/terminate` sent;
- the same with the helper's report read before it was forgotten;
- forgotten with the file not at `expect`: `UncertainEffect("file publish")`,
  and a following `EditFile` requires a fresh `ReadFile`;
- `RunCommand` forgotten mid-read and on a `command_id` continuation:
  `Exit code: unknown`, not uncertain;
- E1 negatives: a ledger refusal with a similar message, a different id in the
  message, and an id whose start reply was lost all keep today's behaviour;
- a relay drop (socket terminated) at each step still fences and reports, as
  today.

## L3: shorter Controller outages (Controller release, then a Machine release)

L3 does not make anything correct that is not already correct. It shrinks the
window in which F1/F2 can happen, for every Provider including Claude.

L3a, Controller drain (Controller-only), all against one monotonic deadline
taken at SIGTERM:

1. Admission closes at once. `spawn_execution_request` is the single choke
   point for the primary connection and the bulk lanes, and it refuses new
   relays with `Unavailable`. The worker keeps them pending under the same
   identity and re-sends them after reconnect.
2. In-flight forwards may finish and queue their replies until SIGTERM + 3 s.
   This runs concurrently with axum's existing graceful HTTP drain, not after
   it. Long polls (`Events`, and `Invoke`/`Observe` waiting up to 20 s) are cut
   at the bound: their repeat after reconnect is answered from the keeper's
   journal.
3. Each Machine connection then waits, within the same deadline, for its
   writer queue to be empty, and sends Close 1012 ("service restart"). This is
   best effort. A reply cut off here is re-sent and answered from the keeper's
   journal. Delivery to the worker is never assumed.
4. The existing persistence shutdown (telemetry, store writer, dispatcher)
   keeps its budget unchanged and starts no later than SIGTERM + 3 s, inside
   `TimeoutStopSec=30s`.

L3b, Machine reconnect cadence (resident Machine code, separate Machine host
release). A Machine whose Controller closed with 1012 retries at 1 s intervals,
each dial and handshake bounded to 5 s, inside a non-extending 60 s
rapid-retry window. After that it falls back to the exponential backoff. A
reset that erased the close frame, an older Machine or an older Controller
behaves as today, with no protocol version needed. The target, with both
Machines upgraded and receiving 1012, is an outage close to the Controller's
own ~19 s start, under the executor's 30 s retention. It is measured through
the first successful relayed request after restart.

L3c, activator quiet moment: not adopted. The component activator is
Columbus-owned. Waiting for "no in-flight execution" on a busy Controller would
delay releases without bound. L1 removes the observed false positive, and
serializing concurrent activations is the coordinator's existing policy.

## L2: trusted receipts in the worker (design only; Machine release)

Goal: settle F3, F4 and lost-start residue from the worker's own records, and
let a proven outcome clear the durable park without the owner. Round 1 review
findings are folded in. L2 is implemented later, after its own design review.

- Identity. Each native request the facade sends carries a fresh
  `cowboy_effect_id` (UUID) in a Cowboy envelope member the worker strips
  before forwarding. A receipt is keyed by (worker epoch, effect id) and
  stores the method and a digest of the native params. A lookup with a
  mismatched method or digest is refused. Process ids or paths are never keys.
- Receipts keep the keeper `operation_id` and binding of every invocation,
  also after the worker's 180 s wait expired (F4). A later status query can
  then `Observe` the original operation without a new invoke. `ResultExpired`,
  `Missing` and `Unknown` stay unknown.
- Negative evidence. "Never reached this worker" is reported only by the same
  worker epoch, atomically with installing a fence that refuses a later frame
  with that effect id. A new worker epoch, an evicted receipt or an expired
  fence answers `history_unavailable`, which stays unknown. The fence lives
  for the worker's lifetime.
- Settlement requires terminal, operation-specific evidence, not delivery of a
  reply. `process/start` completed with a result and its process observed
  ended, or the start refused by the executor. `fs/*` completed. `terminate`
  completed with `running: false` or followed by an observed end. A `read`'s
  completed reply, delivered through a status query that the facade
  acknowledges in a follow-up request, because its value is consumed by the
  agent side.
- Per-effect park. The ledger records each unresolved effect by its sequence.
  The Controller's acknowledgement stays a watermark, but an automatic
  settlement advances it only across a contiguous prefix of resolved
  sequences. A settled N+1 never releases an unresolved N. New losses,
  settlements and owner acknowledgements are serialized in the ledger.
- Durability. A sequence is published only after the ledger write that
  allocated it succeeded. If the write fails, the worker refuses
  auto-settlement for that session until the owner acknowledges. The ledger file
  keeps schema 1, and resolved marks live in a sibling file that an older
  worker ignores. Rollback to an older worker loses only auto-settlement.
- Rollout and floors. New snapshot fields (`settled_through`, L4's `target`
  and `checked`) are optional, but every hop decodes snapshots strictly. The
  Controller reader ships first, then the Machine broker, then workers emit them
  only when the broker's negotiated protocol is at least a new
  `UNCERTAIN_SETTLEMENT_PROTOCOL_VERSION` (as `retire_observation` does). The
  F7 fix ships with the same worker.

## L4: recovery UX for what stays unknown (Controller + Web, later Plugin)

- Park scope: verified per session, not per account.
  `provider_accounts::controls::gate` reads `uncertain(&meta.id)`, and the
  account read model only lists each session's effect. Nothing to narrow.
- Banner: name the operation and, when known, its target
  (`file:<workspace-relative path>` or `process:<id>`, at most 256 bytes,
  never content). Until a worker supplies `target` (L2 rollout order), the
  banner shows the method alone, as today.
- "Check and resume" is one Controller transaction. It verifies the shown
  sequence is still the newest unacknowledged one, inserts a resume note at
  the front of the session's queue, and records the acknowledgement, all
  committed together. If any part fails, nothing changes and the session stays
  parked. The note names the operation and target and asks the agent to inspect
  the target (read the file, check the process or command result) before any
  further change. For files the facade enforces this (L1 drops the `observed`
  digest). For commands and extensions it is guidance only, which the owner
  sees before choosing the action.
- Auto-clear: an outcome proven by L1 never parks. One proven later by the
  worker clears through L2 without owner action.

## Release artifacts

| Slice | Artifacts                                                                                                                                       |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| L3a   | Controller (`.#cowboy-controller-release`)                                                                                                      |
| L1    | Antigravity Plugin (facade only)                                                                                                                |
| L3b   | Machine host release (`.#cowboy-machine-host-release`; the reconnect loop is resident Machine code)                                             |
| L4    | Controller + Web (banner, transaction); `target` arrives with L2                                                                                |
| L2    | Controller reader, then Machine broker, then `.#cowboy-machine-release` with a worker pin advance (includes the F7 fix), then the Antigravity Plugin |
