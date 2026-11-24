# Antigravity Wrap-Up Steering (R1) — Queue-Backed Redesign (Revision 4)

Status: design, revision 4 (2026-11-24). Scoped strictly to **R1 (Owner steering for Antigravity)** on top of **C-auth** (`6be003ec6`, live on Hawk Controller). Explicitly excludes C-budget (`session_owner_turns` / B), Plan Mode (`R3`), and queued-row Steer promotion for Antigravity.

---

## 1. Context, Architectural Choice & Trust Model

- **`agy` 1.5.14 (`google-antigravity-cli`) has no native mid-turn steering RPC.** It accepts user-role input only via `session/prompt`; tool results carry no owner authority. Any mechanism on `agy` 1.5.14 must wait for the active `session/prompt` to finish before dispatching the follow-up `session/prompt`.
- **Why Queue-Backed Wrap-Up Steering (replaces same-turn continuation):**
  - Earlier drafts attempted to hide the `TurnEnd` boundary between two `session/prompt` calls inside one Cowboy turn. Across multiple design reviews, hiding that boundary proved structurally unsound on `agy` 1.5.14 because it required a parallel receipt store, attempt-scoped ledger reopening across an untrusted sandbox boundary, multi-block echo reconciliation, and rollback drain gates — solely to hide a turn boundary that `agy` itself already crosses.
  - **Redesigned R1 mechanism:**
    1. **Authoritative delivery reuses Cowboy's existing C-auth front-of-queue `sessions.queue` path** (`front = true`, `DeliveryOrigin::Owner`). Zero new database tables, zero parallel receipt state machines, zero `not_confirmed` ambiguity, zero manual `send-next` recovery, and full reuse of existing `cmid` idempotency, multi-block echoes, `Edit`/`Delete`/`Clear`/`Retry`, and restart/disconnect recovery.
    2. **Stateless, payload-free Wrap-Up Hint:** when an Owner front-queues a wrap-up steer against a live, non-paused, non-compacting Antigravity turn, the Controller sends a best-effort, **zero-owner-text** `WrapUpHint` targeted to `(worker_epoch, turn_id)` -> `attemptNonce`. The proxy appends at most one fixed Cowboy-authored advisory note to the next root facade tool result asking the model to finish its current step briefly so the queued owner message can run next.
    3. **Normal clean turn boundary:** when turn 1 finishes, Cowboy runs ordinary `Ledger::finish_turn`, `Facade.endTurn`, and `Subagents.finish` cleanly, then existing `Hub::try_drain` immediately dispatches the front-queued Owner prompt as turn 2.
    4. **Root Stop latency fix:** bundled cleanly into P2 `proxy.mjs` so root `session/cancel` synchronously fences admission and immediately forwards `session/cancel` to `agy` before awaiting facade/subagent cleanup (matching the existing subagent cancel path).
