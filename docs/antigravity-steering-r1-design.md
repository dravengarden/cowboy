# Antigravity Current-Turn Steering (R1) — Redesign (Revision 2)

Status: design, revision 2 (2026-11-24). Scoped strictly to **R1 (Owner current-turn steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B) and Plan Mode (`R3`).

This specification replaces the blocked R1 section of `4eed6b2d0:docs/antigravity-parity-round-3.md` (design review `call-1517db7ae578f9050a9232d4`) and closes all 22 findings (15 High, 7 Medium) from Design Review Round 1 (`call-472a0e37073961cd97fb95f1`).

---

## 1. Context, Scope & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority.
- **Mechanism:** Owner current-turn steering for Antigravity is a **proxy-hosted continuation prompt** within the active Cowboy turn:
  1. While attempt 1 is active, the proxy holds at most one pending Owner continuation for that turn's exact `attemptNonce`.
  2. On the next facade tool result, the proxy appends a fixed Cowboy-authored data note (containing **zero owner text**) advising the model to wrap up its current step concisely.
  3. When `agy` finishes attempt 1, the proxy withholds turn completion from the worker, fences facade calls and subagents together, passes a **two-phase trusted worker execution-ledger check** (`cowboy/fenceAttempt` then `cowboy/commitContinuation` after preparation), and dispatches the Owner's blocks as attempt 2 (`session/prompt`) mapped back to attempt 1's worker request `replyId`.
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`, hosting both `src/acp.rs` and the authenticated execution endpoint `src/worker_execution.rs` + `Ledger` + `RelayQuiet`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy`, `agy-proxy` (`cowboy-antigravity-acp`, running under `cga-<key>` in `account_sandbox_v1`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because `agy-proxy` is untrusted, its non-delivery claims can never authorize an automatic resend, and continuation admission, ledger cleanliness, TTL enforcement, and single-use Owner echo authorization remain enforced inside the **trusted worker**.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), and queued-row Steer for Antigravity (disabled in R1; see §8.3).

---

## 2. Crate Boundaries & Wire Contract (Protocol 52 / Runtime 8)

### 2.1 Dependency-Safe Wire Types (`components/wire`, Leaf Crate)

`cowboy-store` depends on `cowboy-wire` (`components/store/Cargo.toml`). To avoid a crate cycle, `cowboy-wire` defines minimal transport enums with zero dependency on `cowboy-store` or caller classification:

```rust
// components/wire/src/runtime_wire.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireDeliveryOrigin {
    Owner,
    NonOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteerRefusalCode {
    Unsupported,
    OriginNotAccepted,
    TargetTurnMismatch,
    Compacting,
    AlreadyPending,
    TurnNotActive,
    ExpiredBeforeHandoff,
    WorkerNotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandRefusal {
    RefusedBeforeHandoff { code: SteerRefusalCode },
}
```

- Controller converts verified `cowboy_store::prompt_origin::DeliveryOrigin` into `WireDeliveryOrigin` at admission (`DeliveryOrigin::Owner => WireDeliveryOrigin::Owner`; all other variants => `WireDeliveryOrigin::NonOwner`).
- Missing `origin: Option<WireDeliveryOrigin>` on any legacy wire frame fails closed as `WireDeliveryOrigin::NonOwner`.

### 2.2 Protocol Bump & Additive Wire Fields

- **Machine protocol:** bump `PROTOCOL_VERSION` to **52** (`ORIGIN_STEERING_PROTOCOL_VERSION = 52`), negotiating transport feature `control_overtake: bool` (true when both endpoints speak Machine >= 52 and `features.paced`).
- **Runtime protocol (worker↔broker and broker↔Controller):** bump `PROTOCOL_VERSION` to **8** (`ORIGIN_STEERING_RUNTIME_PROTOCOL_VERSION = 8`).
- **Additive optional fields (backward-compatible serde defaults):**
  - `WorkerSnapshot`:
    - `origin_steering: bool` (`#[serde(default)]`)
    - `worker_exit_confirmed: bool` (`#[serde(default)]`) — true **only** when OS/systemd/tree-exit verification confirms the worker process tree has exited (never set on socket disconnect or initial placeholders; see §7.3).
    - `completed_turn_id: Option<String>` (`#[serde(default)]`) — exact last turn completed on this worker epoch.
  - `Frame::CommandAck`:
    - `refusal: Option<CommandRefusal>` (`#[serde(default)]`) — typed pre-handoff refusal bound to `command_id`.
  - `Frame::ReplayComplete` (runtime protocol 8): `{ session_id, worker_epoch, through_runtime_seq }` emitted by broker after all buffered events for `(session_id, worker_epoch)` have been sent following reconnect.
  - `CoreCommand::Prompt` & `WorkerCommand::Prompt`:
    - `prompt_seq: Option<u64>` (`#[serde(default)]`) — per-session monotonic prompt sequence minted by Controller (§6.2).
    - `compacting: bool` (`#[serde(default)]`) — turn-scoped compaction bit computed at Controller admission (§4.2).
  - `CoreCommand::Cancel` & `CoreCommand::CancelTurn` (and `WorkerCommand::Cancel` / `CancelTurn`):
    - `after_prompt_seq: Option<u64>` (`#[serde(default)]`) — predecessor prompt fence (§6.2).
  - `CoreCommand::Steer` & `WorkerCommand::Steer`:
    - `origin: Option<WireDeliveryOrigin>` (`#[serde(default)]`)
    - `ttl_ms: Option<u64>` (`#[serde(default)]`) — remaining monotonic budget in milliseconds (§5).
    - `context_generation: Option<i64>` (`#[serde(default)]`) — session context generation captured at admission (§7.5).

---

## 3. Provider-Identity & Mixed-Version Capability Gate

### 3.1 Single Metadata Location & Dual Vendor-Flag Stripping

- **`initialize` result metadata location:** `src/acp.rs` reads top-level `init.meta` (`result._meta` of the ACP `initialize` response, matching `src/acp.rs:3837`), **not** `agentCapabilities._meta`.
- **P2 proxy (`proxy.mjs`) on `initialize` response:**
  1. Always deletes both upstream `result._meta.steering` AND upstream `result._meta["cowboy.dev/originSteering"]` (so an upstream `agy` upgrade can never spoof either native steering or origin steering).
  2. When `!this.managed`, stamps top-level:
     ```json
     "result": {
       "_meta": {
         "cowboy.dev/originSteering": {
           "v": 1,
           "accepts": ["owner"],
           "adapter": "cowboy-antigravity-acp"
         }
       }
     }
     ```
- **Worker-side binding (`src/acp.rs`, `src/worker.rs`):**
  - For `provider_id == "antigravity"`:
    1. **Hard-force** `supports_current_turn = false` and `supports_next_checkpoint = false`, regardless of upstream `_meta.steering`.
    2. Set `origin_steering = true` **if and only if ALL FIVE hold**:
       - `provider_id == "antigravity"`
       - `!state.sink.session_is_managed_read_only(&session_id)`
       - The worker's **signed admitted plugin release** declares R1 origin-steering support (`origin_steering_v1: true` in signed plugin metadata / admitted `antigravity >= 0.2.35`, verified from the signed plugin manifest loaded by the worker — never trusted from untrusted proxy JSON alone)
       - Top-level `init.meta["cowboy.dev/originSteering"]` has `v == 1`, `accepts` containing `"owner"`, and `adapter == "cowboy-antigravity-acp"`
       - Negotiated worker↔broker runtime protocol `>= 8`.
  - For all other providers (`codex`, `claude`, etc.): `origin_steering = false`; existing `supports_current_turn` behavior is unchanged.
  - Worker publishes `origin_steering` in **both** `WorkerSnapshot.origin_steering` AND `CURRENT_TURN_CAPABILITIES_UPDATE` (`"originSteering": bool`, `"currentTurn": false`, `"nextCheckpoint": false`).

### 3.2 Dual-Surface Sanitization at Broker & Controller (Fail Closed)

Both `Frame::Snapshot` AND `Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)` are sanitized identically at every trusted hop:

- **Broker (`components/machine/src/machine_broker.rs`):**
  - Resolves authoritative `provider` from `self.sessions` / `snapshot.launch.provider`.
  - If `provider` cannot be established (missing `launch` or unknown session): **fail closed** — clamp `origin_steering = false` and do not advertise `supports_current_turn = true`.
  - When `provider == "antigravity"`: always clamp `supports_current_turn = false`, `supports_next_checkpoint = false`; clamp `origin_steering = false` unless **both** worker↔broker runtime protocol `>= 8` AND broker↔Controller runtime protocol `>= 8` AND Machine protocol `>= 52`.
