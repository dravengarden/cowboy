# Antigravity Wrap-Up Steering (R1) — Queue-Backed Design (Revision 7)

Status: design, revision 7 (2026-11-24). Scoped strictly to **R1 (Owner wrap-up steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), queued-row Steer promotion for Antigravity, and unrelated `RemoteRuntime::send_pending` / `reset_admission` scheduler rewrites.

---

## 1. Context, Architectural Choice & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority. Any steering mechanism on `agy` 1.5.14 must wait for the active `session/prompt` to finish before dispatching the follow-up `session/prompt`.
- **Queue-Backed Wrap-Up Steering:**
  1. **Authoritative delivery reuses Cowboy's exact existing C-auth front-of-queue `sessions.queue` admission** (`front = true`, `DeliveryOrigin::Owner`, same write-behind queue persistence, `cmid` deduplication while queued/dispatched/echoed, multi-block echoes, `Edit`/`Delete`/`Clear`/`Retry`, and `UndeliveredPrompt` semantics as existing `Inbound::Submit { front: true }` and HTTP `front: true`). Zero new database tables, zero custom persistence transactions, zero parallel receipt state machines, and zero WebSocket-loop blocking.
  2. **Stateless, payload-free, target-fenced `WrapUpHint`:** when an Owner front-queues a wrap-up steer against a captured live, non-paused, non-editing, non-compacting Antigravity turn, the Controller sends a best-effort, **zero-owner-text** `WrapUpHint` over bounded/coalesced drop-on-full paths at every hop, targeted to exact `(worker_epoch, turn_id)` -> `attemptNonce`. The proxy attaches at most one fixed Cowboy-authored advisory note to the next root facade tool result asking the model to finish its current step briefly.
  3. **Normal clean turn boundary:** when turn 1 finishes, Cowboy runs ordinary `Ledger::finish_turn`, `Facade.endTurn`, and `Subagents.finish` cleanly, then `Hub::try_drain` immediately dispatches the front-queued Owner prompt as turn 2.
  4. **Proxy-local root Stop latency fix:** bundled into P2 `proxy.mjs` so root `session/cancel` bypasses `this.queue` synchronously, cancels unwritten `RESERVED` prompts cleanly while releasing the session slot, shares one per-attempt cleanup barrier with prompt completion, and immediately forwards `session/cancel` to `agy` before awaiting cleanup.
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because `WrapUpHint` carries zero owner text and never controls queue admission or transcript echoes, an untrusted proxy can neither forge delivery nor inject Owner text.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), queued-row Steer for Antigravity (§5), and Controller/broker `RemoteRuntime::send_pending` / `reset_admission` changes.

---

## 2. Wire Contract (Protocol 52 / Runtime 8 — Strictly Additive)

Zero database migrations. `cowboy-wire` remains a leaf crate with zero dependency on `cowboy-store`.

- **Leaf `DeliveryTarget` (`components/wire`):** move/define `DeliveryTarget { worker_epoch: String, turn_id: String }` in `components/wire` (re-exported by `src/send_delivery.rs`) so both `CoreCommand::WrapUpHint` and `WorkerCommand::WrapUpHint` use the leaf wire type directly.
- **Machine protocol:** bump `PROTOCOL_VERSION` to **52** (`WRAP_UP_STEERING_PROTOCOL_VERSION = 52`), enforced at both WS tunnel endpoints (`machine_cli.rs` and `machine_connection.rs`) AND bound into broker IPC `Frame::Hello` via optional `#[serde(default)] outer_machine_protocol: Option<u32>` (set by `machine_cli.rs` after WS handshake; `None` from older peers fails closed as `< 52`).
- **Runtime protocol (worker↔broker and broker↔Controller):** bump `PROTOCOL_VERSION` to **8** (`WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION = 8`).
- **Additive wire additions (serde-defaulted for mixed-version compatibility):**
  - `WorkerSnapshot`:
    - `wrap_up_steering: bool` (`#[serde(default)]`) — `true` only for Antigravity when all §3 gates hold; `false` for all other providers.
  - `CoreCommand::WrapUpHint` (Machine >= 52, runtime >= 8):
    `WrapUpHint { session_id: String, target: DeliveryTarget }`
  - `WorkerCommand::WrapUpHint` (runtime >= 8):
    `WrapUpHint { session_id: String, target: DeliveryTarget }`
