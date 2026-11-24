# Antigravity Current-Turn Steering (R1) — Redesign

Status: design, revision 1 (2026-11-24). Scoped strictly to **R1 (Owner current-turn steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Excludes C-budget (`session_owner_turns` / B) and Plan Mode (`R3`).

This redesign replaces the blocked R1 section of the round-3 draft (`4eed6b2d0:docs/antigravity-parity-round-3.md`, design review `call-1517db7ae578f9050a9232d4`) and closes every one of its 7 R1 findings plus the residual F1 provider-identity gate requirement.

## 1. Context & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only through `session/prompt`. Tool results carry no owner authority.
- **Mechanism:** Owner current-turn steering for Antigravity is implemented as a **proxy-hosted continuation prompt** on the active Cowboy turn: while attempt 1 is running, the proxy holds at most one pending continuation, appends a fixed Cowboy-authored data note (containing **no owner text**) to the next facade tool result asking the model to finish its current step briefly, and—when attempt 1 ends cleanly—suppresses turn completion for the worker, passes trusted cleanup and ledger checks, and dispatches the owner's blocks as attempt 2 (`session/prompt`).
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) already assigns `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy`, `agy-proxy` (`cowboy-antigravity-acp`, running under `cga-<key>` on OVH), tool output, agent text, delegated Agent API peers, and legacy components below protocol 52 / 8. Because `agy-proxy` is untrusted, its non-delivery claims (`dropped`/`refused`) can **never** authorize an automatic resend, and continuation eligibility is gated by the **trusted worker execution ledger**.
  - **Out of scope:** Same-UID local processes on the Controller/Machine host (accepted boundary from C-auth).

## 2. Requirements Traceability (Closing All Round-3 Blocking Findings)

| Round-3 Finding | Redesigned Mechanism in This Doc |
|---|---|
| **F1: Gate selected by Provider identity** | Antigravity hard-forces `supports_current_turn = false` at Worker/Broker/Controller even if upstream advertises `_meta.steering`. Antigravity steering requires `origin_steering` (Machine 52 + runtime 8 + proxy `cowboy.dev/originSteering`) AND `DeliveryOrigin::Owner`. (§3) |
| **R1 #1: Continuation after clean cleanup + trusted worker-ledger check** | `Facade.endTurn` returns `{ clean, unconfirmedCommands }`. Before writing attempt 2, proxy calls trusted worker RPC `cowboy/continueTurn { cmid }`, which checks `Ledger` (`active_turn`, `uncertain_effect == None`, `!unwritable`, `processes.is_empty()`, `starting.is_empty()`, `admitted == 0`, `calls == 0`), exact authorized Owner `cmid` (single-use), and worker monotonic TTL deadline. (§4.3) |
| **R1 #2: Ambiguous negative ACKs are `not_confirmed`** | Controller never replays `CoreCommand::Steer` after first wire write (`attempted_sends`). Worker checks `seen_commands` dedup **before** liveness so replayed commands never return `accepted: false`. Only a first-attempt, command-scoped `refused_before_handoff:*` ACK from a trusted hop settles `not_delivered`; every other failure/disconnect/proxy claim settles `not_confirmed`. (§5) |
| **R1 #3: Compaction fence** | Proxy computes `isCompactionPrompt` synchronously at `session/prompt` entry and latches `this.compacting` + `turn.compacting` **before** any `await` (`evidence.mark`, `recall`). Checked at `_session/steering` acceptance and synchronously before continuation stdin write. Controller also refuses while `Hub::is_compacting`. (§4.2) |
| **R1 #4: TTL active through preparation & rechecked before write + conservative transport accounting** | Default TTL 600,000 ms. Each hop deducts local monotonic wait + `HOP_TRANSPORT_MARGIN_MS` (1,000 ms) with zero cross-host wall clock. Proxy runs an independent TTL timer and rechecks synchronously after all `await`s (including `cowboy/continueTurn` and memory recall) immediately before writing to `agy` stdin. Worker independently enforces its own monotonic deadline inside `cowboy/continueTurn`. (§6) |
| **R1 #5: Stop admission at every congestible relay + never overtake own pending Prompt** | Dedicated bounded control lanes with biased selection and per-session prompt fences at all 4 relays: (1) `RemoteRuntime` + dedicated Cancel semaphore/waiter, (2) Controller↔Machine WS tunnel (`machine_connection.rs` & `machine_cli.rs`), (3) Broker `handle_core` outside lifecycle FIFO + per-worker writer, (4) Worker (`Cancel`/`CancelTurn` never await `steering_gate`) & Proxy (`session/cancel` bypasses `fromClient` queue and forwards native cancel before awaiting cleanup). (§7) |
| **R1 #6: Atomic server-side "Send as next message"** | Dedicated owner-only HTTP endpoints `POST /api/sessions/{id}/origin-steers/{cmid}/send-next` and `.../dismiss` backed by `origin_steer_receipts`. Atomically claims `resend_cmid = "origin-steer-resend:{cmid}"` via conditional SQL `UPDATE`, rechecks live worker epoch/turn for `not_confirmed` (refusing `409 original_turn_still_live` while live or reconciling), and enqueues once as `Owner`. (§8) |
| **R1 #7: Rollback compatibility** | Additive `origin_steer_receipts` table. Kill switch `providers.antigravity.origin_steering` (default `true`) disables offer and new steers before rollback while live accepted steers still settle. Pre-R1 Controllers drop stray `STEERING_RESULT_UPDATE` without polluting transcripts and never see `supports_current_turn = true` for Antigravity. (§9) |

