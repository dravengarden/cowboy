# Antigravity Wrap-Up Steering (R1) — Queue-Backed Redesign (Revision 5)

Status: design, revision 5 (2026-11-24). Scoped strictly to **R1 (Owner wrap-up steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), and queued-row Steer promotion for Antigravity.

---

## 1. Context, Architectural Choice & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority. Any mechanism on `agy` 1.5.14 must wait for the active `session/prompt` to finish before dispatching the follow-up `session/prompt`.
- **Queue-Backed Wrap-Up Steering:**
  1. **Authoritative delivery reuses Cowboy's existing C-auth front-of-queue `sessions.queue` path** (`front = true`, `DeliveryOrigin::Owner`), committed through Cowboy's **acknowledged FIFO queue writer** (`Hub::pending_commit_lock` + `in_flight_pending_commits` + `send_acked_update_pending`) before returning HTTP `202` or confirming the WebSocket `cmid`. Zero new database tables, zero parallel receipt state machines, zero `not_confirmed` UX, and full reuse of multi-block echoes, `Edit`/`Delete`/`Clear`/`Retry`, and existing queue semantics.
  2. **Honest durability boundary:** durably committed, still-queued rows survive Controller crash/restart cleanly; once `drain_head` (`src/core/hub_queue.rs:279`) dequeues a row for worker dispatch, the dequeue-to-worker handoff follows Cowboy's existing ordinary prompt dispatch and `UndeliveredPrompt` recovery semantics.
  3. **Stateless, payload-free, target-fenced `WrapUpHint`:** when an Owner front-queues a wrap-up steer against a captured live, non-paused, non-editing, non-compacting Antigravity turn, the Controller sends a best-effort, **zero-owner-text** `WrapUpHint` over a dedicated connection-owned hint channel targeted to exact `(worker_epoch, turn_id)` -> `attemptNonce`. The proxy appends at most one fixed Cowboy-authored advisory note to the next root facade tool result asking the model to finish its current step briefly.
  4. **Normal clean turn boundary:** when turn 1 finishes, Cowboy runs ordinary `Ledger::finish_turn`, `Facade.endTurn`, and `Subagents.finish` cleanly, then `Hub::try_drain` immediately dispatches the front-queued Owner prompt as turn 2.
  5. **Root Stop latency fix:** bundled into P2 `proxy.mjs` so root `session/cancel` shares one idempotent turn-owned cleanup barrier with prompt completion, synchronously fences admission, and immediately forwards `session/cancel` to `agy` before awaiting cleanup.
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because `WrapUpHint` carries zero owner text and never controls queue admission or transcript echoes, an untrusted proxy can neither forge delivery nor inject Owner text.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), and queued-row Steer for Antigravity (§5).

---

## 2. Wire Contract (Protocol 52 / Runtime 8 — Strictly Additive)

Zero database migrations. `cowboy-wire` remains a leaf crate with zero dependency on `cowboy-store`.

- **Leaf `DeliveryTarget` (`components/wire`):** move/define `DeliveryTarget { worker_epoch: String, turn_id: String }` in `components/wire` (re-exported by `src/send_delivery.rs`) so both `CoreCommand::WrapUpHint` and `WorkerCommand::WrapUpHint` use the leaf wire type directly.
- **Machine protocol:** bump `PROTOCOL_VERSION` to **52** (`WRAP_UP_STEERING_PROTOCOL_VERSION = 52`), gated at the Machine tunnel boundary (`machine_cli.rs` / `machine_connection.rs`).
- **Runtime protocol (worker↔broker and broker↔Controller):** bump `PROTOCOL_VERSION` to **8** (`WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION = 8`).
- **Additive wire additions (serde-defaulted for mixed-version compatibility):**
  - `WorkerSnapshot`:
    - `wrap_up_steering: bool` (`#[serde(default)]`) — `true` only for Antigravity when all §3 gates hold; `false` for all other providers.
  - `CoreCommand::WrapUpHint` (Machine >= 52, runtime >= 8):
    `WrapUpHint { session_id: String, target: DeliveryTarget }`
  - `WorkerCommand::WrapUpHint` (runtime >= 8):
    `WrapUpHint { session_id: String, target: DeliveryTarget }`
