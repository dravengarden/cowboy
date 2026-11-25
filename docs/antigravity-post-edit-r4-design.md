# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 7 — Hardened Direct Negotiated Pull)

Status: design, revision 7 (2026-11-25). Hardens the direct, negotiated,
unsequenced Worker -> Machine Broker -> Controller pull RPC (`Machine 54` /
`Runtime 9`) and dedicated `mode: "diagnostics"` WebSocket architecture from
Revision 6 by closing all 25 findings (14 High, 11 Medium) from design review
`call-c52e6bcf17a0ab90432489af`.

- Baseline: `origin/main` at `960c811b6b03673b984ad97e84172d540dca23b6`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) (this
  document explicitly supersedes its §R4 48 KiB inline-only edit limit; see
  §7.3),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Disposition of All 25 Findings from `call-c52e6bcf17a0ab90432489af`

Every High (H1–H14) and Medium (M15–M25) finding from Round 1 of
`agy-post-edit-r4-direct-pull` (`call-c52e6bcf17a0ab90432489af`) is closed
directly in this revision:

| # | Severity | Rev 6 Finding (`call-c52e6bcf17a0ab90432489af`) | Rev 7 Closure |
|---|---|---|---|
| H1 | High | Path checks do not bind Zed to the keeper's directory (A->B->A swap during Zed resolution; LSP/buffer started on B) | **Closed in `zed-adapter` + Machine (§5.1–§5.3):** Carry `expectedRootIdentity: { device, inode }` through `ensureWorktree`, `prepareAgentAcquisition`, `agentAcquisition`, `bufferLanguage` (including `refresh: "agent_owned"`), `source_snapshot::check`, and `WorktreeLease` cache lookup. Each `WorktreeLease` holds an open `O_DIRECTORY \| O_CLOEXEC` directory fd plus a Linux ancestor-chain `inotify` monitor (`IN_MOVE_SELF \| IN_DELETE_SELF` on canonical components; `IN_Q_OVERFLOW` fail-closed). Any A->B->A swap during dispatch taints the lease, triggers immediate `Change::Remove` teardown (dropping any LSP started on B), and fails closed with zero diagnostics. Older `zed` Plugins lacking `root_pin_protocol >= 1` are refused fail-closed. |
| H2 | High | Rebinding a surviving keeper can certify a replacement cwd after Machine restart | **Closed in `execution_host` + `machine_cli/execution.rs` (§5.1):** Keeper opens `physical_cwd` with `O_RDONLY \| O_DIRECTORY \| O_CLOEXEC` at launch, retains `_cwd_handle: File` for its entire lifetime (preventing kernel inode reuse), attaches a Linux ancestor-chain `inotify` monitor (plus logical symlink-alias parent monitor when `cwd != physical_cwd`), latches `workspace_tainted` on any move/delete/overflow, and proves `workspace_root_pin: Option<WorkspaceRootPin>` in `Response::Ready`. Reattaching Machines mint/bind `workspaceRootIncarnation` **only** from keeper-supplied `workspace_root_pin` via `observe_pinned`; older surviving keepers or lost proof omit the capability (never minting from bare pathname observation). |
| H3 | High | Private ingress drops existing owner authorization (`read_parent` + `superagent::restricted`) | **Closed in Controller ingress (§4.1):** Enforce the exact same `read_parent(&state.hub, state.store.as_ref(), &meta)` (disabled/removed owner check) and `superagent::restricted(state, &meta)` checks as `actions.rs:551` at admission, before baseline commit, and before reply, bounded by the remaining inspection deadline. |
| H4 | High | Closing diagnostics during drain can mark real mutations uncertain (`relays.socket_ended()`), and bare `Notify` can miss drain | **Closed in `worker_execution.rs` (§3.1–§3.2):** **Only** the classified `"mutation"` socket calls `relays.socket_ended()`; pre-init failures and `"diagnostics"` sockets never touch `relays.socket_ended()`, `RelayGuard`, or `Ledger`. Replace `Notify` with a latched `tokio::sync::watch::Receiver<bool>` (`drain_rx`) subscribed and checked (`*drain.borrow_and_update()`) **before** answering diagnostics `initialize`, aborting the entire diagnostics task (including its WebSocket writer) immediately via `biased;` select on `begin_drain()`. |
| H5 | High | Schema 1 alone does not establish a safe worker rollback (predecessor of `d3b59b8ca` lacks `active_processes`) | **Closed in Rollout & Conformance (§8.1):** Stop relying on Schema 1 alone. Inventory, pin, and verify all **three** fallback tiers (automatic `previous.conf`, manual release rollback binary, per-session `fallback_targets`) against a populated ledger containing `active_processes`, `uncertain_seq`, and pending effects. For a combined R4+R2 rollout, enforce two-phase Schema 2 admission (prove preservation across all three fallback tiers before enabling any Schema-2 writer). |
| H6 | High | Queued file writes (`fileLocks`) can outlive their authorization (cancel, plan mode switch, uncertain read invalidation) | **Closed in `Facade` (§7.3):** Run the write transaction inside `fileLocks` and re-verify ALL four gates **after** acquiring `fileLocks` and **before** dispatching any helper effect: (1) original MCP call `signal` not aborted & `!this.closing`, (2) originating `turn` still live & unfenced, (3) owner mode grant `decision.confirm()` still unrestricted, (4) `session.observed.get(file) === preLockObserved` and `(this.invalidations.get(file) ?? 0) === preLockInvalidations`. Already-dispatched writes continue settling normally. |
| H7 | High | Zed envelope admits operations outside diagnostics (`openBuffer`/`closeBuffer`) and skips `MachinePluginStore::code_request` acquisition policy | **Closed in `session_code` + `MachinePluginStore` (§5.2):** Replace generic JSON `Request::Zed` with a **closed** typed enum `session_code::Request::ZedDiagnostic { root, incarnation, remaining_ms, operation: ZedDiagnosticOperation }` allowing only `ObserveLanguage`, `RefreshAgentBuffer`, and `AcquireAgentBuffer`. Machine derives `worktree` and `expectedRootIdentity` from trusted pinned state, validates `cowboy-agent-*` lease format, and dispatches through `MachinePluginStore::code_request_stamped` (existing `CodeAgentResourcePolicy` budgeted acquisition path!). |
| H8 | High | Controller timeouts release capacity while target work continues queueing/opening/refreshing buffers | **Closed in Machine target admission (§4.2, §5.2):** Carry `remaining_ms` (`<= 6_000 ms`) to target, enforce a dedicated target diagnostic permit (`MAX_TARGET_POST_EDIT_INFLIGHT = 4`, `try_acquire_owned()`, held inside the target task through native settlement regardless of Controller future drop), bound `route.lock()` wait via `timeout_at(deadline, ...)`, refuse expired queued work without touching Zed, and refuse cold `AcquireAgentBuffer` when `< 500 ms` remains. |
| H9 | High | In-flight caps leave shared transport writers unbounded (`worker.tx`, broker controller sender, WS bridges) | **Closed across all 6 writer hops (§3.4–§3.6):** Give diagnostic requests, replies, AND local refusals bounded drop-on-full low-priority queues selected strictly `biased;` below reliable frames + `SO_SNDBUF`/`TIOCOUTQ` Unix headroom checks (matching R1's `write_with_hints` / `run_worker_unix_writer` / `run_runtime_unix_writer_with_hints` pattern) across Worker->Broker, Broker->Worker, Broker->`machine_cli`, `machine_cli`->Controller WS, Controller->Machine WS, and `machine_cli`->Broker. Diagnostics never enter any unbounded reliable queue. |
| H10 | High | Protocol negotiation and cleanup are not tied to the admitting Controller lease; worker protocol unchecked before local refusal | **Closed in `machine_broker.rs` (§3.5):** Add atomic `Broker::post_edit_controller_target()` acquiring `self.controller.lock()` once to verify `outer >= 54 && runtime >= 9` and capture `(controller.lease, controller.diag_tx)`. Record `controller_lease` on `PendingPostEdit`, scope reply routing and `remove_controller(lease)` teardown strictly to that exact lease, and check `worker.protocol >= 9` before sending even a local refusal. |
| H11 | High | Dropping a waiter does not withdraw its queued request in `diag_rx` | **Closed in `worker.rs` + Controller (§3.4, §4.1):** Register `pending_post_edit[request_id]` **before** `diag_tx.try_send` (removing on enqueue failure). Bind each queued item to `(broker_connection_id, turn_id, expires_at)`. Worker writer re-verifies connection ID, waiter presence, active turn match, non-draining, `Instant::now() < expires_at` (recomputing `remaining_ms`), and headroom before byte 0 (dropping stale items silently), and drains `diag_rx` on disconnect. Controller re-verifies deadline + active turn/epoch before committing baselines. |
| H12 | High | Worker authentication does not establish that an edit committed | **Closed explicitly in Trust Model (§2, §4.1):** Classify `Frame::PostEditInspectRequest` explicitly as an **owner-authorized, session-scoped read** with the identical trust, authorization, quota (`MAX_PER_SESSION=4/MAX_PER_MACHINE=8`), and root-pinning boundary as public `cowboy.code.diagnostics` / `cowboy.code.post_edit` (zero claim of a trusted root-edit proof; proxy-supplied `path` and `expected_lf_sha256` are untrusted matching hints only). |
| H13 | High | Both socket semaphores are acquired after the unbounded part of admission | **Closed in `worker_execution.rs::Admission` (§3.1):** Acquire a bounded pre-initialize `handshake_slots = Semaphore(4)` **before** spawning any task, enforce a 4 KiB first-frame WS limit and 2 s handshake timeout, classify once (`"mutation"` vs `"diagnostics"`), transfer to dedicated `mutation_slot = Semaphore(1)` or `diagnostics_slot = Semaphore(1)`, and release `handshake_slots` immediately (with 64 KiB cap on diagnostics frames). |
| H14 | High | Duplicate MCP IDs can still dispatch another write or produce two replies (`facade.mjs:1875` deletes active entry; race during backpressured `stream.write`) | **Closed in `serveMcp` (§7.4):** Retain one single response owner (`entry`) in `activeCalls` through `finally` on **every** branch (never deleting on duplicate ID or `notifications/cancelled`). Latch `entry.committed = true` synchronously immediately before `stream.write`. Duplicate arrival aborts + suppresses `existing`, emits `-32600` **only** when `!existing.committed && !existing.duplicateRefused`, and never starts another `facade.call`. |
| M15 | Medium | Broker request ID can consume another caller's pending slot | **Closed in `machine_broker.rs` (§3.5):** Broker mints a globally unique `broker_request_id` per admission, keys `pending_post_edit` by `broker_request_id`, peeks **before** removing on reply, and validates all 5 fields (`controller_lease`, `session_id`, `worker_connection_id`, `worker_epoch`, `scope`) before consuming or expiring. |
| M16 | Medium | Adapter start counters cannot order observations across Machine or route lifetimes | **Closed in `CodeRuntimeHost` + `baselines.rs` (§5.3, §6.1):** Scope baseline ordering to `(machine_connection_epoch, route_generation, runtime_incarnation)` + native `BufferSnapshot { instance, buffer, buffer_epoch }`. `CodeRuntimeHost` increments `route_generation` on every route assignment/switch (including returning to an older live Plugin engine) and rejects delayed responses from retired routes. |
| M17 | Medium | Positive R4 incarnation suppresses subsequent public baseline updates (and target rollback 54->53) | **Closed in `baselines.rs` + public reads (§6.1):** Stamp **every** baseline writer (public `code.diagnostics`/`code.post_edit` AND private R4) on `Machine >= 54`. Order `OrderingEpoch::Stamped { conn_epoch, route_gen, runtime_incarnation }` vs `OrderingEpoch::Legacy { conn_epoch }` by Controller-assigned monotonic `conn_epoch` first, so target downgrade 54->53 establishes a strictly newer `conn_epoch` and fences cleanly into `Legacy` (returning `unknown("buffer_replaced")` across the restart). |
| M18 | Medium | Equal-quality diagnostic updates can never refresh the baseline (e.g. dependency edit clears/adds an error at identical file version) | **Closed in `zed-adapter` + `baselines.rs` (§5.3, §6.2):** Add monotonic `buffer.diagnostic_seq: u64` in `plugins/zed/adapter/src/diagnostics.rs` (incremented on every accepted `UpdateDiagnostics`) and expose `diagnosticSeq` on `Response::BufferLanguage`. Accept causally newer `diagnostic_seq` at equal text version/quality; reject strictly older `diagnostic_seq` (`unknown("out_of_order_observation")`); on legacy adapters omitting `diagnosticSeq`, return `unknown("unorderable_observation")` when diagnostics differ at equal version/quality. |
| M19 | Medium | Matching arguments cannot confirm which Web tool card committed (`PlanGate.matchUnbound`) | **Closed in `proxy.mjs` (§7.5):** Remove speculative early `in_progress` diff emission completely. Keep the clean edit diff buffered in `proxy.mjs::results` until `agy`'s own terminal `completed` update arrives (never painting a failed card!). |
| M20 | Medium | Known external content change does not invalidate an outstanding note (definite conflict updates neither counter) | **Closed in `Facade` (§7.3):** Track `fileObservations: Map<file, { generation, rawSha256 }>` updated on every `current()` read, `ReadFile`, base conflict (`rawSha256 = null`, bump `generation`), publish, and `UncertainEffect`. `isFresh()` checks unchanged `generation` and matching `rawSha256`. |
| M21 | Medium | Expired pending reservation can be finalized by its previous owner; write callback confirms relay handoff, not native receipt | **Closed in `extensions.mjs` (§7.2):** Separate `pendingShown: Map<key, token>` (no TTL expiry while pending, cap 64) from `settledShown: Map<key, expiresAtMs>` (10-min TTL, cap 256). Define canonical `key = sha256([sessionId, canonicalPath, lfSha256, noteDigest])`. `commitShown(key, token)` and `releaseShown(key, token)` act **only** when `pendingShown.get(key) === token`. Explicitly distinguish facade-to-relay handoff from native `agy` receipt. |
| M22 | Medium | Final trimming can produce a reminder with no finding (no-baseline error row trimmed by 16 KiB escaping) | **Closed in `post_edit.rs` + `extensions.mjs` (§6.3, §7.2):** Emit bounded `current_errors: u64` for no-baseline `document_current` files in `post_edit::project`. Re-evaluate delivery eligibility **after** both Controller 12 KiB and Proxy 16 KiB final escaping/trimming; return `null` (zero dedupe reservation) if neither diagnostic rows nor positive summary counts survive. |
| M23 | Medium | Proposed `publish` return value loses existing `if (verified)` E1 settlement distinction | **Closed in `Facade` (§7.3):** `publish()` returns `{ verified: boolean, postEdit }`. Both `writeFile` and `editFile` strictly check `if (published.verified)` for E1 byte-only settlement text vs ordinary success. |
| M24 | Medium | Revision 6 silently expanded the agreed edit-size scope above 48 KiB | **Closed explicitly (§2, §7.3, and `antigravity-parity-round-2.md` §R4):** Explicitly supersede the 48 KiB inline-only restriction across both documents. Cover all bounded UTF-8 facade writes up to `LIMITS.writeBytes` (3 MiB), specifying canonical path resolution, LF SHA-256 digesting, and E1 settlement for both inline `--request` and `--request-file` applies. |
| M25 | Medium | Canary lacks an evidence source proving the nonce first appeared in the actual MCP reply | **Closed in Live Acceptance (§9.2):** Specify a canary-only opt-in capture at the `agy`-facing MCP boundary in `serveMcp` (strictly disabled in production), correlated by `(session_id, mcp_call_id)` with the complete native tool transcript and Controller logs, with identical evidence required for the isolated negative control. |

---

## 2. Trust Model, Scope & End-to-End Architecture

### 2.1 Explicit Trust Classification (H12)

Per `docs/antigravity-real-accounts-design.md`, `agy-proxy` executes inside the
untrusted account sandbox unit. Therefore, **this design makes zero claim that
a `Frame::PostEditInspectRequest` cryptographically proves a root edit
committed.**

Instead, `Frame::PostEditInspectRequest` is explicitly authorized as an
**owner-authorized, session-scoped code diagnostic read** with the **exact same
trust, authorization, language-server switch, root-pinning, and resource-budget
boundary as public `cowboy.code.diagnostics` / `cowboy.code.post_edit`** (which
the session's agent already owns when the user enables `"Project language
servers"`):

- `path` and `expected_lf_sha256` carried on the request are **untrusted
  proxy-supplied matching hints**, used solely to check whether Zed's observed
  buffer matches the text the proxy expects and to project a post-edit diff
  against session baselines.
- Every request enforces:
  1. Authenticated active non-managed session worker
     `(machine_id, session_id, worker_epoch, binding)` on its enrolled runtime
     Machine connection;
  2. Live owner authorization (`read_parent` + `superagent::restricted`, H3);
  3. Active `"Project language servers"` switch (`LanguageServerTrust`, off
     fails closed with zero acquisition or LSP launch);
  4. Keeper-pinned `(device, inode)` root verification inside both the target
     Machine and `zed-adapter` (H1 + H2);
  5. Existing shared acquisition shares (`MAX_PER_SESSION = 4`,
     `MAX_PER_MACHINE = 8`, `MAX_WORKTREES_PER_MACHINE = 2`) and target
     `CodeAgentResourcePolicy` budgeting (H7 + H8).
- A compromised proxy therefore gains **zero** capability beyond what the
  session's own `code.diagnostics` / `code.post_edit` grant already permits.

### 2.2 Scope (Including Explicit 48 KiB Supersession, M24)

- **In scope**:
  - Parent-session `WriteFile` and `EditFile` calls on Antigravity sessions
    where `"Project language servers"` is enabled and the session is not
    managed/read-only.
  - All bounded UTF-8 facade writes up to `LIMITS.writeBytes` (3 MiB), covering
    **both** inline `--request` (`<= 48 KiB`) and `--request-file`
    (`> 48 KiB .. 3 MiB`) helper applies. This explicitly supersedes the 48 KiB
    inline-only limit recorded in `antigravity-parity-round-2.md` §R4 (which
    existed only when the Controller parsed inline `--request` payloads); see
    §7.3 for canonical path, LF digest, and E1 settlement rules for
    `--request-file` applies.
- **Out of scope**:
  - Subagent edits (`session.isSubagent`), managed read-only sessions,
    `RunCommand` shell edits, background commands, and non-UTF-8/binary writes.
  - Any mutation of session turn state, `ExecutionRequest` ledger, `RelayGuard`,
    or `UncertainEffect` latching.

### 2.3 End-to-End Flow (1 OVH<->Hawk RTT)

```text
agy (MCP tools/call EditFile/WriteFile)
  │
  ▼
serveMcp / Facade (account unit)
  1. Acquire per-file fileLocks[file]
  2. Re-check signal, live turn, modeGrant.confirm(), observed & invalidations (H6)
  3. Read current() + publishOnce() via existing mutation WS (unchanged!)
  4. On success: update fileObservations & committedRevision; release fileLocks
  5. Return { verified, postEdit } (M23); keep clean diff buffered in proxy.mjs (M19)
  6. If eligible & fresh & not cancelled:
       DiagnosticsClient (dedicated mode:"diagnostics" WS on /run/cowboy-exec.sock)
         │  JSON-RPC: diagnostics/post_edit { path, expectedLfSha256, timeoutMs }
         ▼
       Worker (src/worker_execution.rs + src/worker.rs)
         │  Frame::PostEditInspectRequest (Runtime 9, bounded low-priority lane, H9/H11)
         ▼
       Runtime Machine Broker (components/machine/src/machine_broker.rs)
         │  Mints unique broker_request_id (M15); atomic (lease, diag_tx) (H10)
         │  Bounded low-priority WS lane via machine_cli (H9)
         ▼
       Controller (components/server-runtime + server-agent-api + server-code)
         │  Auth worker + read_parent + superagent::restricted + Trust (H3/H12)
         │  Slot cap 4 (POST_EDIT_INSPECT_SLOTS); remaining_ms <= 6000
         │  session_code::Request::ZedDiagnostic (Machine 54, closed enum, H7)
         ▼
       Target Machine (machine_cli + MachinePluginStore::code_request_stamped + zed-adapter)
         │  Target diag permit cap 4 held through settlement (H8)
         │  Keeper-pinned (dev, ino) + ancestor inotify + WorktreeLease check (H1/H2)
         │  Returns BufferLanguage + diagnosticSeq (M18) + CodeRuntimeStamp (M16/M17)
         ▼
       Controller
         │  Re-verify read_parent + superagent + Trust + scope + deadline (H3/H11)
         │  Record scoped baseline (M16/M17/M18) + project <= 12 KiB (M22)
         │  Frame::PostEditInspectReply on bounded low-priority WS lane (H9)
         ▼
       Broker -> Worker -> DiagnosticsClient -> Facade
         │  Recheck isFresh() (turn, cancel, committedRevision, invalidations,
         │  fileObservations, M20) + format <= 16 KiB + re-verify non-empty (M22)
         │  Tokenized reserveShown(key) (M21)
         ▼
       serveMcp (single activeCalls owner, entry.committed before stream.write, H14)
         - Appends bounded untrusted <system-reminder> to MCP tool text reply ONLY
         - Clean diff in proxy.mjs attaches to agy's terminal completed card (M19)
         - Facade-to-relay flush commits token; write failure releases token (M21)
```

---

## 3. Wire Protocol, Admission Isolation & Low-Priority Transport Writers

### 3.1 Two-Stage Bounded Admission on `/run/cowboy-exec.sock` (H4, H13)

`src/worker_execution.rs::Admission` replaces the single post-accept
`Semaphore(1)` with **two-stage bounded admission** so neither pre-initialize
handshakes nor diagnostics connections can exhaust tasks/memory or block the
mutation socket:

1. **Stage 1 — Pre-initialize handshake cap (`handshake_slots: Arc<Semaphore>`,
   capacity `4`)**:
   - Acquired synchronously via `try_acquire_owned()` inside `Admission::admit`
     **before** spawning any task. If all 4 handshake slots are occupied, drop
     the accepted socket immediately (zero unbounded tasks or buffers).
   - Configure the handshake `WebSocketConfig` with tight pre-auth limits:
     `max_message_size = Some(4096)`, `max_frame_size = Some(4096)` (4 KiB),
     and wrap WS upgrade + first `initialize` JSON-RPC frame in a hard
     **2 second** total deadline.
   - Parse the first JSON-RPC frame once: validate `method == "initialize"`,
     constant-time compare `bearer_token`, and classify optional `params.mode`:
     - absent or `"mutation"` -> `Mode::Mutation`,
     - `"diagnostics"` -> `Mode::Diagnostics`,
     - anything else -> close immediately with error.
2. **Stage 2 — Permit transfer & mode isolation**:
   - `Mode::Mutation`:
     - Acquire dedicated `mutation_slot: Arc<Semaphore>` (capacity `1`) via
       `try_acquire_owned()`; drop `handshake_permit` immediately upon
       entering `serve_mutation`.
     - Upgrade WS read limits to `wire::MAX_FRAME_BYTES`.
     - **Mutation socket exclusivity for `socket_ended()` (H4):** Install a
       `MutationSocketGuard` whose `Drop` (and exit path) is the **sole**
       caller of `relays.socket_ended()`. Pre-initialize failures and
       `Mode::Diagnostics` connections **never** invoke `relays.socket_ended()`,
       never acquire `RelayGuard`, and never touch `Ledger`.
   - `Mode::Diagnostics`:
     - Acquire dedicated `diagnostics_slot: Arc<Semaphore>` (capacity `1`) via
       `try_acquire_owned()`; drop `handshake_permit` immediately.
     - Keep a tight **64 KiB** WS message/frame limit (`MAX_DIAG_WS_BYTES`).
     - Reject immediately if `client.binding.managed.is_some()` (managed
       read-only sessions never get diagnostics) or if `*drain_rx.borrow_and_update()`
       is already `true`.
     - Answer `initialize` with `{ ok: true, mode: "diagnostics",
       supported: broker_supports_post_edit }` (and abort immediately if
       `drain_rx` flips during the reply write; §3.2).

### 3.2 Latched Drain Isolation on `serve_diagnostics` (H4)

Add a latched `drain: watch::Sender<bool>` / `watch::Receiver<bool>` pair
(initialized to `false`) inside `RelayQuiet` (`src/account_sandbox.rs`, exposed
via `client.subscribe_drain()`):

- `RelayQuiet::begin_drain()` calls `let _ = self.drain.send(true);`
  synchronously **before** checking quiet state.
- Inside `serve_diagnostics`, **before** writing the `initialize` reply:
  ```rust
  let mut drain = client.subscribe_drain();
  if *drain.borrow_and_update() {
      return Ok(());
  }
  tokio::select! {
      biased;
      _ = wait_until_drained(&mut drain) => {
          // Drop the entire diagnostic connection future, including any
          // backpressured WebSocket writer, immediately.
      }
      _ = run_diagnostics_session(ws, client) => {}
  }
  ```
- Because `watch::Receiver<bool>` is latched (level-triggered, not
  edge-triggered like `Notify`), `begin_drain()` can **never** be missed
  between the initial check, `initialize` reply write, or subsequent frames.
- Aborting `run_diagnostics_session` drops its WebSocket sink immediately
  without waiting for a stalled reader, without touching `RelayQuiet` counts,
  and without calling `relays.socket_ended()`, so `cowboy/drained` completes
  unblocked while real mutation receipts remain completely unaffected.

### 3.3 Wire Protocol Additions (`Machine 54` / `Runtime 9`)

In `components/wire/src/machine_protocol.rs` and `components/wire/src/runtime_wire.rs`:

- Bump `CURRENT_PROTOCOL_VERSION` to `54` (`POST_EDIT_INSPECT_PROTOCOL_VERSION = 54`)
  and `RUNTIME_PROTOCOL_VERSION` to `9`
  (`POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION = 9`).
- Both remain additive and negotiated through existing `Hello`/`Welcome`
  `min_protocol..=max_protocol` + `outer_machine_protocol` stamping.
- Add unsequenced, non-retrying `Frame` variants (`#[serde(deny_unknown_fields)]`):
  ```rust
  PostEditInspectRequest {
      request: Box<PostEditInspectRequest>,
  },
  PostEditInspectReply {
      reply: Box<PostEditInspectReply>,
  },
  ```
  where:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct PostEditInspectRequest {
      pub session_id: String,
      pub worker_epoch: String,
      /// Worker-generated ID on Worker<->Broker leg; replaced with
      /// broker-minted globally unique correlation ID on Broker<->Controller leg (M15).
      pub request_id: String,
      /// Active native turn ID observed by Worker at admission (H11).
      pub turn_id: String,
      pub scope: crate::execution_protocol::Scope,
      /// Untrusted proxy-supplied workspace-relative POSIX path (<= 4096 bytes).
      pub path: String,
      /// Untrusted proxy-supplied 64-hex lowercase LF-normalized SHA-256 hint.
      pub expected_lf_sha256: String,
      /// Remaining end-to-end call budget in ms, clamped to <= 6_000 ms (H8/H11).
      pub remaining_ms: u64,
  }

  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct PostEditInspectReply {
      pub session_id: String,
      pub worker_epoch: String,
      pub request_id: String,
      pub scope: crate::execution_protocol::Scope,
      pub outcome: PostEditInspectOutcome,
  }

  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
  pub enum PostEditInspectOutcome {
      /// Bounded <= 12 KiB JSON payload matching code.post_edit shape plus
      /// optional `current_errors` count and `matches_expected: bool`.
      Projected {
          payload: serde_json::Value,
          matches_expected: bool,
      },
      /// Closed refusal / shed reason code (never raw error text):
      /// "unsupported" | "restricted" | "permission_denied" |
      /// "superagent_scope_denied" | "capacity" | "timeout" |
      /// "root_changed" | "not_open" | "unavailable".
      Refused {
          reason: String,
      },
  }
  ```
- **Hard frame size caps**: `PostEditInspectRequest <= 8 KiB`;
  `PostEditInspectReply <= 16 KiB` (Controller caps projected payload at
  `12 KiB`, leaving ample envelope headroom).

### 3.4 Worker-Side Dispatch, Waiter-First Registration & Stale Withdrawal (H9, H11)

In `src/worker_execution.rs` and `src/worker.rs`:

- `serve_diagnostics` validates each `diagnostics/post_edit` JSON-RPC request:
  - One in-flight diagnostic call per worker (`Semaphore(1)` inside
    `serve_diagnostics`; a second concurrent call fails immediately with
    `capacity` without reaching the broker).
  - Requires `client.binding.managed.is_none()`, active live non-cancelling
    turn `Some(turn_id)` from worker turn state, non-draining (`!*drain_rx.borrow()`),
    valid relative `path`, valid 64-char hex `expected_lf_sha256`, and
    `timeout_ms` clamped to `500..=6_000 ms`.
- **Waiter registered BEFORE enqueue (H11)**:
  1. Snapshot `(broker_connection_id, negotiated_protocol)` from the current
     worker->broker connection. If disconnected or `negotiated_protocol < 9`,
     return `{ state: "refused", reason: "unsupported" }` locally without
     enqueueing.
  2. Allocate unique `request_id` and compute
     `expires_at = Instant::now() + Duration::from_millis(timeout_ms)`.
  3. Insert `PendingWorkerPostEdit { waiter_tx, broker_connection_id, turn_id,
     expires_at }` into `pending_post_edit` **before** calling
     `diag_tx.try_send(QueuedPostEdit { request_id, broker_connection_id,
     turn_id, expires_at, frame })` (bounded channel cap `4`).
  4. If `try_send` returns `Full` or `Closed`, remove
     `pending_post_edit[request_id]` immediately and answer `capacity` /
     `unavailable`.
- **Stale withdrawal at send time (H9, H11)**:
  - `src/worker.rs`'s broker UnixStream writer selects reliable frames
    (`Snapshot`, `ExecutionRequest`, `Event`, `Heartbeat`, `Ack`) strictly
    `biased;` ahead of `diag_rx`.
  - Before writing **byte 0** of any popped `QueuedPostEdit`, the writer
    atomically verifies ALL FIVE conditions:
    1. `item.broker_connection_id == current_broker_connection_id`,
    2. `pending_post_edit.contains_key(&item.request_id)` (waiter still live!),
    3. Worker's active turn is still `Some(item.turn_id)` and `!draining`,
    4. `let Some(remaining) = item.expires_at.checked_duration_since(Instant::now())`
       with `remaining >= Duration::from_millis(250)` — updating
       `frame.remaining_ms = remaining.as_millis().min(6_000) as u64` right
       before serialization!
    5. `unix_socket_has_hint_headroom(&writer, frame_bytes)` succeeds (H9).
  - If **any** condition fails, drop `item` silently without writing a byte to
    the socket and remove/fail its waiter if still present.
  - On turn end, drain start, or broker disconnect: clear `pending_post_edit`
    (failing waiters immediately) AND drain/discard all items from `diag_rx`.
  - On receiving `Frame::PostEditInspectReply`: validate
    `(session_id, worker_epoch, scope)` and verify the waiter's recorded
    `broker_connection_id` and `turn_id` are still current before delivering.
    Never touch `state.calls`, `state.admitted`, `Client::frames()`, or
    `Ledger`.

### 3.5 Machine Broker Routing, Atomic Lease Capture & Unique Correlation IDs (H9, H10, M15)

In `components/machine/src/machine_broker.rs`:

- **Bounded broker pending table**: `MAX_BROKER_POST_EDIT = 4` across all
  sessions on the broker.
- **Worker-protocol guard (H10)**: On inbound `Frame::PostEditInspectRequest`
  from worker `(session_id, connection_id)`:
  - Look up `WorkerPeer` under `self.workers.lock()`. Validate:
    1. `worker.connection_id == connection_id`,
    2. `worker.epoch == request.worker_epoch`,
    3. `worker.snapshot.launch` has `execution_binding` matching `request.scope`
       with `managed.is_none()`,
    4. `worker.protocol >= 9` (captured at worker `Hello`/`Welcome`). If
       `worker.protocol < 9`, drop silently without sending any reply!
  - Capture `worker_diag_tx = worker.diag_reply_tx.clone()` (bounded cap `4`;
    never `worker.tx`!).
- **Atomic Controller negotiation + lease capture (H10)**:
  - Call `self.post_edit_controller_target()` which acquires
    `self.controller.lock()` **once**, verifies
    `controller.outer_machine_protocol >= Some(54) && controller.runtime_protocol >= 9`,
    and returns `Some((controller.lease, controller.diag_tx.clone()))`.
  - If `None` (no Controller or Controller `< 54/9`), send local refusal
    `Refused { reason: "unsupported" }` via `worker_diag_tx.try_send(...)`
    (drop-on-full; never on reliable `worker.tx`!).
- **Broker-minted correlation ID (M15)**:
  - Never key `pending_post_edit` by the worker-supplied `request_id`!
  - Mint a globally unique `broker_request_id = format!("pe-{}-{:016x}",
    self.broker_nonce, self.next_post_edit_seq.fetch_add(1, Relaxed))`.
  - Check `pending_post_edit.len() < MAX_BROKER_POST_EDIT`; if full, shed via
    `worker_diag_tx.try_send(Refused { reason: "capacity" })`.
  - Insert `PendingPostEdit { worker_request_id, controller_lease, session_id,
    worker_connection_id, worker_epoch, scope, expires_at }` keyed by
    `broker_request_id`, rewrite `request.request_id = broker_request_id`, and
    enqueue on `controller_diag_tx.try_send(...)`. If `Full`/`Closed`, remove
    `pending_post_edit[broker_request_id]` and emit `capacity`/`unavailable` on
    `worker_diag_tx.try_send`.
  - Schedule expiry cleanup (bounded at `request.remaining_ms + 1_000 ms`) that
    removes the entry **only** if `(broker_request_id, controller_lease,
    worker_connection_id)` still matches.
- **Scoped reply validation (H10, M15)**:
  - On inbound `Frame::PostEditInspectReply` arriving on Controller `reply_lease`:
    1. **Peek** `pending_post_edit.get(&reply.request_id)` **without removing**.
    2. Validate ALL FIVE fields match:
       - `entry.controller_lease == reply_lease`,
       - `entry.session_id == reply.session_id`,
       - `entry.worker_epoch == reply.worker_epoch`,
       - `entry.scope == reply.scope`,
       - live `workers.get(&entry.session_id)` still has
         `connection_id == entry.worker_connection_id && epoch == entry.worker_epoch && protocol >= 9`.
    3. Only when all five match: remove `entry`, restore
       `reply.request_id = entry.worker_request_id`, and send via
       `worker.diag_reply_tx.try_send(...)`. Mismatched replies are logged and
       dropped without disturbing any live slot.
- **Lease-scoped teardown (H10)**:
  - `remove_controller(closed_lease)` removes **only** `pending_post_edit`
    entries where `entry.controller_lease == closed_lease`, failing each live
    matching worker (`protocol >= 9`) via `worker.diag_reply_tx.try_send(Refused { reason: "unavailable" })`.
    Entries admitted on a newer Controller lease are untouched.
  - `remove_worker(session_id, closed_conn_id)` removes only entries matching
    `(session_id, closed_conn_id)`.

### 3.6 Bounded Low-Priority Admission Across All 6 Transport Writers (H9)

Diagnostics (requests, replies, **and** local capacity/unsupported refusals)
**never** enter any unbounded reliable queue (`worker.tx`, `controller.tx`,
`machine_write_tx`, or `runtime_write_tx`). Every single hop uses a dedicated
bounded drop-on-full channel selected strictly `biased;` below reliable frames
(extending the proven R1 `wrap_up` pattern):

| # | Hop | Reliable Queue | Bounded Low-Priority Diagnostic Lane | Headroom / Backpressure Behavior |
|---|---|---|---|---|
| 1 | Worker -> Broker (`src/worker.rs`) | Reliable Unix writer loop (`Snapshot`, `ExecutionRequest`, `Event`, `Ack`, `Heartbeat`) | `diag_tx/diag_rx` (cap `4`, `try_send`) | `biased;` below reliable; `unix_socket_has_hint_headroom` before byte 0 + stale/expiry recheck (H11); drop on full/no headroom |
| 2 | Broker -> Worker (`machine_broker.rs` + `wrap_up.rs`) | `worker.tx` (unbounded reliable commands/acks/execution replies) | `worker.diag_reply_tx` / `worker_diag_rx` (cap `4`, `try_send`) | `biased;` below `reliable_rx` and `hint_rx` in `run_worker_unix_writer`; `unix_socket_has_hint_headroom` before byte 0; drop on full/no headroom |
| 3 | Broker -> `machine_cli` (`machine_broker.rs`) | `controller.tx` (unbounded reliable snapshots/acks/execution requests) | `controller.diag_tx` / `controller_diag_rx` (cap `8`, `try_send`) | `biased;` below `reliable_rx` in controller Unix writer; `unix_socket_has_hint_headroom` before byte 0; drop on full/no headroom |
| 4 | `machine_cli` -> Controller WS (`machine_cli.rs` + `machine_transport.rs`) | `machine_write_tx` (unbounded reliable WS frames) | `diag_ws_tx` (cap `8`, `try_send`) passed to `machine_transport::write_with_hints` (generalized for low-priority text queues alongside hints) | `biased;` strictly below `incoming` (reliable, heartbeats, paced execution RPCs); drop on full |
| 5 | Controller -> Machine WS (`machine_connection.rs` + `machine_transport.rs`) | `machine_write_tx` (unbounded reliable WS frames) | `diag_ws_tx` (cap `8`, `try_send`) into `write_machine_messages_with_hints` | `biased;` strictly below reliable `machine_write_tx` and heartbeats; drop on full |
| 6 | `machine_cli` -> Broker (`machine_cli/wrap_up_hint.rs`) | `runtime_write_tx` (unbounded reliable CoreCommands/acks) | `diag_runtime_tx` (cap `8`, `try_send`, admitted only when `outer_protocol >= 54` and frame `<= 16 KiB`) | `biased;` below `reliable` and `hints` in `run_runtime_unix_writer_with_hints`; `unix_socket_has_hint_headroom` before byte 0; drop on full/no headroom |

All 6 writers emit complete newline/length-prefixed frames atomically with zero
mid-frame cancellation. Stalling any writer with reliable traffic (`Stop`,
`CommandAck`, `Ack`, `ExecutionRequest`/`ExecutionReply`, `Heartbeat`) pending
delivers reliable frames strictly ahead of diagnostics and sheds diagnostics
cleanly.

---

## 4. Controller Ingress, Owner Authorization & Concurrency Bounds

### 4.1 Controller Private Ingress & Authorization Recheck (H3, H11, H12)

`code.post_edit_inspect` is **not** added to `Action::AgentInvoke`,
`cowboy_execute`, or any model-facing HTTP/MCP route (`Action::AgentInvoke`
rejects unknown action names as `unknown_operation`).

`Frame::PostEditInspectRequest` is handled solely on the authenticated Machine
runtime connection (`components/server-runtime/src/server/machine_connection/frames.rs`):

1. **Negotiation & shape check**: Verify this Machine connection negotiated
   `outer_machine_protocol >= 54 && runtime_protocol >= 9`; validate
   `request.path` (bounded relative POSIX path, no `..`, `<= 4096`),
   `request.expected_lf_sha256` (64 lowercase hex), and clamp
   `remaining_ms = request.remaining_ms.clamp(250, 6_000)` ->
   `deadline = Instant::now() + Duration::from_millis(remaining_ms)`.
2. **Dedicated concurrency gate**: Acquire one permit from
   `POST_EDIT_INSPECT_SLOTS` (cap `4` global / per-Machine `try_acquire_owned()`,
   completely independent of `execution_calls` and `AgentRead`). If full, reply
   `Refused { reason: "capacity" }` on the bounded low-priority WS lane (§3.6).
3. **Worker & session binding authentication**:
   - Verify the session's live runtime worker in `state.hub` is currently
     attached on **this exact runtime Machine connection**, with exact matching
     `(session_id, worker_epoch)`, exact matching non-managed `execution_binding`
     (`Scope::from_binding == request.scope`), and an active running turn.
   - Load `SessionMeta` for `session_id` (refuse `"unavailable"` if missing or
     managed/system session).
4. **Full owner & superagent authorization at admission (H3)**:
   - Run under `tokio::time::timeout_at(deadline, ...)`:
     - `read_parent(&state.hub, state.store.as_ref(), &meta).await` -> on error,
       reply `Refused { reason: "permission_denied" }` (blocks disabled or
       removed owners, identical to `actions.rs:551`).
     - `superagent::restricted(state, &meta).await` -> if restricted, reply
       `Refused { reason: "superagent_scope_denied" }` (blocks restricted
       superagent scopes, identical to `actions.rs:551`).
   - Capture `LanguageServerTrust` (`observe_language_server_trust`) and
     `SessionReadScope` (bound to the target execution Machine's exact
     `ConnectionToken` and its monotonic `machine_connection_epoch`, §6.1),
     plus the session's keeper-proven `workspaceRootIncarnation` (§5.1). If
     trust is off -> `Refused { reason: "restricted" }`. If target Machine `< 54`
     or `workspaceRootIncarnation` is absent -> `Refused { reason: "unsupported" }`.
5. **Bounded observation on target Machine**:
   - Execute the closed `session_code::Request::ZedDiagnostic` sequence (§5.2)
     on the captured `ConnectionToken`, passing the live recomputed
     `remaining_ms` on each step (`ObserveLanguage` -> optional
     `AcquireAgentBuffer` or `RefreshAgentBuffer` -> bounded poll up to
     `deadline`, with 250 ms sleep clamped to `deadline`).
   - Never call `touch_code_observation`, never mutate `sync_cursor`, and never
     write to `deliveries`.
6. **Mandatory pre-commit & pre-reply re-verification (H3, H11)**:
   - Immediately **before** committing any baseline record in `baselines.rs`
     AND **before** sending `PostEditInspectReply`:
     - Check `Instant::now() < deadline`.
     - Re-verify live worker `(machine_id, session_id, worker_epoch, scope)`
       and active turn in `state.hub`.
     - Re-verify `trust.current(&state.hub)` and `session::current(...)`.
     - Re-run `read_parent(&state.hub, state.store.as_ref(), &meta)` and
       `superagent::restricted(state, &meta)` under the remaining deadline!
     - If **any** recheck fails or times out: commit **nothing** to
       `baselines.rs` and reply `Refused { reason: "permission_denied" |
       "superagent_scope_denied" | "restricted" | "timeout" }`.
7. **Baseline commit & bounded projection**:
   - Compute `matches_expected` by comparing
     `bufferSnapshot.content.sha256 == request.expected_lf_sha256` on a
     `document_current` observation (false if `stale`, `last_observed`, or
     missing snapshot).
   - Commit the baseline using the scoped causal ordering rule (§6.1–§6.2),
     project via `post_edit::project` with the 12 KiB progressive tail-trimmer
     and post-trim eligibility check (§6.3), and send
     `PostEditInspectReply` on the bounded low-priority WS lane (§3.6).

### 4.2 End-to-End Deadline Propagation (H8, H11)

Every hop carries and decrements `remaining_ms` against a single end-to-end
budget (proxy default **4_000 ms**, hard cap **6_000 ms**):

- Proxy `DiagnosticsClient` owns a single whole-call `AbortController` timer
  covering WS connect + initialize + request + reply.
- Worker clamps `timeout_ms` to `500..=6_000 ms` and recomputes `remaining_ms`
  right before writing byte 0 to Broker (dropping if `< 250 ms` remains).
- Controller clamps `remaining_ms <= 6_000 ms`, derives one `deadline`, and
  passes the freshly computed remaining milliseconds on every
  `session_code::Request::ZedDiagnostic` RPC to the target Machine.
- Target Machine clamps `remaining_ms <= 6_000 ms` and enforces `deadline`
  against worktree-lock acquisition and adapter exchange (§5.2).

---

## 5. Target-Side Keeper Root Pinning, Closed Zed Envelope & Atomic Adapter Binding

### 5.1 Keeper-Pinned Directory Identity Across Machine Restarts (H2)

Today `execution_host.rs` stores `inode: (u64, u64)` at keeper launch without
holding an open directory handle or watching for A->B->A swaps, and does not
prove that pinned identity to a reattaching Machine. Revision 7 closes this in
`components/machine/src/execution_host.rs` and
`components/machine/src/machine_cli/execution.rs`:

1. **Keeper creation (`execution_host.rs::run`)**:
   - Resolve `physical_cwd = std::fs::canonicalize(&cwd)` and open
     `physical_cwd` with `O_RDONLY | O_DIRECTORY | O_CLOEXEC`.
   - Retain `_cwd_handle: std::fs::File` inside `Host` for the keeper's entire
     lifetime so the kernel **cannot** recycle `(dev, ino)` even if the
     directory is unlinked and recreated.
   - Read `inode = (metadata.dev(), metadata.ino())` directly via
     `_cwd_handle.metadata()` (never a separate path `stat`).
   - On Linux, attach a non-blocking `inotify` monitor (`RootMonitor`) inside
     `Host`:
     - Watch **every** canonical directory component from `/` down to
       `physical_cwd` with `IN_MOVE_SELF | IN_DELETE_SELF` (zero sibling file
       noise, so busy parent directories like `/tmp` never spam events!).
     - When logical `cwd != physical_cwd` (storage-move directory symlink
       alias), additionally watch the logical symlink's ancestor chain with
       `IN_MOVE_SELF | IN_DELETE_SELF` and the symlink's immediate parent with
       `IN_MOVED_FROM | IN_MOVED_TO | IN_DELETE | IN_CREATE` matching the
       symlink basename.
     - Any matching event **or** `IN_Q_OVERFLOW` permanently latches
       `workspace_tainted = true` (fail closed).
   - `Host::current_workspace()` checks:
     1. Drain non-blocking `RootMonitor`: if any event or overflow fired
        (now or previously), return `false`;
     2. Open `self.cwd` with `O_RDONLY | O_DIRECTORY | O_CLOEXEC`, verify
        `fstat == self.inode` AND `std::fs::canonicalize(&self.cwd) == physical_cwd`.
2. **Keeper proof in `Response::Ready`**:
   - Add additive optional field on `execution_protocol::Response::Ready`:
     ```rust
     #[serde(default, skip_serializing_if = "Option::is_none")]
     workspace_root_pin: Option<WorkspaceRootPin>,
     ```
     where `WorkspaceRootPin { device: u64, inode: u64, physical_path: String }`.
   - `Command::Describe` already calls `self.current_workspace()` and returns
     `Err(Refusal::WorkspaceChanged)` whenever the workspace was moved,
     replaced, or tainted. Only when `self.current_workspace()` passes does it
     return `Response::Ready` containing `workspace_root_pin: Some(...)`.
3. **Reattaching Machine admission (`machine_cli/execution.rs` +
   `workspace_identity.rs`)**:
   - Both fresh `start_keeper` and reattach (`prepare` / `prepare_managed` /
     `Describe` after Machine restart) require `workspace_root_pin: Some(pin)`
     from the keeper!
   - Add `SessionRootIdentities::observe_pinned(logical_cwd, &pin)`:
     - Opens `logical_cwd` with `O_RDONLY | O_DIRECTORY | O_CLOEXEC`,
     - Verifies `fstat == (pin.device, pin.inode)` AND
       `canonicalize(logical_cwd) == Path::new(&pin.physical_path)`,
     - Stores the retained `TrackedRoot` (holding an open dirfd + `pin`) in a
       keeper-pinned map bounded by the existing `512` keeper limit (so FIFO
       eviction of unrelated non-keeper session observations never evicts an
       active keeper pin!).
   - **Fail closed on legacy keepers or lost proof**: If an older surviving
     keeper returns `workspace_root_pin: None`, or `Describe` fails with
     `WorkspaceChanged`, or `observe_pinned` fails/mismatches, the Machine
     **omits** `workspaceRootIncarnation` (never minting a token from bare
     pathname observation!). R4 diagnostics stay unavailable for that session.

### 5.2 Closed `session_code::Request::ZedDiagnostic`, Target Permit & Budgeted Acquisition (H7, H8)

Replace Revision 6's generic `session_code::Request::Zed { request: Value }`
with a **closed**, typed diagnostic request in
`components/wire/src/machine_protocol/session_code.rs` (`Machine 54`):

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZedDiagnosticOperation {
    ObserveLanguage {
        path: String,
    },
    RefreshAgentBuffer {
        path: String,
    },
    AcquireAgentBuffer {
        path: String,
        lease_id: String,
        #[serde(default)]
        refresh_only: bool,
    },
}

// Additive variant on session_code::Request (Machine 54):
ZedDiagnostic {
    root: String,
    incarnation: String,
    remaining_ms: u64,
    operation: ZedDiagnosticOperation,
}
```

Target Machine admission (`components/machine/src/machine_cli/session_code.rs`,
`machine_cli.rs`, `machine_plugins/code_runtime.rs`, `machine_code_plugins.rs`):

1. **Dedicated target diagnostic concurrency permit (H8)**:
   - Add `post_edit_diagnostics: Arc<Semaphore>` (capacity
     `MAX_TARGET_POST_EDIT_INFLIGHT = 4`, completely independent of interactive
     Code operations!) on `MachineCommandContext` / `MachinePluginStore`.
   - Acquire `let _target_permit = post_edit_diagnostics.try_acquire_owned()?;`
     before waiting on any worktree lock or touching `CodeRuntimeHost`; if
     exhausted, refuse immediately with `"capacity"`.
   - **Hold `_target_permit` inside the spawned target task through native
     settlement** (never releasing it early when a Controller timeout drops the
     calling future!).
2. **Closed validation & trusted derivation (H7)**:
   - Reject any relative/empty/oversized `root`, verify `root` is trusted under
     configured workspaces / `worktree_roots`, and call
     `session_identities.lock().verify_pinned(&root, &incarnation)` returning
     the keeper-proven `WorkspaceRootPin { device, inode, physical_path }`
     (failing closed if missing, unpinned, or changed!).
   - Validate `path` (non-empty relative POSIX path, `<= 4096`, `Component::Normal`
     only — no `..`, no `/`, no NUL).
   - For `AcquireAgentBuffer`, validate `lease_id` starts with `"cowboy-agent-"`
     and passes `validate_lease_id` (cannot spoof interactive or arbitrary
     leases).
   - Clamp `remaining_ms = remaining_ms.min(6_000)` ->
     `deadline = Instant::now() + Duration::from_millis(remaining_ms)`. If
     `AcquireAgentBuffer` has `< 500 ms` remaining (or any operation is already
     expired), refuse immediately before touching `CodeRuntimeHost`.
   - Machine constructs the closed Zed adapter JSON payload itself from trusted
     state — `openBuffer`, `closeBuffer`, `ensureWorktree`, `openWorktree`,
     `closeWorktree`, `bufferHover`, `bufferNavigate`, `bufferSymbols`, and
     arbitrary JSON are **unrepresentable**:
     - `ObserveLanguage` -> `{"type":"bufferLanguage","worktree":physical_path,"path":path,"expectedRootIdentity":{"device":d,"inode":i}}`
     - `RefreshAgentBuffer` -> `{"type":"bufferLanguage","worktree":physical_path,"path":path,"refresh":"agent_owned","expectedRootIdentity":{"device":d,"inode":i}}`
     - `AcquireAgentBuffer` -> `{"type":"acquireAgentBuffer","ownerProtocol":2,"worktree":physical_path,"path":path,"leaseId":lease_id,"refreshOnly":refresh_only,"expectedRootIdentity":{"device":d,"inode":i}}`
3. **Routes through `MachinePluginStore::code_request_stamped` (H7, H8, M16, M17)**:
   - `MachinePluginStore::code_request_stamped(plugin_id, payload, legacy, Some(deadline))`
     uses the **exact** existing `CodeAgentResourcePolicy` budgeted path
     (`acquire_agent` for `acquireAgentBuffer`, `request_active` for
     `bufferLanguage`) with three additions:
     - **Deadline-bounded worktree lock wait (H8)**: Acquire `route.lock()` via
       `tokio::time::timeout_at(deadline, route.lock())`. If expired while
       queued behind an interactive Code operation, abort immediately without
       touching Zed and drop `_target_permit`!
     - **Capability floor for pinned root**: Before dispatching a payload
       carrying `expectedRootIdentity`, verify the selected `RunningCodeRuntime`
       advertised `root_pin_protocol >= 1` during its `agentAcquisitionSupport`
       probe (cached per `RunningCodeRuntime`). Legacy sockets or older `zed`
       Plugin generations without `root_pin_protocol >= 1` refuse R4 fail-closed
       (`"unsupported"`).
     - **Route generation & runtime incarnation stamp (M16, M17)**:
       - `CodeRuntimeHost` holds `next_route_generation: AtomicU64` and
         `next_runtime_incarnation: AtomicU64`.
       - Every `RunningCodeRuntime::start_tracked` assigns a fresh monotonic
         `incarnation: u64`.
       - Every time `route.target` is assigned or switched (including when
         following a rollback to an older still-running Plugin engine in
         `self.engines`!), increment `route.route_generation`.
       - Capture `dispatched_gen = route.route_generation` and
         `dispatched_incarnation = runtime.incarnation` before `exchange`. If
         the route retired or switched before the reply returns, reject the
         delayed reply (`"code runtime route retired"`).
       - Return `{ "response": <zed_reply>, "runtimeStamp": { "routeGeneration": g, "runtimeIncarnation": i } }`
         on `ZedDiagnostic`, **and** attach the same additive `runtimeStamp`
         to ordinary `Machine >= 54` `zed` `bufferLanguage` responses so public
         and private readers share one ordering domain (M17)!

### 5.3 Atomic Directory Binding & `diagnostic_seq` Inside `zed-adapter` (H1, M18)

Changes inside `plugins/zed/adapter/src/` (released as a new `zed` Plugin
generation with `root_pin_protocol: 1` on `Response::AgentAcquisitionSupport`):

1. **`PinnedRoot` on `WorktreeLease` (H1)**:
   - Add `RootIdentity { device: u64, inode: u64 }` and `PinnedRoot`:
     - Opens the canonical `worktree` via component-by-component `O_NOFOLLOW |
       O_DIRECTORY | O_CLOEXEC` walk from `/` (rejecting any symlink component),
     - Verifies leaf `fstat == expected`,
     - Retains the open directory `std::fs::File` (preventing inode reuse),
     - Attaches a non-blocking Linux `inotify` watch (`IN_MOVE_SELF |
       IN_DELETE_SELF`) on every canonical directory component from `/` down to
       `worktree` (zero sibling file noise; `IN_Q_OVERFLOW` fails closed),
     - Exposes `is_clean(&self) -> bool` which drains the non-blocking
       `inotify` fd (latching `tainted = true` on any move/delete/overflow)
       AND re-verifies the `O_NOFOLLOW | O_DIRECTORY` walk still lands on
       `expected`!
   - **Cache lookup (`trust::ensure_captured`, `trust::ensure_language`,
     `language_observation::buffer_language`, `agent_refresh::refresh`,
     `agent_acquisition::Registry::prepare/act`)**:
     - Whenever a request carries `expectedRootIdentity: Some(expected)`:
       - Before reusing any cached `WorktreeLease` or `buffers.active` entry
         for `worktree`, require `lease.pinned_root.as_ref().is_some_and(|pin|
         pin.identity == expected && pin.is_clean())`!
       - If an existing `WorktreeLease` at `worktree` holds a different
         `identity`, has `pinned_root.is_none()`, or is tainted: refuse
         immediately (and tear down via `Change::Remove` if idle) — **never**
         serve B's cached worktree or buffer to A!
   - **Atomic dispatch & teardown on in-flight A->B->A replacement**:
     - Verify `PinnedRoot::open_and_verify(&worktree, expected)` **before**
       dispatching `Change::Add`, `OpenBuffer`, `ReloadBuffers`, or
       `zed.language(...)`.
     - Immediately **after** `Change::Add` / `OpenBuffer` / `ReloadBuffers` /
       `zed.language(...)` returns (and before returning any reply to caller),
       re-check `pin.is_clean()`.
     - Because `inotify` catches any `rename` or `rmdir` of `worktree` or any
       ancestor in-kernel — even if swapped `A -> B -> A` in 10 microseconds
       while Zed ran — `pin.is_clean()` returns `false`!
     - On taint detection: permanently mark `lease.tainted = true`, invoke
       `Change::Remove` (dropping the native worktree and any language servers
       Zed started on B!), release/retire the acquisition entry, and return
       `Err("worktree root changed during Zed operation")` with zero
       diagnostics!
   - **`source_snapshot::check` binding**:
     - Accept `expected_root: Option<RootIdentity>` (passed from
       `language_observation::buffer_language` and `agent_refresh::refresh`).
     - In `source_snapshot::identity_with`: after opening `directory` via
       `open(root, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`, verify
       `fstat(&directory) == (expected.device, expected.inode)` **before**
       `openat2(&directory, path, BENEATH | NO_SYMLINKS | NO_XDEV)`. If it
       mismatches, return `State::Unavailable` immediately.
2. **Per-buffer monotonic `diagnostic_seq: u64` (M18)**:
   - In `plugins/zed/adapter/src/diagnostics.rs::Buffer`, add
     `diagnostic_seq: u64` (initialized to `0` on `CreateBufferForPeer::State`,
     incremented `checked_add(1)` on every accepted `UpdateDiagnostics` in
     `Cache::operations`).
   - Expose additive `#[serde(skip_serializing_if = "Option::is_none")]
     diagnostic_seq: Option<u64>` (camelCase `diagnosticSeq`) on
     `Response::BufferLanguage` (read under the same `diagnostics` lock as
     `cache.read(buffer_id, revision)`!).

---

## 6. Controller Baselines & Bounded Projection

### 6.1 Scoped Ordering Across Machine Restarts, Route Switches & Target Downgrades (M16, M17)

In `components/server-agent-api/src/server/agent_api/code_reads/baselines.rs`:

- Assign every Machine WebSocket connection in `MachineControl` a Controller-local
  strictly monotonic `machine_connection_epoch: u64` (across all Machines,
  whether `< 54` or `>= 54`; local adapter uses `0`).
- Define the capture ordering epoch:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub(super) enum OrderingEpoch {
      /// Machine >= 54 (used by BOTH public code.diagnostics/code.post_edit
      /// AND private Frame::PostEditInspectRequest!).
      Stamped {
          conn_epoch: u64,
          route_gen: u64,
          runtime_incarnation: u64,
      },
      /// Machine < 54 or local adapter.
      Legacy {
          conn_epoch: u64,
      },
  }
  ```
- **Comparing two `OrderingEpoch` values**:
  1. Compare `conn_epoch` **first**:
     - `incoming.conn_epoch < existing.conn_epoch` -> stale delayed reply from
       a retired Machine connection; **reject without updating**
       (`unknown("out_of_order_observation")`).
     - `incoming.conn_epoch > existing.conn_epoch` -> **fenced transition to a
       newer Machine connection!** Accept the new observation and reset
       `latest_order` to `incoming` — this handles Machine restarts **and**
       actual target downgrades from `Machine 54` (`Stamped { conn_epoch: 5 }`)
       to `Machine 53` (`Legacy { conn_epoch: 6 }`) cleanly (M17)! Comparison
       against the prior capture still checks `baseline.stream == current.stream`
       and returns `unknown("buffer_replaced")`.
  2. Within the **same** `conn_epoch`:
     - A `Machine >= 54` connection stamps **every** writer (public
       `code.diagnostics`, public `code.post_edit`, and private R4) with
       `Stamped { conn_epoch, route_gen, runtime_incarnation }`, so public and
       private reads never conflict (M17).
     - Compare `route_gen` first, then `runtime_incarnation`:
       - Smaller `(route_gen, runtime_incarnation)` -> stale delayed reply from
         a retired route/adapter -> reject (`unknown("out_of_order_observation")`).
       - Larger `(route_gen, runtime_incarnation)` -> newer route or adapter
         restart (including a route switching back to an older live Plugin
         engine, since `route_gen` still strictly increased, M16!) -> advance
         `latest_order` cleanly; `compare()` returns `unknown("buffer_replaced")`
         because `snapshot.instance` / `stream` differs.

### 6.2 Causal Refresh at Equal Text Version & Equal Quality (M18)

Within the same `OrderingEpoch` and the same `BufferSnapshot.stream`
(`(instance, buffer, buffer_epoch)`):

- Separate `record.latest_order: CaptureOrder` (causal watermark:
  `(ordering_epoch, stream, version, diagnostic_seq: Option<u64>)`) from
  `record.latest: Capture` (evidence capture).
- When `incoming.sha256 == record.latest.sha256 && incoming.utf8_bytes == record.latest.utf8_bytes`:
  1. Compare vector clock `version`:
     - Strictly older `version` -> reject as out-of-order (`unknown("out_of_order_observation")`).
     - Strictly newer `version` -> advance `latest_order` and replace `latest`.
  2. At equal `version`, compare `diagnostic_seq` (from `zed-adapter`, §5.3):
     - `(Some(new_seq), Some(old_seq))`:
       - `new_seq > old_seq` -> **causally newer diagnostic publication** (e.g.
         a dependency edit cleared or introduced an error while this file's text
         stayed identical!) -> advance `latest_order` and **refresh `latest`**
         even at equal quality!
       - `new_seq < old_seq` -> stale out-of-order reply -> do NOT regress
         `latest`, return `unknown("out_of_order_observation")`.
       - `new_seq == old_seq` -> same diagnostic publication -> keep higher
         quality (`entries.is_some()` over `None`; larger server set over
         subset) without regressing.
     - Legacy adapter lacking `diagnostic_seq` (`None`):
       - If diagnostics/servers are identical -> keep higher quality.
       - If diagnostics differ at equal version & equal quality -> causal order
         cannot be established; do **not** guess or overwrite blindly, and
         return `unknown("unorderable_observation")`.
- When `incoming.sha256 != record.latest.sha256 || incoming.utf8_bytes != record.latest.utf8_bytes`:
  - Only promote `record.latest` to `record.baseline` when `incoming` is
    causally newer than `record.latest_order` (newer `OrderingEpoch`, newer
    `stream`, or non-older `version` within the same `stream`); reject
    out-of-order older text replies without clobbering `baseline` or `latest`!

### 6.3 Bounded 12 KiB Projection & Post-Trim Eligibility (M22)

In `components/server-agent-api/src/server/agent_api/code_reads/post_edit.rs`:

- When `comparison` is NOT `complete`/`partial` (`no_baseline` / `unknown`) and
  `file["state"] == "document_current"`:
  - Count total `error` diagnostics on that file (`current_errors: u64`) and
    include `"current_errors": current_errors` on the projected file item
    whenever `current_errors > 0`!
- Enforce a `12 KiB` serialized JSON cap on the projected R4 response by
  progressively dropping tail diagnostic rows from `files[*].diagnostics` and
  setting `truncated = true` (preserving valid `comparison` counts and
  `current_errors`).
- **Post-trim eligibility recheck (M22)**: After trimming completes, recompute
  `deliver` from the **final** trimmed `files` array:
  - A compared file contributes to `deliver` iff `introduced + escalated > 0`.
  - A non-compared `document_current` file contributes to `deliver` iff
    `!diagnostics.is_empty() || current_errors > 0`.
  - If no file contributes after trimming, set `deliver = false`.

---

## 7. Proxy, Facade, `serveMcp` & Web UI Presentation

### 7.1 `DiagnosticsClient` (Dedicated WS Client)

Add `components/provider-runtime/packages/antigravity-acp/diagnostics-client.mjs`
(leaving `execution-connection.mjs` **completely untouched** for mutations!):

- Connects lazily to the descriptor's `unix:/run/cowboy-exec.sock` (or loopback
  TCP in unit tests) with `{ method: "initialize", params: { bearerToken,
  mode: "diagnostics" } }`.
- One single whole-call `AbortController` timer (`timeoutMs`, default `4000`,
  capped at `6000`) covers connect + initialize + JSON-RPC request + reply,
  combined with `(turn.abort.signal, callSignal, closingAbort.signal)` with
  deterministic listener/timer cleanup in `finally`.
- Single-flight: only one diagnostic RPC at a time; never retries; any socket
  error/close/refusal resolves cleanly to `null` (fail open for the edit tool
  call, zero `UncertainEffect`, zero mutation impact).
- `quiesce()` closes `DiagnosticsClient` before `Connection.idle()` so zero
  open sockets remain on idle/drain.

### 7.2 `Extensions`: Formatting, Post-Trim Recheck & Tokenized Dedupe (M21, M22)

In `components/provider-runtime/packages/antigravity-acp/extensions.mjs`:

1. **`lfSha256(bytes)`**: Decodes UTF-8 strictly (`new TextDecoder("utf-8", {
   fatal: true })` — returns `null` for invalid UTF-8 or NUL bytes or
   `bytes.length > 4 * 1024 * 1024`, matching `source_snapshot::identity_with`),
   normalizes `\r\n` and `\r` to `\n` (matching Zed's `text::LineEnding::normalize`),
   and returns lowercase 64-hex SHA-256.
2. **Post-trim non-empty formatting (`postEditFindings`, M22)**:
   - Renders bounded untrusted `<system-reminder>` text (escaping any
     `</system-reminder>` / control tags in untrusted LSP messages/paths).
   - Enforces a hard **16 KiB UTF-8 byte cap** *after* final escaping by
     progressively dropping tail diagnostic rows and marking `truncated: true`.
   - **Re-evaluates eligibility on the final encoded text**: returns `{ text,
     digest }` **only** if at least one diagnostic row survived **or** an
     explicit positive summary count (`introduced > 0`, `escalated > 0`, or
     `current_errors > 0` for `no_baseline`) is rendered in the final encoded
     output. If even the summary cannot fit within 16 KiB, returns `null`
     (never emitting a vacuous reminder and never reserving dedupe!).
3. **Tokenized dedupe reservations & relay-handoff semantics (M21)**:
   - Define the exact dedupe key:
     `key = sha256(JSON.stringify([sessionId, canonicalPath, lfSha256, noteDigest]))`.
   - Keep **two separate maps**:
     - `pendingShown: Map<string, number>` (maps `key -> reservationToken`;
       **no TTL expiry** while in-flight, bounded to `MAX_PENDING_SHOWN = 64`),
     - `settledShown: Map<string, number>` (maps `key -> expiresAtMs` with
       10-minute TTL, bounded to `MAX_SETTLED_SHOWN = 256` via LRU eviction).
   - `reserveShown(key)`: returns `null` if `pendingShown.has(key)` or
     `(settledShown.get(key) ?? 0) > now` or `pendingShown.size >= 64`;
     otherwise allocates `token = ++this.nextReservationToken`, stores
     `pendingShown.set(key, token)`, and returns `token`.
   - `commitShown(key, token)`: acts **only** when
     `pendingShown.get(key) === token` — removes `key` from `pendingShown` and
     sets `settledShown.set(key, now + 10 * 60_000)`.
   - `releaseShown(key, token)`: acts **only** when
     `pendingShown.get(key) === token` — deletes `key` from `pendingShown`.
   - **Semantic boundary**: `send(...)` callback completion confirms **facade-to-MCP-relay
     handoff** (the JSON-RPC reply line flushed to the unit's MCP stream), NOT
     native `agy` model-consumption receipt; `commitShown` records relay
     handoff, while write failure calls `releaseShown(key, token)`.

### 7.3 `Facade`: Post-Lock Authorization, External Change Invalidation, E1 Settlement & `> 48 KiB` Scope (H6, M20, M23, M24)

In `components/provider-runtime/packages/antigravity-acp/facade.mjs`:

1. **Per-file `fileLocks` with post-lock re-authorization (H6)**:
   - Capture `preLockObserved = session.observed.get(file)` and
     `preLockInvalidations = this.invalidations.get(file) ?? 0` before entering
     `withFileLock(file, ...)`.
   - **Inside `withFileLock(file, ...)` and BEFORE any helper effect
     (`fs/writeFile` payload or `process/start` apply)**:
     1. Check `if (signal?.aborted || this.closing)` -> abort immediately
        (nothing written!).
     2. Check originating `turn` is still the live unfenced turn of `session`
        (`this.turns.get(sessionId) === turn && !turn.abort.signal.aborted &&
        !turn.uncertain`).
     3. Re-confirm owner mode grant via `modeGuard.confirm()` (from
        `Facade.answerOnce` / `planGate`) — if the session switched to `plan` /
        `"switching"` / restricted mode while queued behind an earlier write,
        refuse unconditionally before any helper effect!
     4. Re-check `(this.invalidations.get(file) ?? 0) === preLockInvalidations`
        and `session.observed.get(file) === preLockObserved`! If a preceding
        uncertain publish invalidated reads, or an earlier queued write changed
        the file out from under this queued edit, refuse cleanly ("The file
        changed on the target since it was read; read it again.") before
        dispatching any helper effect!
   - Once `publishOnce` actually starts the helper (`process/start`), it
     continues settling to completion even if `signal` aborts afterwards.
2. **External content observation tracking (`fileObservations`, M20)**:
   - Maintain `this.fileObservations = new Map()` (`file -> { generation,
     rawSha256 }`).
   - Helper `recordObservation(file, observedRawSha256)` runs on **every**
     read (`ReadFile`, `current(file)`), every definite conflict
     (`outcome?.status === "conflict"` passes `observedRawSha256 = null`,
     unconditionally bumping `generation` and clearing `rawSha256`), every
     successful `publish` (passes `sha256(bytes)`), and every `UncertainEffect`
     (passes `null`, bumping `generation`).
   - Whenever `observedRawSha256 === null || observedRawSha256 !== prev.rawSha256`,
     increment `generation`!
3. **Explicit `{ verified, postEdit }` return from `publish()` (M23)**:
   - `publish(session, turn, file, base, bytes, opts)` returns:
     ```js
     {
       verified: boolean, // true ONLY on E1 byte-only settlement (result.forgotten && sha256(now) === expect)
       postEdit: postEditTarget | null,
     }
     ```
   - Both `writeFile` and `editFile` check **`if (published.verified)`**
     strictly (never truthiness on the returned object!) to choose between the
     E1 settlement text (`Wrote/Edited ${file}: ${VERIFIED}`) and ordinary
     success text (`Wrote ${file}` / `Edited ${file} (N replacement(s))`).
4. **Full `<= 3 MiB` edit scope including `--request-file` (M24)**:
   - Applies uniformly to both inline `--request` (`<= 48 KiB`) and
     `--request-file` (`> 48 KiB .. LIMITS.writeBytes` = 3 MiB):
     - Canonical path: `file` is resolved under `this.root` via
       `resolve(session, path)` before `publishOnce` in both paths, and
       converted to workspace-relative POSIX path (`posix.relative(this.root, file)`).
     - Digest: `lfSha256(bytes)` hashes the exact `bytes` buffer written by
       `Facade` (returning `null` for non-UTF-8/NUL bytes, skipping diagnostics).
     - E1 settlement: `publishOnce` verifies `this.current(file)` against
       `expect = sha256(bytes)` on `result.forgotten` identically for both
       inline and `--request-file`, and returns `postEdit` only when `bytes`
       are confirmed on disk (`outcome?.status === "applied"`, or `now` matches
       `expect` on E1/helper-error re-read).
5. **Freshness guard (`isFresh()`) before attaching note (M20)**:
   - Runs **after** `DiagnosticsClient` returns and immediately before
     formatting/reserving dedupe:
     1. `!signal.aborted && !this.closing`,
     2. Originating `turn` is still the live unfenced turn,
     3. `this.committedRevision.get(file) === target.startCommittedRev`,
     4. `(this.invalidations.get(file) ?? 0) === target.startInvalidationGen`,
     5. `this.fileObservations.get(file)?.generation === target.startObsGen &&
        this.fileObservations.get(file)?.rawSha256 === target.rawSha256`.
   - Any external change observed via `ReadFile`, `current()`, or base conflict
     advances `fileObservations.generation` and suppresses the stale note!

### 7.4 `serveMcp`: Single Response Owner & Synchronous `committed` Latch (H14)

In `components/provider-runtime/packages/antigravity-acp/facade.mjs::serveMcp`:

- Each active MCP `tools/call` creates ONE owner record in `activeCalls`:
  ```js
  const entry = {
    controller: new AbortController(),
    cancelled: false,        // notifications/cancelled: suppresses post-edit note only
    suppressed: false,       // duplicate ID: suppresses original reply if !committed
    committed: false,        // latched synchronously BEFORE stream.write for ANY reply
    duplicateRefused: false, // ensures at most one -32600 is ever emitted for callKey
  };
  activeCalls.set(callKey, entry);
  ```
- **Invariant 1 — Never delete `entry` before `finally`**:
  - `notifications/cancelled` sets `active.cancelled = true; active.controller.abort();`
    and leaves `active` in `activeCalls`!
  - Duplicate `tools/call` while `activeCalls.has(callKey)` does **NOT** delete
    `existing` and does **NOT** start a new `facade.call`:
    ```js
    const existing = activeCalls.get(callKey);
    existing.suppressed = true;
    existing.controller.abort();
    if (!existing.committed && !existing.duplicateRefused) {
      existing.committed = true;
      existing.duplicateRefused = true;
      await send({ id, error: { code: -32600, message: "Duplicate active MCP request ID" } });
    }
    return;
    ```
  - A 3rd (or Nth) duplicate with the same ID sees `existing` still in
    `activeCalls` with `existing.committed === true`, starts zero writes, and
    emits zero extra replies!
- **Invariant 2 — Synchronous `entry.committed = true` before `stream.write`**:
  - When the original `facade.call` finishes (with `{ mcpResult, dedupe }`):
    - If `!entry.suppressed && !entry.committed`:
      - Synchronously latch `entry.committed = true;` **before** invoking
        `send({ id, result: mcpResult })` (inside `send`, `stream.write` runs
        synchronously before yielding, so any duplicate arriving while
        `stream.write` is backpressured sees `entry.committed === true` and
        cannot emit `-32600` alongside the already-enqueued result!).
      - On `send` resolving `true` (facade-to-relay handoff confirmed): call
        `extensions.commitShown(dedupe.key, dedupe.token)` if `dedupe` present.
      - On `send` resolving `false`: call
        `extensions.releaseShown(dedupe.key, dedupe.token)` and
        `facade.markRelayAmbiguous(sid, relayId)`.
    - If `entry.suppressed` or `entry.committed` was already set by a duplicate:
      call `extensions.releaseShown(dedupe.key, dedupe.token)` and send nothing.
  - Only in `finally` (after `facade.call` settled and any `send` completed):
    `if (activeCalls.get(callKey) === entry) activeCalls.delete(callKey);`.

### 7.5 Web UI Diff Presentation Without Speculative Early Diffs (M19)

In `components/provider-runtime/packages/antigravity-acp/proxy.mjs`:

- **Do NOT emit speculative early `in_progress` diffs on `toolResult`!**
- `Facade.call` delivers the clean result (strictly without `<system-reminder>`)
  to `proxy.mjs::toolResult` at call completion, exactly as today.
- `proxy.mjs::toolResult` buffers the clean diff in `this.results` and attaches
  it **only** when `agy`'s own terminal `status === "completed"` update arrives
  (never attaching a diff to a `"failed"` card, eliminating any risk of
  misattributing a committed edit's diff onto an identical concurrent failed
  call's card!).

---

## 8. Compatibility, Rollback Floor & Coordinated Rollout

### 8.1 Worker Rollback Ledger Compatibility Matrix (H5)

Schema 1 alone does **not** guarantee safe worker rollback (for example, the
predecessor of `d3b59b8ca` uses Schema 1 but lacks `active_processes` and would
skip process recovery and drop the obligation on rewrite).

Before any Hawk or OVH Machine rollout containing R4 (or R4 + R2):

1. **Pin and verify all three fallback worker binaries**:
   - **Automatic fallback**: the exact `previous.conf` / previous worker
     generation configured on the Machine unit.
   - **Manual rollback target**: the exact previous NixOS / Machine host
     release binary retained for operator rollback.
   - **Per-session fallback**: any generation selectable via
     `machine_broker.rs::fallback_targets`.
2. **Populated-ledger preservation conformance test**:
   - Run **each** of the three fallback binaries against a populated worker
     ledger containing:
     - live `active_processes`,
     - non-zero `uncertain_seq` uncertainty sequences, and
     - pending effect obligations.
   - Prove that every fallback binary loads, preserves, recovers, and rewrites
     those fields without losing `active_processes`, resetting uncertainty
     sequences, or dropping pending obligations.
3. **Two-phase Schema 2 gate if combined with R2**:
   - **Phase A**: Ship dual-schema (Schema 1 + Schema 2) readers across all
     machines while keeping writers strictly on Schema 1 (preserving
     `active_processes` + all existing fields), and verify all three fallback
     tiers pass the populated-ledger preservation test.
   - **Phase B**: Enable any Schema-2 writer **only** after Phase A is verified
     across automatic, manual, and per-session fallback binaries.

### 8.2 Mixed-Version Behavior Matrix

| Controller | Runtime Machine (OVH) | Target Machine (Hawk) | Target `zed` Plugin | Worker | Antigravity Proxy | Result |
|---|---|---|---|---|---|---|
| `< 54` | any | any | any | any | any | Broker refuses locally (`unsupported`) if worker `>= 9`; proxy skips note cleanly; zero mutation impact |
| `>= 54` | `< 54` | any | any | any | any | Worker negotiates `< 9`; proxy skips note cleanly |
| `>= 54` | `>= 54` | `< 54` (or downgrade 54->53) | any | `>= 9` | new | Controller sees target `< 54`, returns `unsupported`; public `code.diagnostics` uses `OrderingEpoch::Legacy { conn_epoch: new }` (M17) |
| `>= 54` | `>= 54` | `>= 54` | old (no `root_pin_protocol`) | `>= 9` | new | Target Machine refuses `ZedDiagnostic` fail-closed (`unsupported`) |
| `>= 54` | `>= 54` | `>= 54` (old surviving keeper) | new | `>= 9` | new | Keeper omits `workspace_root_pin`; Machine omits `workspaceRootIncarnation`; Controller returns `unsupported` (H2) |
| `>= 54` | `>= 54` | `>= 54` (new keeper) | new (`root_pin_protocol >= 1`) | `>= 9` | new | Full R4 active (1 OVH<->Hawk RTT, keeper-pinned root, atomic `zed-adapter` check) |

### 8.3 Deployment Sequence (Requires Explicit Owner Approval)

Per `docs/machine-worker-pin-advance-runbook.md`, changes to
`components/wire/src/**` break `retained-worker-interface-compatible` and must
**never** be pushed to `origin/main` ahead of an owner-approved
`inputs.cowboy-workers` pin advance.

1. Keep implementation commits on isolated branch `feat/agy-post-edit-r4` until
   full `just check-compact`, conformance tests, code review, and owner
   approval are complete.
2. Coordinated release order once owner approves:
   - Release and install updated `zed` Code Plugin (`root_pin_protocol: 1` +
     `diagnosticSeq`) on Hawk.
   - Release updated `provider-runtime` + all 7 pinning Agent Plugins
     (`antigravity`, `claude-code`, `claude-deepseek`, `codex`,
     `codex-deepseek`, `deepseek-harness`, `grok`), verifying
     `tools/check-plugin-components.ts` and `tools/worker-registry-input.ts`.
   - Deploy Controller (`Machine 54` / `Runtime 9` reader+handler, backward
     compatible with `53/8` Machines).
   - Advance `inputs.cowboy-workers` and deploy Hawk Machine + OVH Machine via
     the runbook (`tools/ovh-machine-host-release.sh --workers <release>` on
     OVH after verifying all three fallback tiers in §8.1), then install exact
     Antigravity Plugin generation on OVH via `cowboy operator install`.

---

## 9. Verification & Live Acceptance Plan

### 9.1 Automated Tests

- **Wire (`components/wire`)**: `Machine 54` / `Runtime 9` round-trip,
  `deny_unknown_fields`, size bounds, closed `ZedDiagnosticOperation` rejection
  of `openBuffer`/`closeBuffer`/arbitrary JSON, and `Response::Ready`
  backward compatibility with optional `workspace_root_pin`.
- **`zed-adapter` (`plugins/zed/adapter`)**:
  - **H1 replacement-during-dispatch test**: Simulate `A -> B -> A` directory
    swap *during* `AddWorktree` / `OpenBuffer` / `ReloadBuffers` /
    `BufferLanguage`, verifying `inotify` taints the `WorktreeLease`, triggers
    immediate `Change::Remove` teardown, verifies Zed did not serve B's
    project, and returns zero diagnostics.
  - Cache lookup refusal when existing `WorktreeLease` has a different
    `(dev, ino)`; `source_snapshot::check` refusal on mismatched `fstat`.
  - **M18 `diagnostic_seq` test**: Monotonic increment on `UpdateDiagnostics`
    at identical buffer text version.
- **Target Machine (`components/machine`)**:
  - **H2 keeper reattach test**: Create keeper against A, simulate Machine
    restart, replace A with B (and A->B->A), prove `Describe` fails with
    `WorkspaceChanged` and Machine refuses to mint `workspaceRootIncarnation`;
    also prove older keeper omitting `workspace_root_pin` omits capability.
  - **H7 + H8 tests**: Closed `ZedDiagnostic` routes `AcquireAgentBuffer`
    through `CodeAgentResourcePolicy`; `remaining_ms` expiry while waiting on
    `route.lock()` exits without touching Zed; `MAX_TARGET_POST_EDIT_INFLIGHT = 4`
    remains held through native settlement even after caller future drops;
    older `zed` Plugin without `root_pin_protocol` fails closed.
  - **M16 route generation test**: Switching a route back to an older live
    Plugin engine advances `route_generation` and rejects delayed responses
    from retired routes.
  - **H9 + H10 + M15 broker & WS bridge tests**:
    - Stalled writers on every hop (Worker->Broker, Broker->Worker,
      Broker->`machine_cli`, `machine_cli`->WS, Controller->WS,
      `machine_cli`->Broker) with pending `Stop`, `CommandAck`, `Ack`, and
      `ExecutionRequest`/`ExecutionReply` deliver reliable traffic first and
      drop/shed diagnostics cleanly.
    - Controller 54/9 -> 53/8 replacement race around admission, lease-scoped
      disconnect cleanup, worker `< 9` receiving zero local refusal frames,
      two sessions reusing identical worker `request_id` without slot
      collision, and mismatched reply fields leaving live slot intact.
- **Controller (`components/server-runtime`, `components/server-agent-api`,
  `components/server-code`)**:
  - **H3 tests**: Disabled owner (`read_parent` fails) and restricted
    superagent (`superagent::restricted`) refused at admission AND when
    revoked mid-poll (proving zero baseline commit and zero findings reply).
  - **H12 test**: `Action::AgentInvoke` rejects `code.post_edit_inspect`;
    managed read-only sessions and sessions with Language Servers off refuse
    with zero acquisition.
  - **M16 + M17 + M18 tests**: Alternating public `code.diagnostics` /
    `code.post_edit` and private R4 reads on `Machine 54`; actual target
    downgrade `54 -> 53` while Controller retains records; equal-version
    dependency-edit diagnostic update advancing `diagnostic_seq` refreshing
    `latest`, out-of-order older `diagnostic_seq` rejected, legacy adapter
    unorderable equal-version diff returning `unknown`.
  - **M22 test**: 12 KiB tail-trimming preserving `current_errors` for
    no-baseline files and setting `deliver = false` if nothing survives.
- **Worker (`src/worker_execution`, `src/worker`, `src/account_sandbox`)**:
  - **H4 + H13 tests**: Pre-init `handshake_slots = 4` + 4 KiB first-frame cap
    + 2 s timeout; closing diagnostics socket during `begin_drain()` (including
    during `initialize` reply and under a stalled diagnostics writer) NEVER
    calls `relays.socket_ended()` and lets real mutation receipts &
    `cowboy/drained` complete normally.
  - **H11 test**: Waiter cleared on turn end / drain / broker disconnect
    withdraws queued `diag_rx` item before byte 0 so a later broker connection
    or next turn on the same worker epoch never dispatches it.
  - **H5 populated-ledger fallback test**: All three fallback binaries
    preserve `active_processes`, `uncertain_seq`, and pending obligations.
- **Proxy (`components/provider-runtime/packages/antigravity-acp`)**:
  - **H6 test**: Queued second edit behind `fileLocks` refused cleanly when
    cancelled, switched to `plan` mode, or preceded by an `UncertainEffect`
    (zero helper dispatch); already-dispatched write still settles.
  - **H14 test**: Triple reuse of the same MCP `id` executes only 1 write and
    emits at most 1 reply; duplicate arriving during backpressured
    `stream.write` before callback emits zero `-32600`; `notifications/cancelled`
    followed by same-ID reuse before settlement refuses second write.
  - **M19 + M20 + M21 + M22 + M23 + M24 tests**:
    - Identical concurrent `EditFile` calls (one commits, one fails base
      check): zero early diff painted on failed card.
    - External content change observed via definite conflict suppresses
      outstanding note (`fileObservations.generation` check).
    - Expired/slow backpressured send cannot finalize a later reservation
      (tokenized `pendingShown` vs `settledShown`).
    - No-baseline error whose escaped text exceeds 16 KiB renders bounded
      `current_errors` summary or returns `null` with zero dedupe reservation.
    - Ordinary success vs E1 byte-only settlement clean MCP text for both
      `WriteFile` and `EditFile`.
    - `> 48 KiB` `--request-file` edit (both normal apply and E1 settlement)
      produces canonical path, exact LF digest, and post-edit diagnostics.

### 9.2 Live Canary Acceptance With MCP Boundary Evidence (M25)

Because the Web transcript deliberately receives the clean result, `agy` does
not echo MCP result bodies over ACP, and Controller logs record only match and
delivery decisions, add a **canary-only opt-in MCP boundary capture** in
`serveMcp`:

- Gated strictly on an explicit canary-only env flag
  (`COWBOY_CANARY_POST_EDIT_TRACE=1`, unset and disallowed in all production
  configs/units).
- Immediately before `stream.write` on `EditFile` / `WriteFile` MCP replies
  sent to `agy`, emits one bounded structured JSON trace record correlated by
  `(session_id, mcp_call_id, tool_name, path, note_attached, note_bytes,
  note_sha256, note_preview)` to the canary session's diagnostic stream.

Run live acceptance on a dedicated free-tier Antigravity canary account on OVH
(after verifying exact `cowboy operator install` receipts and fresh worker +
Plugin generations) against a Hawk worktree:

1. **Positive control (Language Servers ON)**:
   - Preflight a fixture emitting a versioned `document_current` LSP diagnostic
     where unread file `b` (absent from prompt) defines a symbol producing a
     fresh session-unique nonce solely in the type/signature diagnostic.
   - Instruct the model to `ReadFile(a)` and `EditFile(a)` to reference the
     symbol from `b` without calling any other tool.
   - Collect and correlate all four evidence sources by `(session_id,
     mcp_call_id)`:
     1. Complete native tool transcript containing **only** `ReadFile(a)` and
        `EditFile(a)` (zero `SearchFiles`, `ReadFile(b)`, `RunCommand`, or
        extension calls);
     2. Canary MCP boundary capture proving `note_attached = true` and the
        exact fresh nonce first appeared in `EditFile(a)`'s MCP reply to `agy`;
     3. Controller log `cowboy.code.post_edit_inspect`
        (`matches_expected=true, deliver=true`);
     4. Model final output reproducing the exact fresh nonce, while the Web UI
        `Edit` card shows a clean diff with zero `<system-reminder>` text.
2. **Isolated negative control (Language Servers OFF, distinct fresh nonce in
   `b`)**:
   - Run the identical `ReadFile(a)` + `EditFile(a)` prompt and collect the
     same four evidence sources:
     1. Complete native tool transcript (`ReadFile(a)` + `EditFile(a)` only);
     2. Canary MCP boundary capture proving `note_attached = false` and absence
        of the nonce in `EditFile(a)`'s MCP reply;
     3. Controller log confirming `reason="restricted"` / `deliver=false`;
     4. Model final output confirming absence of the nonce.
3. Verify subagent edits and cancelled-turn edits deliver zero diagnostics and
   record zero `UncertainEffect`.