## 3. Protocol, Capability Gate & Mixed-Version Matrix

### 3.1 Wire Additions (Machine Protocol 52, Runtime Protocol 8)

- `components/wire/src/machine_protocol.rs`:
  - `MACHINE_PROTOCOL_VERSION: u16 = 52`
  - `MACHINE_ORIGIN_STEERING_PROTOCOL_VERSION: u16 = 52`
- `components/wire/src/runtime_wire.rs`:
  - `PROTOCOL_VERSION: u16 = 8`
  - `ORIGIN_STEERING_PROTOCOL_VERSION: u16 = 8`
  - `WorkerSnapshot`: add `#[serde(default)] pub origin_steering: bool` (alongside existing `supports_current_turn: bool`).
  - `CoreCommand::Steer` and `WorkerCommand::Steer`: add optional fields (backward-compatible with older peers because neither enum uses `deny_unknown_fields`):
    - `#[serde(default, skip_serializing_if = "Option::is_none")] pub origin: Option<cowboy_store::prompt_origin::DeliveryOrigin>`
    - `#[serde(default, skip_serializing_if = "Option::is_none")] pub ttl_ms: Option<u64>`
  - `STEERING_RESULT_UPDATE` (`"cowboy_steering_result"`): extend optional payload fields with `"originSteer": true` and `"reason": Option<String>` alongside existing `"cmid"` and `"outcome"`. Older Controllers already ignore `STEERING_RESULT_UPDATE` in `apply_event` (`src/remote_runtime.rs:2740`).

### 3.2 Provider-Identity Gate at Every Hop

1. **Proxy (`components/provider-runtime/packages/antigravity-acp/proxy.mjs`):**
   - Strips any upstream `_meta.steering` from `agy`'s `initialize` response so upstream changes can never accidentally enable legacy `supports_current_turn`.
   - When `!this.managed`, advertises:
     `agentCapabilities._meta["cowboy.dev/originSteering"] = { v: 1, accepts: ["owner"] }`.
   - In `managed` mode, advertises neither `_meta.steering` nor `_meta["cowboy.dev/originSteering"]`.
   - On `_session/steering`: refuses unless `!this.managed` AND `params?._meta?.["cowboy.dev/origin"] === "owner"`.
2. **Worker (`src/acp.rs`, `src/worker.rs`):**
   - In `src/acp.rs:3838` (`initialize` handling), branch explicitly on `provider_id`:
     - If `provider_id == "antigravity"`:
       - `supports_current_turn` is **hard-forced `false`**, even if upstream advertises `_meta.steering`.
       - `origin_steering` is `true` iff `!state.sink.session_is_managed_read_only(&session_id)` AND `initialize_meta._meta["cowboy.dev/originSteering"]` has `v == 1` and `accepts` array containing `"owner"`.
     - For all other providers (`codex`, `claude-code`, etc.): `supports_current_turn` keeps its existing derivation and `origin_steering = false`.
   - `WorkerSnapshot.origin_steering` is updated via `current_turn_support` update (`"originSteering": bool`) and forwarded to broker on protocol >= 8.
   - In `src/worker.rs` (`WorkerCommand::Steer`):
     - If worker's provider is `"antigravity"`: requires `snapshot.origin_steering == true` AND `origin == Some(DeliveryOrigin::Owner)` AND `ttl_ms.is_some_and(|t| t > 0)`. Never accepts `supports_current_turn` for Antigravity.
     - If `origin != Some(DeliveryOrigin::Owner)` on an `origin_steering` session: returns `CommandAck { accepted: false, reason: Some("refused_before_handoff:origin_not_accepted") }`.
     - Passes `_meta["cowboy.dev/origin"] = "owner"` and `_meta["cowboy.dev/steering"] = { ttlMs }` on `_session/steering` to the proxy.