- **Connection-owned fire-and-forget hint path (outside `pending`, replay, overflow refusal, and `CommandAck`):**
  - **Controller (`src/remote_runtime.rs:1755`):** `RemoteRuntime::connected` owns a bounded per-connection `hint_tx: mpsc::Sender<CoreCommand>` (capacity 32), installed on `Shared` only when connected with Machine >= 52 and runtime >= 8 and **dropped on disconnect** so stale hints never survive reconnect. `connected` polls `hint_rx` alongside `notify_rx` and serializes hints through the sole framed writer; `try_send` silently drops on saturation or disconnect.
  - **Broker (`machine_broker.rs:4337`) & Worker (`worker.rs`):** handle `WrapUpHint` on an explicit silent-drop path **outside** `pending_commands`, command-queue overflow refusal, replay, and `CommandAck` handling (using bounded `try_send`, silently dropping if saturated, stale, or unsupported — never dropping the Controller connection and never emitting a `CommandAck`).

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

All capability surfaces — **`Frame::Welcome` initial snapshots**, **`Frame::Snapshot`**, **cached worker state**, and **`Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)`** — are sanitized using the **negotiated protocol versions of the connection being installed** (never reading stale pre-install `Shared.protocol` or pre-install Controller leases!):

- **Broker (`machine_broker.rs:298,:664`):**
  - Stores `negotiated_protocol: u32` inside `WorkerPeer` and retains raw worker capability observations (`raw_wrap_up_steering`, `raw_supports_current_turn`, `raw_next_checkpoint`) separately from projected values.
  - Passes the new Controller connection's negotiated `(machine_protocol, runtime_protocol)` explicitly when building `Frame::Welcome` snapshots (before installing the Controller lease!).
  - Fails closed when `launch.provider` cannot be established (`wrap_up_steering = false`, `supports_current_turn = false`, `next_checkpoint = false`).
  - For `provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; projects `wrap_up_steering = raw_wrap_up_steering && worker.negotiated_protocol >= 8 && controller_runtime_protocol >= 8 && controller_machine_protocol >= 52`.
- **Controller (`remote_runtime.rs:464,:1732`, `src/core/`, `src/config/schema.rs`):**
  - Passes the incoming connection's negotiated `(machine_protocol, runtime_protocol)` explicitly into initial `Frame::Welcome` snapshot adoption (before/alongside setting `Shared.protocol`) and preserves raw worker capability observations so reconnects and config reloads recompute effective projections accurately.
  - Adds Service config key `providers.antigravity.wrap_up_steering` (boolean, default `true`, omitted from Service TOML unless overridden; see §6.2 for mandatory TOML removal before Controller rollback).
  - For `session.meta.provider == "antigravity"`: clamps `supports_current_turn = false` AND `delivery_controls.next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and `providers.antigravity.wrap_up_steering == true`.

### 3.3 Mixed-Version Rollout Matrix (Keyed to Verified Running Launch)

Existing sessions retain their active launch digest until idle adoption (`session_provider_updates.rs:1`), so matrix rows apply to each session's **verified running launch** (not merely whether P2 is installed on disk):