- **End-to-end bounded & coalesced drop-on-full hint path (outside `pending`, replay, overflow refusal, `CommandAck`, and unbounded mailboxes):**
  1. **Controller `RemoteRuntime::connected` (`src/remote_runtime.rs:1755`):** owns a bounded per-connection `hint_tx: mpsc::Sender<CoreCommand>` (cap 32), installed on `Shared` only when connected with Machine >= 52 and runtime >= 8 and **dropped on disconnect**. `connected` polls `hint_rx` strictly below reliable commands (`biased;`) and writes through the sole framed writer; `try_send` silently drops on full/disconnect.
  2. **Controller WS tunnel (`machine_connection.rs`):** drops `WrapUpHint` if the outbound WS queue exceeds its bounded hint water mark (never delaying reliable commands or heartbeats).
  3. **Machine CLI local tunnel (`machine_cli.rs:1579`):** routes incoming `CoreCommand::WrapUpHint` into a separate bounded `hint_runtime_tx` (cap 32, `try_send` drop-on-full) instead of unbounded `runtime_write_tx`; `write_runtime_frames` polls `runtime_write_rx` with `biased;` ahead of `hint_runtime_rx`.
  4. **Broker (`machine_broker.rs:4337`) & Worker (`worker.rs`):** handle `WrapUpHint` on an explicit silent-drop path outside `pending_commands`, overflow refusal, replay, and `CommandAck` (bounded `try_send`, cap 8 per worker; never drops the Controller connection and never emits `CommandAck`).
  5. **Worker-to-ACP coalesced 1-slot handoff (`worker.rs` -> `src/acp.rs`):** `worker.rs` NEVER pushes `WrapUpHint` into the unbounded `cmd_tx` ACP command mailbox! Instead, `worker.rs` writes the latest target into a shared **1-slot `Arc<Mutex<Option<DeliveryTarget>>>` + `Arc<tokio::sync::Notify>`** (`pending_wrap_up_hint`) overwriting any unconsumed hint in O(1). In `src/acp.rs`, `tokio::select! { biased; cmd = cmd_rx.recv() => ..., () = wrap_up_notify.notified() => ... }` ensures `Cancel` and other reliable commands ALWAYS preempt hints even when the ACP consumer is stalled!

---

## 3. Provider-Identity, Compatible Signed Binding & Mixed-Version Gate

### 3.1 Compatible Signed Binding (Zero Unknown Manifest Fields)

`PluginManifest` (`components/plugin-sdk/src/lib.rs:72-83`) and `ProviderPackage` / `RuntimeCapability` (`components/provider-sdk/src/lib.rs:738-739`) use `#[serde(deny_unknown_fields)]`. To keep P2 packages installable and launchable on older/rollback Machines:

- **No new fields or enum variants** are added to `PluginManifest`, `ProviderPackage`, or `RuntimeCapability`.
- **Signed launch binding:** P2 ships on the existing signed manifest surface as Antigravity Plugin **`version = "0.2.35"`** (`cowboy-antigravity-acp` `1.0.27`). Every signed Antigravity release `>= 0.2.35` MUST preserve dual metadata stripping below. At worker launch, the broker verifies the signed installed release and supplies authoritative `launch.provider`, `launch.provider_version`, and `launch.provider_generation_digest` to the worker.
- **Single ACP `initialize` metadata location & dual vendor-flag stripping:**
  - `src/acp.rs` reads top-level `init.meta` (`result._meta`, matching `src/acp.rs:3837`), NOT `agentCapabilities._meta`.
  - P2 `proxy.mjs` always deletes **both** upstream `result._meta.steering` AND upstream `result._meta["cowboy.dev/wrapUpSteering"]` before stamping (only when `!this.managed`):
    ```json
    "result": {
      "_meta": {
        "cowboy.dev/wrapUpSteering": {
          "v": 1,
          "adapter": "cowboy-antigravity-acp"
        }
      }
    }
    ```
- **Worker-side capability computation & dynamic re-evaluation (`src/acp.rs`, `src/worker.rs`):**
  - For `provider_id == "antigravity"`:
    1. **Always hard-force** `supports_current_turn = false` and `supports_next_checkpoint = false`, regardless of upstream `_meta.steering`.
    2. Compute `raw_wrap_up_eligible = true` **iff ALL FOUR hold**:
       - `launch.provider == "antigravity"`
       - `!state.sink.session_is_managed_read_only(&session_id)`
       - Broker-verified signed `launch.provider_version` parses as semver `>= 0.2.35` AND `launch.provider_generation_digest` is present (non-empty)
       - Top-level `init.meta["cowboy.dev/wrapUpSteering"]` has `v == 1` and `adapter == "cowboy-antigravity-acp"`.
    3. Compute `effective_wrap_up_steering = raw_wrap_up_eligible && peer_protocol.load() >= 8`.
  - `worker.rs` stores `raw_wrap_up_eligible` on `Shared` and recomputes `effective_wrap_up_steering` on every `Frame::Welcome` and reconnect, emitting refreshed `Frame::Snapshot` + `CURRENT_TURN_CAPABILITIES_UPDATE` (`"currentTurn": false`, `"nextCheckpoint": false`, `"wrapUpSteering": bool`).