3. **Machine Broker (`components/machine/src/machine_broker.rs`):**
   - Copies `origin_steering` on `WorkerSnapshot` from protocol-8 workers (default `false` for <= 7).
   - Forwards `origin` and `ttl_ms` on `WorkerCommand::Steer` only to protocol-8 workers.
   - In `CoreCommand::Steer` gate (`machine_broker.rs:4555`):
     - If `worker.snapshot.launch.as_ref().is_some_and(|l| l.provider == "antigravity")`: requires `worker.snapshot.origin_steering && origin == Some(DeliveryOrigin::Owner) && worker.protocol >= ORIGIN_STEERING_PROTOCOL_VERSION` (never falls back to `supports_current_turn`).
     - Rejections emit `CommandAck { accepted: false, reason: Some("refused_before_handoff:<detail>") }`.
4. **Controller (`src/core.rs`, `src/remote_runtime.rs`, `components/server-runtime/src/server/send_delivery.rs`):**
   - `SessionMeta` gains `pub origin_steering: bool` (projected from `WorkerSnapshot.origin_steering` only when Machine protocol >= 52 AND Service config `providers.antigravity.origin_steering == true`).
   - Kill switch: `src/config/schema.rs` adds `PROVIDERS_ANTIGRAVITY_ORIGIN_STEERING: Key<bool>` (`providers.antigravity.origin_steering`, default `true`), stored on `AppState` and checked at both projection (`delivery-options`, `SessionMeta`) and admission.
   - In `send_delivery.rs` (`submit` / `begin_origin_steer`):
     - For `meta.provider == "antigravity"`:
       - Checks **only** `meta.origin_steering && state.origin_steering_enabled` (never `meta.supports_current_turn`).
       - `caller != RequestCaller::Person`:
         - If `delivery == Some(CurrentTurn | NextCheckpoint)`: refused with `403 origin_not_accepted`.
         - If `delivery == None` (session default): falls back to ordinary `next_turn` queue (existing behavior).
       - `delivery == Some(NextCheckpoint)` maps to the same single continuation mechanism as `CurrentTurn`.
       - Slash-command refusal (`400 slash_command_not_steerable`): joins all text blocks, trims, tests `^/\S` (matching the JS predicate in `proxy.mjs` against a shared vector table).
       - Compaction refusal (`409 compacting`): checks `state.hub.is_compacting(&session)`.
       - Clear refusal (`409 clearing`): checks `state.hub.clear_pending(&session)`.

### 3.3 Mixed-Version & Dual-Flag Matrix (All Rows Tested)

| Controller | Machine (neg.) | Worker (neg.) | Proxy / Upstream | Antigravity Steer Outcome |
|---|---|---|---|---|
| pre-R1 (or rollback) | any | any | new (even if upstream also sets `_meta.steering`) | Not offered: worker forces `supports_current_turn = false` for Antigravity; old Controller ignores `origin_steering` |
| new (kill switch off) | 52 | 8 | new | Not offered; new steers refused (`origin_steering_disabled`); already-accepted steers still settle |
| new | <= 51 | any | new | Not offered: broker <= 51 drops `origin_steering` and `origin` |
| new | 52 | <= 7 | new | Not offered: worker <= 7 ignores `cowboy.dev/originSteering` and never sets `origin_steering` |
| new | 52 | 8 | old | Not offered: old proxy advertises neither capability |
| new | 52 | 8 | new + upstream dual-flag (`_meta.steering` + `originSteering`) | `supports_current_turn` stays `false`; `origin_steering` is `true`; Owner accepted, Peer/Automation/Cowboy refused `origin_not_accepted` |

## 4. Proxy Continuation, Cleanup & Worker-Ledger Gate

### 4.1 Synchronous Turn & Compaction Installation (`proxy.mjs`)