- **Controller (`src/remote_runtime.rs`, `src/core/`):**
  - For `session.meta.provider == "antigravity"`: always clamp `supports_current_turn = false`, `supports_next_checkpoint = false`; permit `origin_steering = true` only when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and the serialized live config gate `providers.antigravity.origin_steering == true`.

### 3.3 Mixed-Version Rollout Matrix

| Controller | Machine | Worker | Signed Antigravity Plugin / Proxy | `supports_current_turn` | `origin_steering` |
|---|---|---|---|---|---|
| Pre-R1 | Any | Any | P2 (strips `_meta.steering`, sets `cowboy.dev/originSteering`) | `false` | N/A (`false`) |
| R1 (52/8) | Pre-52 | Any | P2 | `false` | `false` (Machine < 52) |
| R1 (52/8) | 52/8 | Pre-8 (retained) | P2 | `false` | `false` (worker↔broker < 8) |
| R1 (52/8) | 52/8 | 8 | Pre-P2 (even if upstream `agy` spoofs flags) | `false` | `false` (signed manifest lacks `origin_steering_v1`) |
| R1 (52/8) | 52/8 | 8 | P2 (`antigravity >= 0.2.35`), kill switch OFF | `false` | `false` (live gate off) |
| R1 (52/8) | 52/8 | 8 | P2 (`antigravity >= 0.2.35`), kill switch ON | `false` | `true` (Owner only) |

### 3.4 Caller Authority Policy at Controller Admission

For an Antigravity session with `origin_steering == true`:
- `RequestCaller::Person` (`DeliveryOrigin::Owner`):
  - `delivery = "current_turn"` (and `session_default` when policy resolves to steer): admitted into the R1 origin-steering path.
  - `delivery = "next_checkpoint"`: refused `not_delivered` (`"next_checkpoint is not supported for Antigravity"`).
- Non-Owner (`RequestCaller::Agent`, `Automation`, `Cowboy`):
  - Explicit `delivery = "current_turn"` or `"next_checkpoint"`: refused synchronously `not_delivered` (`reason = "origin_not_accepted"`).
  - `delivery = "session_default"`: deterministically falls back to `next_turn` queue (never enters origin steering).
- Kill switch `providers.antigravity.origin_steering = false` (default `true` once shipped; serialized under an `RwLock` admission gate): refuses new steers synchronously `not_delivered` (`reason = "disabled_by_config"`) while keeping all outcome ingestion, replay settlement, and manual resend/dismiss endpoints active.

---

## 4. Attempt Identity, Compaction Fence, Two-Phase Trusted Ledger & Checked Write

### 4.1 Exact Native Attempt Identity & Continuation Grant

A native `sessionId` spans multiple turns. Every Antigravity turn and continuation is bound to a worker-minted 128-bit random hex `attemptNonce`:

1. **Controller admission:** requires an exact captured `DeliveryTarget { worker_epoch, turn_id }` matching `supervisor.delivery_target(session)` (otherwise refuses `not_delivered`).
2. **Worker prompt start (`src/acp.rs`):** when dispatching `AgentCommand::Prompt` on Antigravity, worker generates `attempt_nonce = random_128bit_hex()`, stores it on `ActivePrompt { turn_id, attempt_nonce, cancel_generation, compacting }` and in `Ledger::begin_turn_attempt(turn_id, attempt_nonce)`, and passes it on the ACP `session/prompt` request in:
   ```json
   "_meta": {
     "cowboy.dev/attemptNonce": "<32-hex>",
     "cowboy.dev/compacting": false
   }
   ```
3. **Worker steer admission (`src/acp.rs`):**
   - Checks `origin == Some(WireDeliveryOrigin::Owner)`, `provider_id == "antigravity"`, `origin_steering == true`, `!managed_read_only`, exact `target == (worker_epoch, active_prompt.turn_id)`, `!active_prompt.compacting`, `cancel_generation` unchanged, and no existing `ContinuationGrant` for this `attempt_nonce` (max 1 continuation per turn in R1).
   - Also verifies `ttl_ms` remaining after local wait is `> 0` and computes `trusted_deadline = Instant::now() + Duration::from_millis(worker_ttl_ms)` (§5).
   - If any pre-handoff check fails: worker does **not** touch `agy-proxy` and returns `CommandAck { accepted: false, refusal: Some(RefusedBeforeHandoff { code }) }` (only when `command_id` is first-seen; dedup runs first — §7.2).
   - Otherwise, worker registers an authoritative single-use `ContinuationGrant` in `Arc<ContinuationController>` (shared between `src/acp.rs` and `src/worker_execution.rs`):
     ```rust
     enum GrantState { Pending, Fenced, Committed, EmittedSent, Denied }
     struct ContinuationGrant {
         attempt_nonce: String,
         turn_id: String,
         cmid: String,
         cancel_generation: u64,
         blocks: Vec<ContentBlock>, // authoritative copy for synthetic Owner echo
         trusted_deadline: Instant,
         initial_uncertain_seq: u64,
         state: GrantState,
     }
     ```
   - Worker sends `_session/steering` (`SteeringRequest`) to `agy-proxy` with:
     ```json
     {
       "sessionId": "<acp_id>",
       "prompt": [ /* ContentBlock[] */ ],
       "_meta": {
         "cowboy.dev/steering": {
           "v": 1,
           "cmid": "<cmid>",
           "attemptNonce": "<32-hex>",
           "origin": "owner",
           "ttlMs": 588000
         }
       }
     }
     ```
   - Proxy validates `attemptNonce === turn.attemptNonce`, `turn.state === "ACTIVE_ATTEMPT_1"`, `!turn.compacting`, `!turn.cancelled`, `!turn.pendingSteer`, `!this.killed`, and `ttlMs > 0`; stores `turn.pendingSteer` with local deadline `performance.now() + ttlMs`, arms a local expiry timer, and replies `{ outcome: "accepted_pending" }` (never `"injected"`). If proxy refuses or times out before returning `accepted_pending`, worker retires the grant to `Denied` and emits `STEERING_RESULT_UPDATE { cmid, worker_epoch, turn_id, attempt_nonce, outcome: "not_confirmed", reason }` (never `not_delivered` once bytes were handed to the untrusted proxy).

### 4.2 Shared Slash / Compaction Predicate & Turn-Scoped Latch

- **Block normalization:** apply `build_prompt_blocks(text, content)` first — if `content` is empty and `text` is non-empty, normalize to `[{ "type": "text", "text": text }]`.
- **Shared whitespace class (identical in Rust and JS):** `is_ws(ch) = ch.is_whitespace() || ch == '\u{FEFF}'` (matches ECMAScript `\s` / `.trim()` identically in both runtimes).
- **`is_compaction_prompt(blocks)`:** returns `true` iff `blocks` is non-empty, **every** block has `type == "text"` with string `text`, and `trim_ws(blocks.map(b => b.text).join(""))` starts with ASCII case-insensitive `"/compact"` followed immediately by end-of-string or `is_ws(next_char)`.
- **`is_slash_command_steer(blocks)`:** extract all contiguous leading `type == "text"` blocks from index 0 (stopping at the first non-text block), join their `.text` with `""`, strip leading `is_ws`, and return `true` iff the remainder starts with `'/'`.
- Both predicates are cross-tested by one shared JSON test vector file run in both Cargo tests and Node tests (covering `/compact`, `/COMPACT`, leading spaces/BOM, split blocks `["/com", "pact"]`, `["   ", "/compact"]`, mixed text+image blocks, and empty `content` with `text` fallback).
- **Turn-scoped compaction latch across all 3 stages:**
  1. **Controller prompt dispatch:** computes `compacting = is_compaction_prompt(&blocks)` before handing off `CoreCommand::Prompt`, sets `session.active_turn_compacting = true` synchronously under `sessions.lock()` (cleared on turn end or Clear), and forwards `compacting: true` on `CoreCommand::Prompt` / `WorkerCommand::Prompt`. Controller immediately refuses any steer with `not_delivered` (`reason = "compacting"`) while `hub.is_compacting(session_id) || hub.active_turn_is_compacting(session_id)` — even while `/compact` is still sitting in a relay or proxy preparation.
  2. **Worker (`src/acp.rs`):** sets `active_prompt.compacting = cmd.compacting || is_compaction_prompt(&blocks)` synchronously before forwarding `session/prompt`, refusing any steer with `RefusedBeforeHandoff(Compacting)`.
  3. **Proxy (`proxy.mjs`):** computes `compacting = Boolean(params?._meta?.["cowboy.dev/compacting"]) || isCompactionPrompt(params?.prompt)` **synchronously inside `fromClient` when reserving the turn slot (before `this.queue.then(...)` and before any `await` such as `evidence.mark` or `recall`)**, latching `turn.compacting = true` and `this.compacting.add(sessionId)`. Checked at `_session/steering`, continuation eligibility, and final write fence.
