# Antigravity Wrap-Up Steering (R1) — Queue-Backed Redesign (Revision 6)

Status: design, revision 6 (2026-11-24). Scoped strictly to **R1 (Owner wrap-up steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), and queued-row Steer promotion for Antigravity.

---

## 1. Context, Architectural Choice & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority. Any mechanism on `agy` 1.5.14 must wait for the active `session/prompt` to finish before dispatching the follow-up `session/prompt`.
- **Queue-Backed Wrap-Up Steering:**
  1. **Authoritative delivery reuses Cowboy's existing C-auth front-of-queue `sessions.queue` path** (`front = true`, `DeliveryOrigin::Owner`), committed through Cowboy's **acknowledged FIFO queue writer** with a per-session drain fence before returning HTTP `202` or confirming the WebSocket `cmid`. Zero new database tables, zero parallel receipt state machines, zero `not_confirmed` UX, and full reuse of multi-block echoes, `Edit`/`Delete`/`Clear`/`Retry`, and existing queue semantics.
  2. **Honest durability boundary:** durably committed, still-queued rows survive Controller crash/restart cleanly; once `drain_head` (`src/core/hub_queue.rs:279`) dequeues a row for worker dispatch, the dequeue-to-worker handoff follows Cowboy's existing ordinary prompt dispatch and `UndeliveredPrompt` recovery semantics.
  3. **Stateless, payload-free, target-fenced `WrapUpHint`:** when an Owner front-queues a wrap-up steer against a captured live, non-paused, non-editing, non-compacting Antigravity turn, the Controller sends a best-effort, **zero-owner-text** `WrapUpHint` over bounded drop-on-full channels at every hop, targeted to exact `(worker_epoch, turn_id)` -> `attemptNonce`. The proxy attaches at most one fixed Cowboy-authored advisory note to the next root facade tool result asking the model to finish its current step briefly.
  4. **Normal clean turn boundary:** when turn 1 finishes, Cowboy runs ordinary `Ledger::finish_turn`, `Facade.endTurn`, and `Subagents.finish` cleanly, then `Hub::try_drain` immediately dispatches the front-queued Owner prompt as turn 2.
  5. **Root Stop latency fix:** bundled into P2 `proxy.mjs` so root `session/cancel` bypasses `this.queue`, shares one per-attempt cleanup barrier with prompt completion, synchronously fences admission, and immediately forwards `session/cancel` to `agy` before awaiting cleanup.
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because `WrapUpHint` carries zero owner text and never controls queue admission or transcript echoes, an untrusted proxy can neither forge delivery nor inject Owner text.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), and queued-row Steer for Antigravity (§5).

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
  - `Outbound::BootstrapComplete` (WebSocket bootstrap capability):
    - Add optional `#[serde(default)] wrap_up_submit: bool` (or emit a dedicated additive `Outbound::Capabilities { wrap_up_submit: true }` immediately before `BootstrapComplete`; older Controllers omit it, letting Web distinguish an R1 Controller from a legacy Controller independently of `session.wrap_up_steering`!).
- **End-to-end bounded drop-on-full hint path (outside `pending`, replay, overflow refusal, and `CommandAck`):**
  - **Controller `RemoteRuntime::connected` (`src/remote_runtime.rs:1755`):** owns a bounded per-connection `hint_tx: mpsc::Sender<CoreCommand>` (cap 32), installed on `Shared` only when connected with Machine >= 52 and runtime >= 8 and **dropped on disconnect**. `connected` polls `hint_rx` at lower priority than reliable pending commands (`biased;`) and serializes hints through the sole framed writer; `try_send` silently drops on full/disconnect.
  - **Controller WS tunnel (`machine_connection.rs`):** drops `WrapUpHint` if the outbound WS queue exceeds its bounded hint water mark (never blocking reliable commands or heartbeats).
  - **Machine CLI local tunnel (`machine_cli.rs:1579`):** routes incoming `CoreCommand::WrapUpHint` into a separate bounded `hint_runtime_tx` (cap 32, `try_send` drop-on-full) instead of unbounded `runtime_write_tx`; `write_runtime_frames` polls `runtime_write_rx` with `biased;` ahead of `hint_runtime_rx` so expendable hints can never accumulate ahead of reliable commands or heartbeats!
  - **Broker (`machine_broker.rs:4337`) & Worker (`worker.rs`):** handle `WrapUpHint` on an explicit silent-drop path **outside** `pending_commands`, overflow refusal, replay, and `CommandAck` handling (bounded `try_send`, cap 8 per worker, silently dropping if saturated, stale, or unsupported — never dropping the Controller connection and never emitting `CommandAck`).

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