| Controller | Machine | Running Worker | Running Antigravity Launch | `supports_current_turn` / `next_checkpoint` | `wrap_up_steering` |
|---|---|---|---|---|---|
| Pre-R1 | Any | Pre-8 (retained) | Pre-P2 (`< 0.2.35`) | Unchanged pre-R1 behavior (requires R1 worker OR P2 launch to strip upstream `_meta.steering`) | N/A |
| Pre-R1 | Any | Any | Verified P2 (`>= 0.2.35`, strips `_meta.steering`) | `false` / `false` | N/A |
| R1 (52/8) | Pre-52 | Any | Verified P2 (`0.2.35`, compatible schema) | `false` / `false` | `false` (Machine < 52) |
| R1 (52/8) | 52/8 | Pre-8 (retained) | Verified P2 (`0.2.35`) | `false` / `false` | `false` (worker < 8) |
| R1 (52/8) | 52/8 | 8 (R1 clamp) | Pre-P2 (`< 0.2.35`, even if upstream spoofs flags) | `false` / `false` | `false` (version < 0.2.35) |
| R1 (52/8) | 52/8 | 8 | Verified P2 (`0.2.35`), config OFF | `false` / `false` | `false` |
| R1 (52/8) | 52/8 | 8 | Verified P2 (`0.2.35`), config ON | `false` / `false` | `true` (Owner only) |

---

## 4. Admission, Acknowledged Queue Commit, Compaction Fence, Hint & Stop Fixes

### 4.1 Explicit Hub Outcome, Acknowledged Commit & Caller Routing

- **Explicit Hub admission outcome (`src/core/hub_queue.rs:683`):**
  - Replace reliance on `force_submit_once(..., false, ...)`'s interrupt-flag boolean with a dedicated acknowledged Hub method returning:
    ```rust
    pub enum FrontSubmitOutcome {
        Duplicate,
        DispatchedImmediately,
        QueuedFront,
    }
    ```
    preserving legacy `force_submit_once` unchanged for `Interrupt` callers.
  - For wrap-up admission (both HTTP and WS), hold `Hub::pending_commit_lock(session_id)`, install `in_flight_pending_commits`, enqueue an **acknowledged FIFO `StoreWrite::UpdatePending`** (`send_acked_update_pending`), and **await store ACK before mutating Hub/returning HTTP `202`/confirming WS `cmid`**! If the store rejects/fails, abort without mutating Hub or sending a hint (`503` / cmid-addressed WS error).
- **Native mid-turn delivery (`delivery: "current_turn"` / `"next_checkpoint"`):**
  - Remains strictly bound to `supports_current_turn == true` (refused cleanly on Antigravity: `not_delivered`).
- **HTTP `POST /api/sessions/{id}/prompt` (`SendDelivery::WrapUp` = `"wrap_up"`):**
  - Mutually exclusive with `force: true` and `front: true` in `SendDelivery::resolve`.
  - **Owner-only:** requires `matches!(caller, RequestCaller::Person(_))`; `Agent`, `Automation`, and `Cowboy` receive `403 Forbidden`.
  - Requires a validated authored `operation` ID (when supplied: must pass `Hub::is_authored_cmid` and reject all synthetic prefixes including `call-notice:`, `__cont__`, `__wake__`, `__sched__`, `cowboy-`).
  - **Withdrawal-safe HTTP idempotency:** reuses existing `agent_api_operations` (`admit_agent_operation` / `finish_agent_operation`, same as `send_delivery.rs:451`, zero new DB tables!) keyed by `(owner, session, "wrap_up:{operation}")` and canonical `(caller, delivery, text, content, target)` hash — so even if the Owner later deletes the queued row before the HTTP client observes confirmation, a retry returns the saved `202` receipt without re-enqueueing or re-hinting!
  - **Target capture BEFORE enqueueing:** uses `req.target` if supplied, otherwise samples `supervisor.delivery_target(&session_id)` **before** calling `admit_front_prompt_acked` (never after!).
  - Returns `202 Accepted` with `{ status: "queued" | "dispatched" | "already_accepted", delivery: "wrap_up", hinted: bool }` (firing §4.2 hint ONLY when outcome is `QueuedFront`).
