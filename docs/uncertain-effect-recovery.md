# Uncertain effect recovery across Controller restarts

Status: design (2026-11-23). Slices L3 and L1 are implemented first; L4 next;
L2 is designed here and implemented later.

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

| Class | Where                                                                                                                                                         | Who records it                     | Layer   |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- | ------- |
| F1    | Facade: `unknown process id` for a process it started and saw running (the outage outlasted the 30 s retention)                                                | facade report                      | L1      |
| F2    | Facade: its own wall-clock bounds (publish 30 s, terminate 5 s) elapse while requests are held                                                                 | facade report                      | L1      |
| F3    | The worker's native socket closes with requests outstanding (`serve` returns an error, worker restart); the client rejects them "result unknown"                | worker ledger `undelivered` + facade | L1, L2 |
| F4    | Worker-level loss: 180 s bound, or a final refusal after admission                                                                                             | worker ledger                      | L2      |
| F5    | Keeper lost (`Unknown`, `EnvironmentLost`)                                                                                                                     | worker ledger                      | genuine |
| F6    | The outage itself: 19 s Controller restart plus up to 16 s of reconnect backoff, longer than the executor's 30 s retention                                       | (cause of F1/F2)                   | L3      |

## Principles

- Never replay a mutation. A settlement reads state; the only repeated request
  is `process/terminate`, which is idempotent on the executor (a missing or
  exited process answers `running: false`).
- An outcome is settled only by authoritative evidence from the target. A
  wall-clock deadline, a failed read or a missing reply is never evidence.
- What cannot be proven stays `UncertainEffect`, with the existing durable
  park and acknowledgement.
- The worker ledger stays the trusted recorder. The confined proxy can add
  uncertainty, never remove it. Removing a worker-recorded uncertainty is L2
  and happens in the worker.

## L1: settle by observation in the facade (Plugin release)

Scope: `components/provider-runtime/packages/antigravity-acp/facade.mjs` and
the shared `execution-connection.mjs`. Ships in the Antigravity Plugin. The
Claude Plugin only picks up the additive Connection helper. Claude never parks
a session: its lost replies already become tool errors that tell the model to
inspect first, so its behaviour is unchanged in this slice.

### Evidence rules

E1, process ended. For a process whose `process/start` reply the facade
received, `unknown process id` from `process/read` proves the process ended.
The pinned executor removes an entry only (a) 30 s after it closed, or (b) when
`process/terminate` hits it while it is still `Starting`, which cancels the
start. (b) cannot apply after a received start reply, because the entry is
`Running` by then. The keeper keeps one executor session per incarnation.
`initialize` with `resumeSessionId` refuses a different session, so the id
cannot name another executor's table. The exit code and any unread output are
lost. The process is not running and never runs again.

E2, a read can be repeated. `process/read` with the same `afterSeq` returns
the same retained chunks (the executor keeps up to 1 MiB or 50,000 chunks per
process and evicts nothing on read). Repeating a read whose reply was lost is
not a replay. If chunks were evicted, the facade sees a sequence gap and keeps
what it has.

E3, terminate is idempotent. Repeating `process/terminate` for a started
process is safe, as is sending it to a forgotten one.

E4, the publish outcome is in the file. The helper's `apply` keeps the
original in place until the replacement is renamed over it, so the file holds
either the base or the result. Once E1 or an observed exit proves the helper
ended, the file's SHA-256 decides:

- equal to `expect`: applied;
- equal to the base, or still absent for a new file: not applied (a dead
  helper cannot rename later);
- anything else: unknown (another writer changed it).

### Rules per operation

| Lost or unconfirmed                                                        | Settlement                                                                                                                                                                                                      | Result                                                                                                 |
| -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `process/read` reply (connection lost)                                     | Reconnect within the settle window and read again from the same cursor (E2)                                                                                                                                      | settled; uncertain only if the window ends unreachable                                                 |
| `process/read` answers `unknown process id` for a started process          | E1: mark it closed with `exitCode: null` and `reaped: true`                                                                                                                                                      | settled                                                                                                |
| `process/terminate` reply, or termination unconfirmed within 5 s           | Reconnect, terminate again (E3), observe by reading until exited, closed or E1, within the window                                                                                                               | settled when ended; otherwise uncertain, as today                                                      |
| Publish helper without a usable report (timed out, reaped, output missing) | Make sure it ended (terminate and observe as above), then E4                                                                                                                                                     | applied, not applied (`ToolError`: nothing was written, read and retry), or uncertain                    |
| `fs/writeFile` of a large publish request (private payload file)           | The payload lives in Cowboy's private request directory, outside the workspace, and has no effect until a helper reads it. Write a fresh payload under a new random name and remove both after the helper settles | settled; the worker ledger may still record the lost write (F3/F4)                                      |
| `process/start` reply                                                      | Reconnect and read the process id. If it exists, it started: continue normally. If it is unknown, an in-flight start may still land after the query (the worker relays independently of the socket), so this is not provable | started, or uncertain (L2 removes the residue)                                                          |
| `RunCommand` process forgotten (E1)                                        | Answer `Exit code: unknown`, plus a note that the command ended while the Machine connection was interrupted and its final output is unavailable                                                                | settled: the command was admitted and is no longer running; only its report is missing                  |

