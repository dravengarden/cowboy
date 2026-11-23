# Uncertain effect recovery across Controller restarts

Status: L1 and L3 are implemented and accepted (2026-11-23). L1b replaces the rejected
L1w. L2 and L4 are designed here and implemented later.

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

| UTC         | Event                                                                                    |
| ----------- | ---------------------------------------------------------------------------------------- |
| 03:05:55.67 | `cowboy.service` stopping; OVH Machine: `Connection reset without closing handshake`     |
| 03:05:57–13 | OVH Machine reconnect attempts at 1.5, 4, 8.6, 17 s (exponential backoff 1, 2, 4, 8, 16) |
| 03:06:14    | `cowboy.service` started                                                                 |
| 03:06:32.64 | OVH Machine authenticated again (next backoff slot): **37 s** without a Controller       |
| 03:06:36.5  | Held worker invocations complete: `call_observed` 19–41 s for many sessions, no losses   |
| 03:06:39.8  | `sess-1795343291454`: uncertain `process/terminate` recorded                             |
| 03:06:40.4  | Its prompt finished `cancelled` (the proxy ends a turn with an unknown effect)           |

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

### Second case: a stop lost in worker teardown

`sess-1795399435399` (Antigravity) parked with `process/terminate`, sequence
2, with no Controller restart. OVH journal:

| UTC          | Event                                                                                                                |
| ------------ | -------------------------------------------------------------------------------------------------------------------- |
| 04:50:02.028 | the 63-minute prompt finished; the ledger fenced the turn (`cancelled`)                                              |
| 04:50:02.087 | `execution endpoint disconnected` (`endpoint_failed`)                                                                |
| 04:50:02.178 | the account sandbox unit (the proxy) stopped; the worker was replaced for a generation cutover (new worker 04:50:18) |
| 04:50:04.460 | Controller: uncertain `process/terminate` recorded                                                                   |

A `RunCommand` was still running at turn end. The facade's `endTurn` stops a
session's commands, and that `process/terminate` was in flight when the
worker's own teardown closed the endpoint. The worker ledger recorded the
undelivered stop as uncertain (`undelivered`, class F3). The command's process
tree exited shortly afterwards. The stop in flight was not `endTurn`'s own:
`endTurn` had confirmed for only 5 s and left the command to its expiry
timer, whose background stop or read the teardown cut off. L1b (below)
removes that.

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