On `session/prompt` entry (before **any** `await`):
1. Compute `const isCompact = isCompactionPrompt(prompt)`.
2. Install the `turn` object in `this.turns.set(sessionId, { id, preparing: true, cancelled: false, compacting: isCompact, continuation: undefined, ... })`.
3. If `isCompact`, immediately latch `this.compacting.add(sessionId)`.
4. Run asynchronous preparation (`evidence.mark()`, `recall()`).
5. Synchronously check `if (turn.cancelled)` right before `this.agent(forwarded)` — if cancelled during preparation, drop any pending continuation (`cancelled`), clean up `this.turns` / `this.compacting`, and answer the prompt as `{ stopReason: "cancelled" }` **without ever writing to `agy`**.
6. Set `turn.preparing = false` and write `this.agent(forwarded)`.

### 4.2 `_session/steering` Acceptance (`proxy.mjs`)

- Dispatched directly in `fromClient` **bypassing `this.queue`** (never waits behind slow prompt preparation or session setup).
- Refuses immediately (`result: { outcome: "refused", reason }`) and emits `cowboy/steeringOutcome { cmid, outcome: "refused", reason }` if ANY holds:
  - `this.managed !== false`;
  - `params?._meta?.["cowboy.dev/origin"] !== "owner"`;
  - `typeof cmid !== "string" || cmid.length === 0`;
  - `ttlMs` missing, non-finite, or `<= 0`;
  - No active `turn` for `sessionId`, or `turn.cancelled`, or `turn.settling`, or `turn.uncertain !== undefined`, or `turn.violation !== undefined`;
  - `this.compacting.has(sessionId) || turn.compacting`;
  - `turn.continuation !== undefined` (`reason: "busy"`);
  - `isSlashCommand(params.prompt)` (`reason: "slash_command_not_steerable"`).
- Otherwise stores `turn.continuation = { cmid, prompt: params.prompt, deadline: performance.now() + ttlMs, timer, noteSent: false }`, starts independent `setTimeout` firing at `ttlMs` (which synchronously drops `turn.continuation` and emits `cowboy/steeringOutcome { cmid, outcome: "dropped", reason: "expired" }`), and answers `{ outcome: "accepted" }`.
- **Tool-boundary note:** On the next facade tool result for `sessionId` while `turn.continuation && !turn.continuation.noteSent`, `Facade` appends one fixed Cowboy-authored text block (no owner text):
  `"The owner sent a new message. Finish the current step briefly and end your turn."`
  and sets `turn.continuation.noteSent = true`.

### 4.3 Clean Attempt-1 Settlement + Trusted Worker-Ledger Gate (`cowboy/continueTurn`)

When `agy` answers attempt 1's `session/prompt`:
1. Latch `turn.settling = true` synchronously so no new `_session/steering` can be accepted during settlement.
2. Run `const cleanup = await runtime.facade.endTurn(record.sessionId)` and `await this.subagents.finish(record.sessionId)`:
   - **`Facade.endTurn` return value:** `{ clean: boolean, unconfirmedCommands: number }`.
   - `clean` is `true` iff `turn?.uncertain === undefined` AND `session.commands.size === 0` (every command started in attempt 1 was confirmed terminated and removed from `session.commands`; a failed or timed-out `process/terminate` leaves the command in `session.commands` so `clean === false`).
3. Run `const settled = await runtime.evidence.settle(record.cursor, record.sessionId, message.result?.stopReason)`.
4. Evaluate continuation eligibility ONLY if `turn.continuation` is still present:
   - Attempt 1 must have `message.result?.stopReason === "end_turn"`, `settled.class === "ok"`, `turn.violation === undefined`, `turn.uncertain === undefined`, `!turn.cancelled`, `!this.compacting.has(sessionId)`, `!turn.compacting`, `cleanup.clean === true`, `performance.now() < continuation.deadline`, and `!isSlashCommand(continuation.prompt)`.
   - If ANY local check fails -> drop continuation (`cowboy/steeringOutcome { cmid, outcome: "dropped", reason }`) and finish attempt 1 normally.
