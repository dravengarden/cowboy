# Antigravity Wrap-Up Steering (R1) — Queue-Backed Design (Revision 9)

Status: design, revision 9 (2026-11-24). Scoped strictly to **R1 (Owner wrap-up steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), queued-row Steer promotion for Antigravity, and unrelated `RemoteRuntime::send_pending` / `reset_admission` scheduler rewrites.

---

## 1. Context, Architectural Choice & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority. Any steering mechanism on `agy` 1.5.14 must wait for the active `session/prompt` to finish before dispatching the follow-up `session/prompt`.
- **Queue-Backed Wrap-Up Steering:**
  1. **Authoritative delivery reuses Cowboy's exact existing C-auth front-of-queue `sessions.queue` admission** (`front = true`, `DeliveryOrigin::Owner`, same write-behind queue persistence, `cmid` deduplication while queued/dispatched/echoed, multi-block echoes, `Edit`/`Delete`/`Clear`/`Retry`, and `UndeliveredPrompt` semantics as existing `Inbound::Submit { front: true }` and HTTP `front: true`). Zero new database tables, zero custom persistence transactions, zero parallel receipt state machines, and zero WebSocket-loop blocking.
  2. **Stateless, payload-free, target-fenced `WrapUpHint`:** when an Owner front-queues a substantive wrap-up steer with an explicit captured target against a live, non-paused, non-editing, non-compacting Antigravity turn, the Controller attempts a best-effort, **zero-owner-text** `WrapUpHint` over dedicated bounded, reliable-first channels at every hop (bypassing shared FIFOs), targeted to exact `(worker_epoch, turn_id)` -> `attemptNonce`. The proxy attaches at most one fixed Cowboy-authored advisory note to the next root facade tool result asking the model to finish its current step briefly.
  3. **Normal clean turn boundary:** when turn 1 finishes, Cowboy runs ordinary `Ledger::finish_turn`, `Facade.endTurn`, and `Subagents.finish` cleanly, then `Hub::try_drain` immediately dispatches the front-queued Owner prompt as turn 2.
  4. **Proxy-local root Stop latency fix:** bundled into P2 `proxy.mjs` so root `session/cancel` bypasses `this.queue` synchronously, cancels unwritten reservations in a separate `reservedTurns` map (without unsealing managed output or mutating compaction state), immediately forwards `session/cancel` to `agy`, and shares one per-attempt cleanup barrier with prompt completion that preserves the full 60-second root command-stop confirmation window.
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
- **End-to-end bounded, reliable-first drop-on-full hint path (zero shared FIFOs ahead of reliable writes):**
  - **Encoded size & write-deadline enforcement:** every serialized `WrapUpHint` frame is validated `<= MAX_WRAP_UP_HINT_BYTES` (512 bytes; dropped if larger) and each local/IPC hint write uses a short bounded write timeout (`250 ms`), bounding any mid-write residual delay to one small frame.
  1. **Controller direct-to-WS hint path (bypasses the shared runtime UnixStream bridge FIFO!):** `machine_connection.rs` passes a dedicated bounded per-connection `hint_ws_tx: mpsc::Sender<MachineFrame>` (cap 32, `try_send` drop-on-full, **dropped on disconnect**) directly to `RemoteRuntime` when installing a Machine >= 52 / runtime >= 8 connection. `CoreCommand::WrapUpHint` NEVER enters the shared runtime `UnixStream` bridge FIFO; `machine_connection.rs`'s final WS writer selects heartbeats and reliable frames with `biased;` ahead of `hint_ws_rx`.
  2. **Machine CLI local tunnel (`machine_cli.rs:1579`):** routes incoming `CoreCommand::WrapUpHint` into a separate bounded `hint_runtime_tx` (cap 32, `try_send` drop-on-full) instead of unbounded `runtime_write_tx`; `write_runtime_frames` selects `runtime_write_rx` with `biased;` ahead of `hint_runtime_rx`.
  3. **Broker-to-worker dedicated writer lane (`machine_broker.rs:4337`):** handles `WrapUpHint` outside `pending_commands`, overflow refusal, replay, and `CommandAck`; each worker's Unix socket writer task owns a separate bounded `worker_hint_rx` (cap 8, `try_send` drop-on-full) selected strictly BELOW reliable worker commands (`tokio::select! { biased; reliable = worker_cmd_rx.recv() => ..., hint = worker_hint_rx.recv() => ... }`)!
  4. **Worker-to-ACP coalesced 1-slot handoff & reliable-first ACP egress (`worker.rs` -> `src/acp.rs`):** `worker.rs` NEVER pushes `WrapUpHint` into the unbounded `cmd_tx` ACP command mailbox; it overwrites a single shared 1-slot `Arc<Mutex<Option<DeliveryTarget>>>` + `Arc<tokio::sync::Notify>` (`pending_wrap_up_hint`) in O(1). In `src/acp.rs`, the command loop selects `cmd_rx` with `biased;` ahead of `wrap_up_notify` AND gates handing `cowboy/wrapUpHint` to the ACP egress writer behind a 1-in-flight non-blocking guard (dropping/coalescing while a prior hint write is flushing, with reliable ACP requests/cancels always selected first at ACP egress).

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
  - **Host-operated Service kill switch (`providers.antigravity.wrap_up_steering`, boolean, default `true`):** intentionally host-operated via Service TOML (not an agent-mutable MCP control) because it governs host-level Antigravity rollout policy; omitted from Service TOML unless overridden by the host operator (see §6.2 for safe rollback handling). Authorized callers and agents inspect its effective per-session state via `GET /api/sessions/{id}/delivery-options` (`wrap_up_steering`) and `SessionInfo.wrap_up_steering`.
  - For `session.meta.provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and `providers.antigravity.wrap_up_steering == true`.

### 3.3 Mixed-Version Rollout Matrix (Keyed to Verified Running Launch)

Existing sessions retain their active launch digest until idle adoption (`session_provider_updates.rs:1`), so matrix rows apply to each session's **verified running launch**:

| Controller | Machine (outer) | Runtime (inner) | Running Worker | Running Antigravity Launch | `supports_current_turn` / `next_checkpoint` | `wrap_up_steering` |
|---|---|---|---|---|---|---|
| Pre-R1 | Any | Any | Pre-8 (retained) | Pre-P2 (`< 0.2.35`) | Unchanged pre-R1 behavior (requires R1 worker OR P2 launch to strip upstream `_meta.steering`) | N/A |
| Pre-R1 | Any | Any | Any | Verified P2 (`>= 0.2.35`, strips `_meta.steering`) | `false` / `false` | N/A |
| R1 | 51 | 8 | 8 | Verified P2 (`0.2.35`) | `false` / `false` | `false` (outer Machine < 52) |
| R1 | 52 | 7 | 7 (retained) | Verified P2 (`0.2.35`) | `false` / `false` | `false` (inner runtime < 8) |
| R1 | 52 | 8 | 8 (R1 clamp) | Pre-P2 (`< 0.2.35`, even if upstream spoofs flags) | `false` / `false` | `false` (version < 0.2.35; zero nonce injected) |
| R1 | 52 | 8 | 8 | Verified P2 (`0.2.35`), config OFF | `false` / `false` | `false` |
| R1 | 52 | 8 | 8 | Verified P2 (`0.2.35`), config ON | `false` / `false` | `true` (Owner only) |

---

## 4. Queue-Backed Admission, Compaction Alignment, Hint & Proxy Root Stop Fix

### 4.1 Exhaustive Hub Admission Outcome, Substantive Validation & Caller Routing

- **Substantive prompt validation (`has_substantive_prompt_content(blocks)`):**
  - Run `build_prompt_blocks(&text, &content)` and verify the normalized block list contains **actual substantive prompt content**: at least one non-text block (e.g. image/audio/resource) OR at least one text block containing a non-`COWBOY_WS` character! Reject empty or whitespace-only text blocks (even structurally valid `[{"type":"text","text":""}]`) BEFORE admission on BOTH HTTP `delivery: "wrap_up"` (`400 Bad Request`) and WS `Inbound::Submit { wrap_up: true }` (`cmid`-addressed `CommandResult` refusal, no queue insert, no hint).
- **Exhaustive Hub front-submit outcome (`src/core/hub_queue.rs:683`):**
  - Add `Hub::front_submit_with_outcome(&self, session_id, text, content, cmid, caller) -> FrontSubmitOutcome` sharing the exact existing `force_submit` queue admission logic and faithfully reporting every synchronous Hub outcome:
    ```rust
    pub enum FrontSubmitOutcome {
        Refused(&'static str), // session missing or managed child
        Duplicate,             // cmid already queued/dispatched/echoed
        DispatchedImmediately, // handed synchronously to dispatch channel (`try_send` succeeded)
        QueuedFront,           // inserted at front of queue (busy or non-empty queue)
        QueuedFallback,        // selected immediate dispatch, `send_dispatch` returned `Full` or `Closed` -> retained in queue
    }
    ```
    preserving legacy `force_submit_once` unchanged for `Interrupt` callers. Both `TrySendError::Full` AND `TrySendError::Closed` (during dispatcher shutdown) map to `QueuedFallback` after retaining the prompt in the queue!
  - **`hinted: bool` definition:** `true` strictly means Controller's bounded `hint_ws_tx.try_send(...)` accepted a local fire-and-forget enqueue (never a guarantee of remote delivery or model compliance). `QueuedFallback`, `DispatchedImmediately`, `Duplicate`, and `Refused` always report `hinted: false` and never claim `dispatched` for a still-queued row.
  - **Explicit durability & idempotency contract:** inherits Cowboy's exact existing `sessions.queue` write-behind durability and `cmid` deduplication contract (deduplicated while queued, in-flight, or present in transcript; if the Owner explicitly withdraws/deletes a queued row before a client retries, a later retry is treated as a fresh admission, while explicit target pinning below prevents stale retries from hinting a later turn).
- **Native mid-turn delivery (`delivery: "current_turn"` / `"next_checkpoint"`):**
  - Remains strictly bound to `supports_current_turn == true` (refused cleanly on Antigravity: `not_delivered`).
- **HTTP `POST /api/sessions/{id}/prompt` (`SendDelivery::WrapUp` = `"wrap_up"`):**
  - Mutually exclusive with `force: true` and `front: true` in `SendDelivery::resolve`.
  - **Owner-only:** requires `matches!(caller, RequestCaller::Person(_))`; `Agent`, `Automation`, and `Cowboy` receive `403 Forbidden`.
  - Runs all ordinary `api_session_prompt` checks (`!pending_context`, `!plugin_lifecycle_fence`, `!trashed`, `!managed_child`) plus `has_substantive_prompt_content`.
  - **Authored `operation` validation:** if `req.operation` is supplied, validates length `<= 128`, ASCII `[A-Za-z0-9._:-]`, and `Hub::is_authored_cmid` (`hub_queue.rs:715`, rejecting `__*`, `cowboy-*`, `call-notice:*`, `managed-call:*`, `session-notice:*`); if omitted, mints `api:<uuid>`.
  - **Strict explicit target requirement (same rule as WebSocket):** targetless HTTP `delivery: "wrap_up"` (`req.target == None`) is strictly **queue-only (`hinted: false`)** — NEVER sampling a fresh target at admission! Hints fire ONLY when the caller supplies an explicit reusable `req.target: Some(target)` matching the active turn.
  - Outcome mapping: `Refused(reason)` -> `404`/`409`; `Duplicate` -> `202 { status: "already_accepted", delivery: "wrap_up", hinted: false }`; `DispatchedImmediately` -> `202 { status: "dispatched", delivery: "wrap_up", hinted: false }`; `QueuedFallback` -> `202 { status: "queued", delivery: "wrap_up", hinted: false }`; `QueuedFront` -> attempts §4.2 hint if `req.target` is `Some(target)`, returns `202 { status: "queued", delivery: "wrap_up", hinted: <bool> }`.
- **WebSocket `Inbound::Submit` (`src/core.rs:904`, `client_ws.rs:707`):**
  - Keeps existing `Inbound::Submit` tag with optional serde-defaulted `#[serde(default)] wrap_up: bool` and `#[serde(default)] wrap_up_target: Option<DeliveryTarget>`.
  - **Rollback-safe encoding:** Web includes `wrap_up: true` and `wrap_up_target` ONLY when `session.wrap_up_steering === true`; otherwise sends the exact same `cmid` as plain `Inbound::Submit { front: true }` (omitting `wrap_up` and `wrap_up_target`), with identical queue durability and zero decoder errors on pre-R1 Controllers.
  - **Strict WS admission guards:** rejects `wrap_up && force` before dispatch; requires validated authored `cmid` (`Hub::is_authored_cmid`); runs `build_prompt_blocks` + `has_substantive_prompt_content` validation before admission; routes through `Submit`'s full session authorization, system/context checks, Plugin lifecycle fence, and explicit `caller.is_person()` check (returning `cmid`-addressed `CommandResult` refusal on any failure or `FrontSubmitOutcome::Refused`); fires hint ONLY on `QueuedFront` when `wrap_up_target` is `Some(target)` (missing `wrap_up_target` is queue-only).
- **Agent API & CLI (`agent_api/control.rs:788,:1046`, `cli/src/product_cli/session_prompt.rs:70`):**
  - `agent_api/control.rs:788`: explicitly reject `SendDelivery::WrapUp` alongside `CurrentTurn | NextCheckpoint` (`Err("invalid_argument")`).
  - `agent_api/control.rs:1046` (`delivery == "session_default"`): when `info.meta.provider == "antigravity"`, map `session_default` to `"next_turn"` (preserving existing `Steer -> current_turn` mapping for Claude/Codex).
  - `cli/src/product_cli/session_prompt.rs:70`: for `SendDelivery::WrapUp`, accept optional `--target <worker_epoch>:<turn_id>`: on a fresh invocation (no user-supplied `--operation` and no `--target`), query `delivery-options` best-effort, pass `options["target"]` (or `Value::Null` when idle) and print both `operation:` and `target:` to stderr; on a manual retry with `--operation <id>` and no `--target`, send `target: Value::Null` (queue-only) or reuse the printed `--target` explicitly.

### 4.2 Target-Fenced `WrapUpHint`, P2-Gated Nonce & Clean-Boundary Facade Delivery

Controller attempts `supervisor.send_wrap_up_hint(&session_id, target)` **only when ALL hold**:
- Admission returned `FrontSubmitOutcome::QueuedFront`
- Caller supplied `Some(target)` AND `supervisor.delivery_target(&session_id).as_ref() == Some(&target)`
- `info.meta.provider == "antigravity"` AND `info.meta.wrap_up_steering == true` AND live config `providers.antigravity.wrap_up_steering == true`
- `info.meta.status == Status::Busy` AND `!info.meta.paused` AND `s.editing.is_none()` (`hub_queue.rs:197`) AND `!hub.clear_pending(&session_id)` AND `hub.delivery_hold(&session_id).is_none()`
- `!hub.is_compacting(&session_id)` AND `!hub.active_turn_is_compacting(&session_id)`.

Across hops:
1. **P2-gated Worker prompt nonce & exact `turn_id` (`src/worker.rs:992,:1086`, `src/acp.rs`):**
   - `worker.rs` carries the exact runtime `turn_id` published in `TurnStarted` inside `AgentCommand::Prompt` into `ActivePrompt { turn_id, attempt_nonce: Option<String>, compacting, ... }`.
   - **Pre-P2 isolation:** `src/acp.rs` generates a fresh random 128-bit hex `attempt_nonce` and injects `params._meta["cowboy.dev/attemptNonce"]` **ONLY when `raw_wrap_up_eligible == true`** (verified signed launch `>= 0.2.35` AND live P2 `initialize` handshake). On pre-P2 Antigravity launches, `attempt_nonce` is `None` and zero private metadata is injected.
   - On coalesced `pending_wrap_up_hint` wakeup: `src/acp.rs` verifies `effective_wrap_up_steering`, `!managed_read_only`, `worker_epoch == target.worker_epoch`, `active_prompt.turn_id == target.turn_id`, `!active_prompt.compacting`, `!cancelled`, AND `let Some(nonce) = active_prompt.attempt_nonce.as_deref()`, then sends payload-free `cowboy/wrapUpHint { sessionId: acp_id, attemptNonce: nonce }`.
2. **Proxy nonce stripping & ambiguous-boundary hint suppression (`proxy.mjs`, `facade.mjs:193,:299`):**
   - In `proxy.mjs:fromClient` on `session/prompt`: **synchronously** copies `params?._meta?.["cowboy.dev/attemptNonce"]` into the `reservedTurns` entry (`reserved.attemptNonce`) BEFORE any `await`, and **strips `params._meta["cowboy.dev/attemptNonce"]`** (deleting `_meta` if empty) BEFORE forwarding `session/prompt` to `agy`.
   - Creates ONE shared `wrapUp = { pending: false, delivered: false }` latch inside the reservation and passes it into `runtime.facade.beginTurn(sessionId, { wrapUp })` when promoted to `ACTIVE`.
   - `cowboy/wrapUpHint` bypasses `this.queue` synchronously; looks up `this.turns.get(sessionId) ?? this.reservedTurns.get(sessionId)` and is accepted ONLY when `typeof attemptNonce === "string" && entry.attemptNonce === attemptNonce`, `!this.managed`, `!this.subagents.owns(sessionId)`, `!entry.compacting`, `!entry.cancelled`, `!entry.structured`, and `!entry.wrapUp.delivered` -> sets `entry.wrapUp.pending = true`.
   - **Ambiguous-boundary suppression against delayed MCP requests (handles overlapping calls with the same tool name & arguments!):**
     - On a clean `EndTurn` with zero unsettled MCP calls, `agy` has already received every MCP response before completing `session/prompt`, so no delayed MCP request can be waiting on the session's MCP relay.
     - Whenever a root turn ends **ambiguously** (`cancelled`, ACP error, or any unsettled/unconfirmed MCP call), `Facade` latches **`session.ambiguousMcpBoundary = true`** on that session!
     - While `session.ambiguousMcpBoundary === true`, `Facade` **suppresses attaching `WRAP_UP_NOTE` for the entire next turn** (never letting a delayed same-name/same-args MCP request from the stopped earlier turn spend the new turn's hint!), and clears `session.ambiguousMcpBoundary = false` ONLY when a subsequent root turn finishes cleanly (`EndTurn` with zero unsettled MCP calls and no cancellation/error)!
   - Inside `facade.mjs:call(sessionId, name, args)`: captures `const callTurn = session?.turn` at tool-call start. After `this.onResult(call, result)` runs (keeping Cowboy UI presentation clean), attaches `WRAP_UP_NOTE` ONLY when ALL hold:
     - `session.parent === undefined` (root session only) AND `!this.managed`
     - `session.ambiguousMcpBoundary === false`
     - `callTurn !== undefined && session.turn === callTurn`
     - `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`
     - `callTurn.wrapUp?.pending === true && callTurn.wrapUp.delivered === false`
     - `name !== "UpdatePlan"` AND `!result?.isError` AND `Array.isArray(result?.content)`.
   - Atomically sets `callTurn.wrapUp.pending = false; callTurn.wrapUp.delivered = true;` (strictly once per turn) and returns a **non-mutating copy** attaching `WRAP_UP_NOTE = "[Cowboy note: the owner asked you to wrap up your current step briefly so they can follow up next.]"` to the **first** block in `result.content` with `type === "text"` (handling `[text, image]` `ReadFile` results), appending `{ type: "text", text: WRAP_UP_NOTE }` only if `result.content` has no text block. Update `testdata/fake-agy.mjs:75` to consume all content blocks and verify both plain text and `[text, image]` results in the real `agy` canary (§6.1).

### 4.3 Shared Compaction Fence, Context Handoff Gate & Already-Claimed Carriers (`COWBOY_WS`)

- **Closed code-point set `COWBOY_WS`** implemented identically in Rust (`is_cowboy_ws`) and JS (`compaction.mjs:isCowboyWs`), explicitly excluding `U+0085` and including `U+FEFF`:
  `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF`.
- **`is_compaction_prompt(blocks)`:** applies `build_prompt_blocks(text, content)` first; returns `true` iff non-empty, every block is `type == "text"`, and `COWBOY_WS`-trimmed concatenation matches ASCII case-insensitive `/compact` followed by EOF or `COWBOY_WS`.
- **Context handoff & preflight alignment (`context_send`, `background_jobs/dispatch.rs:123-130`, `src/session_context.rs:258,:469`):**
  - Apply the normalized `build_prompt_blocks` + `COWBOY_WS` command/compaction classifier **at `context_send` / next-agent admission BEFORE reserving any context handoff** (so a content-only or split-block `/compact` is refused before marking a handoff accepted or retiring the source session!) AND in `prepare_dispatch_preflight` **BEFORE** both `session_context::rollover::before_dispatch` and `session_context::claim_carrier`, while always preserving final context authorization and revision checks.
  - **Already-claimed pre-upgrade carriers (no speculative silent release!):** if a prompt arriving at `session_context::prompt_content` already owns a persisted context carrier for its exact `cmid` (e.g., an already-admitted pre-upgrade bootstrap/handoff prompt or legacy split-block `/compact` whose non-delivery proof was not persisted before upgrade), **never silently omit the preamble and strand the links** — honor the persisted carrier on that already-admitted prompt (or if an explicit operator repair is needed, surface the unresolved legacy carrier rather than releasing without durable proof).
- **Turn-scoped latch:** Controller latches `active_turn_compacting` on dispatch; Worker latches `active_prompt.compacting`; Proxy computes `reserved.compacting` synchronously inside `fromClient` before any `await`.

### 4.4 Proxy-Local Root Stop Bypass, Staged Reservation & Full-Window Cleanup Barrier

1. **Separate `this.reservedTurns` map, staged local preparation & shutdown settlement (`proxy.mjs:560,:653,:682`):**
   - Keep unwritten reservations in **`this.reservedTurns: Map<sessionId, ReservedTurn>`** (NEVER inserting into `this.turns` until promotion right before `this.agent(forwarded)`), so `holdReply`, `observe`, `collectReply`, and memory attribution read ONLY `this.turns` and late `agy` updates during preparation can never unseal managed output or be misattributed.
   - Reject a new `session/prompt` on `sessionId` if either `this.turns.has(sessionId)` OR `this.reservedTurns.has(sessionId)`.
   - **Stage all async preparation locally (zero shared state mutation before promotion!):** `evidence.mark()`, skill expansion, and `recall()` write ONLY into local variables — **ZERO mutation of `this.compaction.take(...)`, `this.compacting`, `this.sealed`, or `this.turns` before the final synchronous check!**
   - **Closing latch on ALL shutdown paths (`this.closing`):** latch `this.closing = true` synchronously on `this.killed`, client stdin `end`/EOF, AND `agy` child `exit`/`close`, calling `abortAllReservations()` to mark every entry in `this.reservedTurns` cancelled, call `reserved.cancelResolve()`, remove it via `releaseReservation`, and settle each unwritten record exactly once!
   - Funnel **every** pre-write exit (Stop, managed prompt/schema refusal, runtime-readiness failure, exception, or `this.closing`) through ONE identity-checked `releaseReservation(sessionId, reserved)` inside `try / finally`, paired with exactly-once `endPrompt` (`record.finalized = true`).
   - **`session/cancel` (for valid root and child sessions) AND `cowboy/wrapUpHint` bypass `this.queue` synchronously in `fromClient`.**
   - When `session/cancel` arrives while `reserved = this.reservedTurns.get(sessionId)` exists: synchronously sets `reserved.cancelled = true`, increments `this.cancelGen`, calls `reserved.cancelResolve()` (unblocking the raced `Promise.race` so `this.queue` releases immediately), calls `releaseReservation(sessionId, reserved)`, and settles once via `endPrompt`.
   - **One uninterrupted synchronous promotion block:** immediately after all awaits, check `!reserved.cancelled && !record.finalized && !this.closing && this.reservedTurns.get(sessionId) === reserved && this.cancelGen.get(sessionId) === reserved.cancelGen`; ONLY THEN atomically run `this.compaction.take(...)`, update `this.compacting` / `this.sealed`, move `reservedTurns -> this.turns`, call `facade.beginTurn`, and invoke `this.agent(forwarded)`!
2. **Synchronous correction fence & full-window per-attempt cleanup barrier (`proxy.mjs:756-781,:1152`, `facade.mjs:212,:277`):**
   - Scope `attemptCleanup` **per native attempt** (resetting `turn.attemptCleanup = null` whenever `facade.beginTurn(...)` opens a fresh attempt, including inside `proxy.correct()`).
   - **Full root stop confirmation window (`LIMITS.stopConfirmMs` = 60s) & uncertain-stop parking:** inside `startAttemptCleanup(sessionId, turn, { cancelled })`:
     - Synchronously fences `facade` + `subagents` admission.
     - For **root sessions**, ALWAYS awaits command tree termination for the full **`LIMITS.stopConfirmMs` (60s)** window (even when initiated by Stop — because native `session/cancel` is forwarded to `agy` IMMEDIATELY *before* awaiting `attemptCleanup`, the 60s barrier does NOT delay Stop reaching `agy`, while preventing the turn from ending and draining the next Owner prompt while a target command is still running!).
     - If any root command's termination STILL remains unconfirmed after `LIMITS.stopConfirmMs` (60s): calls `facade.fenceLost(turn, "process/terminate")` + `UNCERTAIN_REPORT` (`cowboy/uncertainEffect`) so the trusted worker ledger marks the session uncertain and **parks the session** (preventing the next queued Owner prompt from running concurrently with an unconfirmed command!).
     - `facade.mjs` tracks active `endTurn` promises even after `session.turn` is detached so `facade.settled()` awaits them on shutdown.
   - Root `cancel()` on `turn = this.turns.get(sessionId)` **FIRST synchronously latches `turn.cancelled = true`, increments `this.cancelGen`, and clears `turn.wrapUp.pending = false` BEFORE checking or joining any barrier** (fencing `proxy.correct()` during `await evidence.mark()`). Then synchronously starts `startAttemptCleanup(sessionId, turn, { cancelled: true })` (using cached `this.readyRuntime`), **immediately forwards `this.agent(message)` (`session/cancel`) to `agy`**, and awaits `turn.attemptCleanup`. `fromAgent`'s prompt reply joins that exact same `turn.attemptCleanup`.

---

## 5. HTTP/WS API & Web UI Flow (W1)

- **`GET /api/sessions/{id}/delivery-options` (`send_delivery.rs`):**
  - Exposes `"steer": info.meta.supports_current_turn` (always `false` for Antigravity), `"wrap_up_steering": info.meta.wrap_up_steering`, `"editing": hub.is_editing(&session)`, plus existing `target`, `controls` (with `next_checkpoint: false` for Antigravity), `status`, `paused`, `hold`, `policy`.
- **Web (`web/src/protocol.ts`, `DeliveryPicker.tsx`, `Composer.tsx`, `QueuedDeliveryAction.tsx`, `store.ts`):**
  - `SessionInfo` adds `wrap_up_steering?: boolean`.
  - `DeliveryPicker.tsx` (Composer only, `!props.queued`):
    - Enables **"Wrap up & send next"** (*"Asks agent to finish current step, then sends next"*) **only when ALL hold**: `!options.steer`, `props.busy`, `target != null`, `!props.paused`, `!options?.editing`, current context ready, and `(fresh ? Boolean(options?.wrap_up_steering) : Boolean(props.wrapUpReady))`. Preferred default when `policy.mode === "steer"`.
    - Calls `wrapUpPrompt(sessionId, text, attachments, target)`: invokes **`requireCurrentAgentContext(sessionId)`** ONLY (NEVER `sendWithContextNotice`), persists `(sessionId, target)` in the durable outbox entry (`mode: "front", wrapUp: true, wrapUpTarget: target`), and sends `wrap_up: true, wrap_up_target` when `session.wrap_up_steering === true` (falling back cleanly to plain `Inbound::Submit { front: true }` with the same `cmid` on legacy Controllers or when wrap-up hints are disabled).
  - `QueuedDeliveryAction.tsx` (`props.queued === true`): keeps Steer disabled when `!options.steer` (only `Queue first`, `Send now`, edit, and delete available — zero Peer/Automation origin promotion).

---

## 6. Release Sequencing, Hawk Recovery Scoping & Safe Rollback

### 6.1 Slice Sequencing & Buildable 5-Artifact Conformance Set

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller):**
   - Lands on `main` with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`). Strict M1/P2 boundary: M1 commits do **not** touch `plugins/antigravity/` manifests unless exact signed coverage is already published (`just provider-release-coverage`).
   - **Zero DB migrations.** Activate **Controller** first (keeping `providers.antigravity.wrap_up_steering` omitted from Service TOML so default `true` applies cleanly).
   - **Buildable 5-artifact Machine set (built on Hawk per OVH runtime-only contract):**
     - **Hawk Machine (4 inputs verified via `just session-deletion-production-conformance OLD NEW READER DEFAULT_READER RECEIPT`, corresponding to `--old-writer`, `--new-writer`, `--reader-release`, and `--default-reader-release` in `tools/session_deletion_production_conformance.py:171-174` and `flake.nix:1012`):**
       1. `--old-writer`: first post-wire-52/8 writer revision built against the newly accepted worker pin
       2. `--new-writer`: second distinct post-wire-52/8 writer revision (distinct commit + ELF hash) built against the **same** new worker pin
       3. `--reader-release`: matching reader-only fallback release on the **same** new worker pin — pre-admitted and passed as `--recovery-release` **strictly for atomic recovery of the cutover transaction**
       4. `--default-reader-release`: separate default-reader startup test input (**kept strictly separate from rollback selection**)
       *(Record all 4 immutable store roots and a fresh conformance receipt before cutover.)*
     - **OVH Machine (1 artifact, `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` `1.0.27` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Verified to install and launch on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap (enabled after the real `agy` canary below passes).
4. **Bound R1 Acceptance Canary (Real `agy` Verification):**
   - After pin-advance acceptance verifies target worker generation on Hawk and OVH, run a live canary on a real Antigravity session bound to exact candidate Controller, Machine, Worker, and Plugin `0.2.35` digests confirming: negotiated Machine protocol `52`, both runtime legs `8`, `supports_current_turn == false`, `delivery_controls.next_checkpoint == false`, `wrap_up_steering == true`, **real `agy` 1.5.14 observations proving the model consumed the wrap-up note on BOTH a plain text tool result AND an image-bearing `[text, image]` `ReadFile` result**, immediate drain of the front-queued Owner prompt after `TurnEnd`, and root Stop reaching `agy` immediately during an active facade/extension call.

### 6.2 Safe Component Rollback Boundaries

- **Advisory hints (no drain precondition):** disabling new `WrapUpHint`s requires no receipt drain because hints are stateless and queued Owner messages live in existing `sessions.queue`.
- **Controller rollback (two safe paths — never restarting R1 on a legacy TOML!):**
  - **Preferred post-commit rollback:** deploy an **ancestry-eligible descendant corrective Controller release** (which continues accepting `providers.antigravity.wrap_up_steering = false` in Service TOML, so both activation and any automatic health-check rollback retain the disabled config without touching the TOML file!).
  - **Hard downgrade to a pre-R1 Controller binary (which rejects unknown TOML keys at `src/config.rs:575,:769`):** bind the legacy-compatible TOML swap AND its backup restoration **inside the same activation/recovery transaction** so that if health checks fail, the backup disabled R1 TOML (`wrap_up_steering = false`) is restored **before** the automatic recovery path restarts the previous R1 Controller binary!
  - Web outbox downgrade (§4.1) sends plain `Inbound::Submit { front: true }` on pre-R1 Controllers.
- **Hawk Machine rollback (three distinct scopes):**
  - **Cutover transaction failure:** handled by the pre-admitted exact same-pin `--reader-release` passed as `--recovery-release`.
  - **Post-commit resident-Machine regression (compatible with the accepted worker pin):** build and activate a clean, **same-pin descendant corrective Machine release**.
  - **Post-commit retained-worker or wire regression:** because a same-pin release retains the faulty worker and `flake.nix` requires byte-for-byte wire compatibility with the pin, execute a **separately accepted corrective worker-pin advance** (new pin + fresh 4-input conformance + fresh acceptance). Never select `--default-reader-release` as a rollback target.
- **Worker & Plugin rollback:** governed by Cowboy's graceful turn-settlement boundaries (idle adoption / busy safe-boundary cutover; forced stops can leave in-flight replies uncertain per `docs/uncertain-effect-recovery.md:322`).

---

## 7. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify `DeliveryTarget` in `cowboy-wire` creates zero crate cycles; test P2 `0.2.35` install/launch on pre-R1 `PluginManifest` / `ProviderPackage` decoders.
- **Mixed-version, outer/inner protocol & pre-P2 nonce isolation:** cover all rows of §3.3 (including surviving retained workers across Controller rollback, `Machine 51 + Runtime 8`, `Machine 52 + Runtime 7`, and exact wire bytes received by `agy` under a pre-P2 adapter proving ZERO `cowboy.dev/attemptNonce` leaks), explicit `(outer_machine_protocol, runtime_protocol)` passing during `Frame::Welcome` adoption, `WorkerPeer.negotiated_protocol` retention, `delivery_controls.next_checkpoint == false` clamping, missing `launch.provider` fail-closed at broker, and config-reload recomputation from raw observations.
- **Queue-backed admission & caller routing:**
  - `FrontSubmitOutcome`: unit tests covering all 5 variants (`Refused`, `Duplicate`, `DispatchedImmediately`, `QueuedFront`, and `QueuedFallback` on BOTH `TrySendError::Full` AND `TrySendError::Closed`) confirming `QueuedFallback` returns `queued` with `hinted: false` and `Refused` returns `404`/`409` or `cmid`-addressed WS error.
  - Substantive content validation: empty/whitespace-only text blocks (`[{"type":"text","text":""}]`) rejected on BOTH HTTP `delivery: "wrap_up"` (`400`) and WS `Inbound::Submit { wrap_up: true }` (`cmid`-addressed refusal, no queue insert, no hint).
  - HTTP `delivery: "wrap_up"`: rejects non-Person (`403`), validates authored `operation` when supplied, targetless HTTP is strictly queue-only (`hinted: false`), explicit `target` enables hint on `QueuedFront`; CLI handles busy + idle + retry without hinting a later turn.
  - WS `Inbound::Submit`: rejects `wrap_up && force`, rejects missing/synthetic `cmid`, rejects non-Owner with `cmid`-addressed refusal, treats missing `wrap_up_target` as queue-only, and downgrades to plain `front: true` on legacy Controllers.
  - Agent API rejects `wrap_up` and maps Antigravity `session_default` -> `next_turn`.
- **Reliable-first bounded hint saturation & ambiguous-boundary MCP protection:**
  - Saturation tests with **stalled Controller WS writer** (verifying direct `hint_ws_tx` bypasses shared runtime `UnixStream` FIFO), **stalled `machine_cli` writer**, **stalled broker-to-worker writer** (verifying separate `worker_hint_rx` below reliable queue), AND **stalled worker ACP writer/consumer** (verifying 1-slot coalescing + 1-in-flight guard + 512-byte size cap + 250ms timeout) proving reliable commands, Stop, and heartbeats are never blocked by hint floods.
  - Worker injects `attemptNonce` ONLY on verified P2 launches (fresh per attempt/retry) and carries exact `TurnStarted` `turn_id` into `ActivePrompt`; proxy copies nonce into `reservedTurns`, strips it before `agy`, and drops mismatched nonces.
  - Facade: hint during `reservedTurns` preserved on promotion; overlapping delayed MCP request (same tool name and arguments!) arriving after a cancelled turn sets `session.ambiguousMcpBoundary = true` and does NOT consume the next turn's hint until a clean `EndTurn` resets the boundary; note attaches to first text block (including `[text, image]` `ReadFile`), appends only if no text block exists, runs after `onResult` once per turn without mutating `result`; `testdata/fake-agy.mjs` reads all content blocks.
  - Shared `COWBOY_WS` compaction tests (`U+0085`, `U+FEFF`, split blocks, mixed blocks) including `context_send` rejecting normalized `/compact` before reserving handoff, `prepare_dispatch_preflight` leaving pending context links unclaimed, and honoring already-claimed pre-upgrade carriers without silently stranding links.
- **Proxy reservation seal, staged compaction safety, shutdown settlement & full-window Stop barrier:**
  - `proxy.mjs`: late `agy` text/tool updates arriving while a prompt is in `reservedTurns` stay sealed in managed mode and are never misattributed; cancelling a prompt mid-`recall` does NOT consume `this.compaction.pending` restatement or mutate `this.compacting` (staged locally before promotion); stdin EOF and `agy` child exit latch `this.closing` and settle all `reservedTurns` cleanly; a refused prompt releases `reservedTurns` so the next valid prompt succeeds; root Stop with a slow target command forwards `session/cancel` immediately while `attemptCleanup` holds the full 60s `LIMITS.stopConfirmMs` window before completing the turn (and parks via `UNCERTAIN_REPORT` if still unconfirmed at 60s!); Stop during `proxy.correct()` preparation fences the correction before write, AND Stop/completion after `proxy.correct()` starts runs fresh attempt-2 cleanup.
- **Web (W1):** `DeliveryPicker` gates **"Wrap up & send next"** on busy + live target + `!paused` + `!editing` + current context + `wrap_up_steering` (Composer only; queued rows keep Steer disabled for Antigravity); `wrapUpPrompt` calls `requireCurrentAgentContext` (never `sendWithContextNotice`), binds `(sessionId, target)` in outbox, downgrades cleanly on legacy Controllers, and passes `bun run typecheck && bun run lint`.