| Class | Where                                                                                                                                                                                | Who records it                       | Layer                            |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------ | -------------------------------- |
| F1    | Facade: `unknown process id` for a process it started (the outage outlasted the executor's 30 s retention); misread as "termination unconfirmed"                                     | facade report                        | L1                               |
| F2    | Facade: its own wall-clock bounds (publish 30 s, terminate 5 s) elapse while requests are held                                                                                       | facade report                        | L3 (window), unchanged otherwise |
| F3    | The worker's native socket closes with requests outstanding; the client rejects them "result unknown"                                                                                | worker ledger `undelivered` + facade | L2                               |
| F4    | Worker-level loss: the 180 s bound, or a final refusal after admission                                                                                                               | worker ledger                        | L2                               |
| F5    | Keeper lost (`Unknown`, `EnvironmentLost`)                                                                                                                                           | worker ledger                        | genuine                          |
| F6    | The outage itself: a 19 s Controller restart plus up to 16 s of reconnect backoff, longer than the executor's 30 s retention                                                         | (cause of F1/F2)                     | L3                               |
| F7    | Existing worker bug: after a lost reply, a resend refused for another reason (for example `WorkspaceChanged`) is reported "not sent", although the first send may have been admitted | worker (understates)                 | L2                               |

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

| Situation                                                                       | Today                                                  | With L1                                                                                                                                                                                                                                                                                                            |
| ------------------------------------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Any reader sees E1                                                              | remote error thrown; callers misread it                | the process is marked `closed`, `ended: "forgotten"`, `exitCode` kept if an exit was already observed, otherwise `null`. No further read, terminate or signal is sent for it                                                                                                                                       |
| Publish helper ends by E1, and its `applied`/`conflict` report was already read | `UncertainEffect("process/terminate")`                 | the report decides, as for a normal exit                                                                                                                                                                                                                                                                           |
| Publish helper ends by E1 without a complete report                             | `UncertainEffect("process/terminate")`                 | the existing post-error check: read the file; if its digest equals `expect`, `applied`. This is the same evidence `publish` already accepts after a helper error. Anything else: `UncertainEffect("file publish")` and the file's `observed` digest is dropped, so `EditFile` refuses until the file is read again |
| `RunCommand` (or a continuation) ends by E1                                     | "refused the request: unknown process id", output lost | `Exit code: unknown (the command's process ended while the Machine connection was interrupted; its remaining output is unavailable)` plus the output read so far. Not an uncertain effect: the command was admitted and is no longer running. The narrower fact is stated, not success                             |
| `StopCommand` / expiry / `terminate()` confirming read sees E1                  | `terminate()` rejects, caller treats it as unconfirmed | confirmed closed                                                                                                                                                                                                                                                                                                   |
| Lost replies, timeouts, refusals                                                | fence + park                                           | unchanged                                                                                                                                                                                                                                                                                                          |

The helper is never signalled on the E1 path: E1 means it had already ended. A
helper that is still running at its deadline is terminated exactly as today,
and its publish stays uncertain, as today. E1 settles an outcome only when the
ordinary poll meets it, before any cleanup started. During cleanup (after a
timeout, truncation, loss or failure), E1 only confirms that the process is
closed and never replaces the earlier timeout, loss or failure.

A forgotten process's output may be incomplete. `runBounded` returns
`forgotten: true`. `SearchFiles` answers a retryable read error instead of a
possibly partial result. The readiness probe counts as failed, so the
projection never claims a clean tree from missing output. `RunCommand` states
that the remaining output is unavailable.

When `RunCommand` reaches its deadline it terminates the command directly. A
lost reply there is now classified like `StopCommand` (`UncertainEffect`), a
pre-existing gap the review found.

Whenever a publish ends uncertain (lost start, timed out, digest mismatch),
the facade drops that file's `observed` digest in every session it serves, the
parent and its subagents alike. The next `EditFile` of that path then needs a
fresh `ReadFile`.

The `applied` decision rests on the digest check `publish` already makes after
a helper error. It shows only the file's current bytes, so the tool says
exactly that: `Edited <file>: the requested content is verified on the target;
the helper's own report was lost`. No replacement count is claimed, and no
new evidence class is introduced. A digest equal to the base is never read as
"not applied".

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

## L1b: an ended turn confirms its commands' stops (Plugin release)

Replaces the worker-side design L1w, which design review rejected (see below).

### What the second case actually was

The proxy answers a prompt only after the facade's `endTurn`, which stops the
session's running commands. That stop was confirmed for only 5 s. A command
whose process tree exited later (here a packaged conformance run under
`nix develop`) stayed tracked, and its expiry timer kept sending stops and
confirmation reads every 5 s after the turn had completed. At 04:50:02 the
completed turn let the worker's generation cutover proceed. Teardown stopped
the proxy unit while one of those background requests was in flight, and the
worker ledger recorded it as undelivered (F3).

### Change

`endTurn` waits up to `LIMITS.stopConfirmMs` (60 s) for its commands' stops to
be confirmed, the same bound L1 gives a stopped helper. A command that exits
in time is forgotten before the turn completes. Nothing of the turn is still
being stopped when the worker may tear down, so no request can be cut off. A
cancellation keeps the 5 s wait, because the native cancel follows this cleanup.
With L1's one-stop-per-process rule, the expiry of a command that outlives the
60 s only observes; it never repeats the signal. Such a command is still
running after its turn, which is exactly what an uncertain effect should
report, so a later cut-off read still parks the session, as today.

Test: a command whose process tree exits 7 s after SIGTERM is confirmed and
forgotten at turn end, with a single `process/terminate`.

### Worker-side settlement (L1w): not adopted

The design deferred publishing a lost stop behind a durable pending record in
the worker ledger and settled it by observation. Design review (high,
`call-830719b8`) found it unsound in ways that need changes well beyond this
fix:

- `exited` precedes `closed`, and a process id alone cannot prove permanent
  absence across a keeper replacement or a late start;
- the worker's own cleanup paths (`cancel_turn`, `finish_turn`, `settle_all`)
  send their own stops, so "one stop per process" needs a worker-wide stop
  admission;
- a crash between the keeper's admission and the loss classification leaves
  no record, and the existing `mark_uncertain` publishes even when its write
  failed, so a sequence can be reused after acknowledgement;
- prompt admission can race a late publication, both after `TurnEnded` and at
  a successor's handshake;
- a helper's stop can hide an unknown publish outcome;
- an older worker launched as a fallback drops unknown record fields.

Two of these are pre-existing ledger weaknesses, independent of L1w: publishing
a sequence whose write failed, and the multiple worker stop paths. They are
recorded under Open items and belong with L2.

## L3: shorter Controller outages (Machine host release)

L3 does not make anything correct that is not already correct. It shrinks the
window in which F1/F2 can happen, for every Provider including Claude.

Of the observed 37 s outage, about 18 s was the Machine's own backoff
(1, 2, 4, 8, 16 s). A Machine whose connection had been sustained (the
existing reset condition, at least the 30 s cap) now retries failed dials at
1 s intervals inside a non-extending 60 s rapid-retry window from the
disconnect. Inside that window each dial and WebSocket handshake is bounded
to 10 s. Outside it a dial stays unbounded, as before. Bounding every dial
safely needs staggered attempts across resolved addresses (a blackholed first
address must not starve a reachable fallback). That is a follow-up, and the
pre-existing unbounded dial is unchanged. After the window, or after any failure once a WebSocket opened (an
authentication refusal included), it falls back to the existing exponential
backoff. The
rule is local to `machine_cli/reconnect.rs`. It needs no Controller change, no
close code and no protocol version, and it covers both relay legs (Hawk and
OVH Machines). The target is an outage close to the Controller's own ~19 s
start, under the executor's 30 s retention. It is measured through the first
successful relayed request after a restart. At most three Machines reconnect,
so a 1 s cadence against a starting Controller is negligible load. A refused
dial fails within milliseconds.

Dropped after design review round 2: a Controller-side drain (refuse new
relays, wait for in-flight forwards, close with 1012). It cut replies earlier
than today's process exit, which widens F7 until F7 is fixed. It competed with
the persistence shutdown for the 30 s stop budget. And it could not order a
Close behind queued replies in the transport writer. Its only benefit, replies
the Controller already had, is already recovered from the keeper's journal on
resend. Reconsider it after F7 is fixed, as one shutdown schedule that reserves
the store writer's time.

Activator quiet moment: not adopted. The component activator is
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
- Round 2 review constraints, required before implementation: a may-have-run
  record is persisted before forwarding, and a failed write blocks the request
  instead of only disabling auto-settlement. Settlement needs evidence for the
  enclosing tool effect: a publish needs its report or file verification, not
  only the helper's start and exit. A stop is settled only when no start can
  still land. Admission tombstones outlive receipts, so an evicted receipt
  answers `history_unavailable`, never "never seen". Effect ids are
  single-assignment.
- Rollout and floors. New snapshot fields (`settled_through`, L4's `target`
  and `checked`) are optional, but every hop decodes snapshots strictly. The
  Controller reader ships first, then the Machine broker, then workers emit them
  only when the broker's negotiated protocol is at least a new
  `UNCERTAIN_SETTLEMENT_PROTOCOL_VERSION` (as `retire_observation` does). The
  OVH Machine broker re-encodes snapshots for the Controller, so it also
  projects them down to the Controller's negotiated version (Welcome, snapshots
  and replayed updates). A frame without settlement fields keeps the legacy
  park. The F7 fix ships with the same worker. Retained host builds require
  byte-identical shared wire sources, so the codec change is staged with an
  accepted compatible worker pin.