### Mechanics

- `Connection.reconnected({ windowMs })` retries `ensureConnected` with a
  capped backoff (250 ms up to 2 s) until it connects or the window ends. The
  facade uses `SETTLE_WINDOW_MS = 60_000`, enough for a Controller restart
  plus reconnect. Ordinary calls keep failing fast as today.
- Gate during settlement: while a settlement runs, new mutations of the same
  facade (the session and its subagents, which share one facade) wait for it.
  Settled: they proceed. Uncertain: they are refused, the family is fenced
  (`onLost`), and the report follows exactly as today. The family is therefore
  never mutating while an outcome is open, which is the property the immediate
  `onLost` fence protects, and a settled outcome no longer ends the turn.
- Deadlines: the helper's 30 s bound still stops waiting. Its expiry now leads
  to settlement, not directly to `UncertainEffect("file publish")`.
- A `file publish` or `process/start` that stays uncertain drops the file's
  `observed` digest, so `EditFile` refuses until the model reads the file again
  (enforced, not just requested in the prompt).

### Fault injection tests (fake endpoint)

The fake endpoint gains `retainClosedMs` (forgets a closed process like the
executor) and answers `process/terminate` with `running`. Tests cover:

- the incident: a publish whose helper is forgotten before its reads reach the
  target, with an applied result, ends `Edited` and reports nothing uncertain;
- the helper forgotten with the file at its base: `ToolError`, nothing written;
- the helper forgotten with a third-party change: uncertain;
- a relay drop (socket terminated) at each step: before or after the helper
  start, on a read, on terminate, and on a large request's payload write,
  each with a reconnect inside and outside the window;
- a `RunCommand` forgotten mid-read: `Exit code: unknown`, not uncertain;
- a lost start: started when the id exists, uncertain when it is unknown;
- the gate: a concurrent mutation waits for a settlement and proceeds or is
  refused.

A Controller-restart simulation in the Rust execution conformance harness
exercises the worker and keeper side (L3).

## L3: shorter Controller outages (Controller release, small Machine follow-up)

L3 does not make anything correct that is not already correct. The keeper
journal does that. It shrinks the window in which F1/F2 can happen, for every
Provider including Claude.

L3a, Controller drain (Controller-only). On SIGTERM:

1. New execution relay requests are refused `Unavailable`. The worker keeps
   them pending under the same identity and re-sends them after reconnect.
2. In-flight forwards get up to 8 s to finish and queue their replies. The
   worker otherwise waits out the whole outage for replies the Controller
   already had. Long polls (`Events`, and `Invoke`/`Observe` waiting up to
   20 s) are not worth more than that bound: their repeat after reconnect is
   answered from the keeper's journal.
3. Each Machine WebSocket is closed with code 1012 ("service restart") after
   its queued replies are written, instead of a TCP reset.

The whole sequence stays well inside `TimeoutStopSec=30s` together with the
existing persistence shutdown. A Controller drain adds at most 8 s to a
restart, and only when a forward is still running.

L3b, Machine reconnect cadence (Machine release, separate slice). A Machine
whose Controller closed with 1012 retries every 1 s for up to 60 s before it
falls back to the exponential backoff. The observed outage drops from 37 s to
the Controller's own 19 s start, below the executor's 30 s retention. Both ends
of the relay benefit: the Hawk Machine reconnects with the same loop. An older
Machine ignores the close code and behaves as today. An older Controller sends
no 1012, so a newer Machine behaves as today. No protocol version is needed.

L3c, activator quiet moment: not adopted. The component activator is
Columbus-owned and restarts `cowboy.service` on demand. Waiting for "no
in-flight execution" would rarely find a quiet moment on a busy Controller and
would delay releases without bound. With L1 and L3b, a restart in the middle of
a turn is settled automatically. The better lever is serializing the
activations of concurrent sessions, which the coordinator already does by
policy.

