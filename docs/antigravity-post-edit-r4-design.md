# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 8 — Independent HTTPS Pull + Existing Code-Read Parity)

Status: design, revision 8 (2026-11-25), incorporating owner architectural
decisions **1A + 2A** and closing all 24 findings (11 High, 11 Medium, 2 Low)
from Round 2 design review `call-1b04aa9eb938ab942fd9b1be`.

- Baseline: `origin/main` at `960c811b6b03673b984ad97e84172d540dca23b6`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) (this
  document explicitly supersedes its §R4 48 KiB inline-only edit limit; see
  §6.3),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md),
  [`architecture/20-agent-code-intelligence.md`](architecture/20-agent-code-intelligence.md).

## 1. Owner-Approved Scope Simplification (1A + 2A) & Disposition of `call-1b04aa9eb938ab942fd9b1be`

Round 2 (`call-1b04aa9eb938ab942fd9b1be`) demonstrated that Revision 7's
custom `execution_host` / `zed-adapter` root-pinning stack and primary-WS
diagnostic multiplexing introduced severe rollback/rollout hazards (`Response`
`deny_unknown_fields`, `AgentAcquisitionSupport` exact JSON equality, shared
`sink.send` 15 s stalls) while attempting to guarantee kernel-level `AddWorktree`
path isolation that upstream pathname-based `zed` cannot provide under an
unprivileged host user.

With explicit owner approval (**1A + 2A**), Revision 8 simplifies R4 to reuse
Cowboy's proven production primitives:

- **Decision 1A — Parity with Claude/Codex `code.post_edit` & `SessionReadScope`:**
  R4 reuses Cowboy's existing `SessionReadScope` + `code_reads::diagnostics::read`
  + `acquire::acquire` + `zed-adapter` stack unchanged. **Zero** changes to
  `execution_host.rs`, `execution_protocol::Response::Ready`, `session_code`,
  or `plugins/zed/adapter`. R4 explicitly inherits the exact same read-fence
  contract documented in `components/machine/src/machine_cli/workspace_identity.rs:1-8`
  and deployed for Claude, Codex, and `cowboy.code.diagnostics` (making no claim
  of same-UID kernel mount isolation over upstream Zed's pathname-based
  `AddWorktree`).
- **Decision 2A — Independent Authenticated HTTPS Transport (`POST /api/machine/post-edit-inspect`):**
  `machine_cli` sends diagnostic inspections to Controller over an independent
  HTTPS request authenticated by a per-connection secret token
  (`x-cowboy-machine-post-edit-token`, modeled directly on `machine_lanes.rs`).
  **Zero** diagnostic bytes ever enter the primary Machine WebSocket
  (`machine_transport.rs`), and the outer Machine WebSocket protocol remains
  at **53** (only the local Worker <-> Broker Unix socket adds `Runtime 9`
  diagnostic frames).

### Complete Disposition of All 24 Findings from `call-1b04aa9eb938ab942fd9b1be`