### 3.2 Handshake-Aware Sanitization & Host-Operated Kill Switch

All capability surfaces — **`Frame::Welcome` initial snapshots**, **`Frame::Snapshot`**, **cached worker state**, and **`Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)`** — are sanitized using the **negotiated `(outer_machine_protocol, runtime_protocol)` of the exact connection being installed** (never reading stale pre-install `Shared.protocol` or pre-install Controller leases):

- **Broker (`machine_broker.rs:298,:664`):**
  - Stores `negotiated_protocol: u32` inside `WorkerPeer` and retains raw worker capability observations (`raw_wrap_up_steering`, `raw_supports_current_turn`, `raw_next_checkpoint`) separately from projected values.
  - Reads `outer_machine_protocol` from the incoming Controller IPC `Frame::Hello` (failing closed as `0` when absent) together with the negotiated IPC `runtime_protocol`, and passes BOTH explicitly when building `Frame::Welcome` snapshots (before installing the Controller lease).
  - Fails closed when `launch.provider` cannot be established (`wrap_up_steering = false`, `supports_current_turn = false`, `next_checkpoint = false`).
  - For `provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; projects `wrap_up_steering = raw_wrap_up_steering && worker.negotiated_protocol >= 8 && controller_runtime_protocol >= 8 && outer_machine_protocol >= 52`.
- **Machine WS tunnel (`machine_cli.rs`, `machine_connection.rs`):** enforces the outer Machine `>= 52` gate as defense in depth (clamping `wrap_up_steering = false` and never transmitting `WrapUpHint` on `< 52` tunnels).
- **Controller (`remote_runtime.rs:464,:1732`, `src/core/`, `src/config/schema.rs`):**
  - Passes the incoming connection's negotiated `(machine_protocol, runtime_protocol)` explicitly into initial `Frame::Welcome` snapshot adoption and preserves raw worker observations so reconnects and config reloads recompute effective projections accurately.
  - **Host-operated Service kill switch (`providers.antigravity.wrap_up_steering`, boolean, default `true`):** intentionally host-operated via Service TOML (not an agent-mutable MCP control) because it governs host-level Antigravity rollout policy; omitted from Service TOML unless overridden by the host operator (see §6.2 for mandatory TOML removal before Controller rollback). Authorized callers and agents inspect its effective per-session state via `GET /api/sessions/{id}/delivery-options` (`wrap_up_steering`) and `SessionInfo.wrap_up_steering`.
  - For `session.meta.provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and `providers.antigravity.wrap_up_steering == true`.

### 3.3 Mixed-Version Rollout Matrix (Keyed to Verified Running Launch)

Existing sessions retain their active launch digest until idle adoption (`session_provider_updates.rs:1`), so matrix rows apply to each session's **verified running launch**:

| Controller | Machine (outer) | Runtime (inner) | Running Worker | Running Antigravity Launch | `supports_current_turn` / `next_checkpoint` | `wrap_up_steering` |
|---|---|---|---|---|---|---|
| Pre-R1 | Any | Any | Pre-8 (retained) | Pre-P2 (`< 0.2.35`) | Unchanged pre-R1 behavior (requires R1 worker OR P2 launch to strip upstream `_meta.steering`) | N/A |
| Pre-R1 | Any | Any | Any | Verified P2 (`>= 0.2.35`, strips `_meta.steering`) | `false` / `false` | N/A |
| R1 | 51 | 8 | 8 | Verified P2 (`0.2.35`) | `false` / `false` | `false` (outer Machine < 52) |
| R1 | 52 | 7 | 7 (retained) | Verified P2 (`0.2.35`) | `false` / `false` | `false` (inner runtime < 8) |
| R1 | 52 | 8 | 8 (R1 clamp) | Pre-P2 (`< 0.2.35`, even if upstream spoofs flags) | `false` / `false` | `false` (version < 0.2.35) |
| R1 | 52 | 8 | 8 | Verified P2 (`0.2.35`), config OFF | `false` / `false` | `false` |
| R1 | 52 | 8 | 8 | Verified P2 (`0.2.35`), config ON | `false` / `false` | `true` (Owner only) |

---

## 4. Queue-Backed Admission, Compaction Alignment, Hint & Proxy Root Stop Fix

### 4.1 Explicit Hub Admission Outcome & Caller Routing