## L2: trusted settlement in the worker (design; Machine release)

Goal: remove F3/F4 and the "start unknown" residue, and let a proven outcome
clear the durable park without the owner.

- Receipts. The worker endpoint keeps, per session, a bounded journal (256
  entries, 15 min) of the mutations and result-carrying requests it relayed,
  keyed by the facade's effect key: `process/start`, `process/terminate` and
  `process/read` by `processId`, `fs/*` by path and a facade-chosen `effectId`
  carried beside the native params (stripped before forwarding). Each entry
  holds its relay state: `pending`, `completed` (with the reply), or `unknown`
  (the worker's own invocation lost, F4).
- Query. A new endpoint request, `cowboy/effectStatus {key}`, is answered from
  the journal. For `pending` it waits up to 20 s. The worker relays a request to
  completion independently of the native socket, so after a socket drop (F3)
  the receipt eventually reads `completed`, and the facade learns the real
  reply: the start's outcome, the terminate's `running`, the read's chunks.
- Fencing a never-seen id. Absent from the journal means the request never
  reached this worker (the native socket dropped before the frame was read),
  because the worker records the entry before forwarding. The new worker's
  admission refuses a late frame for a key already queried as absent, closing
  the race the L1 start rule cannot.
- Clearing the park. The ledger's `undelivered` mark records the receipt key.
  When the agent side later obtains that receipt's completed reply through
  `cowboy/effectStatus`, the worker publishes the effect as `settled` (a new
  optional `settled_sequence` on the runtime `UncertainEffect` snapshot). The
  Controller treats a settled sequence as acknowledged for parking. The worker
  is the party that learned the agent now knows the outcome, so the trust
  boundary is unchanged.
- Mixed versions and floors. The facade probes `cowboy/effectStatus`. An older
  worker answers `refused`, and the facade stays on L1. `settled_sequence` is
  `serde(default, skip_serializing_if)`. The runtime snapshot struct denies
  unknown fields, so the Controller reader must ship (Controller release)
  before any worker emits the field. The worker gates emission on the broker's
  protocol (a new `UNCERTAIN_SETTLEMENT_PROTOCOL_VERSION`), like
  `retire_observation`. The worker's durable ledger file keeps schema 1, and the
  settled mark lives only in the published snapshot and the Controller's
  acknowledgement record, so a worker rollback reads its ledger unchanged. This
  changes the worker binary, so it needs `.#cowboy-machine-release` with a
  worker pin advance and its separate maintenance acceptance. It is a
  follow-up, not part of these slices.

## L4: recovery UX for what stays unknown (Controller + Web, later Plugin)

- Park scope: verified per session, not per account.
  `provider_accounts::controls::gate` reads `uncertain(&meta.id)`, and the
  account read model only lists each session's effect. Nothing to narrow.
- Banner: name the operation and its target. The runtime `UncertainEffect`
  gains optional `target` (`file:<workspace-relative path>` or
  `process:<id>`, bounded to 256 bytes, never content) and `checked` (the
  facade's post-reconnect check: `unreachable`, `still_running`,
  `file_changed`, `start_unconfirmed`). Both are optional fields with the same
  Controller-first rollout as L2's `settled_sequence`. Until a worker supplies
  them, the banner shows the method alone, as today.
- One action, "Check and resume": the Controller re-reads the session's
  uncertain record and, if it is still the same sequence, acknowledges it and
  queues a resume note for the agent: which operation and target were
  uncertain, and that it must read the affected file (or inspect the process)
  before editing. The facade already enforces the re-read (L1).
- Auto-clear: an outcome proven by L1 never parks. One proven later by the
  worker (L2) clears through `settled_sequence` without owner action.

## Release artifacts

| Slice | Artifacts                                                                                                                                                                           |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L3a   | Controller (`.#cowboy-controller-release`)                                                                                                                                          |
| L1    | Antigravity Plugin (facade). The Claude Plugin only if released for other reasons: its Connection gains an additive, unused helper                                                    |
| L3b   | Machine host release (`.#cowboy-machine-host-release`; the reconnect loop is resident Machine code, not worker code)                                                                  |
| L4    | Controller (wire fields, reader first) + Web; then the Antigravity Plugin to supply `target`/`checked`                                                                               |
| L2    | Controller (reader) first, then `.#cowboy-machine-release` with a worker pin advance, then the Antigravity Plugin                                                                     |