| # | Severity | Round 2 Finding | Rev 8 Closure |
|---|---|---|---|
| 1 | High | H1: Pinned directory never binds native Zed `AddWorktree` pathname | **Closed by Decision 1A (§2.1):** R4 drops the custom `zed-adapter` root-pin claim and reuses Cowboy's existing `SessionReadScope` + `diagnostics::read` (identical security boundary to Claude/Codex `code.post_edit` and `cowboy.code.diagnostics`; upstream pathname `AddWorktree` explicitly documented as shared platform boundary). |
| 2 | High | H1: Taint teardown (`Change::Remove`) can disrupt live editor owners | **Eliminated by Decision 1A:** No `Change::Remove` or `zed-adapter` changes in R4; interactive editor buffers and leases are never touched. |
| 3 | High | H2: Cached `Ready` pin outlives keeper continuity proof | **Eliminated by Decision 1A:** No keeper `Ready` pin cache added; R4 uses existing `SessionReadScope` + `session::current` (parity with Claude/Codex). |
| 4 | High | H2/H5: New `Response::Ready` field breaks `deny_unknown_fields` on rollback/old workers | **Eliminated by Decision 1A:** `execution_protocol::Response::Ready` is **unchanged**. |
| 5 | High | H6: Authorization can expire after `fileLocks` check during `current()`, `payloadDirectory()`, or `fs/writeFile` | **Closed in `Facade` (§6.3):** Re-run `assertAuthorizedBeforeEffect()` (signal, turn, `modeGuard.confirm()`, read/invalidation fence) immediately before **every** effect enqueue (`current`, `payloadDirectory`, `fs/writeFile`, and `process/start`), cleaning up any staged `--request-file` payload if apply is refused; retain settlement from `process/start` admission onward. |
| 6 | High | H14: Duplicate refusal's backpressured `send` can outlive `activeCalls` deletion in original's `finally` | **Closed in `serveMcp` (§6.4):** Store the chosen `sendPromise` on `entry` synchronously under the `!entry.committed` latch; original `finally` awaits `entry.sendPromise` before deleting `activeCalls[callKey]`. |
| 7 | High | H9: Selected diagnostic WS send can stall primary `machine_transport` writer for 15 s / close primary WS | **Eliminated by Decision 2A (§3.5):** `machine_cli <-> Controller` uses independent HTTPS `POST /api/machine/post-edit-inspect`; zero diagnostic bytes ever touch the primary Machine WebSocket. |
| 8 | High | H9: Controller -> Target Machine RPC still uses shared reliable `AdapterRequest`/`AdapterResponse` | **Closed by Decision 1A + §4.2:** Controller -> Target reuses existing `SessionReadScope` + `diagnostics::read` (co-located on Hawk for OVH sessions; identical to Claude/Codex `code.post_edit`), bounded by dedicated per-target `POST_EDIT_TARGET_SLOTS = 4` held through settlement + single-path limit + strict remaining-deadline gating before each step. |
| 9 | High | H8: Target adapter timeout releases permit while native work continues | **Closed in Controller (§4.2):** Hold per-target `POST_EDIT_TARGET_SLOTS` (`4`) `OwnedSemaphorePermit` inside the spawned read task until the underlying `diagnostics::read` future conclusively settles (returns or hits the existing adapter bound), never releasing early on HTTP caller disconnect or 4 s timeout. |
| 10 | High | H3/H12: Policy revocation unfenced between steps; async store check runs after live checks; pre-reply can't undo baseline commit | **Closed in Controller (§4.1):** Run `authorize_step()` (async `read_parent` + `superagent::restricted` FIRST, followed synchronously with zero intervening `.await` by live `worker` + exact `turn_id` + `trust.current` + `session::current` + deadline check) before admission, before `acquire`, before `refresh`, before each poll, and immediately before the synchronous baseline commit & response construction. |
| 11 | High | H7: Staged Zed Plugin rollout breaks exact JSON check on `AgentAcquisitionSupport` | **Eliminated by Decision 1A:** `plugins/zed/adapter` and `AgentAcquisitionSupport` are **unchanged**. |
| 12 | Medium | H4: `watch::Sender::send(true)` drops value with 0 receivers; reused `RelayQuiet` resets bool across replacement units | **Closed in `account_sandbox.rs` (§3.2):** Use `watch::Sender<DrainState { generation: u64, draining: bool }>` updated exclusively via `send_replace`; `end_drain()` increments `generation`, and `serve_diagnostics` aborts whenever `state.draining || state.generation != admitted_gen`. |
| 13 | Medium | H13: `initialize` bearer conflicts with existing HTTP `Authorization` header + `!Origin` upgrade contract | **Closed in `worker_execution.rs` (§3.1):** Preserve exact HTTP `Authorization: Bearer <token>` header auth and `!Origin` rejection for both modes; classify solely via HTTP upgrade path (`"/"` mutation vs `"/diagnostics"` diagnostics). |
| 14 | Medium | H13: Shared handshake pool can block mutation reconnects | **Closed in `worker_execution.rs` (§3.1):** Keep `handshake_slots = Semaphore(3)` plus a dedicated `mutation_handshake_slot = Semaphore(1)` (admitted only when `mutation_slot` is vacant; rejects `"/diagnostics"` with HTTP `503` before upgrade), guaranteeing saturated diagnostic handshakes can never block mutation reconnect or `cowboy/drained`. |
| 15 | Medium | H10/M15: Lease replacement races pending broker registration | **Closed in `machine_broker.rs` (§3.4):** Guard `{ active_controller, pending_post_edit }` under a single mutex `PostEditRoutingState`; validate lease, check cap `4`, mint unique `broker_request_id`, insert pending entry, and `try_send` to `controller.diag_tx` atomically in one lock acquisition; re-validate on dequeue and reply consumption. |
| 16 | Medium | H11: Controller checks for any running turn without matching `request.turn_id`; worker waiter needs drop guard | **Closed in Controller & Worker (§3.3, §4.1):** Controller matches exact `worker.current_turn_id == Some(request.turn_id)` and exact source `ConnectionToken` at every authorization gate; Worker binds pending registration to an RAII `WaiterGuard` and checks `!waiter_tx.is_closed()` at send. |
| 17 | Medium | H8/H11: Recomputing timers & rounding `< 250 ms` up to `250 ms` renews deadlines | **Closed across all hops (§4.2):** Never round upward; clamp strictly downward (`<= 6_000 ms`) and refuse immediately if `< 250 ms` remains; track monotonic `expires_at` through every local queue, recheck at dequeue, and recheck `>= 500 ms` immediately before cold `acquire` / `refresh`. |
| 18 | Medium | H7: Pinned acquisition payload conflicts with `acquireAgentBuffer` budget schema (`refreshOnly: false`) | **Eliminated by Decision 1A:** R4 reuses existing `acquire::acquire` / `zed_session::agent_buffer_request` untouched. |
| 19 | Medium | H1: Ordinary Code use creates unpinned `WorktreeLease` blocking R4 | **Eliminated by Decision 1A:** `WorktreeLease` is untouched; R4 observes interactive and agent-acquired buffers identically to `code.post_edit`. |
| 20 | Medium | H5: Fallback test asserted `active_processes` survives rewrite instead of recovery + bumped `uncertain_sequence` + fence | **Closed in §7.1:** Assert `Ledger::open` consumes `active_processes`, persists recovery uncertainty with bumped `uncertain_sequence`, and preserves the session fence across a second reopen on every fallback binary (and on populated Schema-2 input before enabling any R2 Schema-2 writer). |
| 21 | Medium | M16/M17: Ordering-epoch transition can retain prior baseline when native stream IDs survive WS reconnect | **Closed in `baselines.rs` (§5.1):** On any accepted `conn_epoch` advancement or numeric `buffer_epoch` advancement, atomically replace `latest` + watermark, clear `baseline = None`, and return `unknown("buffer_replaced")` even if native stream strings match. |
| 22 | Medium | M18: Content-change branch admitted non-older instead of strict vector-clock dominance; equal-version ordering | **Closed in `baselines.rs` + `diagnostics.rs` (§5.1–§5.2):** Require strict vector-clock dominance for changed digests in the same stream and numeric `buffer_epoch` ordering; serialize per-`(SessionCodeScope, path)` reads on Controller (under deadline) so sequential equal-version dependency-edit updates are provably causally ordered without modifying `zed-adapter`. |
| 23 | Medium | M22: Nonempty note can describe digest-mismatched or out-of-order observation | **Closed in Controller & `extensions.mjs` (§5.3, §6.2):** Controller sets `matches_expected = true` only for exact `path` + admissible `document_current` + exact `expected_lf_sha256`, clearing `deliver` / omitting `current_errors` otherwise; Proxy requires `matches_expected === true && deliver === true && path === expectedPath && state === "document_current"` plus post-16 KiB non-empty check before dedupe/delivery. |
| 24 | Medium | M20: Freshness ignores in-flight same-path writes and runs before `serveMcp` reply latch | **Closed in `facade.mjs` (§6.3–§6.4):** Track `activeWrites` + `writeDispatchGen` from write dispatch and run `isNoteStillFresh()` synchronously inside `serveMcp`'s `!entry.committed` reply latch; fall back to clean edit result if stale. |
| 25 | Medium | M19: Terminal completion doesn't prove diff ownership when identical concurrent calls reverse | **Closed in `plan-gate.mjs` + `proxy.mjs` (§6.5):** Mark colliding identical-argument (or inputless-conflicting) concurrent announcements `ambiguous = true`, keep terminal-only diff attachment, and suppress diff attachment whenever attribution is ambiguous. |
| 26 | Medium | M21: Stalled MCP relay can hold `pendingShown` tokens and `activeCalls` IDs indefinitely | **Closed in `facade.mjs::serveMcp` (§6.2, §6.4):** Bound MCP relay `send()` write handoff (`10 s`); on timeout, destroy the irrecoverably stalled relay stream through its ambiguity path (`markRelayAmbiguous`), settling `sendPromise` as `false` and releasing the token/ID. |
| 27 | Medium | M25: Pre-write `serveMcp` trace doesn't prove bytes reached relay's native-facing stdout | **Closed in `facade-relay.mjs` + §8.2:** Capture the final serialized reply at `facade-relay.mjs`'s native-facing `process.stdout.write` callback (with write disposition & nonce digest) and link it to Controller's `cowboy.code.post_edit_inspect` log via a canary-only `canary_inspect_id`. |
| 28 | Low | M24: `TextDecoder` drops leading UTF-8 BOM | **Closed in `extensions.mjs` (§6.2):** Reuse BOM-preserving `decodeText` from `facade-tools.mjs:319` inside `lfSha256` and test BOM + mixed line endings. |
| 29 | Low | `docs/INDEX.md` still advertises Revision 6 contracts | **Closed:** `docs/INDEX.md` line 27 updated in the same commit to reflect Revision 8. |

---

## 2. Trust Model, Scope & End-to-End Architecture

### 2.1 Explicit Trust Classification & Shared Platform Boundary (Decision 1A, H12)

Per `docs/antigravity-real-accounts-design.md`, `agy-proxy` runs inside the
untrusted account sandbox unit. **This design makes zero claim that
`PostEditInspectRequest` proves a root edit committed.**

Instead, `POST /api/machine/post-edit-inspect` is authorized strictly as an
**owner-authorized, session-scoped code diagnostic read** with the **exact same
trust, authorization, language-server switch, root-fence, and resource-budget
boundary as public `cowboy.code.diagnostics` and Claude/Codex
`code.post_edit`**:

- `path` and `expected_lf_sha256` are **untrusted proxy-supplied matching
  hints** used only to check whether Zed's observed buffer matches what the
  proxy expects and to compare against session baselines.
- Every request enforces:
  1. Authenticated live runtime Machine connection (`x-cowboy-machine-post-edit-token`,
     revoked immediately on primary WS disconnect);
  2. Live non-managed session worker matching exact
     `(machine_id, session_id, worker_epoch, current_turn_id, execution_binding)`;
  3. Live owner & scope authorization (`read_parent` + `superagent::restricted`
     + `LanguageServerTrust`, §4.1);
  4. Existing `SessionReadScope` non-projecting root fence (`session::resolve` /
     `session::current`), `zed-adapter` `agent_acquisition/entry.rs` pre/post
     `(dev, ino)` check, and `source_snapshot.rs` `O_DIRECTORY | O_NOFOLLOW` +
     `openat2(BENEATH | NO_SYMLINKS | NO_XDEV)`;
  5. Existing shared acquisition shares (`MAX_PER_SESSION = 4`,
     `MAX_PER_MACHINE = 8`, `MAX_WORKTREES_PER_MACHINE = 2`) and
     `CodeAgentResourcePolicy` budget.