- **Explicit Hub front-submit outcome (`src/core/hub_queue.rs:683`):**
  - Add `Hub::front_submit_with_outcome(&self, session_id, text, content, cmid, caller) -> FrontSubmitOutcome` sharing the exact existing `force_submit` queue admission logic (same write-behind `emit_pending`, same `prompt_already_accepted` deduplication while queued/dispatched/echoed, same synchronous non-blocking execution) and returning:
    ```rust
    pub enum FrontSubmitOutcome {
        Duplicate,
        DispatchedImmediately,
        QueuedFront,
    }
    ```
    preserving legacy `force_submit_once` unchanged for `Interrupt` callers.
  - **Explicit durability & idempotency contract:** inherits Cowboy's exact existing `sessions.queue` write-behind durability and `cmid` deduplication contract (deduplicated while queued, in-flight, or present in transcript; if the Owner explicitly withdraws/deletes a queued row before a client retries, a later retry is treated as a fresh admission, while target pinning below prevents stale retries from hinting a later turn).
- **Native mid-turn delivery (`delivery: "current_turn"` / `"next_checkpoint"`):**
  - Remains strictly bound to `supports_current_turn == true` (refused cleanly on Antigravity: `not_delivered`).
- **HTTP `POST /api/sessions/{id}/prompt` (`SendDelivery::WrapUp` = `"wrap_up"`):**
  - Mutually exclusive with `force: true` and `front: true` in `SendDelivery::resolve`.
  - **Owner-only:** requires `matches!(caller, RequestCaller::Person(_))`; `Agent`, `Automation`, and `Cowboy` receive `403 Forbidden`.
  - Runs all ordinary `api_session_prompt` checks (`!pending_context`, `!plugin_lifecycle_fence`, `!trashed`, `!managed_child`, non-empty `build_prompt_blocks`).
  - **Authored `operation` validation:** if `req.operation` is supplied, validates length `<= 128`, ASCII `[A-Za-z0-9._:-]`, and `Hub::is_authored_cmid` (`hub_queue.rs:715`, rejecting `__*`, `cowboy-*`, `call-notice:*`, `managed-call:*`, `session-notice:*`); if omitted, mints `api:<uuid>` (consistent with `session_api.rs:351`).
  - **Pre-enqueue target capture:** uses `req.target` if supplied, otherwise samples `supervisor.delivery_target(&session_id)` **before** calling `front_submit_with_outcome` (never after!).
  - Returns `202 Accepted` JSON `{ status: "queued" | "dispatched" | "already_accepted", delivery: "wrap_up", hinted: bool }` (firing §4.2 hint ONLY on `FrontSubmitOutcome::QueuedFront`).
- **WebSocket `Inbound::Submit` (`src/core.rs:904`, `client_ws.rs:707`):**
  - Keeps existing `Inbound::Submit` tag with two optional serde-defaulted fields: `#[serde(default)] wrap_up: bool` and `#[serde(default)] wrap_up_target: Option<DeliveryTarget>`.
  - **Rollback-safe encoding:** Web includes `wrap_up: true` and `wrap_up_target` ONLY when `session.wrap_up_steering === true`; otherwise (including rollback to a pre-R1 Controller or when config/worker disables hints) Web sends the exact same `cmid` as plain `Inbound::Submit { front: true }` (omitting `wrap_up` and `wrap_up_target`), with **identical** queue durability and zero decoder errors on pre-R1 Controllers!
  - **Strict WS admission guards:**
    - Rejects `wrap_up: true && force: true` BEFORE either dispatch path (`cmid`-addressed refusal).
    - Requires a validated authored `cmid` (`Hub::is_authored_cmid`) when `wrap_up: true`.
    - Routes through `Submit`'s full session authorization, system/context checks, and Plugin lifecycle fence, plus explicit credential-derived `caller.is_person()` check returning a `cmid`-addressed `CommandResult` refusal for non-Owners.
    - **Strict persisted target requirement for WS hints:** `WrapUpHint` fires on WS `Submit` ONLY when `wrap_up_target` is `Some(target)` (persisted at gesture time); a missing `wrap_up_target` is treated as **queue-only** (`hinted: false`), so a delayed or replayed targetless Submit can never hint a later turn.
- **Agent API & CLI (`agent_api/control.rs:788,:1046`, `cli/src/product_cli/session_prompt.rs:70`):**
  - `agent_api/control.rs:788`: explicitly reject `SendDelivery::WrapUp` alongside `CurrentTurn | NextCheckpoint` (`Err("invalid_argument")`).
  - `agent_api/control.rs:1046` (`delivery == "session_default"`): when `info.meta.provider == "antigravity"`, map `session_default` to `"next_turn"` (preserving existing `Steer -> current_turn` mapping for Claude/Codex).
  - `cli/src/product_cli/session_prompt.rs:70`: for `SendDelivery::WrapUp`, omit `target` (`Value::Null`) by default (with optional `--target` for explicit pinning) so the idle dispatch path works seamlessly and `--operation` retries stay clean.