- **WebSocket `Inbound::Submit` (`src/core.rs:904`, `client_ws.rs:707`):**
  - Keeps the existing `Inbound::Submit` tag (no new WS command tag!) and adds two optional serde-defaulted fields:
    - `#[serde(default)] wrap_up: bool`
    - `#[serde(default)] wrap_up_target: Option<DeliveryTarget>`
  - **Rollback encoding:** Web sends `wrap_up: true` and `wrap_up_target` **only** when the connected Controller advertises `session.wrap_up_steering === true`; on an older/rolled-back Controller where `wrap_up_steering` is absent/false, Web replays any pending outbox entry with the exact same `cmid` as ordinary `Inbound::Submit { front: true }` (omitting `wrap_up` and `wrap_up_target`), avoiding unaddressed decoder errors!
  - Routes through `Submit`'s full existing session authorization, system/context checks, and Plugin lifecycle fence, adds an explicit `caller.is_person()` check for `wrap_up: true` returning a **`cmid`-addressed `CommandResult` refusal** for non-Owners, commits via the acknowledged FIFO queue writer before emitting `confirm_delivered`, and fires §4.2 hint ONLY on `QueuedFront` against `wrap_up_target` (or pre-enqueue captured target).
  - **WS withdrawal idempotency boundary:** for WebSocket `Submit`, `cmid` dedup holds while the row remains queued, in-flight, or in the transcript (matching existing Cowboy queue `Submit` semantics); carrying `wrap_up_target` captured at gesture time ensures a replayed browser submission arriving during a later turn NEVER hints that later turn!
- **Agent API & CLI (`agent_api/control.rs:788,:1046`, `cli/src/product_cli/session_prompt.rs:70`):**
  - `agent_api/control.rs:788`: explicitly reject `SendDelivery::WrapUp` alongside `CurrentTurn | NextCheckpoint` (`Err("invalid_argument")`).
  - `agent_api/control.rs:1046` (`delivery == "session_default"`): when `info.meta.provider == "antigravity"`, map `session_default` to `"next_turn"` (preserving existing `Steer -> current_turn` mapping for Claude/Codex!).
  - `cli/src/product_cli/session_prompt.rs:70`: for `SendDelivery::WrapUp`, read `delivery-options` best-effort and pass `options["target"]` when present or `Value::Null` when idle (so the idle dispatch path stays reachable from CLI!).

### 4.2 Target-Fenced `WrapUpHint` & Turn-Bound Facade Delivery

Controller attempts `supervisor.send_wrap_up_hint(&session_id, captured_target)` **only when ALL hold**:
- Admission returned `FrontSubmitOutcome::QueuedFront`
- `captured_target` is `Some(target)` AND `supervisor.delivery_target(&session_id).as_ref() == Some(&target)` (turn has NOT changed since capture!)
- `info.meta.provider == "antigravity"` AND `info.meta.wrap_up_steering == true` AND live config `providers.antigravity.wrap_up_steering == true`
- `info.meta.status == Status::Busy` AND `!info.meta.paused` AND **`s.editing.is_none()`** (`hub_queue.rs:197` queue-wide edit hold!) AND `!hub.clear_pending(&session_id)` AND `hub.delivery_hold(&session_id).is_none()`
- `!hub.is_compacting(&session_id)` AND `!hub.active_turn_is_compacting(&session_id)`.

Across hops:
1. **Controller -> Broker -> Worker (`src/worker.rs:992,:1086`, `src/acp.rs`):**
   - `worker.rs` carries the exact runtime `turn_id` published in `TurnStarted` inside `AgentCommand::Prompt` into `ActivePrompt { turn_id, attempt_nonce, compacting, ... }` (so `active_prompt.turn_id` is exact and never guessed from mutable snapshot state!).
   - On `WorkerCommand::WrapUpHint { session_id, target }`: worker checks `effective_wrap_up_steering`, `!managed_read_only`, `worker_epoch == target.worker_epoch`, `active_prompt.turn_id == target.turn_id`, `!active_prompt.compacting`, `!cancelled`, and sends payload-free ACP notification `cowboy/wrapUpHint { sessionId: acp_id, attemptNonce: active_prompt.attempt_nonce }` to `agy-proxy`.