### 3.2 Handshake-Aware Sanitization & Raw Observation Retention

All capability surfaces — **`Frame::Welcome` initial snapshots**, **`Frame::Snapshot`**, **cached worker state**, and **`Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)`** — are sanitized using the **negotiated `(outer_machine_protocol, runtime_protocol)` of the exact connection being installed** (never reading stale pre-install `Shared.protocol` or pre-install Controller leases!):

- **Broker (`machine_broker.rs:298,:664`):**
  - Stores `negotiated_protocol: u32` inside `WorkerPeer` and retains raw worker capability observations (`raw_wrap_up_steering`, `raw_supports_current_turn`, `raw_next_checkpoint`) separately from projected values.
  - Reads `outer_machine_protocol` from the incoming Controller IPC `Frame::Hello` (failing closed as `0` when absent) together with the negotiated IPC `runtime_protocol`, and passes BOTH explicitly when building `Frame::Welcome` snapshots (before installing the Controller lease!).
  - Fails closed when `launch.provider` cannot be established (`wrap_up_steering = false`, `supports_current_turn = false`, `next_checkpoint = false`).
  - For `provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; projects `wrap_up_steering = raw_wrap_up_steering && worker.negotiated_protocol >= 8 && controller_runtime_protocol >= 8 && outer_machine_protocol >= 52`.
- **Machine WS tunnel (`machine_cli.rs`, `machine_connection.rs`):** enforces the outer Machine `>= 52` gate as defense in depth (clamping `wrap_up_steering = false` and never transmitting `WrapUpHint` on `< 52` tunnels).
- **Controller (`remote_runtime.rs:464,:1732`, `src/core/`, `src/config/schema.rs`):**
  - Passes the incoming connection's negotiated `(machine_protocol, runtime_protocol)` explicitly into initial `Frame::Welcome` snapshot adoption (before/alongside setting `Shared.protocol`) and preserves raw worker capability observations so reconnects and config reloads recompute effective projections accurately.
  - Adds Service config key `providers.antigravity.wrap_up_steering` (boolean, default `true`, omitted from Service TOML unless overridden; see §6.2 for mandatory TOML removal before Controller rollback).
  - For `session.meta.provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and `providers.antigravity.wrap_up_steering == true`.

### 3.3 Mixed-Version Rollout Matrix (Keyed to Verified Running Launch)

Existing sessions retain their active launch digest until idle adoption (`session_provider_updates.rs:1`), so matrix rows apply to each session's **verified running launch** (not merely whether P2 is installed on disk):

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

## 4. Acknowledged Admission, Compaction Alignment, Hint & Stop Fixes

### 4.1 Drain-Fenced Acknowledged FIFO Commit & Single-Transaction HTTP Receipt

- **Per-session drain & mutation fence (`Session.pending_commit_fence: usize`):**
  - Add `pending_commit_fence: usize` on `Session` (`src/core.rs`). `Hub::ready(s, ...)` returns `false` whenever `s.pending_commit_fence > 0` (fencing `drain_head`, `wake_queued_call_notice`, and immediate direct dispatch in `submit_inner`/`force_submit` across preview, store write, and Hub publication!).
  - `begin_clear_conversation` also serializes on `Hub::pending_commit_lock(session_id)` so Clear cannot race an in-flight acknowledged wrap-up commit.
  - **Acknowledged front-queue commit flow (`Hub::admit_front_prompt_acked` returning `FrontSubmitOutcome { Duplicate, DispatchedImmediately, QueuedFront }`):**
    1. Acquire `Hub::pending_commit_lock(session_id)`.
    2. Under `sessions.lock()`: check `prompt_already_accepted(s, cmid)` (return `Duplicate` if found), increment `s.pending_commit_fence += 1`, install `in_flight_pending_commits` (deferring ordinary `emit_pending` writes until release), and determine whether the session is currently idle with empty queue (`DispatchedImmediately`) or busy/non-empty (`QueuedFront`).
    3. Enqueue **ONE atomic FIFO store write** (`StoreWrite::CommitWrapUpPrompt` — see HTTP vs WS below) that commits the queue prepend (appending/prepending atomically onto the current persisted `sessions.queue` inside the FIFO transaction, NOT overwriting with a stale snapshot!) AND—for HTTP—commits the `agent_api_operations` receipt **in that exact same DB transaction**.
    4. Await store ACK (`ack_rx.await`):
       - **On `Ok(Committed)`:** under `sessions.lock()`, if `QueuedFront` prepend the committed `QueuedMessage` at index `0` of the current Hub `s.queue` (preserving any non-dispatch edits/removals made during the wait); decrement `s.pending_commit_fence -= 1`; call `finish_in_flight_pending_commit` (flushing any deferred ordinary queue writes now merged with the committed front row); broadcast pending snapshot; and **unconditionally call `hub.try_drain(session_id)`** (so if `TurnEnd` arrived during the ACK wait, the newly committed row drains immediately without stranding!). For `DispatchedImmediately`, emit `send_dispatch` only after commit.
       - **On `Err` / `Conflict` / `Existing`:** do NOT mutate `s.queue`, decrement `s.pending_commit_fence -= 1`, call `finish_in_flight_pending_commit`, and call `hub.try_drain(session_id)`!