- **Accepted platform boundary (shared with Claude, Codex, and
  `cowboy.code.diagnostics`):** Per `components/machine/src/machine_cli/workspace_identity.rs:1-8`,
  `SessionRootIdentities` is a read fence rather than a kernel mount proof
  against a same-UID process renaming directories during upstream Zed's
  pathname-based `AddWorktree`. R4 intentionally stays within this exact
  production boundary and makes no changes to `execution_host` or `zed-adapter`.

### 2.2 Scope (Explicit 48 KiB Supersession, M24)

- **In scope**:
  - Parent-session `WriteFile` and `EditFile` calls on non-managed Antigravity
    sessions with `"Project language servers"` enabled.
  - All bounded valid-UTF-8 (BOM-preserving, NUL-free) facade writes up to
    `LIMITS.writeBytes` (3 MiB), covering **both** inline `--request`
    (`<= 48 KiB`) and `--request-file` (`> 48 KiB .. 3 MiB`) applies. This
    explicitly supersedes the 48 KiB inline-only limit in
    `antigravity-parity-round-2.md` §R4 (§6.3 specifies canonical path, LF
    digest, and E1 settlement for both forms).
- **Out of scope**:
  - Subagent edits, managed read-only sessions, `RunCommand` shell writes,
    background commands, binary/non-UTF-8 writes, and any mutation of
    `ExecutionRequest` ledgers, `RelayGuard`, or `UncertainEffect`.

### 2.3 End-to-End Flow (Zero Primary-WS Traffic, 1 OVH<->Hawk HTTPS RTT)

```text
agy (MCP tools/call EditFile/WriteFile)
  │
  ▼
facade-relay.mjs <-> serveMcp / Facade (account unit)
  1. Acquire per-file fileLocks[file]
  2. assertAuthorizedBeforeEffect() before EVERY effect:
     current() -> [payloadDirectory() -> fs/writeFile()] -> process/start (H6)
  3. Increment activeWrites[file] & writeDispatchGen[file] at dispatch (M20)
  4. On settlement: update fileObservations & committedRevision; return { verified, postEdit } (M23)
  5. Keep clean diff buffered in proxy.mjs (terminal-only; suppress ambiguous concurrent matches, M19)
  6. If eligible & fresh & not cancelled:
       DiagnosticsClient (HTTP upgrade Authorization + !Origin to ws://unix:/diagnostics)
         │  JSON-RPC: diagnostics/post_edit { path, expectedLfSha256, timeoutMs, canaryInspectId? }
         ▼
       Worker (src/worker_execution.rs + src/worker.rs)
         │  WaiterGuard registered BEFORE bounded diag_tx (cap 4); Runtime 9 frame on local Unix sock
         ▼
       Runtime Machine Broker (components/machine/src/machine_broker.rs)
         │  Single mutex PostEditRoutingState: unique broker_request_id + bounded controller.diag_tx
         ▼
       machine_cli (components/machine/src/machine_cli.rs)
         │  Independent HTTPS POST {controller_url}/api/machine/post-edit-inspect
         │  Header: x-cowboy-machine-post-edit-token (per-WS-connection secret, revoked on drop!)
         │  Own Semaphore(4) + reqwest timeout(remaining_ms) — ZERO primary Machine WS traffic!
         ▼
       Controller (components/server-runtime + server-agent-api)
         │  Per-target POST_EDIT_TARGET_SLOTS(4) held through settlement (H8)
         │  authorize_step() (async read_parent + superagent, then sync live worker/turn/trust/scope)
         │  Per-(SessionCodeScope, path) serialized diagnostics::read (existing SessionReadScope + Zed!)
         │  Final authorize_step() -> atomic sync baseline commit + <= 12 KiB projection (M16-M18, M22)
         ▼
       HTTPS 200 (<= 16 KiB) -> machine_cli -> Broker -> Worker -> DiagnosticsClient -> Facade
         │  Verify matches_expected === true && deliver === true && path/state match (M22)
         │  Format <= 16 KiB + re-verify non-empty + tokenized reserveShown(key) (M21/M22)
         ▼
       serveMcp (single activeCalls owner, entry.committed + entry.sendPromise, 10 s write bound, H14/M21)
         - Synchronously recheck isNoteStillFresh() at !entry.committed latch (M20)
         - Append bounded untrusted <system-reminder> to MCP tool text reply ONLY
         - facade-relay.mjs forwards to agy stdout (canary-only stdout trace if enabled, M25)
         - Clean diff attaches only on agy's unambiguous terminal completed card (M19)
```

---

## 3. Local Unix / WS Admission, Drain Isolation & Independent HTTPS Transport

### 3.1 HTTP-Header-Authenticated Admission With Reserved Mutation Capacity (H4, H13)

`src/worker_execution.rs::Admission` preserves the **exact** existing HTTP
WebSocket upgrade authentication contract (`worker_execution.rs:285-305`):

- **Identical HTTP authentication for both modes (R2-M2)**:
  - `accept_hdr_async_with_config` validates `Authorization: Bearer <token>`
    (constant-time compare) AND rejects any request containing an `Origin`
    header (`401 Unauthorized`).
  - Classification uses **only** the HTTP upgrade request path:
    - `"/diagnostics"` -> `Mode::Diagnostics`,
    - `"/"` (or any legacy non-`/diagnostics` path sent by unchanged
      `execution-connection.mjs`) -> `Mode::Mutation`.
  - `initialize` is **never** used to carry the bearer token; unchanged
    `execution-connection.mjs` connects to `"/"` with `Authorization` header
    and behaves identically.
- **Reserved mutation reconnect capacity (R2-M3)**:
  - Pre-upgrade bounds: `handshake_slots = Semaphore(3)` (admits either mode)
    **plus** a dedicated `mutation_handshake_slot = Semaphore(1)` (acquired
    ONLY when `handshake_slots` is exhausted AND
    `mutation_slot.available_permits() == 1`).
  - Any socket admitted via `mutation_handshake_slot` rejects
    `path == "/diagnostics"` with HTTP `503 Service Unavailable` inside the
    upgrade callback before completing the WebSocket handshake!
  - Tight pre-auth limits on both handshake tiers: **4 KiB** HTTP header /
    first-frame bound and **1 second** upgrade + `initialize` deadline.
  - Once classified:
    - `Mode::Mutation`: acquires `mutation_slot = Semaphore(1)`, drops its
      handshake permit immediately, upgrades WS limits to `wire::MAX_FRAME_BYTES`,
      and installs `MutationSocketGuard` — the **sole** caller of
      `relays.socket_ended()` (H4).
    - `Mode::Diagnostics`: rejects immediately (HTTP `503` / close) if
      `client.binding.managed.is_some()`, `diagnostics_slot = Semaphore(1)` is
      occupied, or the current `DrainState` is draining; drops its handshake
      permit immediately, keeps a **64 KiB** WS message/frame cap, and NEVER
      calls `relays.socket_ended()`, NEVER holds `RelayGuard`, and NEVER
      touches `Ledger`.

### 3.2 Generation-Scoped Latched Drain Isolation (H4, R2-M1)

In `src/account_sandbox.rs::RelayQuiet`:

- Add `drain: watch::Sender<DrainState>` where
  `struct DrainState { generation: u64, draining: bool }` (initialized to
  `{ generation: 0, draining: false }`), exposed via
  `client.subscribe_drain() -> watch::Receiver<DrainState>`.
- **Always use `send_replace` (never `send`)** so updates persist even when
  zero receivers are currently subscribed:
  - `RelayQuiet::begin_drain()` atomically updates
    `self.drain.send_modify(|s| s.draining = true);` (using `send_modify` /
    `send_replace` which unconditionally mutates stored state and notifies all
    receivers!).
  - `RelayQuiet::end_drain()` (when a replacement unit starts!) atomically
    updates `self.drain.send_modify(|s| { s.generation = s.generation.saturating_add(1); s.draining = false; });`.