2. **Proxy & Turn-Bound Facade Delivery (`proxy.mjs`, `facade.mjs:193,:299`):**
   - `proxy.mjs` creates ONE shared `wrapUp = { pending: false, delivered: false }` latch inside the synchronous `RESERVED` turn object (`fromClient`) and passes that exact latch reference into `runtime.facade.beginTurn(sessionId, { wrapUp })` (so a hint arriving during `RESERVED` is preserved into the active facade turn!).
   - `cowboy/wrapUpHint` bypasses `this.queue` synchronously; accepted ONLY when `turn` exists for `sessionId`, `turn.attemptNonce === attemptNonce`, `!this.managed`, `!this.subagents.owns(sessionId)`, `!turn.compacting`, `!turn.cancelled`, `!turn.structured`, and `!turn.wrapUp.delivered` -> sets `turn.wrapUp.pending = true`.
   - Inside `facade.mjs:call(sessionId, name, args)`: **captures `const callTurn = session?.turn` at tool-call start**. After `this.onResult(call, result)` runs (keeping Cowboy UI presentation clean), checks **all of**:
     - `session.parent === undefined` (root session only, never subagents) AND `!this.managed`
     - `callTurn !== undefined && session.turn === callTurn` (a slow untracked root read finishing after a newer turn opened can NEVER consume the newer turn's hint!)
     - `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`
     - `callTurn.wrapUp?.pending === true && callTurn.wrapUp.delivered === false`
     - `name !== "UpdatePlan"` AND `!result?.isError` AND `Array.isArray(result?.content)`.
   - Atomically sets `callTurn.wrapUp.pending = false; callTurn.wrapUp.delivered = true;` (strictly once per turn!) and returns a **non-mutating copy** appending `WRAP_UP_NOTE = "[Cowboy note: the owner asked you to wrap up your current step briefly so they can follow up next.]"` (honest wording that never claims a specific message is still queued if withdrawn/reordered!).
   - **Block attachment for real `agy` compatibility:** if the last entry in `result.content` has `type === "text"`, clone `result.content` with `"\n\n" + WRAP_UP_NOTE` appended to that last text block (ensuring even single-block `content[0]` consumers see it); otherwise (e.g. image-only `ReadFile`) append `{ type: "text", text: WRAP_UP_NOTE }`. Update `testdata/fake-agy.mjs:75` to read all content blocks and verify with the pinned real `agy` canary (§6.1).

### 4.3 Shared Compaction Fence & Context Preamble Alignment (`COWBOY_WS`)

- **Closed code-point set `COWBOY_WS`** implemented identically in Rust (`is_cowboy_ws`) and JS (`compaction.mjs:isCowboyWs`), explicitly excluding `U+0085` and including `U+FEFF`:
  `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF`.
- **`is_compaction_prompt(blocks)`:** applies `build_prompt_blocks(text, content)` first; returns `true` iff non-empty, every block is `type == "text"`, and `COWBOY_WS`-trimmed concatenation matches ASCII case-insensitive `/compact` followed by EOF or `COWBOY_WS`.
- **Context preamble alignment (`src/supervisor.rs:693`, `src/session_context.rs:258,:469`):** update `session_context::prompt_content` / `is_command` to use the **shared `is_compaction_prompt` / `COWBOY_WS` detector** so `/compact` prompts (including `U+FEFF` and split blocks) NEVER receive a prepended `<system-reminder>` block that would hide `/compact` from Worker and Proxy!
- **Turn-scoped latch:** Controller latches `active_turn_compacting` on dispatch; Worker latches `active_prompt.compacting`; Proxy computes `turn.compacting` synchronously inside `fromClient` when reserving the `RESERVED` turn object before `this.queue.then(...)` and before any `await`.

### 4.4 Root Stop Barrier, Final Pre-Write Fence & `send_pending` Dependency Rules

1. **Proxy single reservation object + final pre-write fence (`proxy.mjs:653,:682`):**
   - Create ONE session-scoped `turn` object synchronously in `fromClient` (`state: "RESERVED"`, holding `id`, `attemptNonce`, `cancelGen`, `compacting`, `wrapUp`, `cancelled: false`, `cleanupPromise: null`, `finalized: false`) and **populate THAT SAME object** through `await evidence.mark()` and `await recall()` (never overwriting it with a fresh `this.turns.set(...)`).
   - Recheck `turn.cancelled || this.cancelGen.get(sessionId) !== turn.cancelGen || this.killed` **after every `await` AND immediately before `this.agent(forwarded)`**. If cancelled before writing to `agy`, settle that reservation **exactly once** as `{ stopReason: "cancelled" }` without writing to `agy`.
2. **Proxy unified turn-owned cleanup barrier (`proxy.mjs:756-781,:1152`, `facade.mjs:212,:277`):**
   - Give every turn ONE idempotent `startTurnCleanup(sessionId, turn, { cancelled })` helper that synchronously fences `facade` + `subagents` admission and memoizes `turn.cleanupPromise = Promise.all([facade.endTurn(...), subagents.finish(...)])`.
   - `facade.mjs` tracks active `endTurn` cleanup promises even after `session.turn` is detached so `facade.settled()` awaits them during graceful shutdown!
   - Root `cancel()` calls `startTurnCleanup(sessionId, turn, { cancelled: true })` synchronously (using cached `this.readyRuntime` when resolved), **immediately forwards `this.agent(message)` (`session/cancel`) to `agy`**, and then awaits `turn.cleanupPromise`. When `agy`'s prompt reply arrives in `fromAgent`, it joins **that exact same `turn.cleanupPromise`** before forwarding the reply!
3. **Controller `RemoteRuntime::send_pending` (`remote_runtime.rs:1169,:1516,:1873`, `reset_admission.rs:35`):**
   - Track sent declarations by **version**: `shared.sent: HashMap<String, u64>` (`key -> sent_version`). Record `required_declaration_version` on each reset `StopSession(reset-*)` and make it eligible ONLY when `shared.sent.get(&ensure_key) == Some(&required_version)`, preserving `reset_admission::current` / `denied` MCP-plan deferral and denial rules.
   - **Session-scoped Stop vs Prompt independence:**
     - `Cancel` / `CancelTurn` preempts pending commands for **unrelated sessions** immediately via single-command re-checked selection.
     - For the **same `session_id`**: when root `Cancel` is admitted, atomically retire any `CoreCommand::Prompt` in `shared.pending` that was **never written to transport** (`!shared.sent.contains_key(&key)`). If an earlier `CoreCommand::Prompt` for the same `session_id` remains unwithdrawn (or transport handoff may have occurred), preserve FIFO ordering between that same-session `Prompt` and `Cancel` so Stop can NEVER overtake a prompt it is meant to stop!

---

## 5. HTTP/WS API & Web UI Flow (W1)

- **`GET /api/sessions/{id}/delivery-options` (`send_delivery.rs`):**
  - Exposes `"steer": info.meta.supports_current_turn` (always `false` for Antigravity), `"wrap_up_steering": info.meta.wrap_up_steering`, `"editing": hub.is_editing(&session)`, plus existing `target`, `controls` (with `next_checkpoint: false` for Antigravity), `status`, `paused`, `hold`, `policy`.
- **Web (`web/src/protocol.ts`, `DeliveryPicker.tsx`, `Composer.tsx`, `QueuedDeliveryAction.tsx`, `store.ts`):**
  - `SessionInfo` adds `wrap_up_steering?: boolean`.
  - `DeliveryPicker.tsx` (Composer only, `!props.queued`):
    - Enables **"Wrap up & send next"** (*"Asks agent to finish current step, then sends next"*) **only when ALL hold**: `!options.steer`, `props.busy`, `target != null`, `!props.paused`, `!options?.editing`, current context ready (no pending `next_agent` context change), and `(fresh ? Boolean(options?.wrap_up_steering) : Boolean(props.wrapUpReady))`. Preferred default when `policy.mode === "steer"`.
    - Calls `wrapUpPrompt(sessionId, text, attachments, target)` -> existing `qAdd("queue", ..., { mode: "front", wrapUp: true, wrapUpTarget: target, origin: "composer" })` (reusing `sendWithContextNotice` / `requireCurrentAgentContext`, IndexedDB durability, optimistic queue row, and rollback-safe `Inbound::Submit` encoding from §4.1!).
  - `QueuedDeliveryAction.tsx` (`props.queued === true`): keeps Steer disabled when `!options.steer` (only `Queue first`, `Send now`, edit, and delete available — zero Peer/Automation origin promotion!).

---

## 6. Release Sequencing, Buildable Conformance Set & Rollback Boundaries

### 6.1 Slice Sequencing & Exact Buildable Artifacts

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller):**
   - Lands on `main` with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`). Strict M1/P2 boundary: M1 commits do **not** touch `plugins/antigravity/` manifests unless signed coverage is already published (`just provider-release-coverage`).
   - **Zero DB migrations.** Activate **Controller** first (keeping `providers.antigravity.wrap_up_steering` omitted from Service TOML so default `true` applies without introducing an unknown TOML key on rollback).
   - **Buildable 5-artifact Machine set (built on Hawk per OVH runtime-only contract):**
     - **Hawk Machine (4 artifacts sharing the same newly accepted post-wire worker pin, per `tools/session_deletion_production_conformance.py:184` and `flake.nix:1012`):**
       1. `--old-writer`: first post-wire-52/8 writer revision built against the new worker pin
       2. `--new-writer`: second distinct post-wire-52/8 writer revision (distinct commit + ELF hash) built against the **same** new worker pin
       3. `--reader-fallback`: matching reader-only fallback release on the same new worker pin
       4. `--default-reader`: separate default-reader release input
       *(Record all 4 immutable store roots and a fresh `just session-deletion-production-conformance` receipt before activation.)*
     - **OVH Machine (1 artifact, `tools/ovh-machine-host-release.sh:36-39`):**
       5. Plain `.#cowboy-machine-release` deployed via `tools/ovh-machine-host-release.sh --workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` `1.0.27` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Verified to install and launch on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap.
