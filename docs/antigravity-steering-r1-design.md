# Antigravity Current-Turn Steering (R1) — Specification (Revision 3)

Status: design, revision 3 (2026-11-24). Scoped strictly to **R1 (Owner current-turn steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), and queued-row Steer for Antigravity.

---

## 1. Context, Scope & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority.
- **Core Architecture — Worker-Driven Continuation (Attempt 1 -> Trusted Worker Fence -> Trusted Worker Attempt 2):**
  - The Owner's steer payload (`blocks`) **never leaves the trusted worker (`cowboy-acp-worker`) during attempt 1**.
  - While attempt 1 is running, the trusted worker holds at most one pending `ContinuationGrant` in memory and sends only a **payload-free hint notification** (`cowboy/steerWaiting`, containing zero owner text) to the untrusted `agy-proxy` (`cowboy-antigravity-acp`).
  - `agy-proxy` appends at most one fixed Cowboy-authored data note (containing zero owner text) to the next facade tool result asking the model to finish its current step briefly.
  - When `agy` finishes attempt 1, `agy-proxy` fences facade calls and runs `subagents.finishAttempt` (preserving the per-turn 8-child budget), settles attempt-1 backend evidence, defers `settleMemory` while `steerWaiting` is set, and returns attempt 1's normal ACP `session/prompt` reply to the **trusted worker** (`src/acp.rs`).
  - Still holding `prompt_lock` (before emitting `TurnEnd`), the **trusted worker** atomically fences attempt 1 in `Ledger` by monotonic `attempt_gen`, verifies zero attempt-1 relays in flight, zero non-server processes, zero uncertain effects, zero extension calls, clean aggregate backend & cleanup evidence, uncancelled state, and unexpired **ACK-calibrated monotonic TTL**.
  - **If any check fails:** zero bytes of the Owner's steer ever left the trusted worker; the trusted worker immediately emits `STEERING_RESULT_UPDATE { outcome: "not_delivered", reason }` and completes the Cowboy turn cleanly.
  - **If all checks pass:** the trusted worker atomically opens `Ledger` attempt generation 2, **performs the checked ACP handoff of `session/prompt` #2 (carrying the Owner's blocks) directly over the trusted worker->unit stdin pipe**, durably journals `"sent"` locally, emits `STEERING_RESULT_UPDATE { outcome: "sent" }`, and keeps holding `prompt_lock` until `session/prompt` #2 completes, rolling attempt 1 + attempt 2 into one Cowboy turn.
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`, hosting both `src/acp.rs` and the execution endpoint `src/worker_execution.rs` + `Ledger`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running together inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because the untrusted proxy never holds Owner blocks before attempt 2 and never emits `"sent"`, it can neither inject Owner blocks early nor forge a `delivered` receipt or Owner echo.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), and queued-row Steer for Antigravity (§8.3).

---

## 2. Crate Boundaries & Wire Contract (Protocol 52 / Runtime 8)

### 2.1 Dependency-Safe Leaf Wire Types (`components/wire`)

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnEndProof {
    /// Upstream ACP `session/prompt` returned an explicit JSON-RPC reply
    /// for this exact `turn_id` (clean stopReason or explicit ACP error).
    NativeReply { stop_reason: Option<String> },
    /// ACP/worker transport failure or timeout without a confirmed native
    /// reply. Never unlocks `send-next` alone; requires confirmed tree exit.
    TransportError,
}
```

- Controller converts verified `cowboy_store::prompt_origin::DeliveryOrigin` into `WireDeliveryOrigin` at admission (`DeliveryOrigin::Owner => WireDeliveryOrigin::Owner`; all other variants => `WireDeliveryOrigin::NonOwner`).
- Missing `origin: Option<WireDeliveryOrigin>` on any legacy wire frame fails closed as `WireDeliveryOrigin::NonOwner`.

### 2.2 Protocol Bump, Chunked Control Framing & Additive Fields

- **Machine protocol:** bump `PROTOCOL_VERSION` to **52** (`ORIGIN_STEERING_PROTOCOL_VERSION = 52`), negotiating transport feature `control_overtake: bool` (true when both sides speak Machine >= 52 and `features.paced`).
- **Runtime protocol (both `machine_cli <-> machine_broker` and `machine_broker <-> worker`):** bump `PROTOCOL_VERSION` to **8** (`ORIGIN_STEERING_RUNTIME_PROTOCOL_VERSION = 8`).
- **Runtime protocol 8 bounded chunked interleaving (`components/wire/src/runtime_wire.rs`):**
  - `MAX_FRAME_BYTES = 32 MiB` (`0x0200_0000`), leaving header bit `0x8000_0000` unused in the 4-byte big-endian frame length prefix.
  - When negotiated runtime protocol `>= 8`, any bulk frame exceeding `16 KiB` is streamed in `16 KiB` chunks using high-bit `0x8000_0000` headers (`[0x8000_0000 | chunk_len, total_len, offset, chunk_bytes...]`), while bounded (`<= 16 KiB`) control frames (`Cancel`, `CancelTurn`, `CalibrateSteerDeadline`, `CommandAck`, `ExecutionReply`) travel as unchunked standard frames that **interleave immediately between 16 KiB chunks**.
  - Pre-8 peers receive standard unchunked frames only (100% backward compatible).
- **Additive optional wire fields (backward-compatible serde defaults):**
  - `WorkerSnapshot`:
    - `origin_steering: bool` (`#[serde(default)]`)
    - `completed_turn_id: Option<String>` (`#[serde(default)]`)
  - `RuntimeEvent::TurnEnded`:
    - Carries explicit `turn_id: Option<String>` (captured directly from the finishing prompt command, never inferred lazily from mutable snapshot state) and `proof: Option<TurnEndProof>` (`#[serde(default)]`; legacy/absent defaults conservatively to `TransportError` for R1 unlock checks).
  - `Frame::CommandAck`:
    - `refusal: Option<CommandRefusal>` (`#[serde(default)]`)
  - `Frame::ReplayComplete` (runtime protocol 8): `{ session_id, worker_epoch, through_runtime_seq }` — emitted by the **live worker itself** after flushing its outbox in response to `Frame::Replay` (relayed FIFO by broker; never fabricated by broker before worker replay).
  - `Frame::WorkerExitedConfirmed` (Machine 52 / runtime 8): `{ session_id, worker_epoch, launch_id, final_runtime_seq }` — emitted by broker ONLY after OS/systemd confirms worker process-tree termination AND after broker has replayed all durable worker steering outcomes through `final_runtime_seq` (§7.3).
  - `CoreCommand::Prompt` & `WorkerCommand::Prompt`:
    - `dispatch_seq: Option<u64>` (`#[serde(default)]`) — monotonic per-session dispatch sequence minted by Hub at dispatch ownership creation (§6.2).
    - `controller_boot_id: Option<String>` (`#[serde(default)]`) — Controller process boot UUID scoping `dispatch_seq` / `cancel_watermark`.
    - `compacting: bool` (`#[serde(default)]`) — turn-scoped compaction bit (§4.2).
  - `CoreCommand::Cancel` & `WorkerCommand::Cancel`:
    - `cancel_watermark: Option<u64>` (`#[serde(default)]`) — unconditional per-session predecessor prompt watermark (§6.2).
    - `controller_boot_id: Option<String>` (`#[serde(default)]`).
  - `CoreCommand::CancelTurn` & `WorkerCommand::CancelTurn`:
    - Keeps exact `target: DeliveryTarget` (`worker_epoch`, `turn_id`) and independent waiter/admission tracking (§6.1).
  - `CoreCommand::Steer` & `WorkerCommand::Steer`:
    - `origin: Option<WireDeliveryOrigin>` (`#[serde(default)]`)
    - `ttl_ms: Option<u64>` (`#[serde(default)]`)
    - `context_generation: Option<i64>` (`#[serde(default)]`)
  - `CoreCommand::CalibrateSteerDeadline` & `WorkerCommand::CalibrateSteerDeadline` (Machine 52 / runtime 8):
    - `{ session_id, command_id, max_remaining_ms: u64 }` — bounded control frame delivering ACK-calibrated deadline to worker (§5).

---

## 3. Provider-Identity, Compatible Signed Binding & Mixed-Version Gate

### 3.1 Compatible Signed Binding (Zero Unknown Manifest Fields)

`PluginManifest` (`components/plugin-sdk/src/lib.rs:72-83`) and `ProviderPackage` / `RuntimeCapability` (`components/provider-sdk/src/lib.rs:738-739`) reject unknown fields and enum variants (`#[serde(deny_unknown_fields)]`). To guarantee P2 packages install and launch cleanly on older/rollback Machines:

- **No new fields or enum variants** are added to `PluginManifest`, `ProviderPackage`, or `RuntimeCapability`.
- **Signed launch binding:** P2 ships on the existing signed surface as Antigravity Plugin **`version = "0.2.35"`** (with its signed `provider_generation_digest` and signed component archive). At worker launch, the broker verifies the installed signed plugin release and populates authoritative `launch.provider`, `launch.provider_version`, and `launch.provider_generation_digest` inside the broker-verified launch context passed to the worker (never supplied by the untrusted proxy).
- **Single ACP `initialize` metadata location & dual vendor-flag stripping:**
  - `src/acp.rs` reads top-level `init.meta` (`result._meta`, matching `src/acp.rs:3837`), NOT `agentCapabilities._meta`.
  - P2 `proxy.mjs` always deletes **both** upstream `result._meta.steering` AND upstream `result._meta["cowboy.dev/originSteering"]` before stamping (when `!this.managed`):
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
- **Worker-side capability computation & dynamic re-evaluation (`src/acp.rs`, `src/worker.rs`):**
  - Because the ACP thread can finish `initialize` before `worker.rs` completes `Frame::Welcome` protocol negotiation with the broker, the worker separates **raw eligibility** from **effective advertised capability**:
    - `raw_origin_steering_eligible` (for `provider_id == "antigravity"`): true **iff ALL FOUR hold**:
      1. `launch.provider == "antigravity"`
      2. `!session_is_managed_read_only`
      3. Broker-verified signed `launch.provider_version` parses as semver `>= 0.2.35` AND `launch.provider_generation_digest` is present and verified by broker launch admission
      4. Top-level `init.meta["cowboy.dev/originSteering"]` has `v == 1`, `accepts` containing `"owner"`, and `adapter == "cowboy-antigravity-acp"`.
    - `effective_origin_steering = raw_origin_steering_eligible && peer_protocol.load() >= 8`.
  - For `provider_id == "antigravity"`, the worker **always hard-forces** `supports_current_turn = false` and `supports_next_checkpoint = false`.
  - On every `Frame::Welcome` (initial connect AND every broker reconnect), `worker.rs` recomputes `effective_origin_steering`, updates its cached `WorkerSnapshot`, and emits updated `Frame::Snapshot` + `CURRENT_TURN_CAPABILITIES_UPDATE` (`"originSteering": bool`, `"currentTurn": false`, `"nextCheckpoint": false`).

### 3.2 Complete Capability Sanitization & Provider-Specific Steer Routing

Every capability surface — **`Frame::Welcome` initial snapshots**, **`Frame::Snapshot`**, **cached worker state**, and **`Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)`** — is sanitized identically at Broker (`machine_broker.rs:664-671`) and Controller (`remote_runtime.rs:1594-1600`):

- **Broker (`machine_broker.rs`):**
  - Resolves authoritative `provider` from `self.sessions` / verified `snapshot.launch.provider`. If `provider` cannot be established: **fail closed** (`origin_steering = false`, never upgrading `supports_current_turn`).
  - For `provider == "antigravity"`: clamps `supports_current_turn = false`, `supports_next_checkpoint = false`; permits `origin_steering = true` ONLY when worker↔broker runtime protocol `>= 8`, broker↔Controller runtime protocol `>= 8`, and Machine protocol `>= 52`. Recomputes cached snapshots on reconnect / protocol negotiation.