- Inside `serve_diagnostics`, **before** answering `initialize`:
  - Subscribe `let mut drain = client.subscribe_drain();` and read
    `let initial = *drain.borrow_and_update();`.
  - If `initial.draining`, close immediately without replying to `initialize`.
  - Record `let admitted_gen = initial.generation;`, and wrap the entire
    diagnostics lifetime (including `initialize` reply write, read loop, and
    reply writer) in:
    ```rust
    tokio::select! {
        biased;
        _ = drain.wait_for(|s| s.draining || s.generation != admitted_gen) => {}
        _ = run_diagnostics_session(ws, client) => {}
    }
    ```
- Even if `end_drain()` runs for a replacement unit before an old diagnostics
  socket observes `draining == true`, `s.generation != admitted_gen` still
  aborts the old diagnostics socket immediately without touching
  `relays.socket_ended()`.

### 3.3 Local Worker <-> Broker Wire (`Runtime 9`) & Waiter-Bound Dispatch (H9, H11, R2-M5, R2-M6)

In `components/wire/src/runtime_wire.rs`:

- Bump `RUNTIME_PROTOCOL_VERSION` to `9`
  (`POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION = 9`).
- **Outer Machine WebSocket protocol stays `53`** (no new `MachineFrame` or
  `CoreCommand` variants!).
- Add unsequenced, non-retrying `Frame` variants (`#[serde(deny_unknown_fields)]`)
  used **only** on local Unix sockets (Worker <-> Broker <-> `machine_cli`):
  ```rust
  PostEditInspectRequest { request: Box<PostEditInspectRequest> },
  PostEditInspectReply { reply: Box<PostEditInspectReply> },
  ```
  where `PostEditInspectRequest` carries:
  `{ session_id, worker_epoch, request_id, turn_id, scope, path, expected_lf_sha256, remaining_ms, canary_inspect_id: Option<String> }`
  (`<= 8 KiB`), and `PostEditInspectReply` carries:
  `{ session_id, worker_epoch, request_id, scope, outcome: Projected { payload, matches_expected } | Refused { reason } }`
  (`<= 16 KiB`).
- **Worker-side dispatch (`src/worker_execution.rs` + `src/worker.rs`)**:
  - Single in-flight diagnostic call per worker (`Semaphore(1)` in
    `serve_diagnostics`).
  - Clamp `timeout_ms` strictly downward (`<= 6_000 ms`); refuse immediately
    if `< 250 ms` (never round upward, R2-M6!).
  - Register `PendingWorkerPostEdit { waiter_tx, broker_connection_id, turn_id,
    expires_at }` in `pending_post_edit` **before** calling
    `diag_tx.try_send(...)` (cap `4`), wrapped in an RAII **`WaiterGuard`**
    whose `Drop` unconditionally removes `request_id` on socket close, handler
    cancellation, or timeout (R2-M5!).
  - Worker's broker UnixStream writer selects reliable frames strictly
    `biased;` ahead of `diag_rx`. Before writing **byte 0**, verify atomically:
    1. `item.broker_connection_id == current_broker_connection_id`,
    2. `pending_post_edit.get(&item.request_id)` exists AND
       `!entry.waiter_tx.is_closed()` (R2-M5),
    3. Active turn is still `Some(item.turn_id)` and `!draining`,
    4. `remaining = item.expires_at.saturating_duration_since(Instant::now()) >= 250 ms`
       (rewriting `frame.remaining_ms = remaining.as_millis().min(6_000) as u64`,
       never rounding up!),
    5. `unix_socket_has_hint_headroom(&writer, frame_bytes)` passes.
  - Otherwise drop silently before byte 0. On turn end, drain, or broker
    disconnect: clear `pending_post_edit` AND drain `diag_rx`.

### 3.4 Broker Routing Serialized With Controller Lease Lifecycle (H9, H10, M15, R2-M4)

In `components/machine/src/machine_broker.rs`:

- Hold `{ active_controller: Option<PostEditController>, pending: HashMap<String, PendingPostEdit> }`
  under **one mutex** `post_edit: parking_lot::Mutex<PostEditRoutingState>`
  (eliminating any lock-gap race between lease capture, pending insertion,
  enqueue, and teardown, R2-M4!):
  - `PostEditController { lease: u64, diag_tx: mpsc::Sender<QueuedBrokerPostEdit> }`
    (cap `8`, set ONLY when `machine_cli` advertises an active Controller
    post-edit HTTP token for this lease!).
- **Admission** on inbound `Frame::PostEditInspectRequest` from worker
  `(session_id, connection_id)`:
  1. Validate live `WorkerPeer`: `connection_id`, `worker_epoch`, non-managed
     `execution_binding` matching `request.scope`, and `worker.protocol >= 9`
     (if `< 9`, drop silently without sending even a local refusal!). Capture
     `worker_diag_tx = worker.diag_reply_tx.clone()` (cap `4`, never `worker.tx`!).
  2. Clamp `remaining_ms <= 6_000`; if `< 250`, reply `timeout` on
     `worker_diag_tx.try_send` and return.
  3. Lock `self.post_edit.lock()` **once**:
     - If `active_controller` is `None` -> unlock and `worker_diag_tx.try_send(Refused("unsupported"))`.
     - If `pending.len() >= MAX_BROKER_POST_EDIT` (`4`) -> unlock and `worker_diag_tx.try_send(Refused("capacity"))`.
     - Mint globally unique `broker_request_id = format!("pe-{}-{:016x}", broker_nonce, next_seq)`.
     - Compute `expires_at = Instant::now() + Duration::from_millis(remaining_ms)`.
     - Insert `PendingPostEdit { worker_request_id, controller_lease, session_id,
       worker_connection_id, worker_epoch, scope, expires_at }` into `pending`.
     - Enqueue `QueuedBrokerPostEdit { broker_request_id, controller_lease,
       expires_at, frame }` via `active_controller.diag_tx.try_send(...)`
       **while still holding the lock**; if `try_send` fails, immediately remove
       the entry from `pending`, unlock, and shed via `worker_diag_tx.try_send`.
  4. In the broker -> `machine_cli` Unix writer (`biased;` below reliable
     `controller.tx`, with `unix_socket_has_hint_headroom`): before writing
     byte 0, lock `self.post_edit.lock()` to confirm `active_controller.lease ==
     item.controller_lease` AND `pending.contains_key(&item.broker_request_id)`
     AND `remaining >= 250 ms` (updating `frame.remaining_ms`); otherwise remove
     stale entry and drop silently!
  5. On inbound `Frame::PostEditInspectReply` from Controller `reply_lease`:
     lock `self.post_edit.lock()`, **peek** `pending.get(&reply.request_id)`,
     verify ALL FIVE (`controller_lease == reply_lease`, `session_id`,
     `worker_epoch`, `scope`, and live `worker` matching `worker_connection_id`
     + `worker_epoch` + `protocol >= 9`), remove only on full match, restore
     `worker_request_id`, and send on `worker.diag_reply_tx.try_send`!
  6. `remove_controller(closed_lease)` acquires `self.post_edit.lock()`, clears
     `active_controller` iff `lease == closed_lease`, and drains ONLY `pending`
     entries with `controller_lease == closed_lease`, notifying matching
     `protocol >= 9` workers via `worker.diag_reply_tx.try_send(Refused("unavailable"))`.
- **All 4 local Unix writers** (Worker->Broker, Broker->Worker,
  Broker->`machine_cli`, `machine_cli`->Broker) use bounded drop-on-full
  low-priority diagnostic channels selected strictly `biased;` below reliable
  frames (and `hints`) with `unix_socket_has_hint_headroom` before byte 0.

### 3.5 Independent Authenticated HTTPS Transport (`machine_cli <-> Controller`, Decision 2A, H9)

Modeled directly on `components/server-runtime/src/server/machine_lanes.rs`:

1. **Per-connection capability & secret token negotiation (on existing `Machine 53` WS upgrade)**:
   - On `/api/machine/connect` WS upgrade, `machine_cli` sends header
     `x-cowboy-machine-post-edit: post-edit-v1`.
   - Controller replies with `x-cowboy-machine-post-edit: post-edit-v1` and
     `x-cowboy-machine-post-edit-token: <256-bit random token>`.
   - **Only after** the Machine's Ed25519 challenge proof succeeds
     (`machine_connection.rs`, right beside `machine_lanes::register`),
     Controller registers `PostEditTarget { connection, machine_id }` under that
     token with an RAII `Registration` guard whose `Drop` removes the token
     immediately when the primary Machine WS ends!
   - `machine_cli` passes `post_edit_supported: bool` in its local broker
     connection setup (activating `PostEditController` only when a token was
     received and the primary WS handshake succeeded).
2. **Independent HTTPS execution (`machine_cli.rs`)**:
   - Dedicated `Semaphore(4)` (`MAX_MACHINE_CLI_POST_EDIT_INFLIGHT = 4`) and a
     reused `reqwest::Client` (HTTPS for remote, HTTP loopback only, matching
     `validate_controller_url`).
   - On receiving `Frame::PostEditInspectRequest` from the local broker socket:
     - Check `remaining_ms >= 250` (clamped `<= 6_000`) and acquire
       `try_acquire_owned()` permit; if exhausted, reply `Refused("capacity")`
       on local `diag_runtime_tx`.
     - Spawn a task tied to this primary WS connection's `CancellationToken`:
       recheck remaining time, issue
       `POST {controller_url}/api/machine/post-edit-inspect` with header
       `x-cowboy-machine-post-edit-token: <token>`, JSON body `<= 8 KiB`, and
       per-request timeout `Duration::from_millis(remaining_ms)`.
     - Bound response body reading strictly to **16 KiB** (`<= 16_384` bytes),
       decode `PostEditInspectReply`, and forward to broker via local bounded
       low-priority `diag_runtime_tx.try_send(...)`.
     - **Zero bytes** ever enter `machine_write_tx` or `machine_transport.rs`;
       a slow or failing diagnostic HTTP request can never delay a single
       primary WS frame or disconnect the primary WS!

---

## 4. Controller Endpoint, Fenced Authorization & Target Concurrency Bounds

### 4.1 `POST /api/machine/post-edit-inspect` & Fenced Step Authorization (H3, H11, H12, R2-H3, R2-M5)

`code.post_edit_inspect` is **not** exposed on `Action::AgentInvoke`,
`cowboy_execute`, or any model-facing route.

`POST /api/machine/post-edit-inspect` authenticates and executes as follows:

1. **Token & body bounds**: Look up `x-cowboy-machine-post-edit-token` in
   `machine_post_edit::TARGETS` (returning `403` if absent or
   `!state.machine_control.is_current(&target.connection)`). Enforce an 8 KiB
   HTTP body cap and `#[serde(deny_unknown_fields)]` decoding.
2. **Deadline without upward rounding (R2-M6)**:
   - Clamp `remaining_ms = request.remaining_ms.min(6_000)`. If
     `remaining_ms < 250`, immediately return `Refused("timeout")`.
   - Set one `deadline = Instant::now() + Duration::from_millis(remaining_ms)`.
3. **Reusable `authorize_step()` ordering (R2-H3, R2-M5)**:
   Every authorization checkpoint executes **asynchronous store checks FIRST**
   and **synchronous live state checks IMMEDIATELY AFTER with zero intervening
   `.await`**:
   - *Async phase (bounded by `deadline`)*:
     1. Load `SessionMeta` for `request.session_id` (refuse managed/system
        sessions).
     2. `read_parent(&state.hub, state.store.as_ref(), &meta).await` ->
        `Refused("permission_denied")` on disabled/removed owner.
     3. `superagent::restricted(state, &meta).await` ->
        `Refused("superagent_scope_denied")` if restricted.
   - *Synchronous phase (zero `.await`!)*:
     4. `Instant::now() < deadline` -> else `Refused("timeout")`.
     5. `state.machine_control.is_current(&target.connection)` -> else
        `Refused("unavailable")`.
     6. Live runtime worker in `state.hub` matches **exact**:
        - `runtime_machine_id == target.machine_id`,
        - `worker_epoch == request.worker_epoch`,
        - **`current_turn_id.as_deref() == Some(request.turn_id.as_str())`**
          (exact turn match, R2-M5!),
        - non-managed `execution_binding` with
          `Scope::from_binding == request.scope`.
     7. `trust.current(&state.hub)` (`LanguageServerTrust`, off ->
        `Refused("restricted")`) and
        `session::current(&state.hub, &state.machine_control, &observed)`
        (scope unchanged, capturing exact target `ConnectionToken` and its
        monotonic `machine_connection_epoch`).
4. **Fenced execution across every step**:
   - Run `authorize_step()` at **admission**.
   - Acquire per-target `POST_EDIT_TARGET_SLOTS` permit (cap `4`, §4.2) and
     per-`(SessionCodeScope, path)` read serialization lock (bounded by
     `deadline`, §5.2).
   - Run `observe_open_buffer` (existing `SessionReadScope` + `zed`).
   - If buffer is not open: re-check `remaining >= 500 ms` **and**
     `authorize_step()` immediately **before** calling existing
     `acquire::acquire` (cold open + `CodeAgentResourcePolicy` budget).
   - If open and `source_state == Changed`: re-check `remaining >= 500 ms`
     **and** `authorize_step()` immediately **before** calling existing
     `RefreshedLanguage` (`refresh: "agent_owned"`).
   - On each bounded 250 ms analysis-wait poll (clamped to `deadline`): re-run
     `authorize_step()` before the next poll read.
   - **Final fenced baseline commit & reply construction**:
     - Run `authorize_step()` one final time.
     - Immediately after its synchronous phase returns `Ok` — with **ZERO
       intervening `.await`** — execute `baselines::record_ordered(...)` and
       `post_edit::project_bounded(...)` synchronously and return the HTTP
       response! A revocation or timeout before this point commits nothing to
       `baselines.rs` and returns a closed `Refused` code.
   - Never call `touch_code_observation`, never mutate `sync_cursor`, and never
     write to `deliveries`.

### 4.2 Per-Target Concurrency Permit Held Through Settlement (H8, R2-H8, R2-H9b)

- Maintain a dedicated per-target-Machine `Semaphore(4)`
  (`POST_EDIT_TARGET_SLOTS`, completely independent of interactive Code reads,
  `execution_calls`, and `AgentRead`).
- Spawn the underlying `diagnostics::read` work into a Controller-owned task
  holding the `OwnedSemaphorePermit` until the underlying `diagnostics::read`
  future conclusively finishes (returns or hits the existing 35 s adapter
  bound).
- The HTTP handler awaits that task's `oneshot` result up to `deadline`. If
  the HTTP caller disconnects or hits its 4 s deadline first, the handler
  returns immediately **while the spawned task retains its `OwnedSemaphorePermit`
  until settlement** (discarding its late result without committing any
  baseline!). Repeated short timeouts can therefore **never** pile more than 4
  concurrent diagnostic operations onto a target Machine.

---

## 5. Controller Baselines, Serialization & Bounded Projection

### 5.1 Scoped Ordering & Strict Causal Rules (M16, M17, M18, R2-M10, R2-M11)

In `components/server-agent-api/src/server/agent_api/code_reads/baselines.rs`:

- Every Machine WebSocket connection in `MachineControl` receives a strictly
  monotonic Controller-assigned `machine_connection_epoch: u64` (`0` for local).
  Both public (`code.diagnostics`, `code.post_edit`) and private (R4) baseline
  writers pass their captured `conn_epoch`.
- Separate `record.latest_order: CaptureOrder` from `record.latest: Capture`:
  ```rust
  struct CaptureOrder {
      conn_epoch: u64,
      instance: String,
      buffer: String,
      buffer_epoch_num: u64, // parsed numerically from snapshot.buffer_epoch
      version: Vec<BufferVersionEntry>,
  }
  ```