5. **Trusted worker-ledger gate (`cowboy/continueTurn`):**
   - Proxy calls `runtime.facade.connection.request("cowboy/continueTurn", { cmid: continuation.cmid })` over the worker's execution endpoint (`src/worker_execution/ledger.rs`).
   - Inside the trusted worker (`Ledger` + turn's active origin-steer state):
     - Atomically verifies under `ledger.state.lock()`:
       1. `state.active_turn()` matches the active Cowboy turn (not closed, not fenced/cancelled);
       2. `state.record.uncertain_effect.is_none()` and `!state.unwritable` (catches any lost `process/terminate`, `fs/writeFile`, `process/start`, or extension call reply);
       3. `state.processes.is_empty()` and `state.starting.is_empty()` (zero live or starting target processes in the trusted ledger);
       4. `state.admitted.get(&turn).copied().unwrap_or(0) == 0` and `state.calls == 0` (zero in-flight mutations or extension calls);
       5. The worker's pending origin-steer authorization matches `cmid`, has not yet been consumed, and `Instant::now() < worker_steer_deadline` (independent trusted monotonic TTL check!).
     - Consumes the single-use continuation authorization and returns `{ result: { allow: true } }`; otherwise returns `{ result: { allow: false, reason } }`.
   - If `!auth.allow` (or RPC error) -> drop continuation (`cowboy/steeringOutcome { cmid, outcome: "dropped", reason: auth.reason ?? "ledger_refused" }`) and finish attempt 1 normally.
6. **Attempt-2 preparation & final synchronous write gate:**
   - Prepare attempt 2 (`evidence.mark()`, skill expansion, `memory.recall()`, restatement if needed).
   - **Final synchronous recheck immediately before `this.agent(continuationPrompt)`:** verify `!turn.cancelled`, `!this.compacting.has(sessionId)`, `performance.now() < continuation.deadline`, and `!isSlashCommand(continuation.prompt)` — if any failed during preparation, drop continuation (`dropped`, reason) and finish attempt 1 normally without writing to `agy`.
   - Otherwise clear `continuation.timer`, update `turn.settling = false`, call `runtime.facade.beginTurn(sessionId)`, accumulate attempt 1's backend evidence & usage on `turn`, write `this.agent(continuationPrompt)` to `agy` stdin, and emit `cowboy/steeringOutcome { cmid, outcome: "sent" }`.
   - **Echo attribution:** When the worker receives `cowboy/steeringOutcome { cmid, outcome: "sent" }`, the worker echoes the stored Owner blocks as `user_message_chunk` (first block tagged with `cmid`, Controller `annotate_dispatched_prompt` authoritatively stamps `Owner` from `prompt_dispatches`) and forwards `STEERING_RESULT_UPDATE` (`originSteer: true`, `outcome: "sent"`).
   - **If `agy` rejects attempt 2:** the turn ends with that JSON-RPC error as any rejected prompt does; the receipt remains `delivered` (attempt 2 was written to `agy`).

## 5. Unambiguous Receipt State Machine & First-Handoff Proof

### 5.1 States

`origin_steer_receipts.status`:
- `pending`: admitted durably, waiting for delivery outcome.
- `delivered`: worker confirmed `outcome == "sent"` (continuation written to `agy`).
- `not_delivered`: **trusted first-handoff proof** that the steer never reached `agy-proxy`.
- `not_confirmed`: every ambiguous failure (`dropped`, `refused`, link drop after wire send, worker epoch exit, restart timeout, missing outcome).

### 5.2 Closing the Worker Dedup-vs-Liveness Race (`src/worker.rs`)

1. **Controller never resends `CoreCommand::Steer` after first transmission:**
   - Before writing `CoreCommand::Steer` to the broker socket, `RemoteRuntime` records `command_id` in `shared.attempted_sends`.
   - On broker reconnect or disconnect, any `CoreCommand::Steer` in `shared.attempted_sends` is **removed from `pending` (never replayed)** and its `origin_steer_receipts` row settles as `not_confirmed` (`"transport_interrupted"`).
2. **Worker checks dedup BEFORE liveness (`src/worker.rs:900-946`):**
   - Step 1: If `shared.has_seen_command(&command_id)` -> return `CommandAck { accepted: true, reason: Some("duplicate") }` (never `accepted: false`).
   - Step 2: Run all liveness/capability/origin/TTL/epoch/turn/drain checks. If any fail -> record `command_id` in `shared.refused_commands` (so any impossible later replay stays refused) and return `CommandAck { accepted: false, reason: Some("refused_before_handoff:<reason>") }`.
   - Step 3: `shared.mark_command(&command_id)` and enqueue `AgentCommand::Steer` (arming the worker's single-use `cowboy/continueTurn` authorization for `(turn_id, cmid, deadline)`). Return `CommandAck { accepted: true, reason: None }`.
3. **Controller settlement rule:**
   - `CommandAck { accepted: false, reason }` settles `not_delivered` **only if** `command_id` matches the receipt's `command_id` AND `reason` starts with `"refused_before_handoff:"` (produced only by Controller pre-wire gates, Broker before `worker.tx.send`, or Worker Step 2 before `AgentCommand::Steer` enqueue).
   - Any other negative ACK, proxy `refused`/`dropped`, disconnect, or ended worker epoch settles `not_confirmed`.
4. **Settle-before-ACK durability (`src/remote_runtime.rs`):**
   - In `handle_frame` for `Frame::WorkerEvent` carrying `STEERING_RESULT_UPDATE` with `originSteer == true`: await `store.settle_origin_steer_receipt(...)` **before** updating `shared.highwaters` and **before** writing `Frame::Ack`. If the DB write fails, return `Err` without ACKing so the worker replays the event on reconnect.
   - On Controller startup (`startup_restore.rs`), reload non-dismissed `origin_steer_receipts`; keep `pending` rows live while their `worker_epoch` is live, and settle them to `not_confirmed` (`"worker_epoch_ended"`) as soon as runtime reconciliation or snapshot proves that epoch is gone.

## 6. Monotonic TTL Accounting

- Constant `DEFAULT_ORIGIN_STEER_TTL_MS = 600_000` (10 min); conservative per-hop transport deduction `HOP_TRANSPORT_MARGIN_MS = 1_000`.
- **Controller:** captures `admitted_mono = Instant::now()`. On first wire write, `remaining = ttl_ms.saturating_sub(admitted_mono.elapsed_ms + 1_000)`. If `0`, settles locally as `not_delivered` (`"refused_before_handoff:expired"`).
- **Broker:** captures `received_mono = Instant::now()`. Before forwarding to worker, `remaining = ttl_ms.saturating_sub(received_mono.elapsed_ms + 1_000)`. If `0`, returns `CommandAck { accepted: false, reason: Some("refused_before_handoff:expired") }`.
- **Worker:** captures `received_mono = Instant::now()`. Deducts local wait + `1_000`. Stores `deadline = Instant::now() + remaining` on the turn's authorized steer record (checked in `cowboy/continueTurn`) and sends `ttlMs: remaining` on `_session/steering`.
- **Proxy:** stores `deadline = performance.now() + ttlMs`, starts independent `setTimeout` for `ttlMs`, rechecks at settlement before preparation, and rechecks synchronously after `cowboy/continueTurn` + recall immediately before writing to `agy` stdin.

## 7. Bounded Stop Admission at Every Relay (With Per-Session Prompt Fence)

**Core invariant:** `Cancel` / `CancelTurn` bypasses congestion from other sessions, bulk payloads, and slow lifecycle commands at every relay, **and never overtakes an unsent `Prompt` for the same `session_id`**.

1. **Controller `RemoteRuntime` + `send_delivery.rs`:**
   - Dedicated `CANCEL_IN_FLIGHT` semaphore (separate from `send_delivery::IN_FLIGHT`) and dedicated `cancel_waiters` map on `RemoteRuntime::Shared` (separate from `shared.steering`), so a slow/pending steer never blocks Stop or Interrupt.
   - In `send_pending`: `CoreCommand::Cancel` and `CoreCommand::CancelTurn` sort to highest priority (`0`), **except** if `shared.pending` holds an unsent `CoreCommand::Prompt` for the same `session_id`, `Cancel`/`CancelTurn` is fenced behind that `Prompt` (never overtakes it).
2. **Controller ↔ Machine WS Tunnel (`machine_connection.rs` & `machine_cli.rs`):**
   - Split both the Controller→Machine runtime bridge (`forward_machine_runtime_frames` + `write_machine_messages`) and Machine→Broker command channel (`runtime_commands`) into `(control_tx, ordinary_tx)` with `tokio::select! { biased; ... }`.
   - Maintain per-session count of `CoreCommand::Prompt` currently queued in `ordinary_tx`. A `Cancel` / `CancelTurn` for `session_id` enters bounded `control_tx` (coalesced / 1 reserved slot per active session) **only when** that session's queued `Prompt` count in `ordinary_tx` is `0`; otherwise it enters `ordinary_tx` directly behind its own pending `Prompt`.
3. **Machine Broker (`machine_broker.rs`):**
   - In `handle_core`: add a bounded control channel `(control_tx, control_rx)` (1 reserved slot per active session via `pending_control_sessions`) executed outside the `command_tx` lifecycle FIFO, so a slow `EnsureSession`/`Reset`/`Hibernate` never stalls Stop.
   - Track `queued_prompt_sessions: Mutex<HashMap<String, usize>>` for `command_tx`. Route `Cancel`/`CancelTurn` to `control_tx` iff `queued_prompt_sessions[session_id] == 0`; otherwise enqueue on `command_tx` behind the pending `Prompt`.
   - On `ConnectedWorker`: split `worker.tx` into `(control_tx, ordinary_tx)` with biased draining and the same per-session queued-`Prompt` count fence.
4. **Worker (`src/acp.rs`) & Proxy (`proxy.mjs`):**
   - Worker `AgentCommand::Cancel` and `AgentCommand::CancelTurn` **never await `steering_gate`** (releasing `steering_gate` before any blocking steering RPC so Stop is immediate even if an RPC is in flight). `AgentCommand::Cancel` also fences `Ledger::cancel_turn()` and revokes the turn's authorized steer token immediately.
   - Proxy `fromClient`: `session/cancel` bypasses serialized `this.queue`.
   - Proxy `AcpProxy.cancel`:
     1. Synchronously latch `turn.cancelled = true`;
     2. Synchronously drop `turn.continuation` and emit `cowboy/steeringOutcome { cmid, outcome: "dropped", reason: "cancelled" }`;
     3. Immediately forward native `session/cancel` to `agy` (`this.agent(message)`);
     4. Then await `facade.endTurn(sessionId, { cancelled: true })` and `subagents.cancelChildren(sessionId)`.

## 8. Durable Receipts, "Send as Next Message" & Web UX (W1)

### 8.1 Additive Store Schema

New migrations `migrations/0090_origin_steer_receipts.sql` & `migrations/sqlite/0064_origin_steer_receipts.sql`:

```sql
CREATE TABLE origin_steer_receipts (
    session_id TEXT NOT NULL,
    cmid TEXT NOT NULL,
    command_id TEXT NOT NULL,
    worker_epoch TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    text TEXT NOT NULL,
    content_json TEXT NOT NULL,
    admitted_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    status TEXT NOT NULL, -- 'pending' | 'delivered' | 'not_delivered' | 'not_confirmed'
    reason TEXT,
    resend_cmid TEXT,
    dismissed_at_ms BIGINT,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, cmid)
);
CREATE INDEX idx_origin_steer_receipts_active
    ON origin_steer_receipts (session_id, updated_at_ms DESC)
    WHERE dismissed_at_ms IS NULL;
```

No FK to `sessions(id)` (avoiding write-behind `InsertSession` races, matching `prompt_dispatches`); cleaned in `Store::purge_deleted` with the same expired-session + 1h orphan sweep pattern.

Admission writes both `origin_steer_receipts` (`status = 'pending'`) and `prompt_dispatches` (`origin = Owner, trigger = Owner`) before handoff.

### 8.2 Server-Side Owner Actions

- `POST /api/sessions/{id}/origin-steers/{cmid}/send-next`:
  - Requires `RequestCaller::Person` (`403 owner_required` otherwise).
  - Loads receipt `(id, cmid)`; requires `status IN ('not_delivered', 'not_confirmed')` and `dismissed_at_ms IS NULL`.
  - If `status == 'not_confirmed'`: checks Controller's live worker inventory. If the receipt's `worker_epoch` is still live and its `turn_id` is active (or Machine runtime is disconnected/reconciling so liveness is unproven), refuses `409 original_turn_still_live`.
  - Atomically claims resend via conditional SQL `UPDATE origin_steer_receipts SET resend_cmid = $resend_cmid, dismissed_at_ms = $now, updated_at_ms = $now WHERE session_id = $1 AND cmid = $2 AND status IN ('not_delivered', 'not_confirmed') AND dismissed_at_ms IS NULL AND resend_cmid IS NULL`, with deterministic `resend_cmid = format!("origin-steer-resend:{cmid}")`.
  - If already claimed (`resend_cmid IS NOT NULL`), returns `200 { status: "already_queued", resend_cmid }` idempotently without duplicate enqueue.
  - Otherwise submits to `Hub::submit` as `RequestCaller::Person` (`DeliveryOrigin::Owner`) with `cmid = Some(resend_cmid)` and the receipt's stored `(text, content)`.
- `POST /api/sessions/{id}/origin-steers/{cmid}/dismiss`:
  - Requires `RequestCaller::Person`. Sets `dismissed_at_ms = now_ms` idempotently.
- `GET /api/sessions/{id}/delivery-options`:
  - Exposes `"origin_steering": info.meta.origin_steering` and `"origin_steer_receipts": [...]` (active non-dismissed receipts, plus `original_turn_live: bool` computed server-side for each `not_confirmed` row).
- WebSocket broadcasts updated `origin_steer_receipts` on status transition so all open devices stay in sync.

### 8.3 Web UI (`web/`)

- `web/src/protocol.ts`: add `origin_steering?: boolean` to `SessionInfo`.
- `web/src/Composer.tsx`, `web/src/QueuedDeliveryAction.tsx`, `web/src/store.ts`:
  - Antigravity Steer availability reads `session.origin_steering === true` (never `supports_current_turn` for Antigravity).
  - When an `origin_steer_receipt` settles `not_delivered` or `not_confirmed`, display the retained steer text banner/card with two actions:
    - **Send as next message** (calls `POST .../send-next`; disabled with explanation while `status === "not_confirmed" && original_turn_live === true`).
    - **Dismiss** (calls `POST .../dismiss`).
  - For `not_confirmed`, explicitly state that the original steer may still have reached the agent. Never auto-enqueue.

## 9. Release Sequencing & Rollback

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller R1)** — lands on `main` together with the `cowboy-workers` pin commit (so `retained-worker-interface-compatible` stays satisfied).
   - Activate **Controller** first (additive migration; older Machines/workers safely ignore new optional fields and `origin_steering` stays `false`).
   - Run full pin-advance acceptance (`docs/machine-worker-pin-advance-runbook.md`), then pause for explicit owner approval before activating Hawk Machine & OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` + 7-plugin closure + Antigravity Plugin `0.2.35`)** — includes the `subagents.test.mjs` `requestMs: 1000` test timing fix. Harmless before M1 (worker <= 7 ignores `cowboy.dev/originSteering`).
3. **Slice W1 (Web)** — atomic `/run/cowboy-web` swap; offers Antigravity Steer only when `origin_steering === true`.
4. **Rollback Procedure:**
   - Before rolling back to a pre-R1 Controller: set `providers.antigravity.origin_steering = false` and wait for `origin_steer_receipts` with `status = 'pending'` to drain to 0 (or live Antigravity turns to end).
   - Roll back Controller (additive table is ignored; Antigravity `supports_current_turn` remains `false`; any stray `STEERING_RESULT_UPDATE` is ignored without entering the transcript).
   - On subsequent re-upgrade, `startup_restore` settles any remaining `pending` receipts whose epochs ended to `not_confirmed`.

## 10. Verification Plan

- **Mixed-version & dual-flag matrix:** automated tests covering all 6 rows in §3.3, including an upstream advertising both `_meta.steering` and `cowboy.dev/originSteering` over rollback links.
- **Worker-ledger & cleanup gate:** tests where `process/terminate` reply is lost (`ledger.uncertain_effect != None`), where a command remains in `Facade` (`cleanup.clean == false`), and where `ledger.live()` is non-empty -> `cowboy/continueTurn` returns `allow: false` and continuation is dropped.
- **Dedup-before-liveness & negative ACK:** worker test proving a replayed `WorkerCommand::Steer` after turn end returns `accepted: true (duplicate)`, never `accepted: false`; Controller test proving post-send disconnect and non-`refused_before_handoff:` ACKs settle `not_confirmed`, while pre-handoff refusal settles `not_delivered`.
- **Compaction & slash fences:** proxy test proving steer arriving during `/compact` preparation (`await recall`) is refused `compacting`; shared Rust+JS slash predicate vector test (`/compact`, leading whitespace, multi-block split `/com` + `pact`).
- **TTL:** proxy test dropping pending continuation while turn progresses; expiry during continuation preparation before final stdin write; cross-hop conservative margin deduction.
- **Stop priority & prompt fence:** real relay congestion test (congested Controller link + blocked broker lifecycle FIFO + outstanding 170s extension call + pending continuation) proving `session/cancel` reaches `agy` promptly and drops continuation; per-session prompt-fence test proving `Cancel` never overtakes an unsent `Prompt` for the same session.
- **Receipts & Send-as-next:** crash-between-event-and-ACK durability test; restart recovery settling ended epoch as `not_confirmed`; concurrent double-click `send-next` atomically enqueueing once; `409 original_turn_still_live` refusal while epoch live; `403 owner_required` for Automation/Agent callers; kill switch off refusing new steers while settling accepted ones.