- **Native mid-turn delivery (`delivery: "current_turn"` / `"next_checkpoint"`):**
  - Remains strictly bound to `supports_current_turn == true` (refused cleanly on Antigravity: `not_delivered`).
- **HTTP `POST /api/sessions/{id}/prompt` (`SendDelivery::WrapUp` = `"wrap_up"`):**
  - Mutually exclusive with `force: true` and `front: true` in `SendDelivery::resolve`.
  - **Owner-only:** requires `matches!(caller, RequestCaller::Person(_))`; `Agent`, `Automation`, and `Cowboy` receive `403 Forbidden`.
  - **Required, bounded authored `operation` ID:** HTTP `delivery: "wrap_up"` **requires** a non-empty `operation` (`<= 128` bytes, ASCII `[A-Za-z0-9._:-]`, passing `Hub::is_authored_cmid`, rejecting all synthetic prefixes `call-notice:`, `__cont__`, `__wake__`, `__sched__`, `cowboy-`, `api:`). Missing/invalid `operation` returns `400 Bad Request`.
  - **Single-transaction durable receipt + queue commit (zero divergence):**
    - `request_hash` hashes ONLY the caller-submitted inputs `(caller, delivery, text, content, submitted_target)` — NEVER a server-sampled turn observation!
    - Inside the single FIFO `StoreWrite::CommitWrapUpPrompt` transaction: checks `agent_api_operations` for `(owner, session, "wrap_up:{operation}")`:
      - Existing with different `request_hash` -> rolls back & returns `409 Conflict`.
      - Existing with same `request_hash` -> rolls back (zero queue mutation, zero hint) & returns saved `202` receipt (`hinted: false`) — even if the Owner later deleted the queued row!
      - New -> commits BOTH `sessions.queue` prepend (when `QueuedFront`) AND the completed `agent_api_operations` receipt (`{ status: "queued" | "dispatched", delivery: "wrap_up", operation }`) in **ONE DB transaction**!
    - **Hint target capture:** if `req.target` was supplied, use it; if `req.target` is `None`, sample `supervisor.delivery_target(&session_id)` once *before* the FIFO commit (never hashed into `request_hash`!), and fire §4.2 best-effort `WrapUpHint` strictly **after** the commit succeeds and ONLY on a fresh `QueuedFront` admission.
- **WebSocket `Inbound::Submit` (`src/core.rs:904`, `client_ws.rs:707`):**
  - Keeps existing `Inbound::Submit` tag with two optional serde-defaulted fields: `#[serde(default)] wrap_up: bool` and `#[serde(default)] wrap_up_target: Option<DeliveryTarget>`.
  - **Bootstrap-negotiated rollback encoding (independent of live `session.wrap_up_steering`!):**
    - Web inspects connection-level `wrap_up_submit === true` from WS bootstrap (NOT `session.wrap_up_steering`!).
    - On any R1 Controller (`wrap_up_submit === true`): ALWAYS sends `Inbound::Submit { front: true, wrap_up: true, wrap_up_target }` so outbox replays ALWAYS take the acknowledged FIFO queue path even when `session.wrap_up_steering` is temporarily `false` (during reconnect, config disablement, or retained-worker rollout; Controller still commits acknowledged front-queue and simply skips the hint).
    - ONLY after identifying a legacy pre-R1 Controller (`BootstrapComplete` received with `wrap_up_submit !== true`): downgrades replayed outbox sends (same `cmid`) to plain `Inbound::Submit { front: true }` (omitting `wrap_up` and `wrap_up_target`).
  - **Strict WS admission guards:**
    - Rejects `wrap_up: true && force: true` BEFORE either dispatch path (`cmid`-addressed refusal!).
    - Requires a validated authored `cmid` when `wrap_up: true` (rejecting missing or synthetic `cmid`!).
    - Routes through `Submit`'s full session authorization, system/context checks, and Plugin lifecycle fence, plus explicit credential-derived `caller.is_person()` check returning a `cmid`-addressed `CommandResult` refusal for non-Owners.
    - Commits through the acknowledged FIFO queue path before emitting `confirm_delivered`.
    - **Strict persisted target requirement for WS hints:** `WrapUpHint` fires on WS `Submit` ONLY when `wrap_up_target` is `Some(target)` (persisted at gesture time); a missing `wrap_up_target` is treated as **queue-only** (never sampling the active turn at admission, so a delayed or replayed targetless Submit can never hint a later turn!).