- **Exact transition rules**:
  1. **`conn_epoch` comparison first**:
     - `incoming.conn_epoch < record.latest_order.conn_epoch` -> reject stale
       observation (`unknown("out_of_order_observation")`), modifying nothing.
     - `incoming.conn_epoch > record.latest_order.conn_epoch` -> **atomic epoch
       transition (R2-M10)**: replace `record.latest = incoming`, update
       `record.latest_order`, **clear `record.baseline = None`**, and return
       `unknown("buffer_replaced")` — even if the native `zed-adapter` and its
       `BufferSnapshot.stream` IDs survived the Machine WebSocket reconnect!
  2. **Within the same `conn_epoch`, check buffer lifetime (`instance`,
     `buffer`, numeric `buffer_epoch_num`, R2-M11)**:
     - Require `snapshot.buffer_epoch` to parse cleanly as `u64` (rejecting
       unparseable/non-canonical values as invalid).
     - If `instance` or `buffer` differs, OR same `(instance, buffer)` with
       `incoming.buffer_epoch_num > existing.buffer_epoch_num`: atomically
       replace `record.latest = incoming`, update `record.latest_order`, clear
       `record.baseline = None`, and return `unknown("buffer_replaced")`.
     - Same `(instance, buffer)` with
       `incoming.buffer_epoch_num < existing.buffer_epoch_num` -> reject stale
       (`unknown("out_of_order_observation")`).
  3. **Same `(conn_epoch, instance, buffer, buffer_epoch_num)` — changed
     content (`sha256` or `utf8_bytes` differs, R2-M11)**:
     - Require **strict vector-clock dominance**
       (`strictly_dominates(incoming.version, latest_order.version)`: every
       replica timestamp in `latest_order.version` is `<=` in `incoming.version`
       and at least one replica timestamp is strictly `>`).
     - Only on strict dominance: promote `record.baseline = Some(record.latest)`,
       set `record.latest = incoming`, update `record.latest_order`, and return
       `compare(&baseline, &incoming, diagnostics)`.
     - Equal or incomparable vector clocks for differing content -> do NOT
       rotate `baseline`; return `unknown("unorderable_observation")`.
  4. **Same `(conn_epoch, instance, buffer, buffer_epoch_num)` — equal
     content (`sha256` and `utf8_bytes` equal, M18)**:
     - Strictly older vector clock -> reject (`unknown("out_of_order_observation")`).
     - Strictly newer vector clock -> update `record.latest` & `latest_order`.
     - Equal vector clock -> because same-`(SessionCodeScope, path)` reads are
       serialized on the Controller (§5.2), this read is causally later than
       the prior completed read; refresh `record.latest` with the newer
       diagnostic evidence (covering dependency-edit diagnostic additions or
       clears!) while never downgrading `entries: Some(_)` to `None` unless the
       new observation genuinely exceeded `MAX_CAPTURED` (returning
       `unknown("over_capture_bound")` on comparison).

### 5.2 Per-`(SessionCodeScope, path)` Read Serialization (M18)

In `components/server-agent-api/src/server/agent_api/code_reads/`:

- Maintain a bounded per-`(SessionCodeScope, path)` async lock map (`Weak<Mutex<()>>`,
  auto-pruned when idle) acquired (bounded by `deadline`) around the
  observation + baseline-record critical section in both `post_edit_inspect`
  and `code.diagnostics`/`code.post_edit`.
- This guarantees that two overlapping diagnostic reads of the same file in the
  same session cannot complete out of order on the Controller, establishing
  real-time happens-before causal ordering for equal-text/equal-version
  dependency-edit updates without modifying `zed-adapter`!

### 5.3 Bounded 12 KiB Projection, `matches_expected` & Post-Trim Recheck (M22, R2-M12)

In `components/server-agent-api/src/server/agent_api/code_reads/post_edit.rs`:

- Compute `matches_expected` strictly: `true` **only** when:
  1. Exactly one file entry exists with `file.path == request.path`,
  2. `file.state == "document_current"` AND baseline admission did NOT classify
     the observation as `out_of_order_observation` or `unorderable_observation`,
  3. `bufferSnapshot.content.sha256 == request.expected_lf_sha256`.
- When `!matches_expected`: do NOT emit `current_errors`, set `deliver = false`,
  and return `Projected { payload: { deliver: false, files: [], truncated: false },
  matches_expected: false }` (preventing a mismatched or out-of-order capture
  from ever emitting a note, R2-M12!).
- When `matches_expected == true` and `comparison` is `unknown` (e.g.
  `no_baseline` or `buffer_replaced`) on `document_current`: emit bounded
  `"current_errors": u64` whenever `current_errors > 0`.
- Trim serialized JSON to `<= 12 KiB` by dropping tail rows from
  `files[*].diagnostics` and setting `truncated = true`, then **recompute
  `deliver`** from the final trimmed payload: `true` iff at least one file has
  `(compared && introduced + escalated > 0)` OR `(!compared && (!diagnostics.is_empty() || current_errors > 0))`.

---

## 6. Proxy, Facade, `serveMcp`, `facade-relay` & Web UI Presentation

### 6.1 `DiagnosticsClient` (Dedicated `/diagnostics` WS Client)

Add `components/provider-runtime/packages/antigravity-acp/diagnostics-client.mjs`
(leaving `execution-connection.mjs` **untouched**):

- Connects lazily to the descriptor's execution socket with HTTP path
  `"/diagnostics"` and the descriptor's `Authorization: Bearer <token>` header
  (matching `worker_execution.rs` HTTP header auth, R2-M2!).
- Single whole-call `AbortController` timer (`timeoutMs`, default `4000`, cap
  `6000`) covering connect + initialize + `diagnostics/post_edit` RPC, combined
  with `(turn.abort.signal, callSignal, closingAbort.signal)` with guaranteed
  timer/listener cleanup in `finally`.
- Single-flight, zero retries, fail-open (`null` on any transport/timeout/refusal
  error; zero `UncertainEffect`). Closed in `quiesce()` before `Connection.idle()`.

### 6.2 `Extensions`: BOM-Preserving `lfSha256`, Strict Eligibility & Tokenized Dedupe (M21, M22, R2-M12, R2-L1)

In `components/provider-runtime/packages/antigravity-acp/extensions.mjs`:

1. **`lfSha256(bytes)` (R2-L1)**: Rejects `bytes.length > 4 * 1024 * 1024` or
   `bytes.includes(0)`, decodes via the existing BOM-preserving
   `decodeText(bytes)` from `facade-tools.mjs:319` (`new TextDecoder("utf-8", {
   fatal: true, ignoreBOM: true })`), normalizes `\r\n` and `\r` to `\n`, and
   returns lowercase 64-hex SHA-256 (exact byte-for-byte parity with
   `source_snapshot.rs`!).
2. **`postEditFindings(reply, { expectedPath, expectedLfSha256 })` (M22, R2-M12)**:
   - Immediately returns `null` unless `reply?.state === "projected" &&
     reply.matches_expected === true && reply.payload?.deliver === true`.
   - Filters `reply.payload.files` to the single entry matching
     `file.path === expectedPath && file.state === "document_current"` (returning
     `null` if absent!).
   - Renders bounded untrusted `<system-reminder>` text (escaping `</system-reminder>`
     and control tags), trims tail diagnostic rows after final escaping to stay
     `<= 16 KiB` UTF-8, and **re-evaluates eligibility on the final encoded
     bytes**: returns `{ text, digest }` only if at least one diagnostic row OR
     a positive summary count (`introduced > 0`, `escalated > 0`, or
     `current_errors > 0`) survived inside the 16 KiB output; otherwise `null`.