### 4.2 Target-Fenced `WrapUpHint`, Nonce Transport & Turn-Bound Facade Delivery

Controller attempts `supervisor.send_wrap_up_hint(&session_id, captured_target)` **only when ALL hold**:
- Admission returned `FrontSubmitOutcome::QueuedFront`
- `captured_target` is `Some(target)` AND `supervisor.delivery_target(&session_id).as_ref() == Some(&target)`
- `info.meta.provider == "antigravity"` AND `info.meta.wrap_up_steering == true` AND live config `providers.antigravity.wrap_up_steering == true`
- `info.meta.status == Status::Busy` AND `!info.meta.paused` AND `s.editing.is_none()` (`hub_queue.rs:197`) AND `!hub.clear_pending(&session_id)` AND `hub.delivery_hold(&session_id).is_none()`
- `!hub.is_compacting(&session_id)` AND `!hub.active_turn_is_compacting(&session_id)`.

Across hops:
1. **Worker prompt nonce & exact `turn_id` (`src/worker.rs:992,:1086`, `src/acp.rs`):**
   - `worker.rs` carries the exact runtime `turn_id` published in `TurnStarted` inside `AgentCommand::Prompt` into `ActivePrompt { turn_id, attempt_nonce, compacting, ... }`.
   - For each Antigravity `session/prompt` attempt (including retries), `src/acp.rs` generates a fresh random 128-bit hex `attempt_nonce` and injects it in `params._meta["cowboy.dev/attemptNonce"]`.
   - On coalesced `pending_wrap_up_hint` wakeup: `src/acp.rs` checks `effective_wrap_up_steering`, `!managed_read_only`, `worker_epoch == target.worker_epoch`, `active_prompt.turn_id == target.turn_id`, `!active_prompt.compacting`, `!cancelled`, and sends payload-free ACP notification `cowboy/wrapUpHint { sessionId: acp_id, attemptNonce: active_prompt.attempt_nonce }`.
2. **Proxy nonce extraction, stripping & turn-bound facade delivery (`proxy.mjs`, `facade.mjs:193,:299`):**
   - In `proxy.mjs:fromClient` on `session/prompt`: **synchronously** copies `params?._meta?.["cowboy.dev/attemptNonce"]` into the `RESERVED` turn object (`turn.attemptNonce`) BEFORE any `await`, and **strips `params._meta["cowboy.dev/attemptNonce"]`** (deleting `_meta` if empty) BEFORE forwarding `session/prompt` to `agy`.
   - Creates ONE shared `wrapUp = { pending: false, delivered: false }` latch inside the synchronous `RESERVED` turn object and passes that exact latch reference into `runtime.facade.beginTurn(sessionId, { wrapUp })`.
   - `cowboy/wrapUpHint` bypasses `this.queue` synchronously; accepted ONLY when `turn` exists for `sessionId`, `typeof attemptNonce === "string" && turn.attemptNonce === attemptNonce`, `!this.managed`, `!this.subagents.owns(sessionId)`, `!turn.compacting`, `!turn.cancelled`, `!turn.structured`, and `!turn.wrapUp.delivered` -> sets `turn.wrapUp.pending = true`.
   - Inside `facade.mjs:call(sessionId, name, args)`: **captures `const callTurn = session?.turn` at tool-call start**. After `this.onResult(call, result)` runs (keeping Cowboy UI presentation clean), checks ALL of:
     - `session.parent === undefined` (root session only) AND `!this.managed`
     - `callTurn !== undefined && session.turn === callTurn`
     - `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`
     - `callTurn.wrapUp?.pending === true && callTurn.wrapUp.delivered === false`
     - `name !== "UpdatePlan"` AND `!result?.isError` AND `Array.isArray(result?.content)`.
   - Atomically sets `callTurn.wrapUp.pending = false; callTurn.wrapUp.delivered = true;` (strictly once per turn) and returns a **non-mutating copy** attaching `WRAP_UP_NOTE = "[Cowboy note: the owner asked you to wrap up your current step briefly so they can follow up next.]"`:
     - **Existing-text-block attachment (handles `[text, image]` `ReadFile` results):** finds the **first** block in `result.content` with `type === "text"` and clones `result.content` with `"\n\n" + WRAP_UP_NOTE` appended to that existing text block; ONLY if `result.content` contains no text block at all, appends `{ type: "text", text: WRAP_UP_NOTE }`. Update `testdata/fake-agy.mjs:75` to consume all content blocks and verify both plain text and `[text, image]` results in the real `agy` canary (§6.1).

### 4.3 Shared Compaction Fence & Context Carrier/Preamble Alignment (`COWBOY_WS`)

- **Closed code-point set `COWBOY_WS`** implemented identically in Rust (`is_cowboy_ws`) and JS (`compaction.mjs:isCowboyWs`), explicitly excluding `U+0085` and including `U+FEFF`:
  `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF`.