- **Trust domains:**
  - **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`). C-auth (`6be003ec6`) establishes `RequestCaller` (`Person` / `Automation` / `Agent` / `Cowboy`) from verified credentials and persists authoritative `DeliveryOrigin` in `prompt_dispatches`.
  - **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text, delegated Agent API peers, and legacy components below Machine protocol 52 / runtime protocol 8. Because `WrapUpHint` carries **zero owner text** and never controls queue admission or transcript echoes, an untrusted proxy can neither forge delivery, inject Owner text, nor corrupt queue/echo state.
  - **Out of scope:** Same-UID local processes on Controller/Machine hosts (existing C-auth boundary), C-budget (`session_owner_turns`), Plan Mode (`R3`), and queued-row Steer for Antigravity (§6.3).

---

## 2. Wire Contract (Protocol 52 / Runtime 8 — Strictly Additive)

Zero database migrations. Zero changes to `components/store` dependencies (`cowboy-wire` remains a leaf crate with zero dependency on `cowboy-store`).

- **Machine protocol:** bump `PROTOCOL_VERSION` to **52** (`WRAP_UP_STEERING_PROTOCOL_VERSION = 52`).
- **Runtime protocol (worker↔broker and broker↔Controller):** bump `PROTOCOL_VERSION` to **8** (`WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION = 8`).
- **Additive wire additions (all serde-defaulted for mixed-version compatibility):**
  - `WorkerSnapshot`:
    - `wrap_up_steering: bool` (`#[serde(default)]`) — `true` only for Antigravity when all §3 gates hold; `false` for all other providers.
  - `CoreCommand::WrapUpHint` (Machine >= 52, runtime >= 8):
    ```rust
    WrapUpHint {
        command_id: String,
        session_id: String,
        target: crate::send_delivery::DeliveryTarget, // exact { worker_epoch, turn_id }
    }
    ```
  - `WorkerCommand::WrapUpHint` (runtime >= 8):
    ```rust
    WrapUpHint {
        command_id: String,
        session_id: String,
        target: crate::send_delivery::DeliveryTarget,
    }
    ```
  - `CoreCommand::WrapUpHint` is **fire-and-forget / best-effort**: sent only on a live connected Machine speaking protocol >= 52 / runtime >= 8, **never** queued in `RemoteRuntime.pending` for reconnect replay, and **never** sent to pre-52 Machines or pre-8 retained workers. If dropped on disconnect or turn change, the Owner's message remains safely queued at the front of `sessions.queue`.

---

## 3. Provider-Identity, Compatible Signed Binding & Mixed-Version Gate

### 3.1 Compatible Signed Binding (Zero Unknown Manifest Fields)

`PluginManifest` (`components/plugin-sdk/src/lib.rs:72-83`) and `ProviderPackage` / `RuntimeCapability` (`components/provider-sdk/src/lib.rs:738-739`) use `#[serde(deny_unknown_fields)]`. To keep P2 packages 100% installable and launchable on older/rollback Machines:

- **No new fields or enum variants** are added to `PluginManifest`, `ProviderPackage`, or `RuntimeCapability`.
- **Signed launch binding:** P2 ships on the existing signed manifest surface as Antigravity Plugin **`version = "0.2.35"`** (`cowboy-antigravity-acp` `1.0.27`). At worker launch, the broker verifies the signed installed release and supplies authoritative `launch.provider`, `launch.provider_version`, and `launch.provider_generation_digest` to the worker (never read from the untrusted proxy).
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
    1. **Always hard-force** `supports_current_turn = false` and `supports_next_checkpoint = false`, regardless of any upstream `_meta.steering`.
    2. Compute `raw_wrap_up_eligible = true` **iff ALL FOUR hold**:
       - `launch.provider == "antigravity"`
       - `!state.sink.session_is_managed_read_only(&session_id)`
       - Broker-verified signed `launch.provider_version` parses as semver `>= 0.2.35` AND `launch.provider_generation_digest` is present (non-empty)
       - Top-level `init.meta["cowboy.dev/wrapUpSteering"]` has `v == 1` and `adapter == "cowboy-antigravity-acp"`.
    3. Compute `effective_wrap_up_steering = raw_wrap_up_eligible && peer_protocol.load() >= 8`.
  - Because the ACP thread can finish `initialize` before `worker.rs` negotiates `Frame::Welcome`, `worker.rs` stores `raw_wrap_up_eligible` on `Shared` and **recomputes `effective_wrap_up_steering` on every `Frame::Welcome` and reconnect**, updating its cached `WorkerSnapshot` and emitting refreshed `Frame::Snapshot` + `CURRENT_TURN_CAPABILITIES_UPDATE` (`"currentTurn": false`, `"nextCheckpoint": false`, `"wrapUpSteering": bool`).
  - For all non-Antigravity providers (`codex`, `claude`, etc.): `wrap_up_steering = false`; existing `supports_current_turn` behavior is unchanged.

### 3.2 Complete Capability Sanitization Across All Surfaces