- **Controller (`remote_runtime.rs`, `src/core/`):**
  - Sanitizes `Frame::Welcome`, `Frame::Snapshot`, and `CURRENT_TURN_CAPABILITIES_UPDATE` identically: for `provider == "antigravity"`, clamps `supports_current_turn = false`, `supports_next_checkpoint = false`; permits `origin_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and the serialized live config gate `providers.antigravity.origin_steering == true`. Recomputes and broadcasts updated session capability whenever protocol negotiates, reconnects, or the live config gate toggles.
- **Provider-specific steer dispatch routers (explicitly bypassing legacy `supports_current_turn` for Antigravity):**
  - `RemoteRuntime::start_steer_targeted` (`src/remote_runtime.rs:1045-1059`), Broker's `CoreCommand::Steer` router (`machine_broker.rs:4551-4560`), and Worker steer admission (`src/acp.rs`):
    - If `provider == "antigravity"`: require `origin_steering == true && origin == Some(WireDeliveryOrigin::Owner) && !next_checkpoint` (with `supports_current_turn` remaining strictly `false`!).
    - For all other providers (`codex`, `claude`, etc.): preserve existing `supports_current_turn == true` check unchanged.

### 3.3 Mixed-Version Rollout Matrix

| Controller | Machine | Worker | Installed Antigravity Plugin | `supports_current_turn` | `origin_steering` |
|---|---|---|---|---|---|
| Pre-R1 | Any | Any | P2 (`0.2.35`, compatible schema, strips `_meta.steering`) | `false` | N/A (`false`) |
| R1 (52/8) | Pre-52 | Any | P2 (`0.2.35`, installs/launches cleanly on old Machine) | `false` | `false` (Machine < 52) |
| R1 (52/8) | 52/8 | Pre-8 (retained) | P2 (`0.2.35`) | `false` | `false` (worker↔broker < 8) |
| R1 (52/8) | 52/8 | 8 | Pre-P2 (`< 0.2.35`, even if upstream `agy` spoofs flags) | `false` | `false` (signed version < 0.2.35) |
| R1 (52/8) | 52/8 | 8 | P2 (`0.2.35`), kill switch OFF | `false` | `false` (live gate off) |
| R1 (52/8) | 52/8 | 8 | P2 (`0.2.35`), kill switch ON | `false` | `true` (Owner only) |

### 3.4 Caller Authority Policy at Controller Admission

For Antigravity with `origin_steering == true`:
- `RequestCaller::Person` (`DeliveryOrigin::Owner`): `delivery = "current_turn"` (and `session_default` resolving to steer) enters R1 origin steering; `delivery = "next_checkpoint"` is refused synchronously `not_delivered`.
- Non-Owner (`Agent`, `Automation`, `Cowboy`): explicit `current_turn` / `next_checkpoint` refused synchronously `not_delivered` (`origin_not_accepted`); `session_default` deterministically falls back to `next_turn`.
- Kill switch `providers.antigravity.origin_steering = false` (serialized under admission `RwLock`): refuses new steers synchronously `not_delivered` (`disabled_by_config`) while keeping all outcome ingestion, replay settlement, and manual resend/dismiss active.

---

## 4. Worker-Driven Continuation, Attempt-Generation Ledger & Compaction Fence

### 4.1 Worker-Owned Grant, Payload-Free Hint & Single Authoritative ACK

1. **Controller admission:** requires an exact captured `DeliveryTarget { worker_epoch, turn_id }` matching `supervisor.delivery_target(session)` (otherwise refuses synchronously `not_delivered`).
2. **Worker prompt start (`src/acp.rs`):** when dispatching `AgentCommand::Prompt` on Antigravity (attempt 1), the trusted worker mints a random 128-bit `attempt_nonce`, allocates monotonic `attempt_gen = ledger.begin_turn_attempt(&turn_id, &attempt_nonce)` (opening `AttemptPhase::Open { attempt_gen, attempt_index: 1 }`), stores `ActivePrompt { turn_id, attempt_nonce, attempt_gen, cancel_generation, compacting }` on `ClientState`, and passes `_meta["cowboy.dev/attemptNonce"]` and `_meta["cowboy.dev/compacting"]` on `session/prompt` #1.
3. **Single authoritative first-handoff ACK (`src/worker.rs:1095-1101` & `src/acp.rs`):**
   - For Antigravity `WorkerCommand::Steer` (runtime protocol 8), `worker.rs` does **NOT** emit `CommandAck` upon queueing to `cmd_tx`. Instead, `worker.rs` first checks `seen_commands` dedup (returning `CommandAck { accepted: true, refusal: None }` for duplicates BEFORE checking liveness!), and for a new `command_id` forwards `AgentCommand::Steer` with a `oneshot::Sender<CommandAck>` into `src/acp.rs`.
   - `src/acp.rs` validates ALL pre-handoff preconditions under trusted state:
     - `provider_id == "antigravity"`, `effective_origin_steering == true`, `!managed_read_only`
     - `origin == Some(WireDeliveryOrigin::Owner)`
     - exact `target == (worker_epoch, active_prompt.turn_id)`
     - `!active_prompt.compacting`, `cancel_generation` unchanged, `active_prompt.attempt_index == 1`, and no existing `ContinuationGrant` for this turn (max 1 continuation per turn in R1)
     - `!is_slash_command_steer(&blocks)` and `ttl_ms > 0`.
   - **If any check fails:** `src/acp.rs` replies immediately on the oneshot with `CommandAck { accepted: false, refusal: Some(RefusedBeforeHandoff { code }) }` — zero bytes of the steer ever crossed into the untrusted unit — and the worker's dedicated writer task sends that single ACK while Stop reads stay unblocked.
   - **If all checks pass:** `src/acp.rs` records `t_worker_recv = Instant::now()`, registers the single-use `ContinuationGrant` **inside trusted worker memory** (keeping `blocks` strictly inside the trusted worker!):
     ```rust
     enum CalibrationState { WaitingForCalibration, Calibrated { deadline: Instant } }
     struct ContinuationGrant {
         command_id: String,
         cmid: String,
         turn_id: String,
         attempt_nonce: String,
         cancel_generation: u64,
         blocks: Vec<ContentBlock>,
         t_worker_recv: Instant,
         provisional_ttl: Duration,
         calibration: CalibrationState,
         initial_uncertain_seq: u64,
         retired: bool,
     }
     ```
     and replies on the oneshot with `CommandAck { accepted: true, refusal: None }`.
4. **Payload-free hint to `agy-proxy`:** After admitting the grant, `src/acp.rs` sends a **payload-free ACP notification** `cowboy/steerWaiting { sessionId, attemptNonce }` (containing **zero owner text and zero blocks**) to `agy-proxy`.
   - While `turn.steerWaiting === true` on attempt 1, `agy-proxy` appends at most one fixed Cowboy-authored data note to the next facade tool result:
     `"\n[Cowboy note: a follow-up message from the owner is waiting; finish the current step briefly so it can be delivered.]"`
   - `agy-proxy` never receives the Owner's blocks before attempt 1 finishes and never reports `"sent"`.

### 4.2 Exact Shared Slash / Compaction Predicate & Turn-Scoped Latch

- **Block normalization:** apply `build_prompt_blocks(text, content)` first — when `content` is empty and `text` is non-empty, normalize to `[{ "type": "text", "text": text }]`.
- **Explicit shared closed Unicode whitespace set `COWBOY_WS`** (identical in Rust `is_cowboy_ws(ch: char)` and JS `isCowboyWs(cp: number)` — **neither** runtime calls `char::is_whitespace()`, JS `.trim()`, or `/\s/` so `U+0085` cannot diverge across hops!):
  - Exact code points (ECMAScript `WhiteSpace ∪ LineTerminator`, **explicitly excluding `U+0085`**):
    `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF`.
- **`is_compaction_prompt(blocks)`:** true iff `blocks` is non-empty, **every** block has `type == "text"` with string `text`, and trimming leading/trailing `COWBOY_WS` from `blocks.map(b => b.text).join("")` starts with ASCII case-insensitive `"/compact"` followed immediately by end-of-string or a `COWBOY_WS` code point.
- **`is_slash_command_steer(blocks)`:** collect contiguous leading `type == "text"` blocks from index 0 (stopping at the first non-text block), join `.text` with `""`, strip leading `COWBOY_WS`, and return true iff the remainder starts with `'/'`.
- Both predicates are cross-tested by one shared JSON test vector file executed by both Cargo and Node tests, explicitly testing `U+0085` and `U+FEFF` before `/` and after `/compact`, split blocks `["/com", "pact"]`, `["   ", "/compact"]`, mixed text+image blocks, and empty `content` with `text` fallback.
- **Turn-scoped compaction latch across all 3 stages:**
  1. **Controller prompt dispatch:** evaluates `compacting = is_compaction_prompt(&blocks)` at dispatch creation, sets `session.active_turn_compacting = true` synchronously under `sessions.lock()` (cleared on turn end or Clear), and sends `compacting: true` on `CoreCommand::Prompt` / `WorkerCommand::Prompt`. Controller refuses any Antigravity steer immediately (`not_delivered`, `compacting`) while `hub.is_compacting(session_id) || hub.active_turn_is_compacting(session_id)`.
  2. **Worker (`src/acp.rs`):** sets `active_prompt.compacting = cmd.compacting || is_compaction_prompt(&blocks)` synchronously before sending `session/prompt`, refusing steers with `RefusedBeforeHandoff(Compacting)`.
  3. **Proxy (`proxy.mjs`):** computes `compacting = Boolean(params?._meta?.["cowboy.dev/compacting"]) || isCompactionPrompt(params?.prompt)` synchronously in `fromClient` when reserving the `RESERVED` turn slot (before `this.queue.then(...)` and before any `await`), latching `turn.compacting = true` and `this.compacting.add(sessionId)`.
- **Steer-payload slash fence:** Controller, Worker, and Proxy refuse any Antigravity steer matching `is_slash_command_steer(&blocks)` (`not_delivered`, `slash_command_not_steerable`).

### 4.3 Attempt-Generation Execution Ledger & Subagent Per-Turn Budget

1. **Attempt-generation binding at execution ingress (`src/worker_execution.rs:737-746`, `src/worker_execution/relay.rs:73-82`, `src/worker_execution/ledger.rs`):**
   - `Ledger` owns a monotonic `attempt_gen: u64` and `AttemptPhase` under its single `Mutex<State>`:
     ```rust
     enum AttemptPhase {
         Open { turn_id: String, attempt_gen: u64, attempt_index: u8 },
         FencedForContinuation { turn_id: String, attempt_gen: u64 },
         TurnFenced,
     }
     ```
   - Every incoming execution request in `worker_execution.rs:serve` acquires its `AttemptRelayGuard` **atomically under `Ledger`'s lock at ingress** (`ledger.try_enter_attempt_relay(&method)`), capturing the current `attempt_gen` and incrementing `in_flight_by_gen[attempt_gen]` **before** spawning the relay task — and keeping that `AttemptRelayGuard` inside `Done` until `serve` has either written the reply frame to the unit socket or recorded it undelivered in `Ledger`!
   - Requests outside `AttemptPhase::Open` (except read-only/cleanup requests explicitly permitted during drain) are rejected **at ingress under the `Ledger` lock**, so an attempt-1 request can **never** slip past `fence_attempt` and later enter attempt 2!
   - Non-server process cleanliness (`src/worker/sandbox_extensions.rs:72-78`, `src/worker_execution/ledger.rs:244-263`): ordinary sandboxed sessions keep Cowboy MCP servers recorded in `ledger.processes` under `SERVER_TURN` (and `ledger.servers`) until teardown. Continuation cleanliness therefore checks **`ledger.has_no_non_server_processes()`** (zero entries in `ledger.processes` whose owner `!= SERVER_TURN` and `!ledger.servers.contains(pid)`, and zero entries in `ledger.starting` outside `ledger.servers`) while continuing to require `ledger.admitted_for_turn(&turn_id) == 0`, `ledger.calls == 0`, `in_flight_by_gen[attempt_gen_1] == 0`, `!ledger.unwritable`, `ledger.uncertain_effect().is_none()`, and `ledger.uncertain_sequence() == grant.initial_uncertain_seq`!
2. **Proxy attempt-1 teardown & per-turn subagent cap (`subagents.mjs:915-925`, `docs/requirements.md:905-909`):**
   - Split `Subagents` teardown into two methods:
     - `finishAttempt(sessionId)` (used when attempt 1 ends with `turn.steerWaiting === true`): fences and awaits every child launched during attempt 1, accumulates **sticky failure/timeout/termination/uncertainty/violation evidence** across both already-settled and newly-stopped children, returns `{ clean, timedOut, failedChildren, terminatedAgent, uncertain, violation }`, and **PRESERVES `group.started`** (so attempt 1 + attempt 2 share ONE single 8-child per-turn cap!).
     - `finishTurn(sessionId)` (used on final turn end or when starting a new non-continuation turn): runs full teardown and resets `group.started` and sticky attempt evidence.
   - When attempt 1 finishes at `proxy.mjs`:
     - Synchronously starts BOTH `runtime.facade.endTurn(sessionId)` (returning `{ clean, unconfirmedCommands }`) AND `this.subagents.finishAttempt(sessionId)` (if `steerWaiting`, else `finishTurn`) before awaiting either.
     - Awaits `evidence.settle` and computes aggregate `withChildren(turn, attempts(turn, settled))` (incorporating sticky child outcomes).
     - If `turn.steerWaiting === true`, defers `settleMemory(turn)` (will settle on attempt 2 end or when worker sends `cowboy/turnFinalize` if attempt 2 is skipped) and returns the normal `session/prompt` #1 response to the trusted worker carrying `_meta.cowboy = { antigravity_backend: aggregate, cleanup_clean: facadeResult.clean && facadeResult.unconfirmedCommands === 0 && subagentResult.clean, ... }`.

### 4.4 Trusted Worker Attempt-2 Handoff, Ambiguous Write Safety & Checked Proxy Write

When attempt 1's `session/prompt` #1 reply arrives at the **trusted worker (`src/acp.rs`)** (still holding `prompt_lock`; Cowboy turn has NOT ended):

1. **No pending grant:** finish turn normally (`TurnEnded { turn_id, proof: NativeReply { stop_reason } }`).
2. **Pending `ContinuationGrant` exists:**
   - **Step A — Fence attempt 1 in `Ledger`:** call `ledger.fence_attempt(&turn_id, attempt_gen_1)` (atomically moving `Open(gen_1) -> FencedForContinuation(gen_1)` under `Ledger` lock) and await `in_flight_by_gen[attempt_gen_1] == 0` boundedly (up to `min(250ms, remaining_ttl)`, aborting immediately on `cancel_generation` change).
   - **Step B — Wait for ACK calibration if still pending (§5):** if `grant.calibration` is still `WaitingForCalibration`, wait boundedly (up to `min(5s, grant.provisional_ttl)`, aborting on cancel) for `CalibrateSteerDeadline`; if calibration does not arrive, fail closed.
   - **Step C — Single-lock trusted cleanliness & deadline check:** under `Ledger` + `ContinuationGrant` lock, verify ALL of:
     - `attempt_1_reply` is `Ok(resp)` with `resp.stop_reason == EndTurn`
     - `resp._meta.cowboy.antigravity_backend.class == "ok"` AND `resp._meta.cowboy.cleanup_clean == true` AND no `policy_violation` / `uncertain_effect`
     - `ledger.attempt_phase == FencedForContinuation(gen_1)` AND `!ledger.closed` AND `!ledger.is_fenced(&turn_id)`
     - `!ledger.unwritable`, `ledger.uncertain_effect().is_none()`, `ledger.uncertain_sequence() == grant.initial_uncertain_seq`
     - `ledger.has_no_non_server_processes()`, `ledger.admitted_for_turn(&turn_id) == 0`, `ledger.calls == 0`, `in_flight_by_gen[gen_1] == 0`
     - `cancel_generation == grant.cancel_generation`, `!active_prompt.compacting`
     - `grant.calibration == Calibrated { deadline }` AND **`Instant::now() < deadline`**.
   - **If ANY check in Steps A–C fails:**
     - Zero bytes of `grant.blocks` ever left the trusted worker!
     - Mark `grant.retired = true`, transition `Ledger` to `TurnFenced`, send `cowboy/turnFinalize { sessionId }` to proxy (so proxy settles memory & resets `group.started`), durably journal `outcome = "not_delivered"` in `origin-steer-outcomes-<session_id>.json` (§7.3), emit `STEERING_RESULT_UPDATE { cmid, worker_epoch, turn_id, outcome: "not_delivered", reason }`, and emit `TurnEnded { turn_id, proof: NativeReply { stop_reason } }`!
   - **If ALL checks pass — Trusted Attempt-2 Handoff:**
     - Atomically open `attempt_gen_2 = ledger.open_continuation_attempt(&turn_id, &attempt_nonce)` (`AttemptPhase::Open { turn_id, attempt_gen: attempt_gen_2, attempt_index: 2 }`) right at the trusted write boundary.
     - Record `grant.blocks` in `state.last_echoed_user_contents` (suppressing duplicate native echo).
     - **Trusted checked write of `session/prompt` #2:** write the ACP `session/prompt` #2 request (carrying `grant.blocks` and `_meta["cowboy.dev/continuation"] = { "v": 1, "attempt": 2, "sameTurn": true, "turnId": turn_id, "cmid": cmid }`) across the trusted worker->unit stdin pipe, checking flush completion!
     - **If the trusted stdin write succeeds cleanly:**
       1. Mark `grant.retired = true`.
       2. Atomically `fsync` `{ launch_id, worker_epoch, turn_id, cmid, outcome: "sent", runtime_seq }` into the worker's durable outcome journal `origin-steer-outcomes-<session_id>.json`.
       3. Emit `STEERING_RESULT_UPDATE { cmid, worker_epoch, turn_id, outcome: "sent" }` (Controller atomically commits receipt `delivered` + all `N` Owner echo blocks in one DB transaction — §7.4!).
       4. Await `session/prompt` #2's ACP reply while still holding `prompt_lock`; when attempt 2 finishes, aggregate usage/backend metadata with attempt 1 and emit the single Cowboy turn's `TurnEnded { turn_id, proof }` (`NativeReply` if ACP replied cleanly, `TransportError` if ACP transport failed!).
     - **If the trusted stdin write fails or is ambiguous (broken pipe / unit transport error):**
       - Do **NOT** emit `"sent"` and do **NOT** emit `TurnEnded(NativeReply)`!
       - Fence `Ledger` to `TurnFenced`, emit `STEERING_RESULT_UPDATE { outcome: "not_confirmed", reason: "unit_write_failed" }`, terminate the account unit cleanly, and emit `TurnEnded { turn_id, proof: TurnEndProof::TransportError }` (which keeps `send-next` fenced until OS/systemd confirms unit tree exit — §7.3!).
3. **Proxy handling of `session/prompt` #2 (`launch.mjs` & `proxy.mjs` — preventing ambiguous writes from outliving the turn):**
   - `proxy.mjs` serializes `fromAgent` prompt-reply processing and continuation write/finalization behind one shared per-session turn serialization state (`turn.ioChain`) so `launch.mjs:489-493` can never dispatch concurrent stdout handlers while a write or finalization is in flight.
   - When `session/prompt` #2 arrives with `_meta["cowboy.dev/continuation"].sameTurn === true`:
     - Does **NOT** reset `group.started` (preserving the 8-child per-turn cap).
     - Records continuation blocks in `turn.suppressedContinuationEchoes` so `agy`'s native echo of attempt 2 is dropped alongside `injectedEcho(update)`.
     - Dispatches to `agy` via `writeAgentChecked(message, timeoutMs)` (checking `agy` alive, `!stdin.destroyed && stdin.writable`, callback error, `drain`, `close`, and `exit`).
     - **Ambiguous-write containment:** if `writeAgentChecked` fails or times out *after* invoking `agy.stdin.write` (so queued bytes cannot be retracted from the kernel pipe), `proxy.mjs` **NEVER** replies immediately while `agy` is alive! Instead it sends native `session/cancel` to `agy` (and if `agy` does not confirm within a short 3s watchdog, calls `terminate()` / `SIGKILL` and awaits `agy` exit!) **before** returning an ACP error for `session/prompt` #2. Thus an ambiguous write can **never** outlive the reported Cowboy turn.

---

## 5. ACK-Calibrated Monotonic Receiver Lease (Zero Uncharged Hop Time)

Default TTL: **600,000 ms (10 minutes)** (configurable per-request within `[30_000, 600_000]`). Zero cross-host wall clock assumptions and zero uncharged time across store waits, bridge queues, WS credit stalls, TCP buffers, broker queues, or worker reads:

1. **Controller admission (`t_admit`):** at HTTP/WS ingress, Controller captures `t_admit = Instant::now()` (Controller monotonic clock) and records `original_ttl_ms`. If `Instant::now() - t_admit >= original_ttl_ms` before durable `sent_to_machine` reservation, Controller settles synchronously `not_delivered` (`ExpiredBeforeHandoff`) without writing to the wire.
2. **Worker receipt (`t_worker_recv`) & ACK:** when the trusted worker receives `WorkerCommand::Steer { ttl_ms }` and admits the `ContinuationGrant`, it records `t_worker_recv = Instant::now()` (worker monotonic clock), sets `grant.calibration = WaitingForCalibration`, and returns `CommandAck { accepted: true }`.
3. **Controller RTT calibration (`t_ack`):** when Controller receives `CommandAck { accepted: true }` at `t_ack = Instant::now()`, it computes:
   ```text
   observed_rtt_ms = t_ack.saturating_duration_since(t_admit).as_millis()
   max_remaining_ms = original_ttl_ms.saturating_sub(observed_rtt_ms)
   ```
   - **Causal soundness proof:** `t_admit < t_worker_recv < t_ack` (strict happens-before), so total transit time `D = t_worker_recv - t_admit` (including DB reservation wait, runtime bridge queues, WS credit stall, TCP buffers, broker queue, and worker socket read) strictly satisfies `D < observed_rtt_ms`. Therefore `t_worker_recv + max_remaining_ms < t_admit + original_ttl_ms` ALWAYS holds with zero wall-clock sync!
   - Controller immediately sends the bounded control frame `CoreCommand::CalibrateSteerDeadline { session_id, command_id, max_remaining_ms }` on the high-priority control lane (travelling alongside `Cancel`!).
4. **Worker deadline enforcement:** upon receiving `CalibrateSteerDeadline`, worker sets `grant.calibration = Calibrated { deadline: t_worker_recv + Duration::from_millis(max_remaining_ms) }` (if `max_remaining_ms == 0` or already passed, immediately retires the grant and emits `not_delivered` — remember, zero Owner blocks left the worker!). Right before trusted attempt-2 handoff (§4.4 Step C), worker enforces `grant.calibration == Calibrated { deadline } && Instant::now() < deadline` (failing closed `not_delivered` if uncalibrated or expired!).

---

## 6. Stop Priority Across Every Link & Complete Predecessor Prompt Fence

### 6.1 Non-Blocking Stop Priority on Every Link & Independent `CancelTurn` Slot

1. **Controller HTTP/WS & separate `CancelTurn` slot (`src/remote_runtime.rs:1123-1128`):**
   - Cancel/Stop uses a dedicated `CANCEL_IN_FLIGHT` semaphore (16 permits) independent of ordinary `IN_FLIGHT`.
   - Give `CancelTurn` its own per-session waiter/command slot (`cancel_turn_waiters`) completely separate from `steering_waiters` / `origin_steer_waiters`, so targeted Owner Interrupt remains available throughout an accepted 10-minute R1 steer.
2. **Controller `RemoteRuntime::send_pending` (`src/remote_runtime.rs:1811-1888`):**
   - Single-command priority picker loop: each iteration locks `shared.pending`, selects the **single highest-priority ready command** (`Cancel` / `CancelTurn` / `CalibrateSteerDeadline` / `StopSession(reset-*)` > `RecoverExecutionSession` / `EnsureSession` > `SetConfigOption` > `Steer` > `Prompt`), unlocks, writes that ONE command, and loops back so any newly arrived Stop preempts the backlog immediately.
3. **Controller↔Machine WS writer (`src/machine_transport.rs:198-205,371-407`):**
   - When `features.control_overtake` is negotiated (Machine >= 52 and `features.paced`), `overtakes(text)` and `Decoder::text` permit bounded (`<= 16 KiB`) `Cancel`, `CancelTurn`, and `CalibrateSteerDeadline` frames to interleave between 16 KiB bulk chunks and bypass `CREDIT_WINDOW` stalls.
4. **Local runtime tunnel (`components/machine/src/machine_cli.rs:1579-1581,3862-3874`) & Broker↔Worker Unix link (`machine_broker.rs`, `src/worker.rs:755-763`):**
   - Both links run runtime protocol 8 **16 KiB chunked framing with unchunked control interleaving** (§2.2) AND **split independent reader and writer tasks**:
     - `machine_cli -> machine_broker`: split `control_tx` (polled `biased` before every 16 KiB chunk) from `bulk_tx` so an overtaking WS Cancel never waits behind a multi-MiB frame on the local runtime tunnel.
     - `machine_broker`: inbound `Cancel`/`CancelTurn`/`CalibrateSteerDeadline` enter a dedicated bounded `control_rx` polled `biased` ahead of slow lifecycle commands; each worker Unix connection runs a dedicated writer task polling `control_queue` `biased` ahead of `bulk_queue` before every 16 KiB chunk.
     - `worker.rs`: split the broker Unix socket into an **independent reader task** (`reader.next()` -> immediate `cancel_tx.send(...)` / `control_tx.send(...)` / `cmd_tx.send(...)`) and an **independent writer task** (draining `ack_rx` > `control_rx` > `outbox_rx` in 16 KiB interleaved chunks), so a blocked or backpressured outbox write can **never** delay reading a `Cancel`!
5. **Worker ACP loop (`src/acp.rs`):**
   - Polls `cancel_rx` in a `biased` select arm ahead of `cmd_rx`; `Cancel` / `CancelTurn` **never await `steering_gate` or `prompt_lock`**, synchronously bump `cancel_generation`, synchronously fence `Ledger` (`TurnFenced`), retire any pending `ContinuationGrant`, and emit `session/cancel` to `agy-proxy` immediately.
6. **Proxy root Stop ordering (`proxy.mjs:756-781`, `facade.mjs:212-222,388-396`):**
   - `session/cancel` bypasses `this.queue` in `fromClient`.
   - In `proxy.mjs:cancel()`, for **root sessions as well as subagents**:
     1. **Synchronously** latch `turn.cancelled = true`, bump `this.cancelGen`, clear `turn.steerWaiting`, and synchronously fence new facade + subagent admission (zero `await`!).
     2. **IMMEDIATELY** forward `session/cancel` to `agy` (`this.agent(message)`) **BEFORE** awaiting facade/subagent cleanup (even while a 170s extension call is blocked in `facade.mjs:212-222,388-396`).
     3. Then asynchronously drain facade + subagent cleanup alongside `agy`'s cancellation.

### 6.2 Complete Predecessor Prompt Fence (From Dispatch Ownership Through ACP)

To prevent root `Cancel` from overtaking any earlier prompt sitting in preflight, transport, broker queues, or proxy stdin — while preserving later prompts and keeping exact `CancelTurn` targets distinct:

1. **Dispatch ownership identity (`src/core/hub_queue/` & `background_jobs/dispatch.rs:90-167`):**
   - Hub assigns a strictly monotonic per-session **`dispatch_seq: u64`** synchronously under `sessions.lock()` **at dispatch ownership creation** (when `s.in_flight = true` and `DispatchReq` is created, BEFORE `background_jobs/dispatch.rs` preflight begins!), tagged with the Controller's `controller_boot_id`.
2. **Unconditional root `cancel_watermark`:**
   - Root `Cancel` updates `s.cancel_watermark = s.cancel_watermark.max(s.last_dispatch_seq)` **unconditionally** under `sessions.lock()` (even if the prompt is still in preflight or no `CoreCommand::Prompt` exists yet!).
   - Targeted `CancelTurn { target }` does **NOT** advance `cancel_watermark` (it targets only its exact `(worker_epoch, turn_id)`).
3. **Checked at EVERY single handoff boundary:**
   - **Boundary 1 (`background_jobs/dispatch.rs:90-167` preflight):** checks `req.dispatch_seq <= hub.cancel_watermark(session_id)` before and after preflight; if tripped, aborts cleanly before creating `CoreCommand::Prompt`.
   - **Boundary 2 (`RemoteRuntime.pending`):** atomically withdraws any unsent `CoreCommand::Prompt` with `dispatch_seq <= cancel_watermark`.
   - **Boundary 3 (`machine_broker.rs` — inbound, lifecycle queue, `pending_commands`, worker-writer queue):** broker stores `(controller_boot_id, cancel_watermark)` **unconditionally** upon receiving `Cancel` (if `controller_boot_id` changes on Controller restart, any held prompt from an older `controller_boot_id` is also withdrawn!). Every broker handoff (inbound decode -> lifecycle -> `pending_commands` -> worker writer) rejects/withdraws any `Prompt` with `dispatch_seq <= cancel_watermark` cleanly and reports cancelled/undelivered, while allowing later prompts (`dispatch_seq > cancel_watermark`) through!
   - **Boundary 4 (`worker.rs` & `src/acp.rs`):** worker records `(controller_boot_id, cancel_watermark)` unconditionally on `WorkerCommand::Cancel` and refuses to start any queued `WorkerCommand::Prompt` with `dispatch_seq <= cancel_watermark`.
   - **Boundary 5 (`launch.mjs:359-376` & `proxy.mjs:559-564`):** `proxy.mjs:fromClient` synchronously reserves a `RESERVED` turn slot capturing `cancelGen` before `this.queue.then(...)`; a same-chunk `session/cancel` marks the `RESERVED` slot cancelled so the queued handler replies `{ stopReason: "cancelled" }` without starting `agy`.

---

## 7. Durable Receipts, Atomic Settlement+Echo, Positive Proof & Safe Rollback

### 7.1 Durable Schema (`migrations/0090_origin_steer_receipts.sql` & `migrations/sqlite/0064_origin_steer_receipts.sql`)

```sql
ALTER TABLE sessions ADD COLUMN context_generation BIGINT NOT NULL DEFAULT 0;
ALTER TABLE prompt_dispatches ADD COLUMN logical_delivery_key TEXT;