4. **Bound R1 Acceptance Canary (Real `agy` Verification):**
   - After pin-advance acceptance verifies target worker generation on Hawk and OVH, run a live canary on a real Antigravity session bound to exact candidate Controller, Machine, Worker, and Plugin `0.2.35` digests confirming: negotiated Machine protocol `52`, both runtime legs `8`, `supports_current_turn == false`, `delivery_controls.next_checkpoint == false`, `wrap_up_steering == true`, **an observation from pinned real `agy` 1.5.14 proving the model received and acted on the appended wrap-up note**, immediate drain of the front-queued Owner prompt after `TurnEnd`, and root Stop reaching `agy` immediately during an active facade/extension call.

### 6.2 Accurate Component Rollback Boundaries

- **Advisory hints (no drain precondition):** disabling new `WrapUpHint`s (config override off) requires no receipt drain because hints are stateless and queued Owner messages live in `sessions.queue`.
- **Controller rollback (mandatory TOML cleanup):** because unknown Service TOML keys are fatal at startup (`src/config.rs:575,:769`), if an operator ever wrote `providers.antigravity.wrap_up_steering` into Service TOML, rollback **MUST** remove that key (and any empty `[providers.antigravity]` / `[providers]` tables) and validate the config file with the rollback Controller binary before activating rollback! Web rollback encoding (§4.1) ensures replayed outbox sends downgrade cleanly to `Inbound::Submit { front: true }`.
- **Hawk Machine rollback:** after worker-pin cutover, recovery MUST target a tested reader-only/default-reader Machine release matching the candidate's exact worker generation (`docs/plugin-session-deletion-journal.md:81`), not the pre-pin Machine binary.
- **Worker & Plugin rollback:** remains governed by Cowboy's graceful turn-settlement boundaries (idle adoption / busy safe-boundary cutover; forced stops can leave in-flight replies uncertain per `docs/uncertain-effect-recovery.md:322`).