Every capability surface — **`Frame::Welcome` initial snapshots**, **`Frame::Snapshot`**, **cached worker state**, and **`Frame::WorkerEvent(CURRENT_TURN_CAPABILITIES_UPDATE)`** — is sanitized identically at Broker (`machine_broker.rs:664-671`) and Controller (`remote_runtime.rs:1594-1600`):

- **Broker (`machine_broker.rs`):**
  - Resolves authoritative `provider` from `self.sessions` / `snapshot.launch.provider`. If `provider` cannot be established: **fail closed** (`wrap_up_steering = false`, never advertising `supports_current_turn = true`).
  - For `provider == "antigravity"`: clamps `supports_current_turn = false`, `supports_next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when worker↔broker runtime protocol `>= 8`, broker↔Controller runtime protocol `>= 8`, and Machine protocol `>= 52`. Recomputes cached snapshots on reconnect/negotiation.
- **Controller (`remote_runtime.rs`, `src/core/`, `src/config/schema.rs`):**
  - Adds Service config key `providers.antigravity.wrap_up_steering` (boolean, default `true`, documented in checked-in service config example).
  - For `session.meta.provider == "antigravity"`: clamps `supports_current_turn = false`, `supports_next_checkpoint = false`; permits `wrap_up_steering = true` ONLY when Machine protocol `>= 52`, broker↔Controller runtime protocol `>= 8`, `!hub.is_managed_child(session_id)`, and `providers.antigravity.wrap_up_steering == true`. Recomputes session capabilities on handshake, reconnect, and config reload.

### 3.3 Mixed-Version Rollout Matrix

| Controller | Machine | Worker | Installed Antigravity Plugin | `supports_current_turn` | `wrap_up_steering` |
|---|---|---|---|---|---|
| Pre-R1 | Any | Any | P2 (`0.2.35`, compatible schema, strips `_meta.steering`) | `false` | N/A (`false`) |
| R1 (52/8) | Pre-52 | Any | P2 (`0.2.35`, installs/launches cleanly on old Machine) | `false` | `false` (Machine < 52) |
| R1 (52/8) | 52/8 | Pre-8 (retained) | P2 (`0.2.35`) | `false` | `false` (worker↔broker < 8) |
| R1 (52/8) | 52/8 | 8 | Pre-P2 (`< 0.2.35`, even if upstream `agy` spoofs flags) | `false` | `false` (signed version < 0.2.35) |
| R1 (52/8) | 52/8 | 8 | P2 (`0.2.35`), config gate OFF | `false` | `false` (config gate off) |
| R1 (52/8) | 52/8 | 8 | P2 (`0.2.35`), config gate ON | `false` | `true` (Owner only) |

---

## 4. Admission, Compaction Fence, Wrap-Up Hint & Root Stop Fix

### 4.1 Caller Authority & Honest Delivery Semantics

- **Native mid-turn delivery (`delivery: "current_turn"` / `"next_checkpoint"`):**
  - Remains strictly bound to `supports_current_turn == true` (therefore refused cleanly on Antigravity: `not_delivered`, `"Antigravity does not support native mid-turn injection; use wrap_up or next_turn"`). Preserves exact semantics for CLI, post-edit diagnostics, and Agent API callers.
- **Explicit HTTP Wrap-Up Steer (`POST /api/sessions/{id}/prompt` with `delivery: "wrap_up"`):**
  - Adds `SendDelivery::WrapUp` (`"wrap_up"`) in `src/send_delivery.rs` (mutually exclusive with `force: true` and `front: true` in `SendDelivery::resolve`).
  - **Owner-only:** requires `matches!(caller, RequestCaller::Person(_))`; `Agent`, `Automation`, and `Cowboy` receive `403 Forbidden` (`"wrap_up steering requires the human owner"`).
  - Runs all ordinary prompt admission checks (`!pending_context`, `!plugin_lifecycle_fence`, `!trashed`, `!managed_child`, non-reserved `cmid`, non-empty `build_prompt_blocks`).
  - Enqueues atomically at queue front via existing `Hub::force_submit_once(&session_id, req.text, req.content, cmid, false, &caller)`:
    - Returns `None` on duplicate `cmid` -> `202 Accepted` `{ "status": "already_accepted", "delivery": "wrap_up", "hinted": false }` (no duplicate hint).
    - Returns `Some(false)` when idle & empty queue (dispatched straight through) -> `202 Accepted` `{ "status": "dispatched", "delivery": "wrap_up", "hinted": false }`.
    - Returns `Some(true)` when queued at front -> fires §4.2 best-effort `WrapUpHint` if eligible, returning `202 Accepted` `{ "status": "queued", "delivery": "wrap_up", "hinted": <bool> }`.
- **WebSocket FrontPrompt (`ClientCommand::FrontPrompt { ..., #[serde(default)] wrap_up: bool }`):**
  - Used by Web `DeliveryPicker` (`Composer` only, never queued rows).
  - If `wrap_up == true` and `!matches!(caller, RequestCaller::Person(_))`: refuses cleanly without queueing or hinting.
  - Calls existing `Hub::force_submit_once(&session_id, text, content, cmid, false, caller)`; when `Some(true)` and `wrap_up == true`, triggers §4.2 best-effort `WrapUpHint`.
- **Agent API `session_default` / non-Owner callers:** never invoke `wrap_up`; non-Owner `session_default` on Antigravity continues to resolve to ordinary `next_turn`.

### 4.2 Best-Effort `WrapUpHint` Conditions & End-to-End Fencing

Controller calls `supervisor.send_wrap_up_hint(&session_id, target)` **only when ALL hold**:
- `force_submit_once` returned `Some(true)` (newly queued at front)
- `info.meta.provider == "antigravity"` AND `info.meta.wrap_up_steering == true`
- Live config `providers.antigravity.wrap_up_steering == true`
- `info.meta.status == Status::Busy` AND `!info.meta.paused` AND `!hub.clear_pending(&session_id)` AND `hub.delivery_hold(&session_id).is_none()`
- `!hub.is_compacting(&session_id)` AND `!hub.active_turn_is_compacting(&session_id)` (§4.3)
- `let Some(target) = supervisor.delivery_target(&session_id)` (exact current `(worker_epoch, turn_id)`).

Across trusted relays -> proxy:
1. **Controller (`RemoteRuntime`):** if the Machine is connected with Machine >= 52 and runtime >= 8, writes `CoreCommand::WrapUpHint { command_id, session_id, target }` directly without storing it in `shared.pending` (dropped silently if disconnected).
2. **Broker (`machine_broker.rs`):** forwards `WorkerCommand::WrapUpHint` to the worker only if `provider == "antigravity"`, worker runtime protocol >= 8, `snapshot.wrap_up_steering == true`, `snapshot.worker_epoch == target.worker_epoch`, and `snapshot.current_turn_id.as_deref() == Some(target.turn_id.as_str())`; otherwise drops silently.
3. **Worker (`src/acp.rs`):** mints a random 128-bit `attempt_nonce` on every Antigravity `session/prompt` (`_meta["cowboy.dev/attemptNonce"]`). On `WorkerCommand::WrapUpHint`, verifies `effective_wrap_up_steering == true`, `!managed_read_only`, `worker_epoch == target.worker_epoch`, `active_prompt.turn_id == target.turn_id`, `!active_prompt.compacting`, and `!cancelled`, then sends ACP notification `cowboy/wrapUpHint { sessionId: acp_id, attemptNonce: active_prompt.attempt_nonce }` to `agy-proxy` (zero owner text!).
4. **Proxy (`proxy.mjs` & `facade.mjs`):**
   - `cowboy/wrapUpHint` is in `RUNTIME_NOTIFICATIONS` and executes synchronously in `fromClient` (bypassing `this.queue`).
   - Ignored unless `turn` exists for `sessionId`, `typeof attemptNonce === "string" && turn.attemptNonce === attemptNonce`, `!this.managed`, `!this.subagents.owns(sessionId)`, `!turn.compacting`, `!turn.cancelled`, `!turn.structured`, and `!turn.wrapUpDelivered`.
   - Sets `turn.wrapUpPending = true` and notifies `facade.setWrapUpPending(sessionId, true)`.
   - Inside `facade.mjs:call(sessionId, name, args)`:
     - Invokes `this.onResult(call, result)` **first** so Cowboy's UI presentation never displays the internal hint.
     - Then, ONLY when `session.parent === undefined` (root session only — never subagents), `!this.managed`, `turn.wrapUpPending && !turn.cancelled && !turn.ended && !turn.uncertain`, `name !== "UpdatePlan"`, `!result?.isError`, and `Array.isArray(result?.content)`:
       - Atomically consumes `turn.wrapUpPending = false; turn.wrapUpDelivered = true;` (strictly **at most once per turn**).
       - Returns a non-mutating copy `{ ...result, content: [...result.content, { type: "text", text: WRAP_UP_NOTE }] }` where:
         `WRAP_UP_NOTE = "[Cowboy note: a follow-up message from the owner is queued next; finish the current step briefly so it can be delivered.]"`.

### 4.3 Shared Compaction Fence (`COWBOY_WS`)

- **Explicit closed whitespace code-point set `COWBOY_WS`** implemented identically in Rust (`is_cowboy_ws(ch: char)`) and JS (`compaction.mjs:isCowboyWs(cp: number)` — **neither** calls `char::is_whitespace()`, `.trim()`, or `/\s/`, avoiding `U+0085` divergence):
  `U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000..=U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF` (explicitly excluding `U+0085`).
- **`is_compaction_prompt(blocks)`:** applies `build_prompt_blocks(text, content)` first; returns `true` iff non-empty, every block has `type == "text"` with string `text`, and trimming `COWBOY_WS` from `blocks.map(b => b.text).join("")` starts with ASCII case-insensitive `"/compact"` followed immediately by end-of-string or a `COWBOY_WS` code point. Cross-tested by shared JSON vectors (including `U+0085`, `U+FEFF`, split blocks `["/com", "pact"]`, mixed blocks, and `text` fallback).
- **Turn-scoped latch:** Controller latches `session.active_turn_compacting` on prompt dispatch (cleared on `TurnEnd` / `Clear`); Worker latches `active_prompt.compacting`; Proxy computes `turn.compacting` synchronously inside `fromClient` when reserving the `RESERVED` turn slot before `this.queue.then(...)` and before any `await` (`evidence.mark`, `recall`). Compaction turns suppress `WrapUpHint` cleanly while still letting the Owner's follow-up sit safely at the front of `sessions.queue` to run immediately after `/compact` finishes.

### 4.4 Proxy Root Stop Fix & Controller Stop Priority

- **Proxy (`proxy.mjs:fromClient` & `proxy.mjs:cancel`, `proxy.mjs:756-781`):**
  - `session/prompt` synchronously reserves a `turn` slot (`state: "RESERVED"`, capturing `attemptNonce`, `cancelGen`, `compacting`) in `fromClient` before chaining onto `this.queue.then(...)`. `session/cancel` bypasses `this.queue`, increments `this.cancelGen`, and marks any `RESERVED` slot cancelled so a same-chunk `session/prompt` + `session/cancel` replies `{ stopReason: "cancelled" }` without ever starting `agy`.
  - For active **root sessions** (matching the existing subagent cancel behavior at `proxy.mjs:771`): `cancel()` synchronously latches `turn.cancelled = true`, clears `turn.wrapUpPending`, invokes `this.subagents.cancelled(sessionId)`, starts `this.subagents.cancelChildren(sessionId)` and (using cached `this.readyRuntime` when resolved) synchronously starts `facade.endTurn(sessionId, { cancelled: true })` (fencing new facade admission with zero `await`), **immediately forwards `this.agent(message)` (`session/cancel`) to `agy`**, and then awaits `fenced` cleanup concurrently! A blocked 170s extension call in `facade.mjs` can no longer delay Stop from reaching `agy`.
- **Controller `RemoteRuntime::send_pending` (`src/remote_runtime.rs:1811-1888`):**
  - Replace snapshot-batch sequential draining with a **single-command priority picker loop** re-checking `shared.pending` after each command write, ranking independent `Cancel` / `CancelTurn` ahead of `EnsureSession` (rank 1) ahead of `RecoverExecutionSession` / `StopSession(reset-*)` (rank 2 — **strictly preserving `reset_admission.rs`'s `EnsureSession -> StopSession(reset-*)` dependency!**) ahead of `SetConfigOption` (rank 3) ahead of `Steer` / `Prompt` (rank 4).

---

## 5. HTTP/WS API & Web UI Flow (W1)

- **`GET /api/sessions/{id}/delivery-options` (`send_delivery.rs`):**
  - Keeps `"steer": info.meta.supports_current_turn` (always `false` for Antigravity).
  - Adds `"wrap_up_steering": info.meta.wrap_up_steering` (`true` only for Antigravity when §3 gates hold).
- **Web (`web/src/protocol.ts`, `DeliveryPicker.tsx`, `Composer.tsx`, `QueuedDeliveryAction.tsx`, `store.ts`):**
  - `SessionInfo` adds `wrap_up_steering?: boolean`.
  - `store.ts` adds `wrapUpPrompt(sessionId, text, attachments)` using existing `qAdd("queue", sessionId, trimmed, attachments, { mode: "front", wrapUp: true, origin: "composer" })` (reusing IndexedDB durability, `cmid` reconciliation, optimistic queue row, and `sendWithContextNotice` / `requireCurrentAgentContext`!).
  - `DeliveryPicker.tsx`:
    - When `options.steer` is true (Claude/Codex): unchanged native **Steer** behavior.
    - When `!options.steer && !props.queued && (fresh ? options?.wrap_up_steering : props.wrapUpReady)` (Antigravity Composer): enables the top action row labeled **"Wrap up & send next"** (secondary: *"Asks agent to finish current step, then sends next"*), preferred when `policy.mode === "steer"`, calling `props.onWrapUp()`.
    - When `props.queued === true` (`QueuedDeliveryAction.tsx`): **Steer stays disabled when `!options.steer`** so queued Peer/Automation rows on Antigravity can never be promoted to Owner; queued rows retain `Queue first`, `Send now`, edit, and delete.
  - Once submitted, the message renders immediately as the **top queued row** above the composer with full existing queue controls (Edit / Delete / Send now / Cancel) — zero separate receipt card, zero 35s polling timeout, zero `not_confirmed` UX.

---

## 6. Release Sequencing, Required Artifacts & Rollback

1. **Slice M1 (Wire 52/8 + Broker + Worker + Controller):**
   - Lands on `main` together with the `cowboy-workers` pin commit (`retained-worker-interface-compatible`). Strict M1/P2 boundary: M1 commits do **not** modify `plugins/antigravity/` manifests unless exact signed coverage is already published (`just provider-release-coverage`).
   - **Zero DB migrations.** Activate **Controller** first (pre-52 Machines and pre-8 workers keep `wrap_up_steering = false`; existing front-of-queue delivery still works).
   - **All 5 required Machine release artifacts** (built on Hawk per OVH runtime-only contract):
     - **Hawk Machine (4 distinct artifacts verified via `just session-deletion-production-conformance` per `tools/session_deletion_production_conformance.py:169-191`):** (1) old-side writer, (2) new-side writer with distinct revision and ELF hash, (3) reader-only fallback, (4) default-reader.
     - **OVH Machine (1 artifact, `tools/ovh-machine-host-release.sh:36-39`):** (5) plain `.#cowboy-machine-release` deployed with `--workers`.
   - **Maintenance Gate:** Pause for explicit Owner go-ahead before executing live Hawk Machine and OVH Machine worker pin cutover.
2. **Slice P2 (`cowboy-antigravity-acp` `1.0.27` + 7-plugin closure + Antigravity Plugin `0.2.35`, zero unknown manifest fields):**
   - Verified to install and launch cleanly on both pre-52 rollback Machines and protocol 52/8 Machines.
3. **Slice W1 (Web):** atomic `/run/cowboy-web` swap.
4. **Bound R1 Acceptance Canary:** after pin-advance acceptance confirms workers run on the target worker generation, verify on a real Antigravity session with exact candidate Controller, Machine, Worker, and Plugin `0.2.35` digests: negotiated Machine protocol `52`, both runtime links `8`, `supports_current_turn == false`, `wrap_up_steering == true`, Owner wrap-up steer appends the fixed note once and drains immediately after `TurnEnd`, and root Stop reaches `agy` immediately during an active facade/extension call.
5. **Rollback (Zero Drain Risk):**
   - Because all Owner prompts live in existing `sessions.queue` and `WrapUpHint` is 100% stateless and best-effort, Controller, Machine, Worker, Plugin, or Web can be toggled off (`providers.antigravity.wrap_up_steering = false`) or rolled back independently at any time with zero drain precondition and zero risk of lost Owner messages or dropped outcomes.

---

## 7. Verification Plan

- **Crate graph & schema compatibility:** `cargo check --workspace`, `just check-module-layers`, `just structure-check`; verify P2 `0.2.35` parses under pre-R1 `PluginManifest` / `ProviderPackage` decoders (zero unknown fields).
- **Mixed-version & sanitization matrix:** automated tests covering all 6 rows of §3.3, `Frame::Welcome` + `Frame::Snapshot` + `CURRENT_TURN_CAPABILITIES_UPDATE` sanitization, `worker.rs` protocol re-evaluation after ACP init, missing `launch.provider` fail-closed at broker, and upstream `_meta.steering` + `_meta["cowboy.dev/wrapUpSteering"]` stripping.
- **Authority & delivery semantics:**
  - HTTP `delivery: "wrap_up"` and WS `FrontPrompt { wrap_up: true }` refuse `Agent` / `Automation` / `Cowboy` (`403` / WS error), accept `Person`, front-enqueue once via `force_submit_once` (idempotent on duplicate `cmid`), and suppress `WrapUpHint` when idle, paused, clearing, holding, or compacting.
  - HTTP `delivery: "current_turn"` / `"next_checkpoint"` continue to refuse cleanly on Antigravity (`supports_current_turn == false`).
- **Wrap-Up Hint fencing & facade note (`antigravity-acp`):**
  - Wrong `attemptNonce`, managed mode, subagent session, cancelled turn, or `/compact` turn (including during `await recall` preparation) ignores `cowboy/wrapUpHint`.
  - Root facade tool call appends `WRAP_UP_NOTE` strictly **at most once per turn**, **after** `onResult` (never leaking into UI presentation), never on `isError` or `UpdatePlan` or subagent tool calls, without mutating original result objects.
  - Shared Rust/JS `COWBOY_WS` compaction vector tests covering `U+0085`, `U+FEFF`, split blocks, mixed blocks, and `text` fallback.
- **Root Stop fix & Controller Stop priority:**
  - `proxy.mjs` test: root `session/cancel` with a slow/blocked `facade.endTurn` (simulating a 170s extension call) delivers `session/cancel` to `agy` immediately BEFORE `endTurn` resolves; same-stdin-chunk `session/prompt` + `session/cancel` cancels the `RESERVED` slot without starting `agy`.
  - `remote_runtime.rs` test: single-item re-checked `send_pending` preempts queued prompts with `Cancel` while preserving `EnsureSession` before `StopSession(reset-*)`.
- **Web (W1):** tests for `DeliveryPicker` (`wrap_up_steering` enabled as **"Wrap up & send next"** in Composer, disabled for queued rows in `QueuedDeliveryAction`), `wrapUpPrompt` durable outbox submission (`mode: "front", wrapUp: true`), and `bun run typecheck && bun run lint`.