- **`is_compaction_prompt(blocks)`:** applies `build_prompt_blocks(text, content)` first; returns `true` iff non-empty, every block is `type == "text"`, and `COWBOY_WS`-trimmed concatenation matches ASCII case-insensitive `/compact` followed by EOF or `COWBOY_WS`.
- **Preflight context carrier & preamble alignment (`background_jobs/dispatch.rs:123-130`, `src/session_context.rs:258,:469`):**
  - Classify the **actual normalized prompt blocks** (`build_prompt_blocks(&text, &content)`) with the shared `COWBOY_WS` command/compaction detector inside `prepare_dispatch_preflight` **BEFORE** both `session_context::rollover::before_dispatch` and `session_context::claim_carrier`, AND inside `session_context::prompt_content` — so content-only, `U+FEFF`, or split-block `/compact` prompts never claim pending context links or receive prepended `<system-reminder>` blocks, leaving links pending for the next ordinary prompt.
- **Turn-scoped latch:** Controller latches `active_turn_compacting` on dispatch; Worker latches `active_prompt.compacting`; Proxy computes `turn.compacting` synchronously inside `fromClient` when reserving the `RESERVED` turn object before any `await`.

### 4.4 Proxy-Local Root Stop Bypass & Per-Attempt Cleanup Barrier

1. **Synchronous Stop bypass, single reservation & slot release (`proxy.mjs:560,:653,:682`):**
   - **`session/cancel` (for valid root and child sessions) AND `cowboy/wrapUpHint` bypass `this.queue` synchronously in `fromClient`** (NOT chained onto `this.queue = this.queue.then(...)`).
   - `session/prompt` synchronously creates ONE session-scoped `turn` reservation object in `this.turns` (`state: "RESERVED"`, holding `id`, `record`, `attemptNonce`, `cancelGen`, `compacting`, `wrapUp`, `cancelled: false`, `attemptCleanup: null`, and a `cancelPromise`/`cancelResolve` pair) and **populates THAT SAME object** through `await Promise.race([evidence.mark(), turn.cancelPromise])` and `await Promise.race([recall(...), turn.cancelPromise])` (never overwriting `this.turns` with a fresh object).
   - If `session/cancel` arrives while `turn.state === "RESERVED"` (same stdin chunk OR while `evidence.mark()` / `recall()` is blocked):
     1. Marks `turn.cancelled = true`, increments `this.cancelGen`, and calls `turn.cancelResolve()` (immediately unblocking the raced `this.queue` handler so the serialized preparation slot is released for the next prompt!).
     2. **Removes that exact reservation from `this.turns` (`if (this.turns.get(sessionId) === turn) this.turns.delete(sessionId)`) BEFORE calling `this.endPrompt(record, { jsonrpc: "2.0", id: record.id, result: { stopReason: "cancelled" } })`** (guarded by `record.finalized = true`)!
   - When the raced preparation handler resumes: checks `turn.cancelled || record.finalized || this.turns.get(sessionId) !== turn || this.cancelGen.get(sessionId) !== turn.cancelGen || this.killed` and returns immediately — guaranteeing the old continuation can neither write to `agy` nor remove a newer turn's reservation!