3. **Tokenized dedupe reservations (`pendingShown` / `settledShown`, M21)**:
   - Canonical key:
     `key = sha256(JSON.stringify([sessionId, canonicalPath, lfSha256, noteDigest]))`.
   - `pendingShown: Map<key, token>` (no TTL while live, cap `64`) separate from
     `settledShown: Map<key, expiresAtMs>` (10-min TTL, LRU cap `256`).
   - `reserveShown(key)` allocates monotonic `token`; `commitShown(key, token)`
     and `releaseShown(key, token)` act **only** when
     `pendingShown.get(key) === token`.
   - `commitShown` records **facade-to-MCP-relay handoff** (not native `agy`
     consumption receipt); relay write failure/timeout calls `releaseShown`.

### 6.3 `Facade`: Per-Effect Re-Authorization, In-Flight Write Tracking, E1 `{ verified, postEdit }` & `<= 3 MiB` Scope (H6, M20, M23, M24, R2-H6, R2-M13)

In `components/provider-runtime/packages/antigravity-acp/facade.mjs`:

1. **Per-file `fileLocks` + `assertAuthorizedBeforeEffect()` before EVERY effect (R2-H6)**:
   - Capture `preLockObserved = session.observed.get(file)` and
     `preLockInvalidations = this.invalidations.get(file) ?? 0` before entering
     `withFileLock(file, ...)`.
   - Helper `assertAuthorizedBeforeEffect()` checks synchronously:
     1. `!signal?.aborted && !this.closing`,
     2. `this.turns.get(sessionId) === turn && !turn.abort.signal.aborted && !turn.uncertain`,
     3. `modeGuard.confirm()` still unrestricted,
     4. `(this.invalidations.get(file) ?? 0) === preLockInvalidations && session.observed.get(file) === preLockObserved`.
   - Invoke `assertAuthorizedBeforeEffect()` at **every** effect boundary inside
     `withFileLock`:
     - Immediately upon acquiring `fileLocks` (before `await this.current(file)`),
     - Immediately after `await this.current(file)` (before `payloadDirectory` /
       `fs/writeFile` or inline `process/start`),
     - For `--request-file` (`> 48 KiB`): after `await payloadDirectory(turn)`
       (before `fs/writeFile`), AND after `await fs/writeFile` (immediately
       before `process/start` apply helper!). If refused after `fs/writeFile`
       staged `payload`, best-effort delete `fs/remove(payload, { force: true })`
       before throwing refusal!
   - Once `process/start` is admitted/enqueued, retain full settlement
     responsibility regardless of later cancellation.
2. **In-flight write & observation tracking (M20, R2-M13)**:
   - Maintain per-file `activeWrites: Map<file, number>`,
     `writeDispatchGen: Map<file, number>`, `committedRevision: Map<file, number>`,
     and `fileObservations: Map<file, { generation, rawSha256 }>`.
   - Increment `activeWrites[file]` and `writeDispatchGen[file]` synchronously
     right before dispatching `publishOnce` (decrementing `activeWrites[file]`
     in `finally`!).
   - `recordObservation(file, observedRawSha256)` runs on every `ReadFile`,
     `current(file)`, definite conflict (`null` -> bump `generation`),
     successful `publish` (`sha256(bytes)`), and `UncertainEffect` (`null` ->
     bump `generation`).
3. **Explicit `{ verified, postEdit }` return (M23) & `> 48 KiB` `--request-file` coverage (M24)**:
   - `publish()` returns `{ verified: boolean, postEdit: target | null }`; both
     `writeFile` and `editFile` strictly check **`if (published.verified)`**
     for E1 byte-only settlement vs ordinary success text.
   - Works uniformly for inline `--request` (`<= 48 KiB`) and `--request-file`
     (`> 48 KiB .. 3 MiB`): canonical `file` via `resolve`, `lfSha256(bytes)`
     on the written buffer, and identical E1 `current()` hash confirmation on
     `result.forgotten`.

### 6.4 `serveMcp`: Single Response Owner, Shared `sendPromise`, Synchronous Freshness Latch & Bounded Relay Handoff (H14, M20, M21, R2-H14, R2-M13, R2-M15)

In `components/provider-runtime/packages/antigravity-acp/facade.mjs::serveMcp`:

- Single owner record per `callKey` in `activeCalls`:
  ```js
  const entry = {
    controller: new AbortController(),
    cancelled: false,
    suppressed: false,
    committed: false,
    duplicateRefused: false,
    sendPromise: null,
  };
  ```
- **Bounded relay write handoff (R2-M15)**: `send(message)` arms a `10 s`
  (`MCP_RELAY_WRITE_TIMEOUT_MS`) timer; if `stream.write`'s callback does not
  fire before timeout, destroy the stalled relay stream (`stream.destroy()`)
  and resolve `false` (marking relay ambiguous via `markRelayAmbiguous` and
  releasing `pendingShown` tokens and `activeCalls` IDs cleanly).
- **Duplicate ID handling (R2-H14)**: Never delete `existing` from
  `activeCalls`. Abort and suppress `existing`; if
  `!existing.committed && !existing.duplicateRefused`, synchronously set
  `existing.committed = true; existing.duplicateRefused = true; existing.sendPromise = send({ id, error: { code: -32600, ... } });`
  and await `existing.sendPromise`.
- **Synchronous freshness check at reply latch (R2-M13)**: When the original
  `facade.call` settles with `{ cleanResult, candidateNote }`, inside the
  synchronous `!entry.suppressed && !entry.committed` branch (zero `.await`
  before `stream.write`!):
  1. Re-run `isNoteStillFresh()` synchronously: checks `!entry.cancelled`,
     `!entry.suppressed`, `!signal.aborted`, `!facade.closing`, live unfenced
     `turn`, `activeWrites[file] === 0`, `writeDispatchGen`, `committedRevision`,
     `invalidations`, and `fileObservations` (`generation` + `rawSha256`).
  2. Only if fresh: reserve dedupe token (`reserveShown(key)`) and attach the
     untrusted `<system-reminder>` to `mcpResult`; otherwise send `cleanResult`.
  3. Synchronously set `entry.committed = true; entry.sendPromise = send({ id, result: mcpResult });`.
  4. On `entry.sendPromise` resolving `true` -> `commitShown(key, token)`; on
     `false` -> `releaseShown(key, token)` + `markRelayAmbiguous`.
- **Retire `callKey` only after BOTH `facade.call` and `entry.sendPromise` settle (R2-H14)**:
  In `finally`: `try { if (entry.sendPromise) await entry.sendPromise.catch(() => {}); } finally { if (activeCalls.get(callKey) === entry) activeCalls.delete(callKey); }`.

### 6.5 Unambiguous Terminal-Only Web UI Diff Attachment (M19, R2-M14)

In `components/provider-runtime/packages/antigravity-acp/{plan-gate.mjs,proxy.mjs}`:

- Never emit speculative early `in_progress` diffs.
- In `PlanGate`: whenever 2 or more open announcements in
  `(sessionId, turn, attempt)` share the same tool name & argument digest (or
  an open inputless announcement conflicts with another call of the same tool),
  mark all colliding announcements `ambiguous = true` and clear any buffered
  `results` entry for them.
- `proxy.mjs::toolResult` buffers the clean diff (without `<system-reminder>`)
  only when `planGate.bind(call)` yields a unique unambiguous announcement, and
  attaches it only on `agy`'s terminal `status === "completed"` update (never
  misattributing a diff across reversed concurrent identical-argument calls!).

---

## 7. Compatibility, Rollback Verification & Coordinated Rollout

### 7.1 Populated-Ledger Fallback Recovery Verification (H5, R2-M9)

Before any Hawk or OVH Machine rollout carrying R4 (or combined R4 + R2):

1. Pin and test all **three** fallback worker binaries (automatic
   `previous.conf`, manual release rollback target, and per-session
   `fallback_targets`) against a populated ledger containing `active_processes`,
   `uncertain_sequence`, and pending effects.