## L4: recovery UX for what stays unknown (Controller + Web, later Plugin)

- Park scope: verified per session, not per account.
  `provider_accounts::controls::gate` reads `uncertain(&meta.id)`, and the
  account read model only lists each session's effect. Nothing to narrow.
- Banner: name the operation and, when known, its target
  (`file:<workspace-relative path>` or `process:<id>`, at most 256 bytes,
  never content). Until a worker supplies `target` (L2 rollout order), the
  banner shows the method alone, as today.
- "Check and resume" (implemented after L2, not before) is one Controller
  transaction, serialized through the Hub queue writer. A prompt already in
  dispatcher preflight is fenced too, and the next dispatch is reserved for the
  inspection note across retries and restarts. It verifies the shown
  sequence is still the newest unacknowledged one, inserts a resume note at
  the front of the session's queue, and records the acknowledgement, all
  committed together. If any part fails, nothing changes and the session stays
  parked. The note names the operation and target and asks the agent to inspect
  the target (read the file, check the process or command result) before any
  further change. The facade enforces it only where L1 drops the `observed`
  digest (uncertain publishes). For commands and extensions it is guidance only, which the owner
  sees before choosing the action. When several effects are unresolved, the
  note lists every effect the acknowledgement covers. Until per-effect records
  exist (L2), the action is offered only for a single unresolved effect.
- A stop that stays uncertain after L1b's 60 s turn-end confirmation shows
  that its process is still running, with its command id.
- Auto-clear: an outcome proven by L1 or L1b never parks. One proven later by
  the worker clears through L2 without owner action.

## Open items

- L3 follow-up: bound every Controller dial, also outside the rapid window,
  with staggered attempts across the resolved addresses (Happy Eyeballs), so
  a blackholed first address cannot starve a reachable fallback. Today a dial
  outside the window is unbounded, as it always was.
- F7 worker fix (with L2).
- Worker ledger: `mark_uncertain` publishes a sequence even when the ledger
  write failed, so a successor can reuse an acknowledged sequence. Publish
  only after the write commits, and hold dispatch on failure (with L2).
- Worker stop paths (`cancel_turn`, `finish_turn`, `settle_all`) each send
  their own `process/terminate`: give the worker one stop admission per
  process (with L2).

## Release artifacts

| Slice | Artifacts                                                                                                                                                                                                    |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| L1    | Antigravity Plugin (facade only)                                                                                                                                                                             |
| L1b   | Antigravity Plugin (facade only)                                                                                                                                                                             |
| L3    | Machine host release: Hawk uses the writer-preserving `.#cowboy-machine-writer-host-release` (active deletion writer); OVH uses `.#cowboy-machine-host-release`. Both retain the accepted worker pin         |
| L4    | Banner wording now needs nothing. `target`, settlement and Check-and-resume follow L2                                                                                                                        |
| L2    | Controller reader, then the Machine broker codec staged with an accepted compatible worker pin, then `.#cowboy-machine-release` with a worker pin advance (includes the F7 fix), then the Antigravity Plugin |