- **Steer-payload slash fence:** Controller refuses any Antigravity steer matching `is_slash_command_steer(&blocks)` synchronously `not_delivered` (`reason = "slash_command_not_steerable"`). Worker and Proxy enforce the exact same check as defense in depth.

### 4.3 Two-Phase Trusted Execution-Ledger Fence & Cleanup Gate

`Ledger` (`src/worker_execution/ledger.rs`) is extended with an explicit per-attempt phase machine on the active turn:

```rust
enum AttemptPhase {
    Open { attempt_nonce: String, attempt_index: u8 },
    FencedForContinuation { attempt_nonce: String },
    TurnFenced,
}
```

And `RelayQuiet` (`src/account_sandbox.rs`, wired to `Relays` in `src/worker_execution/relay.rs`) holds a `RelayGuard` (`client.relays.in_flight()`) for every execution request **until its reply frame is either written to the unit socket or recorded undelivered in `Ledger`**. In production Antigravity (`account_sandbox_v1`), the continuation RPCs travel over the worker's authenticated execution endpoint alongside `cowboy/drained` and `cowboy/uncertainEffect`; in standalone test fixtures without a separate execution socket, the worker's ACP client handler routes them to the same `Ledger` + `ContinuationController` instance (failing closed if neither is bound).

When attempt 1 finishes at `proxy.mjs` (state `ACTIVE_ATTEMPT_1 -> SETTLING_ATTEMPT_1`):

1. **Synchronous dual-fencing (no postponed child shutdown):**
   - Proxy starts **both** `const fenced = runtime.facade.endTurn(record.sessionId);` AND `const childrenFinished = this.subagents.finish(record.sessionId);` **synchronously before awaiting either**, then awaits `const [subagentResult, facadeResult] = await Promise.all([childrenFinished, fenced])`.
   - `Facade.endTurn` returns `{ clean: boolean, unconfirmedCommands: number }`.
   - `Subagents.finish` (`subagents.mjs:915-949`) returns `{ clean: boolean, timedOut: boolean, failedChildren: number, terminatedAgent: boolean, uncertain: boolean, violation: boolean }`; `clean` is `true` **only** when every child of that turn finished or confirmed stop cleanly without timeout, agent termination, child uncertainty, or child policy violation.
2. **Aggregate backend evidence:**
   - Proxy awaits `settled = await runtime.evidence.settle(...)` and computes `const aggregate = withChildren(turn, attempts(turn, settled))` **before** checking continuation eligibility.
   - Local eligibility requires ALL of:
     - `turn.pendingSteer != null` and local TTL remaining `> 0`
     - `message.result?.stopReason === "end_turn"`
     - `facadeResult.clean === true && facadeResult.unconfirmedCommands === 0`
     - `subagentResult.clean === true`
     - `aggregate.class === "ok"` (parent + all children)
     - `!turn.cancelled && turn.violation === undefined && turn.uncertain === undefined && !turn.compacting && !this.killed && !record.finalized`.
   - If any fails: abort continuation and finalize attempt 1 normally (calling `settleMemory(turn)` on finalization).
3. **Phase 1 — Trusted Attempt-1 Fence (`cowboy/fenceAttempt { attemptNonce, cmid }` on execution endpoint):**
   - Runs inside `cowboy-acp-worker`'s trusted `Client::relay` (`src/worker_execution/ledger.rs`), holding 1 `RelayGuard` itself.
   - Atomically transitions `Ledger` from `Open { attempt_nonce, attempt_index: 1 }` to **`FencedForContinuation { attempt_nonce }`** — immediately blocking ALL new effectful execution requests (`admit`, `admit_call`, `fs/*`, `process/start`, `tools/call`, and `/v1/observe` memory writes). Only read-only memory recall (`/v1/recall`) and continuation control RPCs (`cowboy/commitContinuation`, `cowboy/abortContinuation`) are allowed while in `FencedForContinuation`.
   - Waits boundedly (up to `min(250ms, remaining_ttl)`, aborting immediately on `cancel_generation` change) for `client.relays.in_flight() == 1` (proving **zero other relays remain in flight or owe an undelivered reply**).
   - Under the combined `Ledger` + `ContinuationController` lock, verifies ALL invariants:
     - `Ledger.attempt == FencedForContinuation { attempt_nonce }` and `ledger.current_turn() == Some(grant.turn_id)` and `!ledger.closed` and `!ledger.is_fenced(&grant.turn_id)`
     - `!ledger.unwritable`
     - `ledger.uncertain_effect().is_none()` AND `ledger.uncertain_sequence() == grant.initial_uncertain_seq`
     - `ledger.processes.is_empty() && ledger.starting.is_empty() && ledger.admitted.is_empty() && ledger.calls == 0`
     - `client.relays.in_flight() == 1`
     - `grant.state == Pending`, `cancel_generation == grant.cancel_generation`, `!active_prompt.compacting`, `Instant::now() < grant.trusted_deadline`.
   - If ANY check fails: transitions `grant.state = Denied`, marks `Ledger.attempt = TurnFenced` (setting `turn_ended` so `OBSERVATION_GRACE` permits final `settleMemory` while keeping all effects fenced), returns `{ allow: false, reason }`.
   - If ALL pass: transitions `grant.state = Fenced`, keeps `Ledger` in **`FencedForContinuation`** (so attempt 1 remains strictly closed to new effects throughout attempt-2 preparation), returns `{ allow: true, remainingMs }`.
4. **Attempt-2 Preparation (Ledger remains `FencedForContinuation`):**
   - Proxy marks fresh `cursor2 = await runtime.evidence.mark()`, runs optional read-only memory recall bounded by `min(RECALL_BOUND_MS, remaining_local_ttl)`, and rechecks synchronously: `!turn.cancelled`, `turn.violation === undefined`, `turn.uncertain === undefined`, `!turn.compacting`, `!this.killed`, `!record.finalized`, `cursor2 !== undefined`, `remaining_local_ttl > 0`, and `agy` alive + `agy.stdin` writable.
   - If any check fails: calls `cowboy/abortContinuation { attemptNonce, cmid, reason }`, drops continuation, finalizes attempt 1.
5. **Phase 2 — Trusted Commit-Point Grant (`cowboy/commitContinuation { attemptNonce, cmid }`):**
   - Called immediately after preparation finishes, right before writing attempt 2 to `agy.stdin`.
   - Atomically under `Ledger` + `ContinuationController` lock re-verifies EVERY Phase 1 invariant **including** `Ledger.attempt == FencedForContinuation { attempt_nonce }`, `client.relays.in_flight() == 1`, `uncertain_effect == None`, `processes/starting/admitted/calls == 0`, `grant.state == Fenced`, `cancel_generation == grant.cancel_generation`, `!active_prompt.compacting`, AND **`Instant::now() < grant.trusted_deadline`** (enforcing the trusted monotonic TTL at final dispatch).
   - If ALL pass: atomically transitions `grant.state = Committed` (single-use consumed) and opens `Ledger.attempt = Open { attempt_nonce, attempt_index: 2 }`, returning `{ allow: true, remainingMs }`.
   - If ANY check fails (e.g. Cancel arrived or TTL expired during preparation): sets `grant.state = Denied`, marks `Ledger.attempt = TurnFenced`, returns `{ allow: false, reason }`.
   - *(Untrusted proxy enforcement: if a compromised proxy skips `cowboy/commitContinuation` and writes attempt 2 anyway, `Ledger` remains `FencedForContinuation` so attempt 2 receives zero tool/fs/process execution, and the worker rejects any `cowboy/steeringOutcome(sent)` because `grant.state != Committed`.)*

### 4.4 Checked `agy.stdin` Write, Exact `"sent"` Proof & Single-Use Echo

- **Checked write (`launch.mjs:writeAgentChecked(message, timeoutMs)`):**
  - Replaces silent no-op `write(agy.stdin)` (`launch.mjs:472-479,501`) for continuation `session/prompt` dispatch.
  - Synchronously rejects if `agy.killed`, `agy.exitCode !== null`, `agy.signalCode !== null`, `!agy.stdin`, `agy.stdin.destroyed`, `agy.stdin.writableEnded`, or `!agy.stdin.writable`.
  - Registers one-shot listeners for `agy.stdin` `error` & `close`, `agy` `exit`, and a timeout bounded by remaining TTL, then invokes `const flushed = agy.stdin.write(line, "utf8", onCallback)`.
  - Resolves `{ ok: true }` **only** if `onCallback` fires with `err == null` (and if `flushed === false`, `drain` completes within the remaining TTL without error/close/exit) AND `agy.exitCode === null && agy.signalCode === null && !agy.stdin.destroyed`. Cleans up all listeners on settle.