---

## 7. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify `DeliveryTarget` in `cowboy-wire` creates zero crate cycles; test P2 `0.2.35` install/launch on pre-R1 `PluginManifest` / `ProviderPackage` decoders.
- **Mixed-version, handshake & rollback matrix:** cover all rows of §3.3 (including surviving retained workers across Controller rollback!), explicit protocol passing during `Frame::Welcome` adoption before `Shared.protocol` / Controller lease installation, `WorkerPeer.negotiated_protocol` retention, `delivery_controls.next_checkpoint == false` clamping, missing `launch.provider` fail-closed at broker, and config-reload recomputation from raw observations.
- **Acknowledged admission, idempotency & caller routing:**
  - HTTP `delivery: "wrap_up"` and WS `Inbound::Submit { front: true, wrap_up: true }`: commit via acknowledged FIFO `UpdatePending` before `202` / `confirm_delivered` (simulated store failure returns `503` / cmid-addressed WS error with zero Hub mutation and zero hint); `FrontSubmitOutcome::QueuedFront` triggers hint while `DispatchedImmediately` / `Duplicate` do not; `Agent`/`Automation`/`Cowboy` rejected (`403` / cmid-addressed WS error); HTTP withdrawal-safe idempotency via `agent_api_operations`; WS rollback downgrade omitting `wrap_up` on pre-R1 Controller; Agent API rejects `wrap_up` and maps Antigravity `session_default` -> `next_turn`; CLI `wrap_up` works on both busy and idle sessions.