CREATE TABLE origin_steer_receipts (
    session_id TEXT NOT NULL,
    cmid TEXT NOT NULL,
    command_id TEXT NOT NULL,
    logical_delivery_key TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
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
    original_turn_ended_proof TEXT, -- 'turn_ended_native_reply' | 'worker_exit_confirmed'
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

- **Retention rule (no premature deletion of unresolved rows):** dismissed or cleared rows are eligible for 7-day retention pruning **ONLY** when fully resolved: `status IN ('delivered', 'not_delivered') OR (status = 'not_confirmed' AND original_turn_ended_at_ms IS NOT NULL)`. Unresolved content-free settlement rows are retained until genuine turn-end + replay proof exists (or explicit session deletion).

### 7.2 Atomic Steer Admission, Handoff Phase Durability & Typed First-Handoff Refusal

1. **Single-transaction admission (`send_delivery.rs:445-458`):**
   - Validates Owner caller (`RequestCaller::Person`), exact `DeliveryTarget`, non-compacting, non-slash, `!clear_pending`, live config gate, and binds `(session_id, cmid)` to `(caller, target, payload_sha256)` using existing delivery operation idempotency (`send_delivery.rs:445-458`).
   - Inside **ONE `context_generation`-checked SQLite/Postgres transaction**:
     - Verifies `sessions.context_generation == captured_generation`.
     - Inserts `origin_steer_receipts` (`status = 'pending'`, `handoff_phase = 'admitted'`, `logical_delivery_key = cmid`, `payload_sha256`, `block_count = blocks.len()`).
     - Inserts `prompt_dispatches` (`origin = Owner`, `logical_delivery_key = cmid`) in the **same transaction** (if the receipt insert fails or Clear raced, zero orphan provenance row exists!).
   - **Isolation from generic echo hydration & requeue:** receipt-owned `cmid`s do **NOT** arm generic `pending_echoes` in Hub and are explicitly excluded from generic `hydrate_prompt_dispatches` `pending_echoes` hydration and `UndeliveredPrompt` requeue (all R1 echo persistence happens atomically in §7.4).
2. **Serialized first-write reservation (`RemoteRuntime`):**
   - Immediately **before** writing `CoreCommand::Steer` bytes onto Machine WS connection `conn_id`, atomically transitions `handoff_phase: 'admitted' -> 'sent_to_machine'` with `handoff_connection_id = conn_id` (`WHERE command_id = ? AND handoff_phase = 'admitted' AND status = 'pending'`).
   - If `rows_affected == 0` or store error: **never writes to the wire**. `RemoteRuntime` never replays `CoreCommand::Steer` after `handoff_phase` leaves `'admitted'`.
3. **CommandAck processing (`src/remote_runtime.rs`):**
   - Because `src/acp.rs` owns the single authoritative `CommandAck` (§4.1), the worker never ACKs enqueue before proving admission/refusal:
     - `accepted: true`: durably transitions `handoff_phase: 'sent_to_machine' -> 'acked_by_worker'` and immediately dispatches `CalibrateSteerDeadline` (§5).
     - `accepted: false` with `refusal: Some(RefusedBeforeHandoff { code })`: settles **`not_delivered`** (`handoff_phase = 'settled'`) **iff** `command_id`, `handoff_connection_id == Some(current_conn_id)`, `handoff_phase == 'sent_to_machine'`, and `status == 'pending'`. Commits SQL settlement **BEFORE** retiring ACK state.
     - Any other `accepted: false` ACK: durably settles `status = 'not_confirmed'`, `handoff_phase = 'settled'` before retiring ACK state.
4. **Disconnect & Controller restart recovery:**
   - Connection `conn_id` drops while `handoff_phase IN ('sent_to_machine', 'acked_by_worker')` and `status == 'pending'` -> durably settles `status = 'not_confirmed'`, `reason = 'machine_disconnected'` (keeping `original_turn_ended_at_ms = NULL`).
   - Controller restart finds `handoff_phase == 'admitted'` and `status == 'pending'` -> durably settles `status = 'not_delivered'`, `handoff_phase = 'settled'`, `reason = 'controller_restart_before_handoff'`.

### 7.3 Monotonic Receipt Lattice, Durable Worker Outcome Journal & Positive Turn-End Proof

```text
pending  --(pre-wire crash / Controller refusal / typed RefusedBeforeHandoff / trusted worker pre-attempt-2 abort)--> not_delivered  [ABSORBING]
pending  --(disconnect / untyped ACK / unit_write_failed / turn_ended_without_outcome)------------------------------> not_confirmed  [PROVISIONAL]
pending | not_confirmed  --(validated trusted worker STEERING_RESULT_UPDATE outcome="sent")-------------------------> delivered      [ABSORBING]
pending | not_confirmed  --(validated trusted worker STEERING_RESULT_UPDATE outcome="not_delivered" pre-write)------> not_delivered  [ABSORBING]
```

*(Note on `not_confirmed -> not_delivered` upgrade: allowed ONLY on a validated trusted worker `STEERING_RESULT_UPDATE { outcome: "not_delivered" }` proving the Owner's blocks never left the trusted worker before attempt 2, never from an untrusted proxy claim and never downgrading `delivered`!)*

- **Positive Turn-End & Replay Proof (`original_turn_ended_at_ms`):**
  A `not_confirmed` receipt stays **fenced** (`original_turn_live = true`, `send-next` returns `409 original_turn_not_proven_ended`) until Controller durably consumes one of two **positive proofs**:
  1. **Exact `TurnEnded(NativeReply)` event (`src/acp.rs:4698-4714`, `src/worker/remote_sink.rs:60-85`):**
     - `Frame::WorkerEvent(TurnEnded { turn_id: Some(tid), proof: Some(TurnEndProof::NativeReply { .. }) })` for exact `(session_id, worker_epoch)` and `tid == receipt.turn_id` processed in-order through `handle_frame` (proving every lower `runtime_seq` event, including any `"sent"` or `"not_delivered"` outcome, was already durably processed!).
     - Inside the pre-ACK durable transaction: reconciles any still-`pending` receipt for `(worker_epoch, turn_id)` to `not_confirmed` (`turn_ended_without_outcome`) AND sets `original_turn_ended_at_ms = now_ms, original_turn_ended_proof = 'turn_ended_native_reply'`.
     - **Transport-error fence:** `TurnEnded { proof: Some(TurnEndProof::TransportError) }` (or legacy `proof: None`) **NEVER** sets `original_turn_ended_at_ms` — it leaves `not_confirmed` fenced until Proof #2 confirms exact process-tree exit!
  2. **Durable Worker Outcome Journal + Confirmed Process-Tree Exit + Replay Watermark (`machine_broker.rs:664-683`, `src/worker.rs:541-542`):**
     - **Live worker reconnect:** the **live worker itself** emits `Frame::ReplayComplete { session_id, worker_epoch, through_runtime_seq }` after flushing its outbox for `Frame::Replay` (relayed FIFO by broker; broker never fabricates replay completion for a live worker).
     - **Dead / force-stopped worker:** the trusted worker durably `fsync`s every steering outcome into `origin-steer-outcomes-<session_id>.json` (carrying `{ launch_id, worker_epoch, turn_id, cmid, outcome, runtime_seq }`) alongside the execution ledger. Broker binds `launch_id <-> worker_epoch` at launch registration in `provider_store` and records `final_runtime_seq` upon OS/systemd-confirmed worker-tree exit (`confirm_account_unit_gone` / child `waitpid` / durable `account_worker_exit` — **never** on broker↔worker socket loss at `machine_broker.rs:3199` or placeholder snapshots!).
     - Before emitting `Frame::WorkerExitedConfirmed { session_id, worker_epoch, launch_id, final_runtime_seq }`, broker reads `origin-steer-outcomes-<session_id>.json` for `worker_epoch` and replays any durable outcome with `runtime_seq > controller_highwater` in order.
     - Controller unlocks `original_turn_ended_at_ms` (`original_turn_ended_proof = 'worker_exit_confirmed'`) on `WorkerExitedConfirmed` **ONLY** when `controller_highwater[(session_id, worker_epoch)] >= final_runtime_seq` (proving durable Controller consumption of all outcomes through the final watermark!).
- Whenever `original_turn_ended_at_ms` transitions from `NULL` to non-`NULL`, Controller emits content-free `Outbound::OriginSteerChanged { session_id }`.

### 7.4 Single Atomic Settlement + Multi-Block Echo Transaction & Clear Safety

To eliminate divergence between receipt status and multi-block attachments AND prevent late echoes from repopulating a cleared transcript (`hub_dispatch_origin.rs:331-375`, `hub_events.rs:269-270`, `clear_conversation.rs:40-79`):

- **No untagged worker `user_message_chunk` stream for R1 steers:** the trusted worker does **NOT** emit write-behind `user_message_chunk` events for an R1 steer (it still suppresses `agy`'s native echo via `last_echoed_user_contents` + proxy `suppressedContinuationEchoes`).
- **One atomic SQLite/Postgres transaction BEFORE `Frame::Ack` advances `highwaters`:**
  - Serialized through the FIFO persistence worker (strictly ordered against `StoreWrite::ClearEvents`!).
  - Executes inside ONE DB transaction for `STEERING_RESULT_UPDATE { outcome: "sent", session_id, worker_epoch, turn_id, cmid }`:
    1. Reads `origin_steer_receipts` row for `(session_id, cmid)` matching `(worker_epoch, turn_id)` and `sessions.context_generation`.
    2. Updates `origin_steer_receipts` (`status = 'delivered'`, `handoff_phase = 'settled'`, `updated_at_ms = now_ms`) where `status IN ('pending', 'not_confirmed')` (idempotent if already `'delivered'`).
    3. **ONLY IF** `receipt.cleared_at_ms IS NULL AND receipt.context_generation == sessions.context_generation`: inserts ALL `N = receipt.block_count` normalized Owner `user_message_chunk` rows into `events` in that **exact same transaction**, stamped authoritatively with `promptOrigin = Owner` (`logical_delivery_key = receipt.logical_delivery_key`) and deterministic stable per-block IDs:
       - Block `0`: `cmid = <steer_cmid>`
       - Block `i` (`1 <= i < N`): `cmid = format!("cowboy-steer-block:{steer_cmid}:{i}")` (reserved prefix, idempotent `ON CONFLICT DO NOTHING` / unique index so replay never duplicates block 0 or drops attachments!).
    4. If `ClearEvents` already committed (`cleared_at_ms IS NOT NULL` or `context_generation` advanced): updates settlement metadata **without inserting any event row** into the replacement context!
  - Only AFTER this atomic transaction commits: project the committed `N` events into Hub memory (under `runtime_projection_guard`, re-verifying `session.context_generation`), emit `OriginSteerChanged`, and send `Frame::Ack` advancing `highwaters`.
- **Clear conversation (`src/core/clear_conversation.rs`):**
  - Inside the durable `StoreWrite::ClearEvents` transaction:
    1. Increment `sessions.context_generation = context_generation + 1`.
    2. Scrub payloads and hide from UI **WITHOUT manufacturing `original_turn_ended_at_ms` or falsifying `status`** (so unresolved receipts remain visible to the rollback drain query!):
       ```sql
       UPDATE origin_steer_receipts
       SET text = '',
           content_json = '[]',
           cleared_at_ms = ?now_ms,
           dismissed_at_ms = COALESCE(dismissed_at_ms, ?now_ms),
           updated_at_ms = ?now_ms
       WHERE session_id = ? AND cleared_at_ms IS NULL;
       ```
  - `finish_clear_conversation` advances in-memory `session.context_generation` and emits `OriginSteerChanged`.

### 7.5 Atomic `send-next` (Detached, Pending-Write Fenced, Collision-Checked & Preserving `logical_delivery_key`)

- **Reserved internal namespaces (`components/store/src/agent_model.rs:88-95`):** reserve BOTH `cowboy-origin-steer-resend:` and `cowboy-steer-block:` at all client HTTP, WebSocket, and Agent API ingress points.
- **Collision-checked resend ID (safe against legacy pre-upgrade queue rows):**
  - Never skip enqueue merely because a legacy queue row happens to hold a matching deterministic ID!
  - Inside the atomic `send-next` transaction, mint a unique collision-checked internal ID `resend_cmid = format!("cowboy-origin-steer-resend:{cmid}:{}", short_uuid)` (verifying uniqueness against `sessions.queue` and `prompt_dispatches` inside the transaction). Return `already_queued` **ONLY** when `origin_steer_receipts.resend_state == 'enqueued'` and `origin_steer_receipts.resend_cmid` is durably populated!
- **`logical_delivery_key` lifecycle (`hub_queue.rs:279-305`, `prompt_tracking.rs:266-274`):**
  - Carry `logical_delivery_key: Option<String>` across `QueuedMessage -> DispatchReq -> InFlightPrompt -> UndeliveredPrompt requeue -> prompt_dispatches -> startup restore`.
  - Editing a queued resend row's content (`EditQueued`) **clears `logical_delivery_key`** (creating a new logical delivery); reordering (`QueueFirst`, `MoveQueued`) or recoverable `UndeliveredPrompt` requeue **preserves `logical_delivery_key`**.
- **Detached, concurrency-safe `send-next` transaction (`src/core/hub_queue.rs:259-279`, `src/core/hub_managed_calls/withdrawals.rs`):**
  1. Handler verifies session mutation permission AND `RequestCaller::Person`, then spawns a **detached `tokio::spawn`** task (independent of HTTP connection lifetime so a client disconnect mid-flight cannot abandon ACK reconciliation).
  2. Acquires `Hub::pending_commit_lock(session_id)`.
  3. Under `sessions.lock()` (preview only — zero Hub mutation yet): validates session, `!is_managed_child`, `!clear_pending`, reads `receipt`, validates `cleared_at_ms IS NULL`, `context_generation` match, `status`, and `original_turn_ended_at_ms IS NOT NULL` (if `not_confirmed`).
  4. **Installs Cowboy's existing `in_flight_pending_commits` fence** (`claims.in_flight_pending_commits.insert(session_id, DeferredPendingFlush::default())`, preventing ordinary `emit_pending` snapshots from racing/overwriting the acknowledged transaction!).
  5. Enqueues ONE acknowledged FIFO `StoreWrite::UpdatePending` with `origin_steer_resend_claim` that executes inside **one SQLite/Postgres transaction**:
     - Conditional `UPDATE origin_steer_receipts SET resend_cmid = ?, resend_state = 'enqueued', dismissed_at_ms = ?, updated_at_ms = ? WHERE session_id = ? AND cmid = ? AND context_generation = ? AND cleared_at_ms IS NULL AND resend_state = 'none' AND dismissed_at_ms IS NULL AND (status = 'not_delivered' OR (status = 'not_confirmed' AND original_turn_ended_at_ms IS NOT NULL))`.
     - **Only if `rows_affected == 1`**, reads current persisted `sessions.queue` inside the transaction, appends ONLY the single new `QueuedMessage` (never overwriting concurrent edits or resurrecting already-dispatched rows!), and writes `sessions.queue` in the **same transaction**.
     - If `rows_affected == 0`: rolls back without modifying `sessions.queue` and returns the current receipt state.
  6. Awaits FIFO store ACK:
     - On error/rollback: releases `in_flight_pending_commits` via `finish_in_flight_pending_commit` (flushing any deferred ordinary queue writes), leaves receipt un-dismissed and visible (`503`).
     - On `Ok(Committed)`: under `sessions.lock()`, appends **ONLY the single committed `added_message`** to the **current** Hub `s.queue` (preserving every concurrent drain or queue edit!), calls `finish_in_flight_pending_commit`, emits pending snapshot + `OriginSteerChanged`, and calls `hub.try_drain(session_id)`.

---

## 8. Authorization, HTTP/WS API & Unified Web Receipt Flow (W1)

### 8.1 Session Authorization Boundary (`client_ws.rs:269-285`)

- **Never** publish receipt text on unscoped `Outbound::SyncPatch`.
- Emit content-free **`Outbound::OriginSteerChanged { session_id: String }`** gated in `client_ws.rs` by `session_is_visible(hub, principal, &session_id)`.
- HTTP reads (`GET /api/sessions/{id}/delivery-options`, `GET /api/sessions/{id}/delivery/{operation}`) enforce `session_is_visible(hub, principal, &session_id)`.
- HTTP mutations (`POST /api/sessions/{id}/origin-steers/{cmid}/send-next`, `POST /api/sessions/{id}/origin-steers/{cmid}/dismiss`) enforce BOTH session mutation authorization (`require_session_mutable`) AND verified `RequestCaller::Person` (`403 owner_required`).

### 8.2 HTTP Contract

- `GET /api/sessions/{id}/delivery-options`: returns `steer: bool` (always `false` for Antigravity), `controls`, `policy`, `target`, plus:
  - `origin_steering: bool`
  - `context_generation: number` (monotonic session generation for stale-response fencing)
  - `origin_steer_receipts: OriginSteerReceiptView[]` (non-dismissed, non-cleared rows for current `context_generation`, with `{ cmid, status, reason, text, block_count, created_at_ms, updated_at_ms, original_turn_live, resend_cmid }`).
- `POST /api/sessions/{id}/prompt` (`delivery: "current_turn"` on `origin_steering` session): returns HTTP `200` immediately with `{ status: "pending", delivery: "origin_steer", cmid, context_generation, receipt }` (or `{ status: "not_delivered", delivery: "origin_steer", cmid, error }` on synchronous pre-admission refusal).
- `GET /api/sessions/{id}/delivery/{operation}`: looks up `origin_steer_receipts` by `(session_id, cmid = operation)` and returns `{ status: "pending" | "injected" | "not_delivered" | "not_confirmed", delivery: "origin_steer", receipt }` (`404` if never admitted).

### 8.3 Unified Web UI Flow (`web/src/`)

1. **`DeliveryPicker.tsx` & `QueuedDeliveryAction.tsx`:**
   - Fresh options read: `canSteer = Boolean(props.busy && target && (options.steer || (!props.queued && options.origin_steering)))`.
   - Before fresh read: `steerReady` includes `session.origin_steering === true` **only** when `!props.queued`.
   - Queued rows (`QueuedDeliveryAction.tsx`) **disable Steer for Antigravity in R1** (preventing Peer/Automation text promotion and remove-vs-drain duplication).
2. **Authoritative receipt handoff, unadmitted 404 recovery & stale refresh guard (`currentTurnDelivery.ts:79-81`, `store.ts`):**
   - `Composer` submits `POST /api/sessions/{id}/prompt` **once** with client-generated `operation` (`cmid`).
   - On `200` (`delivery === "origin_steer"`): hands `cmid` immediately to server-authoritative `originSteerReceipts[sessionId]` and removes it from the local optimistic outbox (never running the 35s polling loop that fabricates `unknown`, never reposting on reload/reconnect).
   - **Unadmitted recovery (404 handling):** if the POST loses its network response or the tab reloads with an unacknowledged local intent, Web enters **observation-only** reconciliation via `GET /api/sessions/{id}/delivery/{operation}` (never auto-reposting `POST /prompt`!).
     - While the initiating POST promise is still in flight in the same tab: stay observation-only (`pending`).
     - Once the POST has terminated without a response (or on tab reload after local intent) AND `GET /delivery/{operation}` returns `404` AND a fresh `GET /delivery-options` confirms absence of `cmid`: non-admission is proven -> clear the observation entry and **restore the message into an editable local Composer draft** with an explicit notice (*"Message was not sent — restored to draft"*), allowing the Owner to re-send deliberately.
   - **Stale refresh guard:** every `GET /api/sessions/{id}/delivery-options` request carries a monotonic per-session `fetchSeq`; responses with `fetchSeq < latestAppliedFetchSeq` OR `response.context_generation < knownContextGeneration` are discarded so an in-flight pre-Clear or pre-Dismiss fetch can never resurrect cleared or dismissed receipts!
   - **Liveness refresh:** Web refreshes `delivery-options` on session open, WS reconnect, `OriginSteerChanged { session_id }`, AND any session `Status` / `TurnEnded` transition while `originSteerReceipts[sessionId]` has a `not_confirmed` row with `original_turn_live === true`.
   - **Receipt card UX:** `pending` shows preview + "Waiting for current step to finish…"; `delivered` clears (Owner blocks are already in the transcript); `not_delivered` offers **Send as next message** (enabled) + **Dismiss**; `not_confirmed` warns that the original turn may still have received the message and offers **Send as next message** (disabled while `original_turn_live === true`, enabled once `false`) + **Dismiss**. Never auto-enqueues.

---

## 9. Release Sequencing, Bound Acceptance & Safe Rollback

### 9.1 Slice Boundaries, 5 Artifacts & Bound R1 Canary Acceptance

1. **Slice M1 (Wire 52/8 + Store migration + Broker + Worker + Controller R1):**
   - Lands on `main` together with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`).
   - Strict M1/P2 boundary: M1 commits do **NOT** modify `plugins/antigravity/` manifests unless signed coverage is already published (`just provider-release-coverage`).
   - Activate **Controller** first with `providers.antigravity.origin_steering = false` until Machine + Worker + Plugin acceptance passes.
   - **All 5 required Machine release artifacts** (built on Hawk per OVH runtime-only contract):
     - **Hawk Machine (4 distinct artifacts on the new retained worker generation, verified via `just session-deletion-production-conformance` per `tools/session_deletion_production_conformance.py:169-191`):**
       1. Old-side writer Machine release (`--old-writer`)
       2. New-side writer Machine release (`--new-writer`, distinct revision + distinct ELF hash)
       3. Reader-only fallback Machine release (`--reader-fallback`)
       4. Default-reader Machine release (`--default-reader`)
     - **OVH Machine (1 artifact, per `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Includes `subagents.test.mjs` `requestMs: 1000` fix. Verified to install & launch cleanly on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap.
4. **Bound R1 Acceptance Gate (before enabling `providers.antigravity.origin_steering = true`):**
   - `session_deletion_production_conformance.py` (protocol 1-2) proves retained-worker compatibility only, and `ovh-machine-host-release.sh` can retain an old worker. Therefore, before enabling live admission:
     1. Run independent pin-advance acceptance (`docs/machine-worker-pin-advance-runbook.md`) confirming new workers run on the target worker generation.
     2. Run a **protocol 52/8 R1 steering canary bound to the exact candidate Machine, Worker, and Antigravity Plugin `0.2.35` digests**, verifying: negotiated Machine protocol `== 52`, worker↔broker runtime protocol `== 8`, broker↔Controller runtime protocol `== 8`, exact `provider_generation_digest`, live Owner steer delivery, multi-block echo, and Stop-under-congestion before flipping `providers.antigravity.origin_steering = true`.

### 9.2 Safe Rollback Procedure

1. **Immediate mitigation (preferred):** set live config gate `providers.antigravity.origin_steering = false` under the serialized admission lock. Refuses all new steers immediately while preserving full R1 outcome settlement and durable replay ingestion.
2. **Precondition before rolling back Controller binary to pre-R1:** verify across **ALL** rows in `origin_steer_receipts` (including cleared and dismissed rows!) that **zero** rows satisfy:
   `status = 'pending' OR (status = 'not_confirmed' AND original_turn_ended_at_ms IS NULL)`
   (proving every pre-disable grant has reached a terminal state OR has genuine `TurnEnded(NativeReply)` / `WorkerExitedConfirmed` + replay watermark consumption).
3. **If drain cannot be proved:** keep the live kill switch off on R1, force-stop affected workers on the Machine to produce `WorkerExitedConfirmed`, or deploy a **rollback-safe Controller build** that refuses new admission while retaining the pre-ACK durable `origin_steer_receipts` outcome reader.

---

## 10. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify `cowboy-wire` has zero dependency on `cowboy-store`; test installing and launching P2 (`0.2.35`) under pre-R1 `PluginManifest` / `ProviderPackage` decoders.
- **Mixed-version, Welcome sanitization & provider-specific routing:** cover all 6 rows of §3.3, `Frame::Welcome` sanitization, `worker.rs` protocol re-evaluation after ACP init, missing `launch.provider` fail-closed at broker, and full Owner steer end-to-end through HTTP -> `RemoteRuntime` -> Broker -> Worker -> Proxy with legacy `supports_current_turn == false`!
- **Worker-driven continuation & attempt-generation ledger:**
  - Request arriving during attempt 1 cannot enter attempt 2 (`attempt_gen` bound atomically under `Ledger` lock at ingress).
  - Ordinary sandboxed session with active Cowboy MCP server (`SERVER_TURN`) passes `has_no_non_server_processes()` and steers cleanly, while an active attempt process or extension call blocks continuation -> trusted `not_delivered`.
  - Uncertain effect, failed `Facade.endTurn`, failed/timed-out `Subagents.finishAttempt`, sticky failure of an already-settled child, or non-`"ok"` aggregate -> zero Owner bytes leave worker -> trusted `not_delivered`.
  - Attempt 1 + attempt 2 share ONE 8-child subagent limit (`group.started` preserved across `finishAttempt`).
  - Ambiguous `writeAgentChecked` failure/timeout in `proxy.mjs` sends native cancel / `SIGKILL` and waits for `agy` exit before answering `session/prompt` #2; serialized `turn.ioChain` prevents concurrent stdout races.
  - Failed trusted worker unit-stdin write terminates unit and emits `TurnEnded(TransportError)` (keeping `send-next` fenced until `WorkerExitedConfirmed`).
- **ACK-calibrated TTL:** tests with artificially delayed DB reservation, delayed WS/broker queue, and stalled worker receiver proving zero uncharged time and fail-closed `not_delivered` when uncalibrated or expired.
- **Stop priority, local tunnel chunking, independent `CancelTurn` & complete predecessor fence:**
  - Blocked-write tests across `machine_transport.rs` (`control_overtake`), `machine_cli -> broker` local runtime tunnel, `broker -> worker` 16 KiB interleaved link, and `worker.rs` split reader/writer proving `Cancel` arrives promptly during an active multi-MiB blocked write!
  - Root Stop in `proxy.mjs` with a blocked 170s extension call in `facade.mjs` forwarding `session/cancel` to `agy` immediately BEFORE awaiting cleanup.
  - Targeted `CancelTurn` succeeding while an R1 steer is pending for the 10-minute TTL (`cancel_turn_waiters` independent of steer slot).
  - Predecessor prompt fence (`dispatch_seq` / `cancel_watermark`) tested at Controller preflight (`background_jobs/dispatch.rs`), `RemoteRuntime.pending`, broker lifecycle + `pending_commands` + worker writer, worker, and proxy `RESERVED` slot — while preserving later `dispatch_seq > cancel_watermark` prompts and exact `CancelTurn` targets.
- **Receipts, atomic echo, durable outcome journal, Clear & `send-next`:**
  - Single-transaction admission (`origin_steer_receipts` + `prompt_dispatches` + `payload_sha256`); receipt-owned IDs excluded from generic `pending_echoes` hydration and `UndeliveredPrompt` requeue.
  - Single-transaction `"sent"` settlement + all `N` Owner echo blocks (`cmid` + `cowboy-steer-block:<cmid>:<i>`), idempotent on replay; Clear racing `"sent"` inserts zero echo rows into the new context while preserving unresolved settlement rows for rollback drain!
  - Dead/force-stopped worker replays durable `origin-steer-outcomes-<session_id>.json` before `WorkerExitedConfirmed`; `TurnEnded(TransportError)` stays fenced while `TurnEnded(NativeReply)` and `WorkerExitedConfirmed` (after watermark consumption) unlock `send-next`.
  - Detached `send-next` with `in_flight_pending_commits` concurrent with live queue drain and edits: preserves concurrent edits, never resurrects dispatched rows, handles legacy `cowboy-origin-steer-resend:` collision cleanly, carries `logical_delivery_key` through requeue and clears it on `EditQueued`.
  - Shared Rust/JS whitespace & slash predicate vector test including `U+0085` and `U+FEFF` before `/` and after `/compact`.
  - Authorization tests (`OriginSteerChanged` filtered by `session_is_visible`; `send-next`/`dismiss` require `require_session_mutable` + `RequestCaller::Person`).
  - Web (W1) tests: `DeliveryPicker`, queued Steer disabled for Antigravity, single-POST handoff, unadmitted `404` -> editable draft restoration, `fetchSeq` + `context_generation` stale-refresh fencing, and `original_turn_live` unlock refresh.