- **Proxy state transition on attempt 2:**
  - Before calling `writeAgentChecked`: records continuation prompt blocks in `turn.suppressedContinuationEchoes` (so `agy`'s native `user_message_chunk` echo of attempt 2 is suppressed alongside `injectedEcho(update)`), registers `this.pending.set(steerReqId, { ...record, cursor: cursor2, replyId, attempt: 2, attemptNonce, steerCmid })`, reopens `runtime.facade.beginTurn(record.sessionId)`, and records attempt 1's backend evidence and usage for aggregation (same pattern as `this.correct`).
  - **If `writeAgentChecked` resolves `{ ok: true }`:** transitions turn state to `ACTIVE_ATTEMPT_2`, clears `this.settling.delete(replyId)` and `turn.pendingSteer = null`, and sends ACP notification `cowboy/steeringOutcome { attemptNonce, cmid, outcome: "sent" }` to the worker.
  - **If `writeAgentChecked` resolves `{ ok: false, reason }` (or throws / times out):** removes `this.pending.delete(steerReqId)`, calls `cowboy/abortContinuation { attemptNonce, cmid, reason: "stdin_write_failed" }` (which immediately re-fences `Ledger` to `TurnFenced`, transitions `grant.state = Denied`, and prevents any later `sent`), does **NOT** emit `"sent"`, emits `cowboy/steeringOutcome { attemptNonce, cmid, outcome: "dropped", reason: "stdin_write_failed" }`, and finalizes the original `replyId` via `this.endPrompt`.
- **What `"sent"` proves:** A `"sent"` receipt proves that (1) the trusted worker verified clean execution state, zero in-flight relays, active uncancelled turn, and unexpired monotonic TTL at commit point (`cowboy/commitContinuation`), consuming the single-use `ContinuationGrant`, and (2) the proxy's checked write of the continuation `session/prompt` JSON-RPC line to live `agy`'s stdin completed without stream error, close, or process exit. As with ordinary ACP `session/prompt` dispatch, a subsequent crash of `agy` after kernel-pipe handoff is an ordinary turn failure, not a pre-handoff refusal.
- **Worker handling of `cowboy/steeringOutcome` (`src/acp.rs`):**
  - Accepts `outcome == "sent"` **at most once** and **only** when `(attemptNonce, cmid)` matches a `ContinuationGrant` currently in `GrantState::Committed` -> atomically transitions `Committed -> EmittedSent`.
  - Any duplicate, ungranted, pre-commit, wrong-nonce, or post-abort `"sent"` is ignored.
  - On valid `Committed -> EmittedSent`:
    1. Records `grant.blocks` in `state.last_echoed_user_contents` (second defense against duplicate native echo).
    2. Emits one synthetic Owner `user_message_chunk` per block in `grant.blocks` (`index == 0` tagged with `cmid`; all blocks stamped Owner via `steering_echo`).
    3. Emits `STEERING_RESULT_UPDATE { cmid, worker_epoch, turn_id, attempt_nonce, outcome: "sent" }`.
  - On `outcome == "dropped"` (or if attempt 1 ends / cancels while `grant.state` is `Pending`, `Fenced`, or `Committed` without `EmittedSent`):
    - Atomically transitions grant to `Denied` (retiring `attemptNonce` once) and emits `STEERING_RESULT_UPDATE { cmid, worker_epoch, turn_id, attempt_nonce, outcome: "not_confirmed", reason }` (never `"not_delivered"` and never emits Owner echo).
- **Tool-boundary note (no owner text):** While `turn.pendingSteer` is active during `ACTIVE_ATTEMPT_1`, the proxy appends at most one fixed Cowboy-authored data note to the next facade tool result: `"\n[Cowboy note: a follow-up message from the owner is waiting; finish the current step briefly so it can be delivered.]"` — containing zero owner text and carrying no tool/owner authority.

### 4.5 Proxy Per-Turn State Machine Summary

```text
RESERVED (synchronous in fromClient, before this.queue.then)
  |-- cancel arrives while RESERVED --> FINALIZING (retire attemptNonce, reply cancelled)
  v
ACTIVE_ATTEMPT_1 (agy received attempt 1; accepts at most 1 _session/steering)
  |-- attempt 1 replies --> SETTLING_ATTEMPT_1 (dual-fence facade + subagents)
        |-- ineligible / no pendingSteer --> FINALIZING (retire attemptNonce, endPrompt(replyId))
        v
      FENCING_ATTEMPT_1 (cowboy/fenceAttempt)
        |-- allow: false --> FINALIZING (emit dropped, endPrompt(replyId))
        v
      PREPARING_ATTEMPT_2 (evidence.mark + bounded recall; Ledger stays FencedForContinuation)
        |-- cancelled/expired/error --> abortContinuation -> FINALIZING (emit dropped, endPrompt(replyId))
        v
      COMMITTING_ATTEMPT_2 (cowboy/commitContinuation; checks trusted_deadline)
        |-- allow: false --> FINALIZING (emit dropped, endPrompt(replyId))
        v
      WRITING_ATTEMPT_2 (writeAgentChecked -> agy.stdin)
        |-- ok: false --> abortContinuation -> FINALIZING (emit dropped/stdin_write_failed, endPrompt(replyId))
        |-- ok: true  --> emit cowboy/steeringOutcome(sent)
        v
      ACTIVE_ATTEMPT_2 (same replyId, no second steering accepted; attempt 2 reply -> endPrompt(replyId))
        v
      FINALIZING (retire attemptNonce exactly once; endPrompt(replyId))
```

---

## 5. End-to-End Monotonic TTL Bound (Zero Cross-Host Wall Clock)

Default TTL: **600,000 ms (10 minutes)** (configurable per-request within `[30_000, 600_000]`). Never compares cross-host wall clocks.

1. **Controller pre-wire readiness & reservation:** Controller captures `admitted_at = Instant::now()`. `RemoteRuntime` awaits WS writer readiness (no prior bulk frame in progress and `CREDIT_WINDOW` room available) **before** executing the durable `admitted -> sent_to_machine` reservation (§7.2). Right before reservation, it computes `pre_wire_wait_ms = now - admitted_at` and `transit_cap_ms = min(STEER_TRANSIT_CAP_MS, ttl_ms / 3)` where `STEER_TRANSIT_CAP_MS = 10_000`. If `pre_wire_wait_ms + transit_cap_ms >= ttl_ms`, it does **not** reserve or touch the wire and settles synchronously **`not_delivered`** (`ExpiredBeforeHandoff`).
2. **Bounded WS wire transit (`src/machine_transport.rs`):**
   - Immediately after `admitted -> sent_to_machine` commits, `RemoteRuntime` stamps `ttl_ms = original_ttl_ms - pre_wire_wait_ms - transit_cap_ms` and hands the Steer frame to the ready WS writer.
   - While transmitting that Steer frame's chunks, `machine_transport::write` enforces a hard completion/credit deadline of **`transit_cap_ms` (<= 10s, NOT the 60s bulk `CREDIT_STALL`)**. If the WS tunnel cannot finish sending the Steer frame within `transit_cap_ms`, the transport fails the connection (settling the reserved in-flight steer `not_confirmed` — never allowing a silent 60-second credit stall to inflate the deadline).
3. **Broker queue accounting (`machine_broker.rs`):** records `broker_received_at = Instant::now()` on receipt and deducts `broker_wait_ms` right before writing `WorkerCommand::Steer` to the worker Unix socket. If `broker_wait_ms >= ttl_ms`, broker refuses `RefusedBeforeHandoff(ExpiredBeforeHandoff)` without writing to the worker.
4. **Worker trusted deadline (`src/acp.rs` & `src/worker_execution/ledger.rs`):** records `worker_received_at = Instant::now()`, sets `grant.trusted_deadline = worker_received_at + Duration::from_millis(worker_ttl_ms)`. Because actual WS wire flight is hard-capped to `<= transit_cap_ms` (already deducted before chunk 0) and every relay queue wait was deducted, `grant.trusted_deadline <= admitted_at + original_ttl_ms` holds **mathematically without relying on wall-clock sync**.
5. **Commit-point enforcement:** worker enforces `Instant::now() < grant.trusted_deadline` inside **both** `cowboy/fenceAttempt` **and** `cowboy/commitContinuation` (after all attempt-2 preparation, immediately before stdin write). Proxy independently runs a local monotonic timer (`performance.now() + ttlMs`) to drop expired continuations promptly while attempt 1 is still running and rechecks synchronously before `writeAgentChecked`.

---

## 6. Stop Priority Across Every Writer & Per-Session Predecessor Prompt Fence

### 6.1 Per-Writer Stop Priority (No Batch or Bulk-Credit Head-of-Line Blocking)

1. **Controller HTTP/WS admission (`src/core/`, `components/server-runtime/`):** Cancel/Stop uses a dedicated semaphore (`CANCEL_IN_FLIGHT`, 16 permits) independent of ordinary send `IN_FLIGHT`, so congested prompt/steer requests cannot block Stop admission.
2. **Controller `RemoteRuntime::send_pending` (`src/remote_runtime.rs:1811-1888`):**
   - Replace the snapshot-batch drain with a **single-command priority picker loop**: each iteration locks `shared.pending`, picks the **single highest-priority ready command** (priority order: `Cancel` / `CancelTurn` / `StopSession(reset-*)` > `RecoverExecutionSession` / `EnsureSession` > `SetConfigOption` > `Steer` > `Prompt`), unlocks, performs admission + wire write for that ONE command, and loops back to re-inspect `shared.pending` so a newly arriving Stop immediately preempts all remaining backlog.
3. **Controller↔Machine WS writer (`src/machine_transport.rs:198-205,371-407`):**
   - When both endpoints negotiate `features.control_overtake` (Machine >= 52 AND `features.paced`), extend `overtakes(text)` and `Decoder::text` to allow bounded (`<= CHUNK_BYTES`) `CoreCommand::Cancel` and `CoreCommand::CancelTurn` frames (matched by exact serialized JSON prefix) to **interleave between 16 KiB bulk chunks and bypass `CREDIT_WINDOW` stalls**.
   - Pre-52 peers never receive interleaved Cancel frames (preserving backward wire compatibility).
4. **Machine Broker (`components/machine/src/machine_broker.rs`):**
   - Route incoming `CoreCommand::Cancel` / `CancelTurn` on a dedicated bounded `control_tx`/`control_rx` channel polled with `tokio::select! { biased; }` ahead of slow lifecycle commands (`EnsureSession`, worker spawns, generation rollouts).
   - Give each worker Unix socket writer a two-lane queue (`control_queue` for `Cancel` / `CancelTurn` / `ExecutionReply` vs `bulk_queue` for `Prompt` / `Steer` / `Replay`), always draining `control_queue` first.
5. **Runtime Worker (`src/worker.rs:755-763` & `src/acp.rs`):**
   - In `worker.rs:connected`, use `tokio::select! { biased; incoming = reader.next() => ..., ... }` and decouple incoming command reads from outbox flushing:
     - `WorkerCommand::Cancel` and `WorkerCommand::CancelTurn` dispatch immediately onto a dedicated priority `cancel_tx` channel in `src/acp.rs` and send their small `CommandAck` **without** synchronously flushing the full event outbox first.
     - Ordinary outbox flushes drain in bounded batches (max 16 events per loop iteration) before yielding back to the `biased` `reader.next()` arm, so heavy stdout/event bursts never starve Stop reads.
   - In `src/acp.rs`, the command loop polls `cancel_rx` in a `biased` select arm ahead of `cmd_rx`; `Cancel` / `CancelTurn` **never await `steering_gate` or `prompt_lock`**, synchronously increment `cancel_generation`, synchronously fence `Ledger` (`AttemptPhase::TurnFenced`) and deny any `Pending`/`Fenced` `ContinuationGrant`, and immediately emit `session/cancel` to `agy-proxy`.
6. **Proxy (`launch.mjs:359-376` & `proxy.mjs:559-564`):**
   - Both `session/cancel` and `_session/steering` **bypass `this.queue`** and execute synchronously in `fromClient`.

### 6.2 Per-Session Predecessor Prompt Fence (Stops Never Overtake Held or In-Flight Prompts)

To guarantee an overtaking Stop can never hit an idle worker while an earlier Prompt for the same session starts afterward:

1. **Controller (`prompt_seq` scoped to connection epoch):** maintains a strictly monotonic per-session `prompt_seq: u64` stamped onto every `CoreCommand::Prompt` (reset to `0` on each new Machine WS `connection_id`, with broker keying its fence state by the active `connection_id` so Controller restarts/reconnects never alias stale sequence numbers). When `Cancel` / `CancelTurn` is admitted for `session_id`:
   - Captures `after_prompt_seq = session.last_admitted_prompt_seq` for the current `connection_id`.
   - Under `shared.pending.lock()`, **atomically withdraws** any still-unsent `CoreCommand::Prompt` for `session_id` with `prompt_seq <= after_prompt_seq` (emitting clean cancelled/undelivered handling without writing it to the wire).
   - Stamps `after_prompt_seq` onto `CoreCommand::Cancel` / `CancelTurn`. On reconnect (new `connection_id`), any queued pre-reconnect `Prompt` older than a pending `Cancel` is withdrawn before transmission, and `Cancel` carries `after_prompt_seq = 0` on the new `connection_id` while broker treats any `Cancel` from a newer `connection_id` as also withdrawing held `pending_commands` prompts from any earlier `connection_id`.
2. **Broker (`pending_commands` & overtaken WS chunks, `machine_broker.rs:913-948,1315-1339`):**
   - Tracks per-session `(active_connection_id, received_prompt_seq, dispatched_prompt_seq, cancelled_prompt_seq_up_to)` and tags each held `WorkerCommand::Prompt` in `pending_commands` with its `(connection_id, prompt_seq)`.
   - When `Cancel(after_prompt_seq)` arrives on broker's control lane for `conn_id`:
     - **Held prompts in `broker.pending_commands[session_id]`** (during startup or generation drain): atomically withdraws every held `WorkerCommand::Prompt` from an older `connection_id` OR from `conn_id` with `prompt_seq <= after_prompt_seq`, reporting authoritative cancelled/undelivered outcomes for them.
     - **In-flight chunked `Prompt` overtaken on WS (`received_prompt_seq < after_prompt_seq` on `conn_id`):** records `cancelled_prompt_seq_up_to[session_id] = max(..., after_prompt_seq)`. When the delayed `Prompt` finishes decoding with `prompt_seq <= cancelled_prompt_seq_up_to`, broker cancels it immediately instead of starting it. Any *subsequent* Prompt submitted after Stop has `prompt_seq > after_prompt_seq` and runs normally.
     - **Already-dispatched prompt (`dispatched_prompt_seq >= after_prompt_seq` or active turn on worker):** forwards `Cancel` immediately to the worker's `control_queue`.
3. **Proxy (`launch.mjs:359-376` same-stdin-chunk race):**
   - In `proxy.mjs:fromClient`, receiving `session/prompt` **synchronously** reserves a `turn` slot in `this.turns` (`state: "RESERVED"`, capturing `attemptNonce`, `replyId: id`, `cancelGeneration = this.cancelGen.get(sessionId) ?? 0`, `compacting`) **before** chaining onto `this.queue = this.queue.then(...)`.
   - If `session/cancel` arrives in the very same stdin chunk before the queued async prompt handler executes, `session/cancel` synchronously increments `this.cancelGen` and marks `turn.cancelled = true` on the `RESERVED` slot. When the queued handler runs, it checks `turn.cancelled || turn.cancelGeneration !== this.cancelGen.get(sessionId)` before writing to `agy` and immediately completes the prompt as `{ stopReason: "cancelled" }` without starting `agy`.

---

## 7. Durable Receipt Store, Monotonic Settlement Lattice & Positive Turn-End Proof

### 7.1 Durable Schema (`migrations/0090_origin_steer_receipts.sql` & `migrations/sqlite/0064_origin_steer_receipts.sql`)

```sql
-- Added to sessions (default 0):
ALTER TABLE sessions ADD COLUMN context_generation BIGINT NOT NULL DEFAULT 0;

-- Added to prompt_dispatches (nullable, logical anchor = COALESCE(logical_delivery_key, cmid)):
ALTER TABLE prompt_dispatches ADD COLUMN logical_delivery_key TEXT;

CREATE TABLE origin_steer_receipts (
    session_id TEXT NOT NULL,
    cmid TEXT NOT NULL,
    command_id TEXT NOT NULL,
    logical_delivery_key TEXT NOT NULL,
    context_generation BIGINT NOT NULL,
    worker_epoch TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    status TEXT NOT NULL,           -- 'pending' | 'delivered' | 'not_delivered' | 'not_confirmed'
    handoff_phase TEXT NOT NULL,    -- 'admitted' | 'sent_to_machine' | 'acked_by_worker' | 'settled'
    handoff_connection_id TEXT,
    reason TEXT,
    text TEXT NOT NULL,
    content_json TEXT NOT NULL,
    block_count INTEGER NOT NULL,
    resend_cmid TEXT,
    resend_state TEXT NOT NULL DEFAULT 'none', -- 'none' | 'enqueued'
    original_turn_ended_at_ms BIGINT,
    original_turn_ended_proof TEXT, -- 'turn_ended_event' | 'worker_exit_confirmed' | 'context_cleared'
    cleared_at_ms BIGINT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    dismissed_at_ms BIGINT,
    PRIMARY KEY (session_id, cmid)
);
CREATE UNIQUE INDEX idx_origin_steer_receipts_command_id ON origin_steer_receipts (command_id);
CREATE INDEX idx_origin_steer_receipts_session_active ON origin_steer_receipts (session_id, context_generation, dismissed_at_ms);
CREATE INDEX idx_origin_steer_receipts_epoch_turn ON origin_steer_receipts (session_id, worker_epoch, turn_id);
```

- Retention & cleanup: purged on session deletion (`delete_session`), scrubbed on Clear (§7.5), and dismissed rows older than 7 days pruned by existing housekeeping.

### 7.2 Handoff Phase Durability, No-Replay & Typed First-Handoff Refusal

1. **Admission:** after validating Owner caller, target turn, non-compacting, non-slash, `!clear_pending`, and live config gate:
   - Write `prompt_dispatches` (with `logical_delivery_key = cmid`) and arm exact `block_count` in Hub via `hub.record_prompt_dispatch(...)` + `hub.set_pending_echo_blocks(session_id, &cmid, blocks.len())`.
   - Atomically insert `origin_steer_receipts` (`status = 'pending'`, `handoff_phase = 'admitted'`, `context_generation`, `block_count`, `logical_delivery_key = cmid`), verifying in SQL that `sessions.context_generation == captured_generation`.
2. **Serialized first-write reservation (`RemoteRuntime`):**
   - After awaiting WS writer readiness and verifying remaining TTL `> transit_cap_ms` (§5), and immediately **before** writing `CoreCommand::Steer` chunk 0 onto Machine WS connection `conn_id`, execute durable conditional transition:
     `UPDATE origin_steer_receipts SET handoff_phase = 'sent_to_machine', handoff_connection_id = ?conn_id WHERE command_id = ?cmd AND handoff_phase = 'admitted' AND status = 'pending'`.
   - If `rows_affected == 0` or store write fails: **do NOT write to the wire** (enforcing strict at-most-once transmission across crashes and reconnects).
   - `RemoteRuntime` NEVER replays a `CoreCommand::Steer` after `handoff_phase` leaves `'admitted'`.
3. **Worker dedup-before-liveness (`src/worker.rs`):**
   - In `handle_command`, check `seen_commands.contains(&command_id)` **before** checking worker liveness or turn state, returning `CommandAck { accepted: true, refusal: None }` for duplicates so a replayed command can never produce a false negative ACK.
4. **CommandAck processing (`src/remote_runtime.rs`):**
   - On `CommandAck { command_id, accepted: true }`: durably transition `handoff_phase: 'sent_to_machine' -> 'acked_by_worker'` (status remains `'pending'`; durable row survives removal of `pending`/`attempted_sends`).
   - On `CommandAck { command_id, accepted: false, refusal: Some(RefusedBeforeHandoff { code }) }`: settle **`not_delivered`** (`handoff_phase = 'settled'`) **if and only if** `receipt.command_id == command_id && receipt.handoff_connection_id == Some(current_conn_id) && receipt.handoff_phase == 'sent_to_machine' && receipt.status == 'pending'`. **Commit this SQL settlement BEFORE retiring the ACK / removing pending state**, and disarm `pending_echoes` (`hub.disarm_pending_echo`).
   - On any other `accepted: false` ACK (untyped, wrong connection, already `acked_by_worker`, unknown error): durably settle `status = 'not_confirmed'`, `handoff_phase = 'settled'` before retiring ACK state.
5. **Disconnect & Controller restart recovery (`startup_restore.rs`):**
   - Machine WS connection `conn_id` drops:
     - Any receipt with `handoff_connection_id == Some(conn_id)` and `handoff_phase IN ('sent_to_machine', 'acked_by_worker')` and `status == 'pending'` -> durably settle `status = 'not_confirmed'`, `reason = 'machine_disconnected'` (keeps `original_turn_ended_at_ms = NULL` until proven ended).
   - Controller startup restore:
     - Any receipt with `handoff_phase == 'admitted'` and `status == 'pending'` (crashed before wire reservation) -> durably settle `status = 'not_delivered'`, `handoff_phase = 'settled'`, `reason = 'controller_restart_before_handoff'`, and disarm `pending_echoes`.
     - Any receipt with `handoff_phase IN ('sent_to_machine', 'acked_by_worker')` and `status IN ('pending', 'not_confirmed')` and `cleared_at_ms IS NULL`: restore its exact `block_count` in Hub `pending_echoes` (`hub.set_pending_echo_blocks(session_id, &cmid, receipt.block_count)`) so a multi-block late `"sent"` replay after restart still authorizes all `block_count` Owner blocks.

### 7.3 Monotonic Receipt State Lattice & Positive Turn-End Proof

Every transition is keyed by `(session_id, cmid, command_id, worker_epoch, turn_id)` and guarded by SQL `WHERE` clauses:

```text
pending  --(pre-wire crash / Controller refusal / typed first-handoff RefusedBeforeHandoff)-->  not_delivered  [ABSORBING]
pending  --(disconnect / untyped ACK / dropped / expired / turn_ended_without_outcome)------->  not_confirmed  [PROVISIONAL]
pending | not_confirmed  --(validated worker STEERING_RESULT_UPDATE outcome="sent")---------->  delivered      [ABSORBING]
```

- **Late replay upgrade:** If a disconnect settled a receipt `not_confirmed`, a valid replayed `STEERING_RESULT_UPDATE { outcome: "sent" }` for the exact `(session_id, worker_epoch, turn_id, cmid)` upgrades `not_confirmed -> delivered` (durable write committed **BEFORE** `Frame::Ack` advances `highwaters`), emits `OriginSteerChanged`, and ensures the Owner echo is present (idempotent on `cmid`). Because `send-next` is fenced until positive turn-end proof + replay completion, `resend_state` cannot become `'enqueued'` before a replayed `"sent"` arrives.
- **No downgrade:** `delivered` and `not_delivered` are absorbing. No disconnect, timeout, or ambiguous event can ever downgrade `delivered`.
- **Missing-outcome reconciliation & Positive Turn-End Proof (`original_turn_ended_at_ms`):**
  - A `not_confirmed` receipt stays **fenced** (`original_turn_live = true`, `send-next` returns `409 original_turn_not_proven_ended`) until Controller durably observes **positive proof** that `(worker_epoch, turn_id)` ended AND its event stream up to that end was consumed:
    1. **Exact turn terminal event:** `Frame::WorkerEvent` for exact `(session_id, worker_epoch)` carrying `TurnEnded` / `TurnFailed` / `TurnCancelled` for `turn_id` processed in-order through `handle_frame` (every lower `runtime_seq` event, including any `"sent"` outcome, was already processed). Inside the pre-ACK durable transaction: reconcile any still-`pending` receipt for `(worker_epoch, turn_id)` to `not_confirmed` (`reason = 'turn_ended_without_outcome'`) AND set `original_turn_ended_at_ms = now_ms, original_turn_ended_proof = 'turn_ended_event'` on all `not_confirmed` receipts for `(worker_epoch, turn_id)`.
    2. **Confirmed worker-tree termination + replay barrier:** broker sets `worker_exit_confirmed = true` **only** after OS/systemd/subtree exit proof (`confirm_account_unit_gone` / child `waitpid` / durable `account_worker_exit`, **never** on broker↔worker socket loss at `machine_broker.rs:3199` and **never** on initial placeholder snapshots) **AND** `Frame::ReplayComplete` for `(session_id, worker_epoch)` has been processed. Reconciles any remaining `pending` receipt for `worker_epoch` to `not_confirmed` (`worker_terminated_without_outcome`) and sets `original_turn_ended_at_ms = now_ms, original_turn_ended_proof = 'worker_exit_confirmed'`.
  - **Explicitly NOT proof:** disconnected Machine, unreconciled reconnect snapshot (before `ReplayComplete`), placeholder snapshot (`launch.is_none()`), absence from broker inventory, or socket-drop `WorkerState::Crashed` with `worker_exit_confirmed == false`.
  - Whenever `original_turn_ended_at_ms` transitions from `NULL` to non-`NULL`, Controller emits content-free `Outbound::OriginSteerChanged { session_id }` so open Web clients refresh `original_turn_live`.

### 7.4 Atomic `send-next` Transaction & Reserved Resend Namespace

- **Reserved internal namespace:** reserve `cowboy-origin-steer-resend:` in `crate::agent_model` and reject it at ALL client HTTP, WebSocket, and Agent API prompt/steer/operation ingress points (alongside `call-notice:`), preventing client-supplied `operation` IDs from colliding with or pre-claiming a resend.
- **Logical delivery anchor:** `send-next` mints `resend_cmid = format!("cowboy-origin-steer-resend:{}", receipt.cmid)` and attaches `logical_delivery_key = receipt.logical_delivery_key` to the queued item and its subsequent `prompt_dispatches` row so future C-budget (`session_owner_turns` / B) shares one anchor across original steer and manual resend.
- **Single atomic store transaction (reusing `Hub::pending_commit_lock` + acknowledged FIFO `StoreWrite::UpdatePending`):**
  1. Handler verifies session mutation permission AND `RequestCaller::Person`.
  2. Acquires `Hub::pending_commit_lock(session_id)`.
  3. Under `sessions.lock()` (preview only — zero Hub mutation yet): validates session exists, `!is_managed_child`, `!clear_pending`, reads `receipt` for `(session_id, cmid)`:
     - If `receipt.cleared_at_ms.is_some()` or `receipt.context_generation != session.context_generation`: return `409 context_cleared`.
     - If `receipt.status == "delivered"`: return `409 already_delivered`.
     - If `receipt.resend_state == "enqueued"`: return idempotent `200 { status: "already_queued", resend_cmid }`.
     - If `receipt.status == "pending"`: return `409 steer_still_pending`.
     - If `receipt.status == "not_confirmed"` and `receipt.original_turn_ended_at_ms.is_none()`: return `409 original_turn_not_proven_ended`.
     - Builds `next_queue` by appending `QueuedMessage { id: next_qid(), text: receipt.text, content: decoded_content, cmid: Some(resend_cmid), origin: Some(DeliveryOrigin::Owner), initiator: Some(DeliveryOrigin::Owner), logical_delivery_key: Some(receipt.logical_delivery_key) }` (skipping append if `resend_cmid` already exists).
  4. Enqueues ONE acknowledged FIFO `StoreWrite::UpdatePending` carrying `origin_steer_resend_claim: Some(OriginSteerResendClaim { cmid, expected_context_generation, resend_cmid, require_turn_ended_if_not_confirmed: true, dismissed_at_ms })`.
  5. Inside **one SQLite / Postgres transaction**, the store executes:
     - Conditional `UPDATE origin_steer_receipts SET resend_cmid = ?, resend_state = 'enqueued', dismissed_at_ms = ?, updated_at_ms = ? WHERE session_id = ? AND cmid = ? AND context_generation = ? AND cleared_at_ms IS NULL AND resend_state = 'none' AND dismissed_at_ms IS NULL AND (status = 'not_delivered' OR (status = 'not_confirmed' AND original_turn_ended_at_ms IS NOT NULL))`.
     - **Only if `rows_affected == 1`**, writes `sessions.queue = next_queue` in the **same transaction**.
     - If `rows_affected == 0`, rolls back without touching `sessions.queue` and returns the current receipt state (mapped cleanly to `already_queued` / `already_delivered` / `409`).
  6. Awaits FIFO store ACK. On failure: neither DB nor Hub changed; receipt remains visible and un-dismissed (`503`).
  7. Only after `Ok(Committed)`: applies `next_queue` to Hub, emits pending snapshot + `OriginSteerChanged`, calls `hub.try_drain(session_id)`, and returns `200 { status: "queued", resend_cmid }`.

### 7.5 Clear Conversation Interaction (`src/core/clear_conversation.rs`)

- Inside the durable `StoreWrite::ClearEvents` transaction:
  1. Atomically increment `sessions.context_generation = context_generation + 1`.
  2. Scrub and close all prior-generation receipts for `session_id`:
     ```sql
     UPDATE origin_steer_receipts
     SET text = '',
         content_json = '[]',
         status = CASE WHEN status = 'pending' THEN 'not_confirmed' ELSE status END,
         reason = COALESCE(reason, 'context_cleared'),
         original_turn_ended_at_ms = COALESCE(original_turn_ended_at_ms, ?now_ms),
         original_turn_ended_proof = COALESCE(original_turn_ended_proof, 'context_cleared'),
         cleared_at_ms = ?now_ms,
         dismissed_at_ms = COALESCE(dismissed_at_ms, ?now_ms),
         updated_at_ms = ?now_ms
     WHERE session_id = ? AND cleared_at_ms IS NULL;
     ```
- `finish_clear_conversation` advances in-memory `session.context_generation`, disarms any `pending_echoes`, and emits `OriginSteerChanged`.
- Steer admission refuses while `hub.clear_pending(session_id)` AND verifies `sessions.context_generation == captured_generation` inside the receipt `INSERT`, closing any race with Clear.
- Late worker events or reconnect replays referencing a receipt with `receipt.cleared_at_ms.is_some()` or `receipt.context_generation < session.context_generation` are fenced: they never restore scrubbed text, never un-dismiss the receipt, and never emit an echo into the replacement context.

---

## 8. Authorization, HTTP/WS API & Unified Web Receipt Flow (W1)

### 8.1 Session Authorization Boundary (`client_ws.rs`, `session_api.rs`, `send_delivery.rs`)

- **No receipt text on unscoped `SyncPatch`:** `client_ws.rs:280` broadcasts unrecognized `SyncPatch` scopes to all connected principals. R1 **never** publishes receipt payloads on `SyncPatch`.
- **Content-free session-scoped WS invalidation:** add `Outbound::OriginSteerChanged { session_id: String }`, explicitly gated in `client_ws.rs` by `session_is_visible(hub, principal, &session_id)`.
- **HTTP reads (`GET /api/sessions/{id}/delivery-options` and `GET /api/sessions/{id}/delivery/{operation}`):** enforce the exact same `session_is_visible(hub, principal, &session_id)` check as session transcript reads.
- **HTTP mutations (`POST /api/sessions/{id}/origin-steers/{cmid}/send-next` and `POST /api/sessions/{id}/origin-steers/{cmid}/dismiss`):** enforce **both** session mutation authorization (`require_session_mutable`) AND verified `RequestCaller::Person` (`403 owner_required` for Automation/Agent/Cowboy).

### 8.2 HTTP Contract

- `GET /api/sessions/{id}/delivery-options`:
  - Returns existing `steer: bool` (always `false` for Antigravity), `controls`, `policy`, `target`, plus:
    - `origin_steering: bool`
    - `origin_steer_receipts: OriginSteerReceiptView[]` (non-dismissed, non-cleared rows for current `context_generation`, each with `{ cmid, status, reason, text, block_count, created_at_ms, updated_at_ms, original_turn_live, resend_cmid }`, where `original_turn_live = (status == "not_confirmed" && original_turn_ended_at_ms.is_none())`).
- `POST /api/sessions/{id}/prompt` (`delivery: "current_turn"` on an `origin_steering` session):
  - Returns HTTP `200` immediately with `{ status: "pending", delivery: "origin_steer", cmid, receipt }` once durably admitted (or `{ status: "not_delivered", delivery: "origin_steer", cmid, error, receipt }` on synchronous pre-handoff refusal).
- `GET /api/sessions/{id}/delivery/{operation}`:
  - Checks `origin_steer_receipts` by `(session_id, cmid = operation)` and returns `{ status: "pending" | "injected" | "not_delivered" | "not_confirmed", delivery: "origin_steer", receipt }` (`delivered` maps to `"injected"` for compatibility alongside full `receipt`).

### 8.3 Unified Web UI Flow (`web/src/`)

1. **`DeliveryPicker.tsx`:**
   - On fresh options read: `canSteer = Boolean(props.busy && target && (options.steer || (!props.queued && options.origin_steering)))`.
   - Before fresh read: `steerReady` includes `session.origin_steering === true` **only** when `!props.queued`.
   - Next-checkpoint remains gated strictly on `options?.controls.next_checkpoint` (`false` for Antigravity).
2. **Disable queued Steer for Antigravity (`QueuedDeliveryAction.tsx`):**
   - When `session.provider === "antigravity"` (or `session.origin_steering && !session.supports_current_turn`), queued rows do **not** offer Steer (eliminating Peer/Automation text promotion to Owner and remove-vs-drain duplication). Queued rows retain `Send now`, `Queue first`, edit, and delete.
3. **Single authoritative receipt flow (no 35s polling loop / no contradictory outbox states):**
   - For `origin_steering` sessions, `Composer` sends `POST /api/sessions/{id}/prompt` **once** with a client-generated `operation` (`cmid`).
   - Upon `200` with `delivery === "origin_steer"`: immediately hands `cmid` to the server-authoritative `originSteerReceipts[sessionId]` store slice and removes it from the local optimistic outbox (never running the legacy 35s `deliverCurrentTurn` polling loop that fabricates `unknown`, and never reposting on reload/reconnect).
   - If the POST network request itself fails with no response: outbox enters observation-only reconciliation via `GET /api/sessions/{id}/delivery/{operation}` (never reposting `POST /prompt`).
   - **Live reconciliation & liveness refresh:** Web fetches `GET /api/sessions/{id}/delivery-options` on initial session open, WS reconnect, `Outbound::OriginSteerChanged { session_id }`, AND any session `Status` / `TurnEnded` transition while `originSteerReceipts[sessionId]` contains a `not_confirmed` row with `original_turn_live === true` — ensuring **Send as next message** unlocks immediately when the original turn is proven ended even while `status` remains `not_confirmed`.
   - **Receipt card UX:**
     - `pending`: subtle "Waiting for current step to finish…" indicator with the steered preview.
     - `delivered`: card clears; Owner message is already in the transcript (deduplicated by `cmid`).
     - `not_delivered`: shows retained text + **Send as next message** (enabled) + **Dismiss**.
     - `not_confirmed`: shows retained text + explicit warning (*"Delivery could not be confirmed; the original turn may still have received this message"*) + **Send as next message** (disabled with tooltip while `original_turn_live === true`, enabled once `false`) + **Dismiss**. Never auto-enqueues.

---

## 9. Release Sequencing, Required Artifacts & Safe Rollback

### 9.1 Slice Boundaries & Required M1 Artifacts

1. **Slice M1 (Wire 52/8 + Store migration + Broker + Worker + Controller R1):**
   - Lands on `main` together with the `cowboy-workers` pin commit (preserving `retained-worker-interface-compatible`).
   - **Strict M1/P2 boundary:** M1 commits do **NOT** modify `plugins/antigravity/` manifests or component releases unless the exact signed coverage is already published and verified by `just provider-release-coverage`.
   - **Activate Controller first** (additive migration; pre-52 Machines and pre-8 workers keep `origin_steering = false`).
   - **All 5 required Machine release artifacts** (built on Hawk per the OVH runtime-only contract and verified BEFORE live cutover):
     - **Hawk Machine (4 distinct artifacts on the new retained worker generation, verified via `just session-deletion-production-conformance` per `tools/session_deletion_production_conformance.py:169-191`):**
       1. Old-side writer Machine release (`--old-writer`)
       2. New-side writer Machine release (`--new-writer`, distinct revision and distinct ELF hash from old-side writer)
       3. Reader-only fallback Machine release (`--reader-fallback`)
       4. Default-reader Machine release (`--default-reader`)
     - **OVH Machine (1 artifact, per `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` + 7-plugin closure + Antigravity Plugin `0.2.35` with signed `origin_steering_v1` manifest capability):**
   - Includes the `subagents.test.mjs` `requestMs: 1000` timing fix. Safe to publish independently (`worker < 8` ignores `cowboy.dev/originSteering`).
3. **Slice W1 (Web):**
   - Atomic `/run/cowboy-web` swap.

### 9.2 Safe Rollback Procedure (No Lost Late Outcomes)

A disconnect can settle a receipt `not_confirmed` (making `status = 'pending'` count 0) while its continuation is **still live** on a worker; rolling back directly to a pre-R1 Controller would cause `remote_runtime.rs` to drop replayed `STEERING_RESULT_UPDATE` frames while advancing `highwaters` and sending `Frame::Ack`. Therefore:

1. **Immediate mitigation (preferred):** toggle live config gate `providers.antigravity.origin_steering = false` on the R1 Controller. This immediately refuses all new Antigravity steers under the serialized admission lock while preserving full R1 outcome settlement and replay ingestion for every in-flight grant.
2. **Precondition before rolling back Controller binary to pre-R1:** verify via durable store query that **zero** rows exist in `origin_steer_receipts` where:
   - `status = 'pending'`, **OR**
   - `status = 'not_confirmed' AND original_turn_ended_at_ms IS NULL AND cleared_at_ms IS NULL`
   (proving every pre-disable grant has either settled `delivered`/`not_delivered`, been cleared, or has positive turn-end + replay-watermark consumption).
3. **If drain cannot be proved** (e.g. a Machine is offline with an un-proved `not_confirmed` receipt): either (a) keep the live kill switch off on R1 until the Machine reconnects and finishes replay, (b) force-stop the affected Antigravity worker on that Machine to produce confirmed worker-tree termination, or (c) roll back only to a **rollback-safe build** that retains the read-only `origin_steer_receipts` settlement reader while keeping steer admission disabled.

---

## 10. Verification Plan

- **Crate graph & wire checks:** `cargo check --workspace`, `just check-module-layers`, `just structure-check` confirming `cowboy-wire` has zero dependency on `cowboy-store`.
- **Mixed-version & dual-flag matrix:** automated tests covering all 6 rows of §3.3, including spoofed upstream `_meta.steering` + `_meta["cowboy.dev/originSteering"]` on pre-P2 signed manifest, missing `launch.provider` fail-closed at broker, and sanitization of BOTH `WorkerSnapshot` and `CURRENT_TURN_CAPABILITIES_UPDATE`.
- **Two-phase ledger & cleanup gate:**
  - `cowboy/fenceAttempt` blocks new attempt-1 mutations/calls during attempt-2 preparation (`FencedForContinuation`).
  - `cowboy/fenceAttempt` waits/rejects while `client.relays.in_flight() > 1` (undelivered reply still pending in `Relays`).
  - Lost `process/terminate` reply (`uncertain_effect != None`), unconfirmed `Facade.endTurn`, failed/timed-out `Subagents.finish`, or non-`"ok"` `withChildren` aggregate -> continuation denied.
  - Cancel or TTL expiry occurring *during* attempt-2 preparation -> `cowboy/commitContinuation` returns `allow: false` and keeps ledger fenced.
  - Compromised proxy attempting to emit `sent` without a `Committed` grant (or twice) -> ignored, zero Owner echo.
- **Checked `agy.stdin` write:** unit tests in `antigravity-acp` injecting destroyed stdin, write callback `EPIPE`, and premature `agy` exit -> `writeAgentChecked` returns `{ ok: false }`, calls `cowboy/abortContinuation`, never emits `"sent"`, worker settles `not_confirmed`.
- **Monotonic TTL:** tests proving pre-wire readiness check, `transit_cap_ms` deduction + 10s Steer transit cap under simulated `CREDIT_WINDOW` stall, broker queue deduction, and post-preparation deadline enforcement in `cowboy/commitContinuation`.
- **Stop priority & predecessor prompt fence:**
  - Single-item re-checked `send_pending` preemption under backlog.
  - `machine_transport.rs` negotiated `control_overtake` interleaving bounded `Cancel` across 16 KiB chunks and `CREDIT_WINDOW` stall (and never interleaving on pre-52 peer).
  - `worker.rs` biased command read handling `Cancel` immediately under heavy outbox backlog.
  - Held `Prompt` in `broker.pending_commands` and overtaken chunked `Prompt` on WS cancelled cleanly via `after_prompt_seq` without cancelling later `prompt_seq > after_prompt_seq`.
  - Same-stdin-chunk `session/prompt` + `session/cancel` at `launch.mjs`/`proxy.mjs` cancelling the `RESERVED` slot before `agy` starts.
- **Receipts, monotonic lattice, positive turn-end proof, atomic `send-next` & Clear:**
  - Pre-wire crash (`admitted`) -> `not_delivered`; post-send/post-ACK disconnect (`sent_to_machine`/`acked_by_worker`) -> `not_confirmed`; late replayed `"sent"` upgrades `not_confirmed -> delivered` and is never downgraded.
  - Socket-loss `Crashed` (`worker_exit_confirmed == false`), placeholder snapshots, and unreconciled reconnects keep `send-next` fenced (`409 original_turn_not_proven_ended`); exact `TurnEnded` or `worker_exit_confirmed == true` + `ReplayComplete` unlocks `send-next` and emits `OriginSteerChanged`.
  - Atomic `send-next` transaction failure leaves receipt un-dismissed and queue unchanged; concurrent double-clicks enqueue exactly once with reserved `cowboy-origin-steer-resend:` prefix and shared `logical_delivery_key`.
  - Client-submitted `cowboy-origin-steer-resend:*` rejected at HTTP/WS/Agent API ingress.
  - Multi-block steer preserves exact `block_count` across Controller restart, emits all Owner blocks once, and suppresses `agy`'s native echo.
  - `ClearEvents` scrubs receipt payloads, increments `context_generation`, blocks racing steer admission, and fences late outcomes.
  - Authorization tests: `OriginSteerChanged` filtered by `session_is_visible`; `send-next`/`dismiss` reject non-visible principals and Automation/Agent callers (`403`).
- **Web (W1):** tests for `DeliveryPicker` (`origin_steering` enabled in Composer, disabled for queued rows in `QueuedDeliveryAction`), single-POST handoff into `originSteerReceipts` (no 35s timeout / no repost on reload), and `original_turn_live` unlock refresh on status/turn-end events.