- **Agent API & CLI (`agent_api/control.rs:788,:1046`, `cli/src/product_cli/session_prompt.rs:70`):**
  - `agent_api/control.rs:788`: explicitly reject `SendDelivery::WrapUp` alongside `CurrentTurn | NextCheckpoint` (`Err("invalid_argument")`).
  - `agent_api/control.rs:1046` (`delivery == "session_default"`): when `info.meta.provider == "antigravity"`, map `session_default` to `"next_turn"` (preserving existing `Steer -> current_turn` mapping for Claude/Codex!).
  - `cli/src/product_cli/session_prompt.rs:70`: for `SendDelivery::WrapUp`, omit `target` (`Value::Null`) by default (allowing an optional `--target` flag for callers who want explicit turn pinning) so rerunning `--operation <id>` after a lost response always sends the exact same `request_hash` and retrieves the saved receipt cleanly, and the idle dispatch path stays reachable!

### 4.2 Target-Fenced `WrapUpHint`, Nonce Transport & Turn-Bound Facade Delivery

Controller sends `supervisor.send_wrap_up_hint(&session_id, captured_target)` **strictly after acknowledged commit** and **only when ALL hold**:
- Admission was a fresh `FrontSubmitOutcome::QueuedFront` (never on replay / duplicate / immediate dispatch)
- `captured_target` is `Some(target)` AND `supervisor.delivery_target(&session_id).as_ref() == Some(&target)`
- `info.meta.provider == "antigravity"` AND `info.meta.wrap_up_steering == true` AND live config `providers.antigravity.wrap_up_steering == true`
- `info.meta.status == Status::Busy` AND `!info.meta.paused` AND `s.editing.is_none()` (`hub_queue.rs:197`) AND `!hub.clear_pending(&session_id)` AND `hub.delivery_hold(&session_id).is_none()`
- `!hub.is_compacting(&session_id)` AND `!hub.active_turn_is_compacting(&session_id)`.

Across hops:
1. **Worker prompt nonce & exact `turn_id` (`src/worker.rs:992,:1086`, `src/acp.rs`):**
   - `worker.rs` passes the exact runtime `turn_id` published in `TurnStarted` inside `AgentCommand::Prompt` into `ActivePrompt { turn_id, attempt_nonce, compacting, ... }`.
   - For each Antigravity `session/prompt` attempt (including retries!), `src/acp.rs` generates a fresh random 128-bit hex `attempt_nonce` and attaches it in `params._meta["cowboy.dev/attemptNonce"]`.
   - On `WorkerCommand::WrapUpHint { session_id, target }`: worker verifies `effective_wrap_up_steering`, `!managed_read_only`, `worker_epoch == target.worker_epoch`, `active_prompt.turn_id == target.turn_id`, `!active_prompt.compacting`, `!cancelled`, and sends payload-free ACP notification `cowboy/wrapUpHint { sessionId: acp_id, attemptNonce: active_prompt.attempt_nonce }`.
