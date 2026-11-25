# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 6 — Direct Negotiated Pull)

Status: design, revision 6 (2026-11-25). Redesigns the post-edit diagnostic
transport mechanism after three completed review rounds on the `cowboy mcp`
stdio-tunneling approach (`call-f901a15e9dfcc1b559cbcd58`,
`call-34b1b1a7de46cb99f61d1082`, `call-00d757ca8d14bb051d3c2edd`), replacing
target-side `cowboy mcp` stdio tunneling with a **direct, negotiated,
unsequenced Worker -> Machine Broker -> Controller pull RPC** (`Machine 54` /
`Runtime 9`) paired with a **dedicated `mode: "diagnostics"` WebSocket** on
`/run/cowboy-exec.sock` and **Machine-verified root-incarnation Zed admission**.

- Baseline: `origin/main` at `960c811b6b03673b984ad97e84172d540dca23b6`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Why the Mechanism Was Redesigned & Disposition of `call-00d757ca8d14bb051d3c2edd`

Revisions 3–5 tried to avoid bumping the wire protocol by routing post-edit
diagnostics through a private target-side `cowboy mcp` stdio process over the
shared execution keeper (`process/write` / `process/read`), `Action::AgentInvoke`,
and the shared mutation WebSocket. Three review rounds proved that work-around
created 5 of the 6 remaining High findings (spoofable `AgentInvoke` ingress,
mutable CLI symlink targets, queued/stalled execution-transport send fences,
shared WebSocket drain blocking, and crashed-worker sidecar cleanup) while
adding two unnecessary OVH↔Hawk round trips.

Because R4 and R2 (`User MCP servers`) already require a coordinated Hawk/OVH
`cowboy-workers` pin advance, Revision 6 replaces the entire `cowboy mcp`
tunnel with a first-class direct pull channel:

1. **Dedicated `mode: "diagnostics"` WebSocket** on `/run/cowboy-exec.sock`
   between Proxy and Worker (separate `Semaphore(1)`, separate writer, zero
   `RelayGuard`, closed immediately on `L1c` `begin_drain()`, leaving
   `execution-connection.mjs` untouched for mutations).
2. **Direct negotiated, unsequenced, non-retrying Worker <-> Broker <-> Controller
   RPC** (`Frame::PostEditInspectRequest` / `Frame::PostEditInspectReply`,
   `Machine 54` / `Runtime 9`) — zero `cowboy mcp` process, zero keeper hops,
   zero `Client::frames()` / bulk-lane retries, zero sidecar files, and **1
   OVH↔Hawk RTT instead of 3**.
3. **Keeper-captured `workspaceRootIncarnation` + atomic Machine-side
   `session_code::Request::Zed`** returning a **Machine-issued monotonic
   `adapter_incarnation: u64`**, closing remote worktree replacement races and
   Zed adapter restart ordering deterministically.

### Complete Disposition of All 20 Findings from `call-00d757ca8d14bb051d3c2edd`

