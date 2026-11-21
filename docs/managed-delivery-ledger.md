# Managed prompt delivery ledger (broker tombstones) — rejected

Status: **rejected, not implemented** (2026-11-21). The owner chose this
narrow scope over a general exactly-once protocol (compare the abandoned
transport in [`widgets.md`](widgets.md) §1.9–§1.13). The first Astra Max
design review (call `call-f5c59c49ec773fae38b44b76`, verdict
needs-attention, 15 high and 3 medium findings) showed that the narrow scope
is not sound on its own; closing it would rebuild that general protocol, so
the owner stopped it. The windows stay documented in
[`review-generation-binding.md`](review-generation-binding.md).

Why it fails (the review's blocking findings, grouped):

- **No coverage of the past or of other builds.** A prompt routed by a
  protocol-7 broker before the upgrade has no entry, so the first withdrawal
  would certify a live turn as unseen. A Machine rollback, or a broker that
  could not read the dataset, still forwards a late dispatch after a
  committed withdrawal; the dataset is therefore safety-critical under
  `plugin-state-dataset-design.md` and would need a reader floor. Eviction
  at capacity can erase a running call's evidence.
- **Epoch scoping is wrong both ways.** A replacement worker after a
  generation cutover would run a restored row again, while a `routed` commit
  followed by a lost channel write suppresses a prompt no worker received.
- **The Controller cannot consume the answers safely.** A duplicate
  acknowledgement plus a stale idle snapshot releases the dispatch guard
  early; a pending copy's own Prompt command is left outstanding; each
  Machine connection has its own runtime, so a pending withdrawal does not
  survive a reconnect; worker outbox events are acknowledged before the Hub
  persists them, so a terminal event can be lost; a `routed` withdrawal still
  leaves later copies and a session-scoped Cancel eligible for delivery.
- **Other fences are missing.** Terminal Session deletion does not refuse a
  late Prompt; a client may submit an ordinary prompt with a `managed-call:`
  cmid; both mixed-version directions change queue behaviour.
- **Release cost.** The wire change forces a worker pin advance, and Hawk's
  deletion and incarnation writers need the writer-host artifact built at
  the advanced pin (`docs/machine-worker-pin-advance-runbook.md`).

Two findings were independent, Controller-only defects and were fixed
instead: a withdrawn call's `Stopping` and `Cancelled` now commit in one
transaction (a crash between them used to leave `Stopping`, whose stop grace
then stopped the child), and the recovery resubmission re-reads the call
after its slow reconfirmation so a cancellation committed meanwhile wins.

The original proposal follows for the record.

## Problem

A managed call has exactly one prompt per round, addressed by the stable
client message id `managed-call:<call_id>` (cmid). The Controller removes the
prompt's queue row at drain time and persists that removal write-behind
(`src/core/hub_queue.rs`, `src/core/persistence_queue.rs`). Every runtime
`command_id` is new per send and per Controller process
(`src/remote_runtime.rs`), and the only Machine-side dedup is by
`command_id` in memory (broker pending queue, worker `seen_commands`). Hence,
as recorded in [`review-generation-binding.md`](review-generation-binding.md):

1. **Duplicate execution.** A Controller crash after the Machine accepted the
   prompt but before the row removal committed restores the row; a later
   drain delivers it again under a new `command_id`, and the surviving worker
   runs a second turn.
2. **False cancellation.** Withdrawing such a restored row on cancellation is
   no proof that no turn started, yet the call finishes `Cancelled` at once
   while the accepted turn may still run.
3. **Late cancellation.** A cancellation committed between the runner
   recording `Running` and the first submission falls back to the native stop
   and its 60 s grace, although nothing was delivered.

Out of scope (unchanged, still documented residuals): a requeue after a full
dispatch channel or onto a child whose worker exited waits for the next drain
trigger; a replacement worker relies on replay of runner-written preferences.

## Native coverage and residual gap

ACP has no idempotency key for `session/prompt`, and neither Codex nor Claude
deduplicates a resent user turn. The residual gap is Cowboy's own
Controller→Machine hop, so the fix belongs to the Machine broker, the one
process that survives a Controller restart and sees every write to a worker.
Antigravity is not a managed-call target (`CALL_TARGETS`, `docs/requirements.md`)
and is unaffected; as a parent it only consumes call results.

Deletion trigger: if the runtime gains an end-to-end delivery identity (for
example a durable Controller dispatch ledger acknowledged by the Machine), or
ACP adds prompt idempotency, this dataset can be removed.

## Design

### Broker dataset `managed-deliveries`

A new Machine file namespace below the Machine state directory, using the
shared `machine_broker/namespace.rs` mechanics (no links, exclusive lock,
retained handles, staged write + fsync + rename + directory fsync).

```json
{"schema":1,"owner":{"machine_id":"…","service_id":null},
 "entries":[{"session_id":"…","cmid":"managed-call:…","at_ms":0,
             "state":"routed","worker_epoch":"…"},
            {"session_id":"…","cmid":"managed-call:…","at_ms":0,
             "state":"withdrawn"}]}
```

- Only cmids with the `managed-call:` prefix are recorded. Keys are
  `(session_id, cmid)`; call ids are unique, so a key never recurs for new
  work.
- `routed {worker_epoch}` is committed **before** the broker writes the prompt
  to that worker's channel. `withdrawn` is committed before the broker answers
  a withdrawal as withdrawn. A record never moves from `withdrawn` to
  `routed`; `routed` may be re-pointed to a newer epoch (see below).
- Bounds: 4096 entries, 1 MiB. Entries of a session are dropped when the
  broker commits that session's terminal deletion. When full, the oldest
  entry by `at_ms` is evicted. Eviction only matters if the Controller
  re-delivers an old call's prompt, which it already prevents independently
  (terminal transitions withdraw the row, and Controller start removes
  restored rows of ended calls before Machines reconnect).
- Classification (per `docs/plugin-state-dataset-design.md`): **advisory**.
  An older Machine ignores the dataset; a Controller negotiating it learns the
  capability from the runtime protocol and falls back to today's behaviour
  otherwise. No owner floor or writer admission is needed, and an unreadable
  or invalid dataset never stops the Machine: the broker logs, leaves the file
  untouched, routes prompts without recording them (today's behaviour) and
  answers every withdrawal `unavailable`, so the Controller uses the native
  stop. No `withdrawn` answer is ever given without a committed tombstone.

### One serialization point

All decisions run under one broker mutex holding the store:

- **Write to a worker** (`route_worker` with a connected worker, and every
  prompt in `flush_pending`): for a managed cmid, under the store lock,
  - `withdrawn` → drop the prompt and answer the Controller
    `CommandAck {accepted: false, reason: MANAGED_PROMPT_WITHDRAWN}`;
  - `routed` to the same `worker_epoch` → drop it as a duplicate (below);
  - otherwise commit `routed {epoch}` and write the frame while still holding
    the lock. If the commit fails, the prompt is not written and the
    Controller gets `accepted: false` with a retryable reason (ordinary
    requeue; no prompt reached any worker).
- **Arrival** (`CoreCommand::Prompt`): under the store lock, `withdrawn`
  refuses as above; `routed` to the current worker's epoch, or the same cmid
  already in the broker's pending queue, is a duplicate; otherwise the prompt
  continues to the existing route/queue path.
- **Withdrawal** (`CoreCommand::WithdrawManagedPrompt`): under the store lock,
  `routed` (any epoch) → answer `routed`; otherwise remove any pending copy
  from the broker queue, commit `withdrawn`, answer `withdrawn`. A failed
  commit answers `unavailable` (the pending copy is not restored; the call is
  being cancelled anyway and the Controller falls back to the native stop).

Because every worker write checks and commits under the same lock, a
`withdrawn` answer is a proof that no worker received the prompt and none ever
will from this Machine.

### Duplicates

A duplicate is answered `CommandAck {accepted: true, reason: "duplicate"}`
followed by the worker's current snapshot, so the Controller's idle
reconciliation can release its dispatch guard if the original turn already
ended. The original turn's events keep reaching the Controller through the
worker's outbox replay.

Dedup is scoped to the worker epoch that received the prompt. A replacement
worker (crash, hibernation) is a new process with no in-flight turn of its
own, so the existing recovery path that resubmits to it keeps working; the
record is re-pointed to the new epoch. A broker restart loses only the
in-memory pending queue; `routed` is recorded only for prompts actually
written, so no prompt is wrongly treated as delivered.

### Wire

Runtime protocol 8 (`MANAGED_DELIVERY_PROTOCOL_VERSION`):

- `CoreCommand::WithdrawManagedPrompt {session_id, command_id, cmid}`, sent
  only when the negotiated broker protocol is ≥ 8.
- `Frame::ManagedPromptWithdrawal {session_id, command_id, cmid, outcome}`,
  `outcome ∈ {withdrawn, routed, unavailable}`.
- `MANAGED_PROMPT_WITHDRAWN` refusal reason constant.

Worker frames are unchanged; workers never see these messages. The change to
`components/wire` makes this a full Machine release (worker pin advance), not
a host-only release.

### Controller

- `RemoteRuntime::withdraw_managed_prompt(session, cmid)` returns
  `Withdrawn | Routed | Unsupported | Unavailable`, with a bounded timeout
  (`Unavailable`). Pending withdrawals replay on reconnect; the broker answer
  is idempotent. `Supervisor` exposes it, and
  `supports_managed_withdrawal(session)`.
- A refusal with `MANAGED_PROMPT_WITHDRAWN` is never requeued: the Controller
  drops the prompt, removes any queued copy and releases the dispatch guard.
- Cancellation (coordinator `cancel_call` and runner `withdraw_unstarted`):
  discard the queued row as today, then
  - on a Machine with the capability, ask the broker: `withdrawn` →
    `Running → Stopping → Cancelled` at once, whether or not a queue row
    existed (closes 2 and 3); `routed` or `unavailable` → the ordinary native
    stop (closes 2: a restored row's withdrawal alone never finishes the call);
  - on an older Machine, today's behaviour.
  The coordinator only discards the row; the Machine round trip runs in the
  runner, which the coordinator notifies.
- A review that is no longer admissible still fails at once; the runner also
  sends a best-effort withdrawal so a restored copy can never be delivered.

## Compatibility

| Controller | Machine | Behaviour |
| --- | --- | --- |
| old | old | today |
| new | old (protocol 7) | today (no withdrawal sent) |
| old | new | duplicates absorbed by the broker; cancellation as today |
| new | new | this design |

Rolling the Machine back leaves the file unread; rolling forward again reads
it (all entries remain valid because call ids are unique).

## Tests

- Broker: arrival/withdraw/flush under every ordering (withdraw before
  arrival, between arrival and flush, after write); duplicate on the same
  epoch absorbed, redelivery to a new epoch allowed; commit failure on route
  and on withdraw; restart reload; bounds and eviction; terminal deletion
  removes a session's entries; invalid file disables the capability without
  rewriting it.
- Controller: withdrawn refusal is not requeued and releases the guard;
  cancellation outcomes (`withdrawn`, `routed`, `unavailable`, unsupported)
  drive the expected ledger transitions, including a restored row and a
  cancellation before first submission; duplicate ack plus idle snapshot
  releases the guard.