2. **Proxy nonce extraction, stripping & turn-bound facade delivery (`proxy.mjs`, `facade.mjs:193,:299`):**
   - In `proxy.mjs:fromClient` on `session/prompt`: **synchronously** copies `params?._meta?.["cowboy.dev/attemptNonce"]` into the `RESERVED` turn object (`turn.attemptNonce`) BEFORE any `await`, and **strips `params._meta["cowboy.dev/attemptNonce"]`** (deleting `_meta` if empty) BEFORE forwarding `session/prompt` to `agy`!
   - Creates ONE shared `wrapUp = { pending: false, delivered: false }` latch inside the synchronous `RESERVED` turn object and passes that exact latch reference into `runtime.facade.beginTurn(sessionId, { wrapUp })`.
   - `cowboy/wrapUpHint` bypasses `this.queue` synchronously; accepted ONLY when `turn` exists for `sessionId`, `typeof attemptNonce === "string" && turn.attemptNonce === attemptNonce`, `!this.managed`, `!this.subagents.owns(sessionId)`, `!turn.compacting`, `!turn.cancelled`, `!turn.structured`, and `!turn.wrapUp.delivered` -> sets `turn.wrapUp.pending = true`.
   - Inside `facade.mjs:call(sessionId, name, args)`: **captures `const callTurn = session?.turn` at tool-call start**. After `this.onResult(call, result)` runs (keeping Cowboy UI presentation clean), checks ALL of:
     - `session.parent === undefined` (root session only) AND `!this.managed`
     - `callTurn !== undefined && session.turn === callTurn`
     - `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`
     - `callTurn.wrapUp?.pending === true && callTurn.wrapUp.delivered === false`
     - `name !== "UpdatePlan"` AND `!result?.isError` AND `Array.isArray(result?.content)`.
   - Atomically sets `callTurn.wrapUp.pending = false; callTurn.wrapUp.delivered = true;` (strictly once per turn!) and returns a **non-mutating copy** attaching `WRAP_UP_NOTE = "[Cowboy note: the owner asked you to wrap up your current step briefly so they can follow up next.]"`:
     - **Existing-text-block attachment (handles image-bearing `[text, image]` `ReadFile` results!):** finds the **first** block in `result.content` with `type === "text"` and clones `result.content` with `"\n\n" + WRAP_UP_NOTE` appended to that existing text block; ONLY if `result.content` contains no text block at all, appends `{ type: "text", text: WRAP_UP_NOTE }`. Update `testdata/fake-agy.mjs:75` to consume all content blocks and test both plain text and `[text, image]` results in the real `agy` canary (§6.1).

### 4.3 Shared Compaction Fence & Context Carrier/Preamble Alignment (`COWBOY_WS`)

- **Closed code-point set `COWBOY_WS`** implemented identically in Rust (`is_cowboy_ws`) and JS (`compaction.mjs:isCowboyWs`), explicitly excluding `U+0085` and including `U+FEFF`:
  `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF`.
- **`is_compaction_prompt(blocks)`:** applies `build_prompt_blocks(text, content)` first; returns `true` iff non-empty, every block is `type == "text"`, and `COWBOY_WS`-trimmed concatenation matches ASCII case-insensitive `/compact` followed by EOF or `COWBOY_WS`.
- **Preflight context carrier & preamble alignment (`background_jobs/dispatch.rs:123-130`, `src/session_context.rs:258,:469`):**
  - Classify the **actual normalized prompt blocks** (`build_prompt_blocks(&text, &content)`) with the shared `COWBOY_WS` command/compaction detector inside `prepare_dispatch_preflight` **BEFORE** both `session_context::rollover::before_dispatch` and `session_context::claim_carrier`, AND inside `session_context::prompt_content` — so content-only, `U+FEFF`, or split-block `/compact` prompts never claim pending context links or receive prepended `<system-reminder>` blocks, leaving links pending for the next ordinary prompt!
- **Turn-scoped latch:** Controller latches `active_turn_compacting` on dispatch; Worker latches `active_prompt.compacting`; Proxy computes `turn.compacting` synchronously inside `fromClient` when reserving the `RESERVED` turn object before any `await`.

### 4.4 Root Stop Bypass, Per-Attempt Cleanup Barrier & Stop-vs-Prompt Ordering

1. **Proxy synchronous Stop bypass + single reservation + final pre-write fence (`proxy.mjs:560,:653,:682`):**
   - **`session/cancel` (for valid root and child sessions) AND `cowboy/wrapUpHint` bypass `this.queue` synchronously in `fromClient`** (NOT chained onto `this.queue = this.queue.then(...)`!).
   - `session/prompt` synchronously creates ONE session-scoped `turn` reservation object in `this.turns` (`state: "RESERVED"`, holding `id`, `record`, `attemptNonce`, `cancelGen`, `compacting`, `wrapUp`, `cancelled: false`, `attemptCleanup: null`) and **populates THAT SAME object** through `await evidence.mark()` and `await recall()` (never replacing it with a fresh `this.turns.set(...)`).
   - If `session/cancel` arrives synchronously while the reservation is still unwritten (same chunk OR blocked inside `await evidence.mark()` / `await recall()`): `cancel()` marks `turn.cancelled = true`, increments `this.cancelGen`, and **immediately settles the unwritten reservation once** (`this.endPrompt(record, { jsonrpc: "2.0", id: record.id, result: { stopReason: "cancelled" } })`, guarded by `record.finalized = true`) even while preparatory awaits are still pending!
   - After every `await` AND immediately before `this.agent(forwarded)`, the prompt handler rechecks `turn.cancelled || record.finalized || this.cancelGen.get(sessionId) !== turn.cancelGen || this.killed` and returns without writing to `agy` or double-settling.