| # | Severity | Rev 5 Finding | Rev 6 Closure |
|---|---|---|---|
| 1 | High | `worker_epoch` does not authenticate diagnostic caller on model-facing `Action::AgentInvoke` | **Eliminated by redesign (§3, §4):** `code.post_edit_inspect` is **not** exposed on `Action::AgentInvoke` / `cowboy_execute`. Requests arrive solely on the authenticated Worker -> Broker -> Controller `Frame::PostEditInspectRequest` lane, where Broker validates `(session_id, connection_id, worker_epoch, execution_binding)` and Controller validates live worker authority on that exact runtime Machine connection. |
| 2 | High | Remote root verification does not bind the native Zed operation (`session::adapter_request` sends path-only requests) | **Closed atomically at Machine admission (§5.1–§5.2):** `machine_cli/execution.rs` captures `workspaceRootIncarnation` via `session_identities.observe(&binding.workspace.cwd)` at keeper creation and exposes it in `Response::Ready` while still valid. Controller routes Zed `BufferLanguage` / `acquireAgentBuffer` / `RefreshedLanguage` through `session_code::Request::Zed { root, incarnation, request }` (`Machine 54`), which verifies `identities.verify(&root, &incarnation)` atomically immediately before and after forwarding to `CodeRuntimeHost` (and fails closed if the target Machine `< 54` or root changed). |
| 3 | High | Store-path check accepts mutable symlink targets for `cowboy mcp` | **Eliminated by redesign:** no `cowboy mcp` binary is launched for post-edit diagnostics. |
| 4 | High | `claim_send` runs before `write_frame` awaits socket writability | **Eliminated by redesign (§3):** diagnostics never enter `Client::frames()`, `process/write`, or the execution keeper transport. `Frame::PostEditInspectRequest` is an unsequenced, non-retrying, shed-first read RPC over a dedicated bounded queue (`try_send`) with no target-process effect. |
| 5 | High | Diagnostic reply can stall in shared `send_bounded` before drain starts and block mutation settlement / `cowboy/drained` | **Eliminated by redesign (§2):** diagnostics runs over its **own dedicated `mode: "diagnostics"` WebSocket** on `/run/cowboy-exec.sock` with its own `Semaphore(1)`, writer, and 2-slot cap — zero `RelayGuard`, zero shared `send_bounded`, immediately aborted/closed on `RelayQuiet::begin_drain()`. |
| 6 | High | Diagnostic startup timeout can erase `.diag-servers.json` cleanup record | **Eliminated by redesign:** no private `diagnostics_server` process and no sidecar cleanup file exist. |
| 7 | Medium | Single worker send can still have unknown backend outcome via `machine_cli/execution.rs` / bulk-lane retry | **Eliminated by redesign:** diagnostics does not use `machine_cli/execution.rs`, keeper `Command::Invoke`, or bulk-lane resends; disconnect immediately resolves as unavailable. |
| 8 | Medium | Diagnostic quality can prevent causal version watermark from advancing (`baselines.rs`) | **Closed (§5.3):** `Record` tracks causal `latest_order: SnapshotOrder` (`adapter_incarnation`, `buffer_epoch`, `buffer`, `version`) **independently** of `latest: Capture`. Strictly newer versions always advance `latest_order`; equal-version evidence replacement requires non-regression in **both** `entries` (`!(latest.entries.is_some() && capture.entries.is_none())`) and `servers` (`latest.servers.is_subset(&capture.servers)`). |
| 9 | Medium | Controller wall-clock timestamps cannot order Zed adapter incarnations across restarts | **Closed (§5.1, §5.3):** `CodeRuntimeHost` assigns a strictly monotonic `incarnation: u64` (`AtomicU64`) on every `RunningCodeRuntime::start` and returns `adapter_incarnation` inside `session_code::Request::Zed` replies (and local colocated replies). `baselines::record_observed` orders strictly by `(adapter_incarnation, buffer_epoch, version)` with zero Controller timestamps. |
| 10 | Medium | Diagnostic traffic can exhaust shared admission (`execution_calls` permit, proxy 15-slot pool) before its semaphore | **Eliminated by redesign (§2, §3, §4):** diagnostics has completely separate capacity at **every** hop: proxy `DiagnosticsClient` (max 2, never uses `execution-connection.mjs`'s 15-slot pool), worker diagnostics WS (max 2, separate socket), broker shed-first channel (max 4, never uses `ExecutionRequest` queue), and Controller `POST_EDIT_INSPECT_SLOTS` (cap 4 `try_acquire`, never touches `execution_calls` or `AgentRead`). |
| 11 | Medium | Optional discovery stays disabled after 1 s poll finishes before 15 s startup | **Eliminated by redesign:** no async discovery process; availability is determined directly by negotiated Runtime `>= 9` + Machine `>= 54` + keeper `workspaceRootIncarnation`. |
| 12 | Medium | Result contract omitted `cowboy mcp` text-block JSON decoding | **Eliminated by redesign:** Controller returns typed `PostEditInspectResult::Ok(Value)` directly inside `Frame::PostEditInspectReply` -> Worker forwards directly as JSON-RPC `result` on the diagnostics WS. |
| 13 | Medium | Whole-call timer changes legacy `execution-connection.mjs` reconnect behavior (`NaN`) | **Eliminated by redesign:** `execution-connection.mjs` is **100% untouched**; the dedicated `diagnostics-client.mjs` handles its own finite positive deadline covering connect + initialize + request. |
| 14 | Medium | Note missing explicit commit identity (`lfSha256`) and exact `isFresh()` predicate | **Closed (§6.2–§6.3):** `publish()` (under per-file `fileLocks`) explicitly computes `lfSha256(bytes)` (LF-normalized via `replaceAll("\r\n", "\n")`, returning `null` for `> 4 MiB`, NUL, non-UTF-8, or lone `\r`) and returns `{ file, relPath, expectedSha256, revision, invalidation }`, rechecked verbatim by `isFresh()` at handoff. |
| 15 | Medium | MCP cancellation drops clean edit settlement or allows ID reuse (`facade.mjs:1839`) | **Closed (§6.3):** `serveMcp` keeps a cancelled entry in `activeCalls` (setting `entry.cancelled = true` and aborting `entry.controller`, never deleting early until `finally`) so duplicate ID reuse while the edit finishes is still rejected with `-32600`; `entry.cancelled` cancels only diagnostic lookup and note attachment while still settling the clean edit reply cleanly (distinct from duplicate-ID `entry.suppressed`). |
| 16 | Medium | Early diff emission can persist a tentative (inputless) `plan-gate` tool binding | **Closed (§6.3):** `proxy.mjs::toolResult` emits an early `in_progress` `tool_call_update` for committed `WriteFile` / `EditFile` **only** when `planGate.bindConfirmed(call, turn)` confirms an input-verified (non-tentative) binding; tentative inputless announcements remain buffered until confirmed by `agy`'s later update. |
| 17 | Medium | Encoded byte limits can drop valid introduced/escalated counts after JSON escaping | **Closed (§4.3, §6.2):** both Controller (`<= 12 KiB` serialized JSON) and Proxy (`<= 16 KiB` escaped `<system-reminder>`) progressively trim `diagnostics[]` rows from the tail and set `truncated: true`, preserving valid `{ path, state, comparison: { state, introduced, resolved, escalated } }` counts. |
| 18 | Medium | `writes.rs` terminal-marker quota vs keeper replay horizon | **Deferred out of R4:** R4 does not use `writes.rs` at all; removing the unrelated `writes.rs` refactor keeps R4 minimal and focused. |
| 19 | Medium | Rollback compatibility floor for cleanup sidecar and R2 Schema-2 readers | **Closed (§7–§8):** R4 has **no** sidecar file and keeps Ledger **Schema 1** (zero rollback cleanup hazard). R4's Machine 54 / Runtime 9 pin advance is strictly Schema-1-only and rollback-safe; R2 retains its independent two-phase dual-schema reader rollout floor. |
| 20 | Medium | Canary excludes only 3 tools and could leak nonce via `SearchFiles` or other tools | **Closed (§9):** live acceptance inspects the **complete** canary tool transcript verifying ONLY `ReadFile(a)` + `EditFile(a)` ran (zero other facade or extension calls) and confirms the fresh nonce first appeared in `EditFile(a)`'s MCP reply. |

---

## 2. Proxy ↔ Worker Channel: Dedicated Diagnostics WebSocket

### 2.1 Worker Side (`src/worker_execution.rs`, `src/account_sandbox.rs`)

- `/run/cowboy-exec.sock` (unchanged in `account-sandbox` daemon) continues to
  serve the mutation WebSocket under `Admission.connections: Arc<Semaphore>`
  (permits = 1) with full `RelayQuiet` / `L1c` drain / `cowboy/drained` rules.
- `Admission` adds an independent `diag_connections: Arc<Semaphore>` (permits = 1).
- On WebSocket accept (after Bearer token authentication):
  - Read the first `initialize` frame (10 s timeout).
  - If `params.mode == "diagnostics"` (`clientName == "cowboy-antigravity-diagnostics"`):
    - Acquire `diag_connections.try_acquire_owned()` (independent of
      `connections` — never contends with or displaces the mutation socket!).
    - Require `client.ledger().is_some()`, `!client.relays.draining()`, and
      `client.diagnostics_supported()` (negotiated worker↔broker runtime
      protocol `>= 9` AND keeper `workspaceRootIncarnation` captured on
      `Scope::from_binding(&client.binding)`).
    - Answer `{"id": first["id"], "result": {"postEditDiagnostics": {"version": 1}}}`
      (or close/refuse cleanly if unsupported/draining).
    - Run `serve_diagnostics(socket, client)` in its own task:
      - **Never** enters `RelayQuiet` (`try_enter` is never called; zero
        `RelayGuard`).
      - **Never** shares `ids`, `writer`, or `send_bounded` with `serve()`.
      - Accepts **only** `method == "cowboy/postEditDiagnostics"` with at most
        `MAX_DIAG_IN_FLIGHT = 2` concurrent requests (shedding excess
        immediately with local refusal).
      - Selects on `client.relays.draining_started()` (added to `RelayQuiet`
        via `drain_notify: tokio::sync::Notify`, woken in `begin_drain()`):
        the instant `begin_drain()` fires, `serve_diagnostics` drops all
        in-flight waiters and closes the diagnostics WebSocket immediately —
        zero possibility of blocking mutation replies or `cowboy/drained`.
  - Otherwise (`mode` absent): runs existing `serve()` unchanged!
    - On the mutation socket's `initialize` response (already sent by
      `worker_execution.rs:281`), `worker_execution.rs` also checks and caches
      `workspaceRootIncarnation` from `Command::Describe`'s `initialization`
      (verifying `scope == Scope::from_binding(&client.binding)`), and sets
      `cowboy/extensions` field `"postEditDiagnostics": { "version": 1 }` only
      when `client.diagnostics_supported()` is true (otherwise omits it).