2. **Per-attempt cleanup barrier (`proxy.mjs:756-781,:1152`, `facade.mjs:212,:277`):**
   - Scope `attemptCleanup` **per native attempt** (resetting `turn.attemptCleanup = null` whenever `facade.beginTurn(...)` opens a fresh attempt — including inside `proxy.correct()` — so managed correction attempts never inherit attempt 1's completed cleanup barrier).
   - `startAttemptCleanup(sessionId, turn, { cancelled })` synchronously fences `facade` + `subagents` admission and memoizes `turn.attemptCleanup = Promise.all([facade.endTurn(...), subagents.finish(...)])`. `facade.mjs` tracks active `endTurn` promises even after `session.turn` is detached so `facade.settled()` awaits them on shutdown.
   - Root `cancel()` on an `ACTIVE` turn calls `startAttemptCleanup(sessionId, turn, { cancelled: true })` synchronously (using cached `this.readyRuntime`), **immediately forwards `this.agent(message)` (`session/cancel`) to `agy`**, and then awaits `turn.attemptCleanup`. `fromAgent`'s prompt reply joins that exact same `turn.attemptCleanup`.

---

## 5. HTTP/WS API & Web UI Flow (W1)

- **`GET /api/sessions/{id}/delivery-options` (`send_delivery.rs`):**
  - Exposes `"steer": info.meta.supports_current_turn` (always `false` for Antigravity), `"wrap_up_steering": info.meta.wrap_up_steering`, `"editing": hub.is_editing(&session)`, plus existing `target`, `controls` (with `next_checkpoint: false` for Antigravity), `status`, `paused`, `hold`, `policy`.
- **Web (`web/src/protocol.ts`, `DeliveryPicker.tsx`, `Composer.tsx`, `QueuedDeliveryAction.tsx`, `store.ts`):**
  - `SessionInfo` adds `wrap_up_steering?: boolean`.
  - `DeliveryPicker.tsx` (Composer only, `!props.queued`):
    - Enables **"Wrap up & send next"** (*"Asks agent to finish current step, then sends next"*) **only when ALL hold**: `!options.steer`, `props.busy`, `target != null`, `!props.paused`, `!options?.editing`, current context ready, and `(fresh ? Boolean(options?.wrap_up_steering) : Boolean(props.wrapUpReady))`. Preferred default when `policy.mode === "steer"`.
    - Calls `wrapUpPrompt(sessionId, text, attachments, target)`: invokes **`requireCurrentAgentContext(sessionId)`** ONLY (NEVER `sendWithContextNotice`, so a staged context handoff in another tab can never redirect a target-bound send to another session!), persists `(sessionId, target)` in the durable outbox entry (`mode: "front", wrapUp: true, wrapUpTarget: target`), and sends `wrap_up: true, wrap_up_target` when `session.wrap_up_steering === true` (falling back cleanly to plain `Inbound::Submit { front: true }` with the same `cmid` on legacy Controllers or when wrap-up hints are disabled).
  - `QueuedDeliveryAction.tsx` (`props.queued === true`): keeps Steer disabled when `!options.steer` (only `Queue first`, `Send now`, edit, and delete available — zero Peer/Automation origin promotion).

---

## 6. Release Sequencing, Pre-Admitted Hawk Recovery & Rollback Boundaries

### 6.1 Slice Sequencing & Buildable 5-Artifact Conformance Set

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller):**
   - Lands on `main` with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`). Strict M1/P2 boundary: M1 commits do **not** touch `plugins/antigravity/` manifests unless exact signed coverage is already published (`just provider-release-coverage`).
   - **Zero DB migrations.** Activate **Controller** first (keeping `providers.antigravity.wrap_up_steering` omitted from Service TOML so default `true` applies cleanly).
   - **Buildable 5-artifact Machine set (built on Hawk per OVH runtime-only contract):**
     - **Hawk Machine (4 inputs verified via `just session-deletion-production-conformance OLD NEW READER DEFAULT_READER RECEIPT`, corresponding to `--old-writer`, `--new-writer`, `--reader-release`, and `--default-reader-release` in `tools/session_deletion_production_conformance.py:171-174` and `flake.nix:1012`):**
       1. `--old-writer`: first post-wire-52/8 writer revision built against the newly accepted worker pin
       2. `--new-writer`: second distinct post-wire-52/8 writer revision (distinct commit + ELF hash) built against the **same** new worker pin
       3. `--reader-release`: matching reader-only fallback release on the **same** new worker pin
       4. `--default-reader-release`: separate default-reader startup test input (**kept strictly separate from rollback selection**)
       *(Record all 4 immutable store roots and a fresh conformance receipt, AND **pre-admit the exact same-pin `--reader-release` through Hawk's component-owner ancestry and reader-floor checks before cutover**, retaining its GC-rooted store path as the designated Hawk recovery target.)*
     - **OVH Machine (1 artifact, `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` `1.0.27` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Verified to install and launch on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap (enabled after the real `agy` canary below passes).
4. **Bound R1 Acceptance Canary (Real `agy` Verification):**
   - After pin-advance acceptance verifies target worker generation on Hawk and OVH, run a live canary on a real Antigravity session bound to exact candidate Controller, Machine, Worker, and Plugin `0.2.35` digests confirming: negotiated Machine protocol `52`, both runtime legs `8`, `supports_current_turn == false`, `delivery_controls.next_checkpoint == false`, `wrap_up_steering == true`, **real `agy` 1.5.14 observations proving the model consumed the wrap-up note on BOTH a plain text tool result AND an image-bearing `[text, image]` `ReadFile` result**, immediate drain of the front-queued Owner prompt after `TurnEnd`, and root Stop reaching `agy` immediately during an active facade/extension call.

### 6.2 Accurate Component Rollback Boundaries

- **Advisory hints (no drain precondition):** disabling new `WrapUpHint`s requires no receipt drain because hints are stateless and queued Owner messages live in existing `sessions.queue`.
- **Controller rollback (mandatory TOML cleanup):** because unknown Service TOML keys are fatal at startup (`src/config.rs:575,:769`), if an operator ever wrote `providers.antigravity.wrap_up_steering` into Service TOML, rollback **MUST** remove that key (and any empty `[providers.antigravity]` / `[providers]` tables) and validate the config file with the rollback Controller binary before activating rollback. Web outbox downgrade (§4.1) sends plain `Inbound::Submit { front: true }` on pre-R1 Controllers.
- **Hawk Machine rollback:** after worker-pin cutover, recovery MUST target the pre-admitted same-pin `--reader-release` store root (`docs/plugin-session-deletion-journal.md:81`), NOT the pre-pin Machine binary and NOT the `--default-reader-release` startup test artifact.
- **Worker & Plugin rollback:** governed by Cowboy's graceful turn-settlement boundaries (idle adoption / busy safe-boundary cutover; forced stops can leave in-flight replies uncertain per `docs/uncertain-effect-recovery.md:322`).

---

## 7. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify `DeliveryTarget` in `cowboy-wire` creates zero crate cycles; test P2 `0.2.35` install/launch on pre-R1 `PluginManifest` / `ProviderPackage` decoders.
- **Mixed-version, outer/inner protocol & rollback matrix:** cover all rows of §3.3 (including surviving retained workers across Controller rollback, `Machine 51 + Runtime 8`, and `Machine 52 + Runtime 7`), explicit `(outer_machine_protocol, runtime_protocol)` passing during `Frame::Welcome` adoption, `WorkerPeer.negotiated_protocol` retention, `delivery_controls.next_checkpoint == false` clamping, missing `launch.provider` fail-closed at broker, and config-reload recomputation from raw observations.
- **Queue-backed admission & caller routing:**
  - `FrontSubmitOutcome::QueuedFront` triggers hint while `DispatchedImmediately` / `Duplicate` do not; `Agent`/`Automation`/`Cowboy` rejected (`403` on HTTP, `cmid`-addressed refusal on WS).
  - HTTP `delivery: "wrap_up"`: validates authored `operation` when supplied; CLI omits target by default and works on both busy and idle sessions.
  - WS `Inbound::Submit`: `wrap_up && force` rejected before dispatch; missing/synthetic `cmid` rejected; missing `wrap_up_target` is queue-only (zero hint); rollback encoding omits `wrap_up` on legacy Controllers.
  - Agent API rejects `wrap_up` and maps Antigravity `session_default` -> `next_turn`.
- **End-to-end bounded & coalesced hint saturation + turn-bound facade note:**
  - Saturation tests across Controller `hint_tx`, WS writer, `machine_cli` `hint_runtime_tx` vs `runtime_write_tx`, broker, worker, AND **stalled worker ACP consumer** (`pending_wrap_up_hint` 1-slot coalescing) proving hint floods never block reliable commands, Stop, or heartbeats.
  - Worker injects `params._meta["cowboy.dev/attemptNonce"]` (fresh per attempt/retry) and carries exact `TurnStarted` `turn_id` into `ActivePrompt`; proxy copies nonce synchronously, strips it before `agy`, and drops delayed hints with mismatched nonces.
  - Facade: hint during `RESERVED` preserved into facade turn; slow untracked root read finishing after a newer turn cannot consume the newer turn's hint; note attaches to first text block (including `[text, image]` `ReadFile`), appends only if no text block exists, runs after `onResult` once per turn without mutating `result`; `testdata/fake-agy.mjs` reads all content blocks.
  - Shared `COWBOY_WS` compaction tests (`U+0085`, `U+FEFF`, split blocks, mixed blocks) including `prepare_dispatch_preflight` leaving pending context links unclaimed for split-block `/compact` through Worker and Proxy (including reconnect).
- **Proxy-local root Stop bypass & per-attempt cleanup barrier:**
  - `proxy.mjs`: `session/cancel` bypasses `this.queue`; cancel in same chunk AND cancel arriving while blocked in `await evidence.mark()` / `await recall()` immediately removes that exact reservation from `this.turns`, unblocks `this.queue`, and settles once so a subsequent prompt can start immediately while the old continuation can neither write to `agy` nor remove the new reservation; root Stop with blocked `facade.endTurn` forwards `session/cancel` immediately while `fromAgent` reply and `facade.settled()` join the same `turn.attemptCleanup`; Stop and natural completion AFTER `proxy.correct()` starts a correction attempt run fresh attempt-2 cleanup.
- **Web (W1):** `DeliveryPicker` gates **"Wrap up & send next"** on busy + live target + `!paused` + `!editing` + current context + `wrap_up_steering` (Composer only; queued rows keep Steer disabled for Antigravity); `wrapUpPrompt` calls `requireCurrentAgentContext` (never `sendWithContextNotice`), binds `(sessionId, target)` in outbox, downgrades cleanly on legacy Controllers, and passes `bun run typecheck && bun run lint`.