2. **Proxy per-attempt cleanup barrier (`proxy.mjs:756-781,:1152`, `facade.mjs:212,:277`):**
   - Scope `attemptCleanup` **per native attempt** (resetting `turn.attemptCleanup = null` whenever a new `facade.beginTurn(...)` opens — including inside `proxy.correct()` — so managed correction attempts NEVER inherit attempt 1's completed cleanup barrier!).
   - `startAttemptCleanup(sessionId, turn, { cancelled })` synchronously fences `facade` + `subagents` admission and memoizes `turn.attemptCleanup = Promise.all([facade.endTurn(...), subagents.finish(...)])`. `facade.mjs` tracks active `endTurn` promises even after `session.turn` is detached so `facade.settled()` awaits them on shutdown.
   - Root `cancel()` calls `startAttemptCleanup(sessionId, turn, { cancelled: true })` synchronously (using cached `this.readyRuntime`), **immediately forwards `this.agent(message)` (`session/cancel`) to `agy`**, and then awaits `turn.attemptCleanup`. `fromAgent`'s prompt reply handler joins that exact same `turn.attemptCleanup`!
3. **Controller `RemoteRuntime::send_pending` & Broker Stop-vs-Prompt Ordering (`remote_runtime.rs:1169,:1516,:1873`, `machine_broker.rs`, `reset_admission.rs:35`):**
   - **Declaration version tracking, supersession & reconnect:** track `shared.sent` as `HashMap<String, u64>` (`key -> sent_version`); tag each reset `StopSession(reset-*)` with `(reset_id, required_ensure_version)`. Whenever `insert_pending` replaces `ensure:{id}` with a newer version, atomically retire superseded older reset stops (`required_ensure_version < new_version`). On reconnect, `send_declarations` skips sessions with an active reset so `send_pending` replays the current `(EnsureSession(v), StopSession(reset-*))` pair through `reset_admission`. Recheck both active `reset_id` and `shared.sent.get(&ensure_key) == Some(&required_ensure_version)` at the final write reservation.
   - **Provably-unsent retirement via reconnect-stable `attempted_sends`:** under the same lock as the final write reservation (`shared.pending` + `shared.attempted_sends`), a pending same-session `CoreCommand::Prompt` is retired by root `Cancel` **ONLY if `!shared.attempted_sends.contains(&key)`** (never using `shared.sent`!). Retired prompts settle through Cowboy's existing cancellation path (releasing `prompt_tracking`, Hub `in_flight`/`dispatch_pending`, and queue guards without automatic requeue). If a same-session `Prompt` is already in `attempted_sends`, preserve FIFO ordering between that `Prompt` and `Cancel` (while still letting `Cancel` preempt unrelated sessions!).
   - **Broker-held prompts during generation drain & reconnect (`machine_broker.rs`):** when `CoreCommand::Cancel` arrives at broker, check `broker.pending_commands[session_id]` FIRST: atomically withdraw any parked `WorkerCommand::Prompt` held for startup or generation cutover drain and settle it cleanly as cancelled (NOT triggering automatic requeue on Controller!), while still forwarding `Cancel` to any active/draining worker running a live turn.

---

## 5. HTTP/WS API & Web UI Flow (W1)

- **`GET /api/sessions/{id}/delivery-options` (`send_delivery.rs`):**
  - Exposes `"steer": info.meta.supports_current_turn` (always `false` for Antigravity), `"wrap_up_steering": info.meta.wrap_up_steering`, `"editing": hub.is_editing(&session)`, plus existing `target`, `controls` (with `next_checkpoint: false` for Antigravity), `status`, `paused`, `hold`, `policy`.
- **Web (`web/src/protocol.ts`, `DeliveryPicker.tsx`, `Composer.tsx`, `QueuedDeliveryAction.tsx`, `store.ts`):**
  - `SessionInfo` adds `wrap_up_steering?: boolean`; WS bootstrap stores controller-level `wrap_up_submit: boolean`.
  - `DeliveryPicker.tsx` (Composer only, `!props.queued`):
    - Enables **"Wrap up & send next"** (*"Asks agent to finish current step, then sends next"*) **only when ALL hold**: `!options.steer`, `props.busy`, `target != null`, `!props.paused`, `!options?.editing`, current context ready, and `(fresh ? Boolean(options?.wrap_up_steering) : Boolean(props.wrapUpReady))`. Preferred default when `policy.mode === "steer"`.
    - Calls `wrapUpPrompt(sessionId, text, attachments, target)`: invokes **`requireCurrentAgentContext(sessionId)`** ONLY (NEVER `sendWithContextNotice`, so a staged context handoff in another tab can never redirect a target-bound send to another session!), persists `(sessionId, target)` in the durable outbox entry (`mode: "front", wrapUp: true, wrapUpTarget: target`), and encodes using connection-level `wrap_up_submit` (§4.1).
  - `QueuedDeliveryAction.tsx` (`props.queued === true`): keeps Steer disabled when `!options.steer` (only `Queue first`, `Send now`, edit, and delete available — zero Peer/Automation origin promotion).

---

## 6. Release Sequencing, Pre-Admitted Hawk Recovery & Rollback Boundaries

### 6.1 Slice Sequencing & Buildable 5-Artifact Conformance Set

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller):**
   - Lands on `main` with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`). Strict M1/P2 boundary: M1 commits do **not** touch `plugins/antigravity/` manifests unless exact signed coverage is already published (`just provider-release-coverage`).
   - **Zero DB migrations.** Activate **Controller** first (keeping `providers.antigravity.wrap_up_steering` omitted from Service TOML so default `true` applies cleanly).
   - **Buildable 5-artifact Machine set (built on Hawk per OVH runtime-only contract):**
     - **Hawk Machine (4 inputs, `tools/session_deletion_production_conformance.py:184`, `flake.nix:1012`):**
       1. `--old-writer`: first post-wire-52/8 writer revision built against the newly accepted worker pin
       2. `--new-writer`: second distinct post-wire-52/8 writer revision (distinct commit + ELF hash) built against the **same** new worker pin
       3. `--reader-fallback`: matching reader-only fallback release on the **same** new worker pin
       4. `--default-reader`: separate default-reader startup test input (**kept strictly separate from rollback selection!**)
       *(Record all 4 immutable store roots and a fresh `just session-deletion-production-conformance` receipt, AND **pre-admit the exact same-pin `--reader-fallback` through Hawk's component-owner ancestry and reader-floor checks before cutover**, retaining its GC-rooted store path as the designated Hawk recovery target!)*
     - **OVH Machine (1 artifact, `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` `1.0.27` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Verified to install and launch on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap (enabled after the real `agy` canary below passes!).
4. **Bound R1 Acceptance Canary (Real `agy` Verification):**
   - After pin-advance acceptance verifies target worker generation on Hawk and OVH, run a live canary on a real Antigravity session bound to exact candidate Controller, Machine, Worker, and Plugin `0.2.35` digests confirming: negotiated Machine protocol `52`, both runtime legs `8`, `supports_current_turn == false`, `delivery_controls.next_checkpoint == false`, `wrap_up_steering == true`, **real `agy` 1.5.14 observations proving the model consumed the wrap-up note on BOTH a plain text tool result AND an image-bearing `[text, image]` `ReadFile` result**, immediate drain of the front-queued Owner prompt after `TurnEnd`, and root Stop reaching `agy` immediately during an active facade/extension call.

### 6.2 Accurate Component Rollback Boundaries

- **Advisory hints (no drain precondition):** disabling new `WrapUpHint`s (config override off) requires no receipt drain because hints are stateless and queued Owner messages live in `sessions.queue`.
- **Controller rollback (mandatory TOML cleanup):** because unknown Service TOML keys are fatal at startup (`src/config.rs:575,:769`), if an operator ever wrote `providers.antigravity.wrap_up_steering` into Service TOML, rollback **MUST** remove that key (and any empty `[providers.antigravity]` / `[providers]` tables) and validate the config file with the rollback Controller binary before activating rollback. Bootstrap-negotiated Web encoding (§4.1) downgrades replayed outbox sends cleanly to `Inbound::Submit { front: true }`.
- **Hawk Machine rollback:** after worker-pin cutover, recovery MUST target the pre-admitted same-pin `--reader-fallback` store root (`docs/plugin-session-deletion-journal.md:81`), NOT the pre-pin Machine binary and NOT the `--default-reader` startup test artifact.
- **Worker & Plugin rollback:** governed by Cowboy's graceful turn-settlement boundaries (idle adoption / busy safe-boundary cutover; forced stops can leave in-flight replies uncertain per `docs/uncertain-effect-recovery.md:322`).

---

## 7. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify `DeliveryTarget` in `cowboy-wire` creates zero crate cycles; test P2 `0.2.35` install/launch on pre-R1 `PluginManifest` / `ProviderPackage` decoders.
- **Mixed-version, outer/inner protocol & rollback matrix:** cover all rows of §3.3 (including surviving retained workers across Controller rollback, `Machine 51 + Runtime 8`, and `Machine 52 + Runtime 7`), explicit `(outer_machine_protocol, runtime_protocol)` passing during `Frame::Welcome` adoption, `WorkerPeer.negotiated_protocol` retention, `delivery_controls.next_checkpoint == false` clamping, missing `launch.provider` fail-closed at broker, and config-reload recomputation from raw observations.
- **Drain-fenced acknowledged admission, single-transaction HTTP receipt & caller routing:**
  - Concurrent `drain_head` and `TurnEnd` during wrap-up ACK wait: `pending_commit_fence` blocks drain until commit, prevents resurrection of dispatched heads, and reruns `try_drain` on release so the new row is never stranded.
  - HTTP `delivery: "wrap_up"`: single FIFO transaction commits `sessions.queue` + `agent_api_operations` together (simulated failure commits neither); missing/synthetic `operation` -> `400`; retry after Owner withdrawal returns saved `202` without re-enqueueing; CLI retry with omitted target keeps identical `request_hash` and works on idle sessions.
  - WS `Inbound::Submit`: `wrap_up && force` rejected before dispatch; missing/synthetic `cmid` rejected; non-Owner gets `cmid`-addressed refusal; missing `wrap_up_target` is queue-only (zero hint); connection-level `wrap_up_submit` keeps acknowledged commit active on R1 even when `session.wrap_up_steering` flips `false`, downgrading to `front: true` only on legacy Controllers.
  - Agent API rejects `wrap_up` and maps Antigravity `session_default` -> `next_turn`.
- **Bounded drop-on-full hint saturation & turn-bound facade note:**
  - Saturation tests across Controller `hint_tx`, WS writer, `machine_cli` `hint_runtime_tx` vs `runtime_write_tx`, broker, and worker proving hint floods never block reliable commands or heartbeats.
  - Worker injects `params._meta["cowboy.dev/attemptNonce"]` (including fresh nonce on retry) and carries exact `TurnStarted` `turn_id` into `ActivePrompt`; proxy copies nonce synchronously, strips it before `agy`, and drops delayed hints with mismatched nonces.
  - Facade: hint during `RESERVED` preserved into facade turn; slow untracked root read finishing after a newer turn cannot consume the newer turn's hint; note attaches to first text block (including `[text, image]` `ReadFile`), appends only if no text block exists, runs after `onResult` once per turn without mutating `result`; `testdata/fake-agy.mjs` reads all content blocks.
  - Shared `COWBOY_WS` compaction tests (`U+0085`, `U+FEFF`, split blocks, mixed blocks) including `prepare_dispatch_preflight` leaving pending context links unclaimed for split-block `/compact` through Worker and Proxy (including reconnect).
- **Root Stop bypass, per-attempt cleanup barrier & Stop-vs-Prompt ordering:**
  - `proxy.mjs`: `session/cancel` bypasses `this.queue`; cancel in same chunk AND cancel arriving while blocked in `await evidence.mark()` / `await recall()` settles reservation once without starting `agy`; root Stop with blocked `facade.endTurn` forwards `session/cancel` immediately while `fromAgent` reply and `facade.settled()` join the same `turn.attemptCleanup`; Stop and natural completion AFTER `proxy.correct()` starts a correction attempt run fresh attempt-2 cleanup!
  - `remote_runtime.rs` & `machine_broker.rs`: `Cancel` preempts unrelated-session backlog, retires only `!attempted_sends` same-session prompts (settling tracking/queue guards cleanly without auto-requeue), preserves FIFO when already in `attempted_sends`, withdraws broker-held prompts during generation drain & reconnect without auto-requeue, tracks `shared.sent` declaration versions with supersession & reconnect replay through `reset_admission`.
- **Web (W1):** `DeliveryPicker` gates **"Wrap up & send next"** on busy + live target + `!paused` + `!editing` + current context + `wrap_up_steering` (Composer only; queued rows keep Steer disabled for Antigravity); `wrapUpPrompt` calls `requireCurrentAgentContext` (never `sendWithContextNotice`), binds `(sessionId, target)` in outbox, uses bootstrap `wrap_up_submit` for rollback encoding, and passes `bun run typecheck && bun run lint`.