### 2.2 Proxy Side (`components/provider-runtime/packages/antigravity-acp/diagnostics-client.mjs`)

- `components/provider-runtime/execution-connection.mjs` is **completely
  untouched** (zero risk to legacy mutation reconnects, `pending`, `abandoned`,
  or `idle()`).
- New lightweight `DiagnosticsClient` (`diagnostics-client.mjs`, ~120 lines):
  - Created only for non-managed root sessions when `Extensions` sees
    `postEditDiagnostics?.version === 1`.
  - Connects to the same `executionUrl(descriptor.endpoint)` with the same
    `Authorization: Bearer <token>`, sending
    `initialize { clientName: "cowboy-antigravity-diagnostics", mode: "diagnostics" }`.
  - `inspect({ path, expectedSha256, timeoutMs = 3500, signal })`:
    - Starts one finite positive `deadline = Date.now() + (timeoutMs + 2500)`
      (default `6000 ms`) **before** connecting; races connect + initialize +
      request against BOTH `deadline` and `signal` abort (combined
      `callTurn.abort`, per-call MCP `signal`, and `facade.closingAbort`).
    - Caps local concurrent requests at 2; on timeout, abort, or disconnect,
      immediately rejects/returns `null`, cleans up every timer and abort
      listener in `finally`, and never throws `UncertainEffect`.
  - `close()` terminates the diagnostics socket immediately; called by
    `facade.quiesce()` **before** `this.connection.idle()` runs during
    shutdown.

---

## 3. Worker ↔ Broker ↔ Controller Wire Protocol (`Machine 54` / `Runtime 9`)

### 3.1 Protocol Versions & Types (`components/wire/src/`)

- `machine_protocol.rs`:
  - `pub const POST_EDIT_INSPECT_PROTOCOL_VERSION: u16 = 54;`
  - Bump `MACHINE_PROTOCOL_VERSION` from `53` to `54`.