2. For each fallback binary, assert the real `Ledger::open` recovery contract:
   - `Ledger::open` consumes `active_processes`, records `process/terminate`
     uncertainty, and persists an incremented durable `uncertain_sequence`;
   - A **second** reopen on the persisted ledger retains the incremented
     `uncertain_sequence` and session fence without losing pending obligations.
3. For a combined R4 + R2 rollout (Phase A dual-schema readers -> Phase B
   Schema-2 writer), run the identical recovery + second-reopen proof on
   populated Schema-2 input across every allowed fallback binary before
   enabling any Schema-2 writer.

### 7.2 Mixed-Version & Deployment Order (Owner Approval Required)

- **No `execution_host` change, no `zed` Plugin change, and no outer Machine
  WebSocket protocol bump (`Machine 53` unchanged).**
- Legacy Controller (`POST /api/machine/post-edit-inspect` absent, no
  `x-cowboy-machine-post-edit` upgrade header): `machine_cli` keeps
  `active_controller = None`; Broker replies `unsupported` to `Runtime 9`
  workers; Proxy skips note cleanly.
- Legacy Machine (`Runtime < 9`): Worker skips diagnostic requests; Proxy skips
  note cleanly.
- Because `components/wire/src/runtime_wire.rs` changes (`Runtime 9`) affect
  `retained-worker-interface-compatible`, keep all commits on isolated branch
  `feat/agy-post-edit-r4` until full gates, code review, and explicit owner
  approval for the coordinated `inputs.cowboy-workers` pin advance + Hawk/OVH
  rollout (or combined R4 + R2 pin advance).

---

## 8. Verification & Live Canary Acceptance

### 8.1 Automated Test Suite

- **Wire (`components/wire`)**: `Runtime 9` `PostEditInspectRequest/Reply`
  round-trip, `deny_unknown_fields`, size caps, and unchanged `Response::Ready`
  compatibility with old Machine/worker decoders.
- **Worker (`src/worker_execution`, `src/worker`, `src/account_sandbox`)**:
  - Unchanged `execution-connection.mjs` HTTP `Authorization` header auth +
    `!Origin` rejection on `"/"`; `/diagnostics` path isolation; saturated
    3-slot diagnostic handshakes never blocking mutation reconnect on
    `mutation_handshake_slot` or `cowboy/drained`.
  - `DrainState` `send_replace` with 0 initial receivers + replacement unit
    `end_drain()` generation increment aborting old diagnostics socket without
    ever calling `relays.socket_ended()`.
  - `WaiterGuard` drop cleanup on socket/handler cancel, `!waiter_tx.is_closed()`
    check at send, stale/expired withdrawal before byte 0, no upward deadline
    rounding, and three-tier populated-ledger recovery + second-reopen proof.
- **Machine Broker & `machine_cli` (`components/machine`)**:
  - Single-lock `PostEditRoutingState` under concurrent lease replacement and
    teardown; unique `broker_request_id` across two sessions with identical
    worker `request_id`; `worker.protocol < 9` receiving zero local refusal
    frames; stalled local Unix writers delivering reliable traffic ahead of
    diagnostics.
  - Independent HTTPS `POST /api/machine/post-edit-inspect` with per-connection
    token revoked immediately on primary WS close; stalled diagnostic HTTP call
    causing **zero** delay on primary WS `Stop`, `Heartbeat`, or
    `ExecutionReply`.
- **Controller (`components/server-runtime`, `components/server-agent-api`)**:
  - `authorize_step()` revocations (owner disabled, superagent restricted,
    language servers toggled off, turn changed from `turn-A` to `turn-B`,
    scope changed) tested at admission, before `acquire`, before `refresh`, and
    right before final synchronous baseline commit (proving zero commit and
    zero note).
  - `POST_EDIT_TARGET_SLOTS(4)` held through slow adapter settlement even after
    HTTP caller times out at 4 s; `< 500 ms` remaining refusing cold `acquire`.
  - `baselines.rs`: `conn_epoch` reconnect transition clearing `baseline` even
    when native stream IDs survive; numeric `buffer_epoch` ordering; strict
    vector-clock dominance for changed text; per-`(scope, path)` read
    serialization refreshing equal-version dependency-edit diagnostics;
    `matches_expected = false` suppressing nonempty notes & `current_errors`.
- **Proxy (`components/provider-runtime/packages/antigravity-acp`)**:
  - Per-effect `assertAuthorizedBeforeEffect()` tested across cancellation,
    plan-mode switch, and `UncertainEffect` during `current()`,
    `payloadDirectory()`, and `fs/writeFile` (verifying staged payload cleanup
    and zero `process/start` dispatch).
  - `serveMcp`: triple same-ID reuse with backpressured duplicate `-32600`
    refusal (verifying `activeCalls` stays held until `entry.sendPromise`
    settles); synchronous `isNoteStillFresh()` rejecting notes when an
    in-flight same-path write or `ReadFile` / conflict intervenes right before
    `stream.write`; `10 s` stalled-relay write timeout destroying stream and
    releasing `pendingShown` tokens and `activeCalls` IDs.
  - `extensions.mjs` & `facade.mjs`: BOM-preserving `lfSha256` parity with
    mixed `\r\n`/`\r`/`\n`; digest-mismatched nonempty reply rejected; 16 KiB
    post-trim `current_errors` vs `null`; ordinary vs E1 `{ verified, postEdit }`
    clean text for `WriteFile` & `EditFile`; `> 48 KiB` `--request-file` edit
    (normal + E1).
  - `plan-gate.mjs` & `proxy.mjs`: reversed ACP announcement and MCP completion
    order for two identical concurrent `EditFile` calls (one commits, one fails
    base check) verifying zero misattributed diff on either card.

### 8.2 Live Canary Acceptance With Native-Facing Relay Stdout Evidence (M25, R2-M16)

Gated strictly on an explicit canary-only env flag
(`COWBOY_CANARY_POST_EDIT_TRACE_FILE`, unset and forbidden in all production
units/configs):

- `facade-relay.mjs` line-frames outbound JSON-RPC replies written to its
  **native-facing `process.stdout`**, records `{ event: "canary.relay.stdout",
  mcp_call_id, write_ok, bytes, note_attached, nonce_sha256_match,
  canary_inspect_id }` in the `process.stdout.write` callback, proving the
  exact serialized bytes left the relay into `agy`'s stdin!
- Each canary edit generates a unique `canary_inspect_id` forwarded on
  `POST /api/machine/post-edit-inspect` and logged on Controller's
  `cowboy.code.post_edit_inspect` event (absent in production).

Run live acceptance on a dedicated free-tier Antigravity canary account on OVH
against a Hawk worktree:

1. **Positive control (Language Servers ON)**:
   - Unread file `b` defines a symbol emitting a fresh session-unique nonce in
     a versioned `document_current` LSP diagnostic on file `a`.
   - Model runs **only** `ReadFile(a)` and `EditFile(a)`.
   - Correlate all four artifacts:
     1. Complete native tool transcript (`ReadFile(a)` and the sole
        `EditFile(a)` only);
     2. `facade-relay.mjs` native-stdout capture (`write_ok: true`,
        `note_attached: true`, exact nonce match, `canary_inspect_id`);
     3. Controller log `cowboy.code.post_edit_inspect` matching exact
        `canary_inspect_id` (`matches_expected=true, deliver=true`);
     4. Model final answer reproducing the exact nonce, while Web UI `Edit`
        card shows clean diff without `<system-reminder>`.
2. **Isolated negative control (Language Servers OFF, distinct fresh nonce)**:
   - Collect the identical four artifacts proving sole `EditFile(a)`,
     `facade-relay.mjs` native-stdout capture with `write_ok: true,
     note_attached: false, nonce_sha256_match: false`, Controller log matching
     `canary_inspect_id` (`reason="restricted"`), and model output omitting the
     nonce.
3. Verify subagent edits and cancelled-turn edits deliver zero diagnostics and
   zero `UncertainEffect`.