- **Target fencing, turn-bound facade note & compaction preamble alignment:**
  - Controller pre-enqueue target capture drops hint if turn ends before enqueue; worker carries exact `turn_id` from `TurnStarted` into `ActivePrompt`; `s.editing.is_some()` / paused / clearing / compacting suppresses hint.
  - Proxy/facade: hint arriving during `RESERVED` is preserved into the facade turn; slow untracked root read finishing after a newer turn opened cannot consume the newer turn's hint; note appends once per turn after `onResult` (never on errors, `UpdatePlan`, or subagents) without mutating `result`; `testdata/fake-agy.mjs` reads all content blocks.
  - Shared Rust/JS `COWBOY_WS` compaction tests (`U+0085`, `U+FEFF`, split blocks, mixed blocks) including end-to-end `session_context::prompt_content` preamble suppression through Worker and Proxy (including reconnect).
- **Root Stop barrier, pre-write fence & `send_pending` dependency tests:**
  - `proxy.mjs`: cancel in same stdin chunk AND cancel arriving mid-preparation (during `await evidence.mark` / `await recall`) settles reservation once without starting `agy`; root Stop with blocked `facade.endTurn` forwards `session/cancel` to `agy` immediately while `fromAgent` reply and `facade.settled()` join the exact same `turn.cleanupPromise`.
  - `remote_runtime.rs`: `Cancel` preempts unrelated-session backlog, retires unsent same-session `Prompt` (and preserves ordering if already handed to transport), tracks `shared.sent` declaration versions so `StopSession(reset-*)` never runs against stale `EnsureSession` metadata, and preserves `reset_admission.rs` deferral/denial rules.
- **Web (W1):** `DeliveryPicker` gates **"Wrap up & send next"** on busy + live target + `!paused` + `!editing` + current context + `wrap_up_steering` (Composer only; queued rows keep Steer disabled for Antigravity), `wrapUpPrompt` outbox + rollback encoding, and `bun run typecheck && bun run lint`.