- `runtime_wire.rs`:
  - `pub const POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION: u16 = 9;`
  - Bump `PROTOCOL_VERSION` from `8` to `9`.
  - Add two **unsequenced** variants to `Frame` (never in `outbox`, never
    subject to `Ack`/`Replay`, never retried across reconnect):
    ```rust
    PostEditInspectRequest {
        request: Box<PostEditInspectRequest>,
    },
    PostEditInspectReply {
        reply: Box<PostEditInspectReply>,
    },
    ```
    where (both `#[serde(deny_unknown_fields)]`):
    ```rust
    pub struct PostEditInspectRequest {
        pub session_id: String,
        pub worker_epoch: String,
        pub request_id: String,
        pub binding: BindingV1,
        pub root_incarnation: String,
        pub path: String,
        pub expected_sha256: String,
        pub timeout_ms: u64,
    }

    pub struct PostEditInspectReply {
        pub session_id: String,
        pub worker_epoch: String,
        pub request_id: String,
        pub scope: Scope,
        pub result: PostEditInspectResult,
    }

    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum PostEditInspectResult {
        Ok { value: serde_json::Value },
        Refused { code: String },
    }
    ```
  - `PostEditInspectRequest::validate(&self)` checks: `valid_id` on
    `session_id`/`worker_epoch`/`request_id`, valid `binding`, 32 lowercase hex
    `root_incarnation`, strict normalized relative `path` (`1..=4096`,
    `Component::Normal` only, no `\0`/`\`/control chars), 64 lowercase hex
    `expected_sha256`, `timeout_ms` in `1..=4000`, and encoded size `<= 24 KiB`.
- `machine_protocol/session_code.rs`:
  - Add `Request::Zed { root: String, incarnation: String, request: serde_json::Value }`
    (gated on `POST_EDIT_INSPECT_PROTOCOL_VERSION = 54`).

### 3.2 Worker Dispatch (`src/worker_execution.rs`, `src/worker.rs`)

- When `serve_diagnostics` receives `cowboy/postEditDiagnostics`:
  1. Validates `only_keys(&["path", "expectedSha256", "timeoutMs"])`, strict
     normalized relative `path`, 64 lowercase hex `expectedSha256`, optional
     integer `timeoutMs` in `1..=4000` (default `3500`).
  2. Checks active turn via `ledger.active_turn()` (refuses outside an active
     turn; does **not** increment `state.calls` or `state.admitted` and never
     blocks `finish_turn` or `endTurn`!).
  3. Checks `MAX_DIAG_IN_FLIGHT = 2` (dedicated counter on `Client`, completely
     independent of `Client::pending` and `Ledger::calls`).
  4. Checks cached `root_incarnation` (captured from keeper `Command::Describe`
     `cowboyWorkspaceRootIncarnation` for `Scope::from_binding(&client.binding)`).
  5. Enqueues `Frame::PostEditInspectRequest` on a **dedicated bounded
     `mpsc::channel(4)`** (`diag_tx.try_send(...)`, shedding immediately if
     full — never touching `Client::pending` or `outbox`!), registers a
     `oneshot` waiter keyed by `request_id`, and selects across:
     - `tokio::time::timeout(Duration::from_millis(timeout_ms + 2000), rx)`
     - `ledger.turn_closed(&turn)`
     - `client.relays.draining_started()`
  6. In `src/worker.rs` broker connection loop:
     - Only drains `diag_rx` when negotiated `worker_broker_protocol >= 9`
       (otherwise rejects immediately as `"unsupported"`).
     - On broker disconnect: clears all pending diagnostic `oneshot` waiters
       immediately (never retries across reconnect!).
     - On `Frame::PostEditInspectReply { reply }`: verifies
       `reply.session_id == self.session && reply.worker_epoch == self.epoch && reply.scope == Scope::from_binding(&self.binding)` and delivers to the matching `oneshot` waiter.

### 3.3 Machine Broker Relay (`components/machine/src/machine_broker.rs`)

- **Worker -> Controller hop (`Frame::PostEditInspectRequest`):**
  1. Verify `broker.worker_matches(session_id, connection_id, epoch)`.
  2. Verify `request.validate()`, `request.session_id == session_id`,
     `request.worker_epoch == epoch`, `!broker.cancelled_sessions.lock().contains(session_id)`,
     and exact `launch.execution_binding == request.binding` (no departing
     cleanup exemption!).
  3. Verify the active Controller lease negotiated **both**
     `core_runtime_protocol >= 9` AND `core_outer_machine_protocol >= 54` (from
     Core's `Frame::Hello::outer_machine_protocol`). If not connected or below
     either floor, reply locally with `PostEditInspectResult::Refused { code: "unsupported" }` (never sending an unknown frame to a legacy Core!).
  4. Enforce a dedicated broker-wide in-flight cap (`MAX_BROKER_POST_EDIT = 4`)
     using non-blocking shed-first admission (if full, reply locally
     `Refused { code: "capacity" }`; never blocks `RuntimeEvent`,
     `ExecutionRequest`, or `CoreCommand`!).
  5. Record `(request_id -> (session_id, worker_epoch, connection_id))` in a
     bounded map (cleared when reply returns, request times out after 10 s, or
     Core/worker disconnects) and forward `Frame::PostEditInspectRequest` to
     Controller.
- **Controller -> Worker hop (`Frame::PostEditInspectReply`):**
  - Look up and remove `(request_id)` from the broker's in-flight map; verify
    `worker_matches` + `worker.epoch == reply.worker_epoch` + exact
    `Scope::from_binding(&binding) == reply.scope` before forwarding to
    `worker.tx` (never forwarded to legacy workers `< 9`).
  - On Core disconnect: immediately reply `Refused { code: "unavailable" }` to
    every in-flight diagnostic waiter and clear the map.

---

## 4. Controller Design (`components/server-runtime`, `components/server-agent-api`)

### 4.1 Authenticated Runtime-Frame Ingress (Not `Action::AgentInvoke`)

- `code.post_edit_inspect` is **NOT** registered in `actions.rs` /
  `Action::AgentInvoke` / `cowboy mcp`. Model-facing `cowboy_execute` cannot
  invoke it.
- In Controller's runtime broker handler (`components/server-runtime/src/server/runtime_broker.rs`
  / `remote_runtime.rs`), handle `Frame::PostEditInspectRequest { request }` on
  the authenticated runtime Machine connection:
  1. Immediately check `request.validate()`.
  2. Acquire dedicated non-blocking Controller permit
     `POST_EDIT_INSPECT_SLOTS.try_acquire_owned()` (`Semaphore::new(4)` —
     completely separate from `execution_calls` and `AgentRead`!). If full,
     immediately reply `Refused { code: "capacity" }`.
  3. Synchronously establish `deadline = Instant::now() + Duration::from_millis(request.timeout_ms)`
     before any `.await`.
  4. **Authenticate live worker authority on this exact runtime Machine:**
     verify that `state.hub.session_info(&request.session_id)` exists, is not
     managed/superagent-restricted, runs on the exact runtime `machine_id` that
     owns this connection, and `runtime.managed_call_authority(&state.service_id, &meta)`
     (or live `WorkerSnapshot`) matches **all** of:
     - `worker.worker_epoch == request.worker_epoch`,
     - `worker.state` is active (`Running | Busy`),
     - `launch.execution_binding == request.binding`,
     - execution placement matches `SessionReadScope` resolved via
       `session::resolve(&state.hub, &state.machine_control, &state.service_id, &request.session_id)`.
  5. Capture `observed: SessionReadScope` and `trust: LanguageServerTrust`
     pinned to `observed.session()` before any `.await`.
  6. Never call `touch_code_observation`, `codex_push::sync_cursor`, or
     `deliveries::begin`.

### 4.2 Bounded Inspection, Re-Authorization & Byte-Trimmed Projection

- Helper `still_authorized()` synchronously checks:
  - runtime Machine connection still current,
  - live worker `(session_id, worker_epoch, binding)` still active & unchanged,
  - `trust.current(&state.hub)`,
  - `session::current(&state.hub, &state.machine_control, &observed)`.
- Runs `diagnostics::inspect_for_expected_bound(state, &observed, &request.root_incarnation, &request.path, &request.expected_sha256, deadline, still_authorized)`:
  - Every `BufferLanguage`, `acquireAgentBuffer`, `RefreshedLanguage`, and
    200 ms poll sleep runs under `tokio::time::timeout_at(deadline, ...)` via
    the root-incarnation-bound path in §5.2!
  - Immediately stops on: revoked auth/trust/scope, root-changed refusal,
    `SourceState::Matching` with mismatched `sha256`, permanent error,
    `deadline` expiry, or exact `document_current && snapshot.matches_content(expected_sha256)`.
- **Pre-commit fence & tail-trimmed byte budget (M17):**
  - Re-check `still_authorized()`, `Instant::now() < deadline`,
    `identities_match`, `state == "document_current"`, and
    `snapshot.matches_content(expected_sha256)`.
  - Commit via `baselines::record_ordered(observed.session(), path, adapter_incarnation, pending, &diagnostics)` (§5.3). If `"superseded"`, return
    `{ deliver: false, files: [{ path, state: "stale" }], truncated: false, matches_expected: false }`.
  - Project with `post_edit::project(&observation)`, attach
    `matches_expected: true, analyzed_sha256: expected_sha256`.
  - **Progressive byte-cap trimming (`<= 12 * 1024` bytes):** if
    `serde_json::to_vec(&projected).len() > 12 * 1024`, pop items from the tail
    of `file["diagnostics"]` and set `projected["truncated"] = true` until it
    fits `<= 12 * 1024` bytes (preserving valid `{ path, state, comparison: { state, introduced, resolved, escalated } }` counts!). Fail closed only if
    even the empty-diagnostics envelope exceeds `12 KiB`.
  - Re-check `still_authorized()`, emit structured log
    `cowboy.code.post_edit_inspect` (session, path, state, matches_expected,
    deliver — never repository text), and return `Frame::PostEditInspectReply`.

---

## 5. Target Machine Root-Incarnation Zed Admission & Causal Baselines

### 5.1 Keeper Root Capture & Monotonic `adapter_incarnation` (`components/machine`)

1. **Keeper root incarnation (`components/machine/src/machine_cli/execution.rs`):**
   - When `machine_cli/execution.rs` spawns or binds an execution keeper for
     `binding.workspace.cwd`, it captures
     `root_incarnation = session_identities.lock().observe(&binding.workspace.cwd)`
     and stores `(cwd, root_incarnation)` on the keeper's `Active` handle.
   - When answering `Command::Describe` -> `Response::Ready { initialization, .. }`,
     if `session_identities.lock().verify(&cwd, &root_incarnation)` still holds,
     Machine annotates `initialization["cowboyWorkspaceRootIncarnation"] = root_incarnation`
     (omitting it if the directory was replaced since keeper creation!).
2. **Monotonic Zed runtime incarnation (`components/machine/src/machine_code_plugins.rs`):**
   - Add `next_incarnation: AtomicU64` (initialized to `1`) on `CodeRuntimeHost`
     and `incarnation: u64` on `RunningCodeRuntime` (assigned via
     `fetch_add(1, Ordering::AcqRel)` in `RunningCodeRuntime::start`).
   - Return `(runtime.incarnation, response)` from `CodeRuntimeHost::request_with_incarnation("zed", ...)` (and for legacy external `--zed-adapter-socket` if configured, assign `0` / fail closed on cross-restart ordering).
3. **Atomic Machine-side `session_code::Request::Zed` (`session_code.rs`, `machine_cli.rs`):**
   - Extend `session_code::Request` (`Machine 54`) with:
     ```rust
     Zed {
         root: String,
         incarnation: String,
         request: serde_json::Value,
     }
     ```
   - `session_code::admit` validates `root`, `incarnation` (32 hex), workspace
     allowlist, `request` object (requiring `request["worktree"] == root` so
     the inner Zed payload cannot name a different worktree!), and verifies
     **`session_identities.lock().verify(&root, &incarnation)`** BEFORE
     forwarding `request` to `CodeRuntimeHost::request_with_incarnation("zed", ...)`.
   - Immediately **after** Zed responds, Machine re-verifies
     `session_identities.lock().verify(&root, &incarnation)` (rejecting with
     `"Session root changed"` if the worktree was replaced during the Zed RPC!)
     and returns `{ "adapter_incarnation": incarnation_u64, "response": zed_value }`.

### 5.2 Controller Root-Bound Zed Transport (`components/server-code`)

- Add `session::zed_request_bound(hub, control, local_socket, scope, root_incarnation, request) -> Result<(u64, ZedAdapterResponse)>`:
  - Checks `session::current(hub, control, scope)` before and after.
  - On **remote** (non-colocated) Machines: requires negotiated Machine
    protocol `>= 54`; sends `session_code::Request::Zed { root: scope.session().cwd(), incarnation: root_incarnation, request }` on `scope.connection()` (captured `ConnectionToken`); fails closed if Machine `< 54` or verification fails!
  - On **colocated** Machines (e.g., Hawk Controller + Hawk target): verifies
    `scope.local.is_current(scope.session().cwd())` and routes through the
    Machine's `session_code::Request::Zed` on `scope.connection()` (since Hawk
    runs `cowboy-machine` on Machine protocol `>= 54` with its own
    `CodeRuntimeHost`!), or for standalone `"local"` test mode uses local
    `LocalRoot` + monotonic `adapter_incarnation = 1`.
- Both `observe_open_buffer_bound`, `query_open_buffer_bound` (`RefreshedLanguage`),
  and `acquire::acquire_bound` (`acquireAgentBuffer`) route **exclusively**
  through `session::zed_request_bound` — zero path-only unbound Zed calls!

### 5.3 Separated Causal Watermark & Evidence Quality (`baselines.rs`, `snapshot.rs`)

- Expose `BufferSnapshot::version(&self) -> &[CodeBufferVersion]` and
  `BufferSnapshot::buffer_epoch_u64(&self) -> Option<u64>` in `snapshot.rs`.
- In `baselines.rs`:
  - `SnapshotOrder { adapter_incarnation: u64, buffer_epoch: u64, buffer: String, version: Vec<CodeBufferVersion> }`.
  - `Record` stores `latest_order: SnapshotOrder`, `latest: Capture`, and
    `baseline: Option<Capture>` (plus `touched: Instant` for LRU eviction only —
    never used for causal ordering!).
  - Across **all** baseline writers (`code.diagnostics`, `code.post_edit`,
    `code.post_edit_inspect`; legacy callers without Machine `adapter_incarnation`
    pass `0` and preserve existing behavior when `latest_order.adapter_incarnation == 0`):
    1. `new_order.adapter_incarnation < record.latest_order.adapter_incarnation` ->
       **reject (`unknown("superseded")`)**, do not mutate `record`.
    2. `new_order.adapter_incarnation > record.latest_order.adapter_incarnation` ->
       restarted Zed adapter -> set `record.latest_order = new_order`,
       `record.latest = capture`, `record.baseline = None`, return
       `unknown("buffer_replaced")`.
    3. Equal `adapter_incarnation`:
       - `new_order.buffer_epoch < record.latest_order.buffer_epoch` ->
         **reject (`unknown("superseded")`)**.
       - `new_order.buffer_epoch > record.latest_order.buffer_epoch` ->
         reopened buffer (across all buffer IDs!) -> set
         `record.latest_order = new_order`, `record.latest = capture`,
         `record.baseline = None`, return `unknown("buffer_replaced")`.
       - Equal `buffer_epoch`, different `buffer` -> **reject (`unknown("superseded")`)**.
       - Equal `(adapter_incarnation, buffer_epoch, buffer)`:
         - `capture.version` strictly dominates `record.latest_order.version`:
           - **ALWAYS advance `record.latest_order.version = capture.version`**
             (so a delayed older v2 can never overtake v3!).
           - If `(sha256, utf8_bytes)` changed -> rotate
             `record.baseline = Some(replace(&mut record.latest, capture))` and
             compare.
           - If `(sha256, utf8_bytes)` unchanged -> replace `record.latest`
             only if non-regressing in BOTH `entries`
             (`!(record.latest.entries.is_some() && capture.entries.is_none())`)
             AND `servers` (`record.latest.servers.is_subset(&capture.servers)`);
             otherwise keep `record.latest` (while `latest_order.version` has
             advanced!) and compare against `record.baseline`.
         - `capture.version == record.latest_order.version`:
           - Different content -> **reject (`unknown("superseded")`)**.
           - Same content -> replace `record.latest` **only** if strictly
             non-regressing in BOTH `entries` AND `servers`
             (`capture.entries.is_some() && record.latest.servers.is_subset(&capture.servers) && (record.latest.entries.is_none() || capture.servers.len() > record.latest.servers.len())`),
             preserving `record.baseline`.
         - Older or incomparable `version` -> **reject (`unknown("superseded")`)**
           without mutating `record`.

---

## 6. Proxy, Facade & Presentation Design (`components/provider-runtime/packages/antigravity-acp/`)

### 6.1 Unchanged `execution-connection.mjs` & `Extensions.call`

- `execution-connection.mjs` is untouched.
- `Extensions.call` is untouched (all `cowboy/extensionCall` tools remain
  effectful).
- `Extensions` records `this.postEditVersion = postEditDiagnostics?.version === 1 ? 1 : 0`.

### 6.2 `postEditFindings` & Byte-Trimming + Reservation Dedup (`extensions.mjs`)

- `lfSha256(bytes)`: returns `null` if `bytes.length > 4 * 1024 * 1024`,
  `bytes.includes(0)`, `decodeText(bytes) === undefined`, or
  `text.replaceAll("\r\n", "\n").includes("\r")`; otherwise returns
  `sha256(Buffer.from(text.replaceAll("\r\n", "\n")))`.
- `postEditFindings(value, { path, expectedSha256 })`:
  - Requires `value?.deliver === true`, `value?.matches_expected === true`,
    `value?.analyzed_sha256 === expectedSha256`, and a matching entry in
    `value.files` with `file.path === path && file.state === "document_current"`
    that has ANY of:
    - `Array.isArray(file.diagnostics) && file.diagnostics.length > 0`, OR
    - `Number.isInteger(file.comparison?.introduced) && file.comparison.introduced > 0`, OR
    - `Number.isInteger(file.comparison?.escalated) && file.comparison.escalated > 0`.
  - Formats the Claude-parity `<system-reminder>` JSON payload with
    `/[<>&\u2028\u2029]/g` escaped to `\uXXXX`.
  - **Tail-trimming under `16 KiB` UTF-8 cap (M17):** if the escaped reminder
    exceeds `16 * 1024` bytes, progressively pop entries from the tail of
    `diagnostics[]` and set `truncated = true` until it fits (retaining valid
    `comparison` counts!); return `null` only if even with `diagnostics`
    omitted the envelope exceeds `16 KiB`.
  - **Race-free handoff reservation (`pendingShown` + `shown`):**
    `reserveShown(key)` checks `!isShownOrPending(key)` (TTL `600_000 ms`) and
    inserts into `this.pendingShown`; `commitShown(key)` moves to `this.shown`
    (cap 32) on successful handoff; `releaseShown(key)` removes from
    `this.pendingShown` on failed/suppressed handoff.

### 6.3 `Facade`, `serveMcp`, & `proxy.mjs` (M14, M15, M16)

- **Serialized same-path publication (`this.fileLocks`):** `publish()` chains
  per-file via `this.fileLocks` (cleaned when tail settles), runs
  `publishOnce()` (including E1 byte verification), updates `committedRevision`
  ONLY on confirmed success (never on definite conflict / `ToolError`), and
  tracks `activePublishes` (decremented in `finally`) and `invalidations` (only
  on `UncertainEffect`) separately. Unrelated files publish in parallel.
- **Commit identity & explicit freshness predicate:**
  - Confirmed `WriteFile` / `EditFile` records
    `postEdit.target = { file, relPath: posix.relative(this.root, file), expectedSha256: lfSha256(bytes), revision, invalidation }`
    when eligible (`!this.managed`, `session.parent === undefined`,
    `!this.subagents?.owns?.(callTurn.sessionId)`,
    `this.diagnosticsClient !== undefined`, `file !== this.root && inside(this.root, file)`,
    `expectedSha256 !== null`).
  - `isFresh()` checks **all** of:
    1. `session.turn === callTurn`,
    2. `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`,
    3. `!noteSignal.aborted` (combined `callTurn.abort`, per-call MCP `signal`,
       and `this.closingAbort`),
    4. `this.committedRevision.get(file) === revision`,
    5. `(this.activePublishes.get(file) ?? 0) === 0`,
    6. `(this.invalidations.get(file) ?? 0) === invalidation`,
    7. `relayId === undefined || !session.ambiguousRelays.has(relayId)`.
- **Verified early Web diff emission (`proxy.mjs::toolResult`, M16):**
  - `Facade.call` invokes `this.onResult(call, cleanResult)` immediately after
    `this.answer(...)` resolves (before awaiting diagnostics).
  - Add `planGate.bindConfirmed(call, turn)` returning `toolCallId` **only**
    when the binding is input-verified (`entry.hasInput === true`, non-tentative,
    non-contradicted).
  - In `proxy.mjs::toolResult`, for committed `WriteFile` / `EditFile`
    (`!result.isError`), emit an immediate `in_progress` `tool_call_update`
    carrying `content` via `this.holdReply` **only** when
    `planGate.bindConfirmed(call, turn)` returns a confirmed ID; tentative
    inputless bindings stay buffered until verified by `agy`'s later update.
- **Separated MCP cancellation vs duplicate suppression (`serveMcp`, M15):**
  - On `notifications/cancelled` / `$/cancel_request`: do **NOT** delete
    `entry` from `activeCalls` while `facade.call(...)` is still running! Set
    `entry.cancelled = true` and call `entry.controller.abort()` (cleanup
    happens strictly in `finally` after `facade.call` and `send` finish, so
    duplicate ID reuse while an edit is still finishing is still refused with
    `-32600`!).
  - Pass `handoff` to `facade.call(...)`. Right before `send({ id, result })`:
    - If `entry.suppressed || streamEnded || stream.destroyed || stream.writableEnded`:
      skip sending (duplicate-ID suppression).
    - Otherwise, if `candidate && !entry.cancelled && candidate.isFresh() && extensions.reserveShown(candidate.key)`:
      attach `\n\n${candidate.note}` to a non-mutating copy of `result` (before
      `WRAP_UP_NOTE` if present). If `entry.cancelled` or `!isFresh()`, drop
      **only** the note — still send the clean committed edit reply normally!
    - Call `delivered = await send({ id, result })`.
    - If reserved: call `commitShown(key)` iff `delivered === true`, else
      `releaseShown(key)`.

---

## 7. Mixed-Version Compatibility Matrix

| Controller | Execution Machine (Hawk/Falcon) | Runtime Machine (OVH Broker) | OVH Worker | Proxy Plugin | Behavior |
|---|---|---|---|---|---|
| 53 | 53 | 53 | 8 (old) | New | `postEditDiagnostics` absent -> clean no-op. |
| 53 | 53 | 54 | 9 (new) | New | Broker sees Controller `< 54` -> refuses locally (`unsupported`); zero unknown frames sent to Controller 53. |
| 54 | 53 | 54 | 9 (new) | New | Keeper lacks `cowboyWorkspaceRootIncarnation` (or Machine `< 54` lacks `session_code::Request::Zed`) -> fails closed cleanly without unbound remote reads. |
| 54 | 54 | 53 | 9 (new) | New | Worker negotiates runtime protocol `8 < 9` with broker -> `postEditDiagnostics` absent -> clean no-op. |
| 54 | 54 | 54 | 9 (new) | Old | Old proxy ignores `postEditDiagnostics` and never opens `mode: "diagnostics"` WS -> unchanged. |
| 54 | 54 | 54 | 9 (new) | New | Full direct 1-RTT post-edit diagnostics with keeper-bound root incarnation and monotonic `adapter_incarnation`. |

---

## 8. Release & Rollout Plan (Schema-1 Safe, Coordinated Pin Advance)

1. **Wire & pin advance discipline:**
   - Because Revision 6 bumps Machine protocol `53 -> 54` and Runtime protocol
     `8 -> 9` (`components/wire/src/`), R4 commits stay on their isolated
     branch (never pushed to `origin/main` ahead of an approved pin advance!)
     until coordinated with the owner-approved `cowboy-workers` pin advance per
     [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).
   - R4 introduces **zero** ledger schema changes (remains 100% **Schema 1**)
     and **zero** sidecar cleanup files, so rollback across Schema-1 worker
     generations is unconditionally safe. If combined with R2 (`User MCP
     servers`), R2's independent two-phase dual-schema (1+2) reader floor
     remains mandatory across automatic, manual, and per-session OVH fallback
     paths before enabling any Schema-2 writer.
2. **Component / Provider release:**
   - Bump `cowboy-antigravity-acp`, `provider-runtime`, Antigravity Plugin
     (`0.2.38` -> `0.2.39`), and all 6 other pinning Providers; refresh
     `components/registry.json` and `components/worker-registry-input.json`;
     sign and publish all 7 updated manifests before Controller activation.
3. **Rollout order & exact OVH install:**
   - Advance `inputs.cowboy-workers` with full conformance (including Claude +
     connected Code acceptance on Hawk), activate Controller 54 + Hawk Machine
     54 + OVH Machine 54 (`tools/ovh-machine-host-release.sh --workers <release>`).
   - Install the exact signed Antigravity Plugin on OVH via
     **`cowboy operator install`** specifying target `ovh`, exact version,
     exact signed digest, and a stable operation ID; verify the installation
     receipt and open a fresh session confirming both new worker generation and
     new Antigravity Plugin generation before running live acceptance.

---

## 9. Verification Plan

### Deterministic Tests

- **Wire (`components/wire`):**
  - `PostEditInspectRequest::validate()`, `deny_unknown_fields`, byte cap,
    protocol version floors (`54` / `9`), and `session_code::Request::Zed`
    serialization/validation.
- **Target Machine (`components/machine`):**
  - `machine_cli/execution.rs`: captures `workspaceRootIncarnation` at keeper
    spawn, exposes it in `Response::Ready` while valid, omits if root replaced.
  - `machine_code_plugins.rs`: monotonic `RunningCodeRuntime.incarnation: u64`
    across Zed adapter restarts.
  - `session_code::Request::Zed`: verifies `identities.verify(&root, &incarnation)`
    and `request["worktree"] == root` both before and after `zed` request,
    refuses replaced directories atomically, and returns `adapter_incarnation`.
  - `machine_broker.rs`: authenticates `(session_id, connection_id, worker_epoch, binding)`,
    sheds above `MAX_BROKER_POST_EDIT = 4`, refuses locally when Core `< 54` or
    runtime `< 9`, clears waiters on Core disconnect, and tests `ExecutionRequest`
    / `review.start` succeeding under diagnostic saturation.
- **Controller (`components/server-runtime`, `components/server-agent-api`, `components/server-code`):**
  - Direct `Frame::PostEditInspectRequest` ingress authenticates live worker
    `(machine_id, session_id, worker_epoch, binding)`; `Action::AgentInvoke`
    rejects `code.post_edit_inspect` (`unknown_action`).
  - Dedicated `POST_EDIT_INSPECT_SLOTS` (cap 4) sheds without touching
    `execution_calls` or `AgentRead`; zero `touch_code_observation`, zero
    `sync_cursor`, zero `deliveries` writes.
  - `baselines::record_ordered`: separate `latest_order` causal watermark vs
    `latest` capture quality; strictly monotonic `adapter_incarnation` across
    both completion orders around a Zed restart; cross-buffer `buffer_epoch`
    protection; equal-version non-regression in both `entries` and `servers`.
  - Progressive tail-trimming under 12 KiB preserving valid `comparison` counts.
- **Worker (`src/worker_execution`, `src/worker`, `src/account_sandbox`):**
  - Dedicated `mode: "diagnostics"` WS on `/run/cowboy-exec.sock` with its own
    `Semaphore(1)`, zero `RelayGuard`, immediately closed on `begin_drain()`
    even when its writer is stalled/backpressured while mutation replies and
    `cowboy/drained` complete normally.
  - Non-retrying `diag_tx.try_send` shed-first dispatch, immediate cleanup on
    broker disconnect, active-turn fencing without touching `state.calls` or
    `state.admitted`, and mixed-version negotiation (`< 9` / `< 54`).
- **Proxy (`components/provider-runtime/packages/antigravity-acp`):**
  - `DiagnosticsClient`: whole-call finite deadline covering connect + init +
    request, combined `(turn.abort, signal, closingAbort)`, zero timer/listener
    leaks, closed in `quiesce()` before `Connection.idle()`;
    `execution-connection.mjs` unchanged.
  - `Extensions`: `lfSha256` (LF, CRLF, lone `\r`, NUL, `> 4 MiB`), truncated
    introduced-only and escalation-only results, tail-trimming after HTML/JSON
    escaping under 16 KiB, and `reserveShown`/`commitShown`/`releaseShown`
    under overlapping handoffs.
  - `Facade`, `serveMcp`, & `proxy.mjs`: per-file `fileLocks` serializing
    overlapping publishes, definite conflict preserving `committedRevision`,
    `serveMcp` retaining cancelled `entry` in `activeCalls` through settlement
    (refusing ID reuse while settling clean edit reply and dropping only note),
    `proxy.mjs::toolResult` emitting early `in_progress` diff only on
    `planGate.bindConfirmed` (keeping tentative inputless bindings buffered).
- **Repo gate:** `just check-compact` + full pin-advance conformance suite per
  runbook.

### Live Acceptance (Post-Rollout, M20)

On a dedicated free-tier Antigravity canary account on OVH (after exact
`cowboy operator install` receipt verification and fresh session verification
of both worker and Plugin generations) against a Hawk worktree with Language
Servers enabled:
1. Preflight a fixture emitting a **versioned `document_current` LSP
   diagnostic** (not save-triggered) where file `b` (unread, absent from prompt)
   defines a symbol carrying a fresh session-unique diagnostic-only nonce in
   its type/signature mismatch diagnostic.
2. Instruct the model to `ReadFile(a)` and `EditFile(a)` to reference the
   symbol from `b` without calling any other tool.
3. Verify the **complete** canary tool transcript contains **only**
   `ReadFile(a)` and `EditFile(a)` (zero `SearchFiles`, `ReadFile(b)`,
   `RunCommand`, or extension calls), confirm the fresh nonce first appeared in
   `EditFile(a)`'s MCP reply, verify Controller logged
   `cowboy.code.post_edit_inspect` (`matches_expected=true, deliver=true`), and
   verify the model reports the exact nonce.
4. In an isolated negative-control session (Language Servers disabled, fresh
   nonce in `b`), perform the equivalent edit and verify the model does not
   produce the nonce.
5. Verify Web UI `Edit` card shows clean diff without `<system-reminder>`
   clutter, and verify subagent edits and cancelled-turn edits leave no
   diagnostics and zero `UncertainEffect`.
