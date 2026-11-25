# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 5)

Status: design, revision 5 (2026-11-25). Closes all 6 high and 15 medium
findings from design review `call-34b1b1a7de46cb99f61d1082` on Revision 4
(following `call-f901a15e9dfcc1b559cbcd58` on Revision 3 and earlier
Controller-pushed Revisions 1–2).

- Baseline: `origin/main` at `960c811b6b03673b984ad97e84172d540dca23b6`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Scope & Complete Disposition of Rev 4 Findings (`call-34b1b1a7de46cb99f61d1082`)

R4 delivers post-edit diagnostics for root, non-managed Antigravity sessions via
a **dedicated worker-only execution-socket method (`cowboy/postEditDiagnostics`)**
routed over an **isolated built-in `cowboy mcp` server launched strictly from a
keeper-verified immutable Nix-store `cowboyAgentCli`** to a **new Controller
action `code.post_edit_inspect`**. Generic `cowboy/extensionCall` settlement
stays completely unchanged, and all 74 retained-worker interface files remain
byte-identical to the locked `cowboy-workers` input.

### Complete Disposition of All 21 Findings from `call-34b1b1a7de46cb99f61d1082`

| # | Severity | Rev 4 Finding | Rev 5 Closure |
|---|---|---|---|
| 1 | High | `agent_cli_directory` accepts mutable executables (any absolute path ending in `cowboy`), and `Response::Ready` has `scope`, not `session_id` | Validate `cli.command` directly in `extension_broker.rs` as an immutable Nix-store executable (`^/nix/store/[^/:\0]+/bin/cowboy$`, no `.` or `..`, exact 64 lowercase hex `sha256`, bounded `version`), verify `ready.scope == Scope::from_binding(&client.binding)`, and invalidate the diagnostics server if it closes or the binding ends (§4.1). |
| 2 | High | Delayed diagnostic handed to `cowboy mcp` stdio can borrow a replacement worker's grant when `CallHost::install` replaces `context.json` | Worker stamps its own `worker_epoch` (`client.epoch()`) into `code.post_edit_inspect`'s arguments. Controller adds `Authority::worker_epoch(&self) -> &str` in `authority.rs`, verifies `authority.worker_epoch() == args.worker_epoch` at gateway admission (`authorize_for_worker_epoch`), and re-verifies it before every native call, baseline commit, and reply (§3.1–§3.2, §4.2). |
| 3 | High | Pinned connection leaves remote worktree identity unbound (`SessionReadScope.remote_root` starts `None`) | Call `state.machine_control.observe_session_read_root(&mut observed)` immediately at admission (charged to `deadline`), requiring an observed remote root incarnation on non-colocated Machines (fail closed if unsupported or unreachable), and verify `session_read_root_is_current(&observed)` before native acquisition, refresh, baseline commit, and reply (§3.1–§3.2). |
| 4 | High | Send fence in `Client::frames` misses frames stalled in `worker.rs:945/989/1085` and `Refusal::Unavailable`/`Capacity` retries after `sending` incremented `sends` | Replace `Client::sending` with per-frame `Client::claim_send(&frame) -> Option<&Frame>` checked immediately before every `write_frame` at all three sites (`worker.rs:945, 989, 1085`): if a fenced `Command::Invoke` expired/closed/draining, remove from `pending`, resolve `NotSent` (`sends == 0`) or `MaybeSent` (`sends > 0`), and skip writing. In `Client::complete`, never schedule the 1 s retry timer on `Unavailable`/`Capacity` for a fenced `Command::Invoke` (deliver refusal immediately; only `Command::Observe` continues for already-pending ops) (§4.3). |
| 5 | High | Waking the diagnostic task leaves its `RelayGuard` in `Done` and in the `L1c` drain barrier during WebSocket write backpressure | Make `Done._in_flight: Option<RelayGuard>`. Admit `POST_EDIT_DIAGNOSTICS` in `worker_execution.rs:serve()` only when `!client.relays.draining()`, with `in_flight = None` (never enters `RelayQuiet`), and drop any `POST_EDIT_DIAGNOSTICS` `Done` silently without writing to the WebSocket once `client.relays.draining()` becomes true. On proxy shutdown (`beginShutdown`/`quiesce`), abort `this.closingAbort` + all `turn.abort` before `Connection.idle()`, keeping all mutation `RelayGuard` & `cowboy/drained` rules intact (§4.3, §5.3). |
| 6 | High | An old buffer can overwrite its replacement's baseline across different buffer IDs within an instance or across Zed adapter restarts | Compare `buffer_epoch` across **all** buffer IDs within the same `instance` (rejecting lower `buffer_epoch` or mismatched `buffer` at equal epoch as `"superseded"` without mutating `record`). Across different `instance`s, record `started_at: Instant` before observation and maintain bounded `retired_instances: VecDeque<String>` (cap 8) + `recorded_at: Instant`: reject any `instance` already in `retired_instances` or any observation whose `started_at < record.recorded_at` as `"superseded"` (§3.3). |
| 7 | Medium | Equal text versions can still regress diagnostic evidence (server count alone allows different servers or `entries: None` downgrades) | At equal `(instance, buffer, buffer_epoch, version)` and identical content: never replace `record.latest` unless `(record.latest.entries.is_none() && capture.entries.is_some())` OR `(capture.entries.is_some() && record.latest.servers.is_subset(&capture.servers) && capture.servers.len() > record.latest.servers.len())`. Otherwise preserve existing `record.latest` untouched (§3.3). |
| 8 | Medium | Optional diagnostics discovery inside `Broker::discover` shares the 40 s deadline and can erase legacy extension discovery | Run diagnostics discovery (`Describe` + `diagnostics_server` startup) on its own independent background task and watch channel (`diagnostics: watch::Receiver<Option<DiagnosticsReady>>`, 15 s own timeout), completely decoupled from legacy `Broker::discover`. `Broker::list()` awaits only legacy `discovered()` and reads `diagnostics` non-blockingly / with a brief bounded poll without ever failing or hiding legacy tools (§4.1). |
| 9 | Medium | Shared admission capacity (execution socket 16 ids, `Client::pending` 24 slots, Controller) still lets diagnostics starve mutations | Reserve protected mutation headroom at every shared gate: (a) worker execution socket sheds `POST_EDIT_DIAGNOSTICS` when `ids.len() >= 12` or `diag_ids >= 2` (reserving 4 slots for mutations); (b) `Client::pending` sheds fenced diagnostic `Invoke` when `pending.len() >= MAX_PENDING - 4`; (c) Controller gates `code.post_edit_inspect` behind its own non-blocking `try_acquire` semaphore (`MAX_POST_EDIT_INSPECT = 4`) independent of `AgentRead` / `review.start`. Test `review.start` succeeding at diagnostic saturation (§4.2, §4.3, §3.1). |
| 10 | Medium | Private `diagnostics_server` can survive a crashed worker on a retained keeper because `record_server` is in-memory only | Use a dedicated sub-prefix `"cowboy-worker-mcp-diag-"` (still under `SERVER_PREFIX` so `worker_event()` filters it and units cannot name it). Persist active diagnostic server process IDs in a bounded atomic `0o600` sidecar file next to the ledger (`<ledger>.diag-servers.json`, max 4 KiB, scoped to `(session_id, environment.id)`) **before** `process/start` without changing Ledger Schema 1. On worker startup/reopen (before starting a new server), send best-effort `process/terminate` to any prior entry and clear confirmed exits; also explicitly terminate `diagnostics_server` immediately if optional initialization fails (§4.1). |
| 11 | Medium | Relative remaining-time stamp expires only at the caller (queued/transit work can still execute on Controller after caller gave up) | Narrow and state the contract accurately: relative `timeout_ms` (`<= 3500 ms`) bounds **Controller execution duration** (avoiding cross-host OVH↔Hawk clock skew), while Worker enforces `send_before` at `claim_send` and foreground expiry, and Proxy enforces its whole-call deadline. Explicitly account for in-transit work: a request already written before `claim_send` closed may finish within its bounded `<= 3500 ms` Controller cap (protected by `worker_epoch` + `still_authorized` + causal baseline fencing) while its `ReadCallGuard` stays held until backend completion or `CALL_TIMEOUT` (§2, §4.2). |
| 12 | Medium | Turn cancellation does not cancel the proxy diagnostic wait when the MCP stream stays open | Combine ALL three abort signals (`callTurn.abort.signal`, per-call MCP `signal`, and facade `this.closingAbort.signal`) via a clean helper (using `AbortSignal.any` or explicit listener forwarding with guaranteed cleanup) and pass the combined signal into `Connection.call(..., { signal, abortInFlight: true })`, removing all timers and listeners on success, refusal, disconnect, timeout, and abort (§5.1, §5.3). |
| 13 | Medium | Completion order can reverse committed edit revisions across overlapping same-path publishes | Serialize same-path `publish()` through full settlement (including E1 byte verification) across the entire `Facade` via a per-file promise chain (`this.fileLocks`, cleaned when tail settles). Overlapping edits on the same file execute strictly in order; unrelated files still publish in parallel (§5.3). |
| 14 | Medium | Repeat suppression races overlapping stream handoffs (two edits check `shown` before either write callback commits) | Add in-flight reservation `this.pendingShown = new Set()` in `Extensions`. Immediately at `serveMcp` handoff (before `send`), atomically check `!isShownOrPending(key)` and call `reserveShown(key)`: commit `pendingShown -> shown` only when `await send(...)` returns `true`, and `releaseShown(key)` on suppression, error, or write failure. Cancellation of diagnostics affects only note attachment, never the clean edit reply's settlement (§5.2, §5.3). |
| 15 | Medium | Early `onResult` only buffers the edit in `proxy.mjs:1009` and loses its Web diff on cancellation while diagnostics waits | In `proxy.mjs::toolResult`, for a committed `WriteFile` / `EditFile` (`!result.isError`), immediately emit an `in_progress` `tool_call_update` carrying `content` (diff + clean summary) through the existing `holdReply` turn+tool binding (matching `toolProgress` at line 1049), while retaining the cached entry so a normal terminal `tool_call_update` completes cleanly without duplicate blocks (§5.3). |
| 16 | Medium | Proxy drops introduced findings when only counts fit (`introduced > 0` with empty/absent `diagnostics[]` due to truncation) | In `Extensions.postEditFindings`, accept a `document_current` entry for exact `path` whenever ANY of: `(Array.isArray(file.diagnostics) && file.diagnostics.length > 0)`, `(Number.isInteger(file.comparison?.introduced) && file.comparison.introduced > 0)`, or `(Number.isInteger(file.comparison?.escalated) && file.comparison.escalated > 0)` (§5.2). |
| 17 | Medium | `writes.rs` helper fix still misses actual Antigravity invocations (`facade.mjs` passes `"sh"`, embedding `apply` inside `APPLY`) | Pass `"apply"` as `$0` in `facade.mjs:publishOnce` (`["/bin/sh", "-c", APPLY, "apply", "--request", b64]`) AND recognize both `["apply", "--request"]` and Antigravity's exact `APPLY` script shape in `writes.rs:helper_request`, tested directly with `facade.mjs`'s actual argv (§6). |
| 18 | Medium | Replay can resurrect completed write evidence in `writes.rs` after `helper_output` removes the finished entry | Retain a bounded terminal marker set `completed_helpers: HashMap<(String, String), Instant>` (5-minute TTL, per-session cap) when a helper finishes (`applied` or `closed`), and check existing active/completed identities **before** applying per-session quotas so a replayed `process/start` cannot resurrect stale write evidence (§6). |
| 19 | Medium | Combined rollout lacks an R2 Schema-2 rollback floor | R4 introduces **zero** ledger schema changes (remains 100% Schema 1) and can advance its worker pin independently of R2's Schema-2 writer. If combined with R2, R2's two-phase rollout floor remains mandatory: dual-schema (1+2) readers must be proven across automatic, manual, and per-session OVH fallback paths before any Schema-2 writer is enabled (§8). |
| 20 | Medium | `cowboy operator converge` cannot select an exact canary artifact on OVH | Use **`cowboy operator install`** with explicit OVH target, exact Plugin version, exact signed digest, and a stable operation ID; gate canary acceptance on the verified installation receipt and fresh Session identities advertising both the intended worker generation and Antigravity Plugin generation (§8, §9). |
| 21 | Medium | Live canary rules forbid all reads before `EditFile` (violating `facade.mjs:1090`), and save-triggered diagnostics are excluded from `document_current` | Permit the required `ReadFile` of `a.rs` while keeping the nonce-bearing dependency file `b.rs` strictly unread and absent from the prompt; preflight a fixture emitting a versioned `document_current` LSP diagnostic (not save-triggered); use a fresh nonce in an isolated negative-control session (§9). |

---

## 2. End-to-End Architecture, Trust Model & Deadline Contract

### Trust Boundaries

- **Trusted:** Controller (`cowboy-server`), Machine host (`cowboy-machine` +
  `CallHost` + `Gateway`), runtime worker (`cowboy-acp-worker`), target
  execution keeper (`cowboy-execution-host`), and the keeper-verified immutable
  `/nix/store/.../bin/cowboy` binary.
- **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp` inside
  `account_sandbox_v1`), repository content, LSP diagnostic strings, mutable
  non-store CLI paths, and legacy peers.

### Bounded Reply-Wait Contract (Honest Cross-Host Semantics)

Because OVH (worker) and Hawk (Controller) clocks can skew, R4 does **not**
rely on synchronized cross-host wall clocks for admission cancellation:

- **Proxy (`execution-connection.mjs`, `facade.mjs`):** starts a local
  whole-call deadline (`7500 ms` default) before `ensureConnected()`, combined
  with `callTurn.abort`, per-call MCP `signal`, and `closingAbort`. Expired or
  aborted waits retire cleanly and return the edit result unchanged.
- **Worker (`extension_broker.rs`, `worker_execution.rs`, `worker.rs`):**
  computes `send_before = Instant::now() + min(requestedMs, 3500) + 2000` (max
  `5500 ms`), enforces `SendFence` at `Client::claim_send` right before every
  `write_frame`, and computes `controller_timeout_ms = min(remaining - 500ms, requestedMs, 3500)` right after winning `diagnostics_server.io.lock()`.
- **In-transit accounting:** once `claim_send` transmits the `process/write`
  frame (`sends >= 1`), the request may still run across `cowboy mcp`
  (unmodified `cli.rs` 5 s connect + 5 s exchange budget), `Gateway` (12 s
  after §6 fix; 2 s on un-updated Hawk hosts), and Controller (bounded strictly
  to `<= 3500 ms`). Its `ReadCallGuard` stays held until the backend reply
  returns (or `CALL_TIMEOUT = 170 s` on indeterminate loss), and any late
  Controller execution is fenced by `worker_epoch`, `authorize`, `trust`,
  `remote_root`, and causal baseline ordering before any baseline write.
- **Explicit §D4 purity note:** `code.post_edit_inspect` may trigger bounded
  connection-pinned Zed buffer acquisition (`acquire_scoped`, 300 s idle
  release) and causally fenced `baselines::record` (only on exact
  `document_current` + `expected_sha256` match). It never touches workspace
  files, shell cursors, filesystem observers (`touch_code_observation` omitted),
  `deliveries` repeat suppression, credentials, or user processes.

---

## 3. Controller Design (`components/server-agent-api`, `components/server-code`, `components/store`)

### 3.1 Worker-Epoch & Grant-Bound Authorization (`authority.rs`, `coordinator.rs`, `actions.rs`, `code_reads.rs`)

- Add `pub fn worker_epoch(&self) -> &str { &self.worker_epoch }` on
  `Authority` (`components/store/src/managed_calls/authority.rs`; not in
  `retained-worker-interface-files`).
- Add `coordinator::authorize_for_worker_epoch(state, connection, grant, expected_worker_epoch)`
  returning `Result<(SessionMeta, Ledger), &'static str>` only when
  `authorize(state, connection, grant)` succeeds **and** the issued & current
  `Authority::worker_epoch() == expected_worker_epoch` (preventing a delayed
  diagnostic from a dead worker's `cowboy mcp` from borrowing a replacement
  worker's newly installed `context.json` grant).
- Thread `connection: &ConnectionToken` and `grant: &Grant` from
  `coordinator::dispatch` (`Action::AgentInvoke`) -> `agent_api::invoke` ->
  `actions::invoke` -> `actions::execute` -> `code_reads::post_edit_inspect`.
- Non-blocking Controller admission gate: `static POST_EDIT_INSPECT_SLOTS: Semaphore = Semaphore::new(4)`.
  Acquire via `try_acquire()` at entry — shed immediately under load without
  queueing behind or starving `AgentRead` or `review.start`.
- **Entry sequence (before any `.await`):**
  1. Synchronously validate `args` with strict
     `only_keys(&["path", "expected_sha256", "worker_epoch", "timeout_ms"])`:
     - `path`: strict normalized relative path (`hover::Locator::parse` rule:
       `1..=4096`, no `\0`/`\`/control chars, `Component::Normal` only).
     - `expected_sha256`: exact 64 lowercase ASCII hex digits.
     - `worker_epoch`: `crate::managed_calls::valid_id(worker_epoch)`.
     - `timeout_ms`: optional integer `1..=3500`, default `3500`.
  2. Establish `deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms)`.
  3. Verify `authorize_for_worker_epoch(state, connection, grant, &worker_epoch)`.
  4. Capture `mut observed: SessionReadScope` via `session::resolve(...)` and
     `trust: LanguageServerTrust` via `observe_language_server_trust(...)`
     pinned to `observed.session()`.
- **Async authorization & remote worktree binding (H3, M12):**
  1. Charge `read_parent` and `superagent::restricted` to `deadline`.
  2. Call `state.machine_control.observe_session_read_root(&mut observed)`
     charged to `deadline`. For non-colocated Machines, require
     `observed.remote_root().is_some()` (refuse if the Machine lacks
     `session_code::PROTOCOL_VERSION` or root observation fails — never leave a
     remote worktree unbound!).
  3. Never call `touch_code_observation`, `codex_push::sync_cursor`, or
     `deliveries::begin`.

### 3.2 Connection- & Root-Pinned Acquisition and Bounded Inspection (`diagnostics.rs`, `acquire.rs`, `zed_session.rs`)

- Define `still_authorized_sync()` (`authorize_for_worker_epoch(...).is_ok() && trust.current(&state.hub) && session::current(&state.hub, &state.machine_control, &observed)`)
  and `still_authorized_root().await` (sync check + `state.machine_control.session_read_root_is_current(&observed).await` charged to `deadline`).
- **Connection-pinned acquisition (H8):** add
  `acquire::acquire_scoped(state, observed: &SessionReadScope, path, trust, authorize_check)`
  routing `acquireAgentBuffer` through `code_reads::session::adapter_request`
  on `observed`'s captured `ConnectionToken` (never re-selecting
  `control.operation_connection(machine_id)`), re-verifying
  `still_authorized_root().await` immediately before sending.
- **Single-clock observation loop (`diagnostics::inspect_for_expected`):**
  - Record `observation_started = Instant::now()` before the first native
    query (for cross-instance baseline ordering in §3.3).
  - Run every `observe_open_buffer`, `acquire_scoped`, `RefreshedLanguage`, and
    200 ms poll sleep under `tokio::time::timeout_at(deadline, ...)`, checking
    `still_authorized_sync()` around every `.await` and
    `still_authorized_root().await` before native acquisition/refresh and
    before commit.
  - Stop immediately on: revoked auth/trust/root, `SourceState::Matching` with
    mismatched `sha256` (superseded on disk), permanent error, `deadline`
    expiry, or exact `document_current && snapshot.matches_content(expected_sha256)`.
- **Pre-commit fence & byte bound:**
  - Verify `still_authorized_root().await`, `Instant::now() < deadline`,
    `identities_match`, `state == "document_current"`, and
    `snapshot.matches_content(expected_sha256)`.
  - Commit via `baselines::record_observed(observed.session(), path, pending, &diagnostics, observation_started)` (§3.3). If `"superseded"`, return `{ deliver: false, files: [{ path, state: "stale" }], truncated: false, matches_expected: false }`.
  - Project with `post_edit::project(&observation)`, attach
    `matches_expected: true, analyzed_sha256: expected_sha256`, verify encoded
    JSON `<= 12 * 1024` bytes, re-check `still_authorized_sync()`, log
    `cowboy.code.post_edit_inspect` (no repository text), and return.

### 3.3 Causal Cross-Buffer & Cross-Instance Baseline Ordering (`baselines.rs`, `snapshot.rs`) (H6, M7)

- Expose `BufferSnapshot::version(&self) -> &[CodeBufferVersion]` and
  `BufferSnapshot::buffer_epoch_u64(&self) -> Option<u64>` in `snapshot.rs`.
- Store `instance: String`, `buffer: String`, `buffer_epoch: u64`,
  `version: Vec<CodeBufferVersion>`, `servers: BTreeSet<u64>`, `entries: Option<Vec<Entry>>` on `Capture`, and store `recorded_at: Instant` +
  `retired_instances: VecDeque<String>` (cap 8) on `Record` in `baselines.rs`.
- Enforce ordering across **all** baseline writers (`code.diagnostics`,
  `code.post_edit`, `code.post_edit_inspect`):
  1. **Same `instance` (across ALL `buffer` IDs — Zed assigns monotonic
     `buffer_epoch` instance-wide):**
     - `capture.buffer_epoch < record.latest.buffer_epoch`: **reject
       (`unknown("superseded")`), do not mutate `record`**.
     - `capture.buffer_epoch > record.latest.buffer_epoch`: reopened buffer ->
       set `record.latest = capture`, clear `record.baseline = None`, update
       `recorded_at = Instant::now()`, return `unknown("buffer_replaced")`.
     - `capture.buffer_epoch == record.latest.buffer_epoch`:
       - Different `buffer` ID -> **reject (`unknown("superseded")`)**.
       - Same `buffer` ID:
         - `capture.version` strictly dominates `record.latest.version`:
           if content changed -> rotate `record.baseline = Some(replace(&mut record.latest, capture))`, update `recorded_at`, and compare; if same content -> replace `record.latest` only if it does not downgrade `entries.is_some()` to `None` and `record.latest.servers.is_subset(&capture.servers)`.
         - `capture.version == record.latest.version` (M7):
           - Different content -> **reject (`unknown("superseded")`)**.
           - Same content -> replace `record.latest` **only** if
             `(record.latest.entries.is_none() && capture.entries.is_some())`
             OR `(capture.entries.is_some() && record.latest.servers.is_subset(&capture.servers) && capture.servers.len() > record.latest.servers.len())`.
             Otherwise preserve `record.latest` unchanged and compare against
             existing `record.baseline`.
         - Older or incomparable `version` -> **reject (`unknown("superseded")`)**.
  2. **Different `instance` (Zed adapter restart):**
     - If `record.retired_instances.contains(&capture.instance)` OR
       `observation_started < record.recorded_at`: **reject
       (`unknown("superseded")`), do not mutate `record`**.
     - Otherwise push `record.latest.instance` into `record.retired_instances`
       (bounded 8), set `record.latest = capture`, clear `record.baseline = None`,
       update `recorded_at = Instant::now()`, and return
       `unknown("buffer_replaced")`.

---

## 4. Worker Design (`src/worker_execution/`, `src/worker.rs`, `src/account_sandbox.rs`)

### 4.1 Immutable Store CLI Verification, Decoupled Discovery & Crash Cleanup (H1, M8, M10)

- **Decoupled diagnostics discovery (M8):**
  - `Broker::start` spawns a separate background task for optional diagnostics
    discovery with its own 15 s timeout and its own
    `diagnostics: watch::Receiver<Option<DiagnosticsReady>>`, completely
    independent of legacy `Broker::discover` (40 s, required bindings +
    `cowboy` + `memory`).
  - `Broker::list()` awaits only legacy `discovered()` and reads the current
    `diagnostics` state (with at most a brief 1 s non-fatal wait if still
    starting, never failing or blocking legacy extension tools).
- **Strict immutable Nix-store CLI check (H1):**
  - Query `client.call(Command::Describe)` -> `Response::Ready { scope, initialization, .. }`.
  - Verify `scope == Scope::from_binding(&client.binding)`.
  - Parse `initialization.get(wire::AGENT_CLI_INITIALIZATION_KEY)` as
    `wire::Executor` and verify **strictly** (not just `agent_cli_directory`):
    - `cli.command` starts with `"/nix/store/"`, ends with `"/bin/cowboy"`,
      and the single `<entry>` between them is non-empty, contains no `/`, `:`,
      `\0`, and is neither `.` nor `..`;
    - `cli.sha256` is exact 64 lowercase ASCII hex digits;
    - `cli.version` is non-empty, `<= 64` bytes, no `\0`.
  - If any check fails, leave `diagnostics = None` (withholding
    `postEditDiagnostics`).
- **Crash-safe private server lifecycle (M10):**
  - Dedicated process prefix `DIAG_SERVER_PREFIX = "cowboy-worker-mcp-diag-"`
    (starts with `SERVER_PREFIX = "cowboy-worker-mcp-"` so `worker_event()`
    hides its output and units can never name it).
  - Sidecar file `<ledger_path>.diag-servers.json` (`0o600`, atomic temp +
    rename, max 4 KiB, scoped to `{ schema: 1, session_id, environment_id, processes: [...] }`; Ledger Schema 1 itself is untouched!).
  - On startup (before starting a new `diagnostics_server`), read any prior
    sidecar matching `(session_id, environment_id)`, send best-effort
    `process/terminate` to every listed `cowboy-worker-mcp-diag-*` ID, and clear
    entries once terminated/forgotten.
  - Record the new `diagnostics_server.process` in the sidecar **before**
    `process/start`. If initialization (`initialize` / `tools/list`) fails or
    omits `cowboy_execute`, **explicitly send `process/terminate`** to
    `diagnostics_server.process` and clear the sidecar before reporting
    `diagnostics = None`!
  - If `diagnostics_server` later closes (`io.closed`), mark diagnostics
    unavailable (never respawn without re-running immutable CLI & `scope`
    verification).
- `Broker::list()` emits `"postEditDiagnostics": { "version": 1 }` **only**
  when `diagnostics` is `Some(ready)` (omitted otherwise). Generic `tools[]`
  emits **no** `readOnly` bits.

### 4.2 Early Interception, Capacity Shedding & Worker-Epoch Stamping (H2, M9, M11, M16)

- Intercept `POST_EDIT_DIAGNOSTICS` (`"cowboy/postEditDiagnostics"`) at the
  **very top** of `Client::relay_guarded` (`ledger.rs`), strictly **before**
  the no-ledger passthrough: refuse locally if `ledger` or `extensions` is
  missing.
- **Capacity reservation for protected mutations (M9):**
  - Execution socket (`worker_execution.rs:serve()`): track `diag_ids: usize`.
    Shed `POST_EDIT_DIAGNOSTICS` immediately with local refusal if
    `client.relays.draining()`, `diag_ids >= 2`, or `ids.len() >= 12`
    (reserving 4 of 16 socket slots for mutations/extensions).
  - `Client::call_counted_fenced` (`worker_execution.rs`): shed fenced
    diagnostic `Invoke` if `pending.len() >= MAX_PENDING - 4` (reserving 4 of
    24 transport slots for protected mutations/control).
- Validate `params` strictly (`only_keys(&["path", "expectedSha256", "timeoutMs"])`,
  strict normalized relative `path`, 64 lowercase hex `expectedSha256`,
  optional integer `timeoutMs` in `1..=3500`, default `3500`).
- Acquire **`ReadCallGuard`** via `ledger.admit_read_call()` (`MAX_READ_CALLS = 4`).
- After winning `diagnostics_server.io.lock()` (racing `send_before`,
  `ledger.turn_closed(&turn)`, and `client.relays.draining_started()`):
  - Compute `remaining = send_before.saturating_duration_since(Instant::now()).saturating_sub(Duration::from_millis(500))`.
  - If `remaining < 100 ms` or `!fence.allows_send()`: return
    `Failure::NotSent` without writing.
  - Synthesize the locked-down MCP call with `worker_epoch: client.epoch()`:
    ```json
    {
      "name": "cowboy_execute",
      "arguments": {
        "action": "code.post_edit_inspect",
        "arguments": {
          "path": "<validated path>",
          "expected_sha256": "<validated sha256>",
          "worker_epoch": "<client.epoch()>",
          "timeout_ms": "<min(remaining_ms, requested_ms, 3500)>"
        },
        "operation": "post-edit-<uuid>"
      }
    }
    ```

### 4.3 `ReadCallGuard`, Per-Frame `claim_send` Fence & Independent `L1c` Drain Retirement (H3, H4, H5)

- **`ReadCallGuard` (`ledger.rs`, H3):** owns `{ ledger: Arc<Ledger>, turn: String }`, increments/decrements **only** `state.read_calls` (`MAX_READ_CALLS = 4`) via `Drop`, never touching `state.calls`, `state.admitted`, or `settle_call`.
- **Per-frame `Client::claim_send` & no automatic retry on fenced `Invoke` (`worker_execution.rs`, `worker.rs`, H4):**
  - Replace `execution.sending(&frame)` at ALL THREE write sites
    (`worker.rs:945, 989, 1085`) with `if let Some(frame) = execution.claim_send(&frame) { write_frame(&mut writer, frame).await?; }`.
  - Under `self.pending.lock()`, `claim_send` checks: if `pending` has
    `send_fence` AND `matches!(request.command, Command::Invoke { .. })` AND
    `!fence.allows_send()`, remove `pending` from `self.pending`, drop its
    `sender` (classifying `NotSent` if `pending.sends == 0`, else `MaybeSent`),
    and return `None` so `write_frame` is skipped! Otherwise increment
    `pending.sends += 1` and return `Some(frame)`.
  - In `Client::complete` on `Refusal::Unavailable | Refusal::Capacity`: if
    `pending.send_fence.is_some()` and `matches!(pending.request.command, Command::Invoke { .. })`, do **not** schedule the 1 s retry timer — remove and
    deliver the refusal immediately (classified `Err(false)` if `sends <= 1`,
    else `Err(true)`). Once `Invoke` returns `Outcome::Pending`, subsequent
    `Command::Observe` polling carries no `SendFence` block so it observes
    completion without starting new work.
  - In `Server::write`: re-check `fence.allows_send()` before every `"starting"`
    retry iteration.
- **Independent `L1c` retirement (`worker_execution.rs`, `relay.rs`, `account_sandbox.rs`, H5):**
  - `Done._in_flight` becomes `Option<RelayGuard>`: `None` for
    `POST_EDIT_DIAGNOSTICS`, `Some(guard)` for every other method.
  - `RelayQuiet::begin_drain()` notifies `drain_notify` (`draining_started()`),
    immediately waking foreground `post_edit_diagnostics` waiters.
  - In `worker_execution.rs:serve()`: when a `POST_EDIT_DIAGNOSTICS` `Done`
    arrives and `client.relays.draining()` is true, drop it silently without
    writing to the WebSocket (preventing backpressure from delaying mutation
    replies or `cowboy/drained`).
  - `Client::carries_result` returns `false` and `unread_effects` returns empty
    for `POST_EDIT_DIAGNOSTICS`; all other methods retain exact `RelayGuard`
    and `cowboy/drained` accounting.

---

## 5. Proxy & Facade Design (`components/provider-runtime/`)

### 5.1 `execution-connection.mjs` (M12, M14, M15)

- `Connection.call(method, params, options = {})` computes
  `deadline = Date.now() + options.timeoutMs` before `ensureConnected()`, races
  `ensureConnected()` against both `options.signal` abort and `deadline` expiry
  (without cancelling the shared `this.connecting` promise), and passes only
  positive remaining time to `request()`.
- Shared `retirePending(id, sendStartedAt)` for both local `timeoutMs` and
  `abortInFlight: true`: removes all timers and abort listeners on every exit
  path (success, refusal, disconnect, timeout, abort), releases `this.abandoned`
  slot at `Math.max(0, this.requestTimeoutMs - (Date.now() - sendStartedAt))`,
  and checks `this.abandoned.has(frame.id) || this.retired.has(frame.id)` on
  incoming frames while preventing `retired` FIFO eviction of active
  `abandoned` IDs.

### 5.2 `antigravity-acp/extensions.mjs` (M14, M16)

- Reads `this.postEditVersion = postEditDiagnostics?.version === 1 ? 1 : 0`.
  `Extensions.call` remains unchanged (effectful).
- `postEditFindings(value, { path, expectedSha256 })` requires
  `value?.deliver === true`, `value?.matches_expected === true`,
  `value?.analyzed_sha256 === expectedSha256`, and a matching
  `file.path === path && file.state === "document_current"` entry with ANY of:
  - `Array.isArray(file.diagnostics) && file.diagnostics.length > 0`, OR
  - `Number.isInteger(file.comparison?.introduced) && file.comparison.introduced > 0`, OR
  - `Number.isInteger(file.comparison?.escalated) && file.comparison.escalated > 0`.
- Escapes `/[<>&\u2028\u2029]/g` to `\uXXXX`, wraps in Claude-parity
  `<system-reminder>...</system-reminder>`, enforces `<= 16 KiB` UTF-8 cap.
- **Race-free repeat suppression (`pendingShown` + `shown`, M14):**
  - `reserveShown(key)` checks `!isShownOrPending(key)` (pruning `>= 600_000 ms`
    entries) and inserts `key` into `this.pendingShown`; returns `false` if
    already shown or in-flight.
  - `commitShown(key)` moves `key` from `pendingShown` to `shown` (bounded 32).
  - `releaseShown(key)` deletes `key` from `pendingShown` on failed/suppressed
    handoff.

### 5.3 `antigravity-acp/facade.mjs`, `serveMcp`, & `proxy.mjs` (M12–M15)

- **Same-path publication serialization (M13):** `Facade` maintains
  `this.fileLocks = new Map()`. `publish()` chains onto the existing promise
  for `file`, runs `publishOnce()` (including E1 byte verification), updates
  `committedRevision` only on confirmed success (never on definite conflict /
  `ToolError`), and releases the tail lock in `finally`. Separate `activePublishes`
  and `invalidations` (on `UncertainEffect`) stay per-file. Unrelated files
  publish concurrently.
- **Combined cancellation signal & shutdown abort (H5, M12):**
  - `Facade` maintains `this.closingAbort = new AbortController()` aborted in
    `quiesce()` (called by `proxy.mjs::beginShutdown()` before
    `Connection.idle()`).
  - `fetchPostEditCandidate` combines `callTurn.abort.signal`, per-call MCP
    `signal`, and `this.closingAbort.signal` (with guaranteed listener cleanup
    in `finally`) and passes the combined signal to `postEditInspect`.
- **Immediate Web diff publication before diagnostic wait (M15):**
  - `Facade.call` invokes `this.onResult(call, cleanResult)` immediately after
    `this.answer(...)` resolves, before awaiting `fetchPostEditCandidate`.
  - In `proxy.mjs::toolResult`, when `call.name === "WriteFile" || call.name === "EditFile"` and `result?.isError !== true`, immediately emit an
    `in_progress` `tool_call_update` carrying `content` (diff + clean result)
    through `this.holdReply` (using the active turn+tool binding), while
    retaining the cached entry so a later terminal `tool_call_update` completes
    cleanly without duplicating content.
- **Atomic `serveMcp` handoff reservation & settlement (M14):**
  - `serveMcp` passes a `handoff` object to `facade.call(...)`.
  - Immediately before `send({ id, result })` (after `!entry.suppressed && !streamEnded && !stream.destroyed && !stream.writableEnded && !entry.controller.signal.aborted`):
    - Re-check `candidate.isFresh()` and atomically call
      `extensions.reserveShown(candidate.key)`. If both hold, append
      `\n\n${candidate.note}` to a non-mutating copy of `result` (before
      `WRAP_UP_NOTE` if present); diagnostic cancellation/staleness drops ONLY
      the note, never the clean edit reply!
    - Call `delivered = await send({ id, result })`.
    - If reserved: call `extensions.commitShown(key)` iff `delivered === true`,
      otherwise `extensions.releaseShown(key)`.

---

## 6. Independent Pre-Existing Bug Fixes (R1 #8, M17, M18, M13)

1. **`components/server-runtime/src/server/execution/writes.rs` + `facade.mjs`:**
   - Pass `"apply"` as `$0` in `facade.mjs:publishOnce`
     (`["/bin/sh", "-c", APPLY, "apply", "--request", b64]`), and recognize both
     `["apply", "--request"]` and Antigravity's `APPLY` script argv in
     `writes.rs:helper_request` (tested against `facade.mjs`'s exact argv).
   - Unwrap `let body = reply.get("result").unwrap_or(reply);` in
     `helper_output` (only when `reply.get("error").is_none()`).
   - Preserve accumulated `output` and `last_seq` on replayed `process/start`
     with matching `(path, expect)`, skip duplicate chunk `seq <= last_seq`,
     and retain bounded terminal markers `completed_helpers` (5-minute TTL,
     checked before per-session quota) so replayed starts/reads cannot
     resurrect completed write evidence.
   - Enforce 5-minute TTL and per-session cap (`MAX_PER_SESSION = 32` inside
     global `MAX_PENDING = 256`) across all pending maps (`pending`, `helpers`,
     `reads`, `completed_helpers`) so one busy session cannot evict another
     session's live evidence.
2. **`components/store/src/managed_calls/gateway.rs`:**
   - Change line 158 to `Duration::from_millis(request.action.wait_ms()) + Duration::from_secs(12)`
     (strictly outside `CallHost`'s `wait_ms() + 10 s`). Unmodified `cli.rs`
     keeps its 5 s exchange ceiling for `AgentInvoke` (covering Zed's 4 s
     `ANALYSIS_WAIT` and R4's `<= 3.5 s` budget).

---

## 7. Mixed-Version Compatibility Matrix

| Controller | Hawk Machine (`gateway.rs`) | OVH Worker | Proxy Plugin | Behavior |
|---|---|---|---|---|
| Old | Old (2 s) | Old | New | `postEditDiagnostics` absent -> clean no-op; `extensionCall` unchanged. |
| Old | Old (2 s) | New | Old | Old proxy ignores `postEditDiagnostics` -> unchanged; `extensionCall` unchanged. |
| Old | Old (2 s) | New | New | `code.post_edit_inspect` refused as unknown action -> proxy returns clean edit result; worker never marks uncertain. |
| New | Old (2 s) | New | New | Warm LSP (`< 2 s`) succeeds; cold (`> 2 s`) degrades cleanly without failing edits or marking uncertain. |
| New | New (12 s) | New | New | Full warm + cold diagnostics up to 3.5 s Controller budget (within 5 s `cli.rs` ceiling). |
| Any | Any (unpinned/mutable CLI) | New | New | Worker withholds `diagnostics_server` and `postEditDiagnostics` -> clean no-op. |

---

## 8. Release & Rollout Plan (Rollout Floor & Exact OVH Install) (M19, M20)

1. **Retained-worker interface verification against locked `cowboy-workers`:**
   Evaluate `nix eval .#packages.x86_64-linux.cowboy-machine-host-release.drvPath`
   and verify all 74 files in `retained-worker-interface-files` match
   `flake.lock`'s `cowboy-workers` input byte-for-byte. R4 merges cleanly to
   `origin/main` without breaking host-only Machine releases.
2. **Component / Provider release:**
   Bump `cowboy-antigravity-acp`, `provider-runtime`, Antigravity Plugin
   (`0.2.38` -> `0.2.39`), and all 6 other pinning Providers; refresh
   `components/registry.json` and `components/worker-registry-input.json`; sign
   and publish all 7 manifests before Controller activation.
3. **Worker pin advance & R2 Schema-2 rollback floor (M19):**
   R4 keeps Ledger **Schema 1** (zero schema migration), so R4's worker update
   is fully rollback-safe across all Schema-1 workers. If R4's pin advance is
   combined with R2, R2's two-phase rollout floor remains mandatory: automatic,
   manual, and per-session OVH fallback readers must be proven compatible with
   Schema 2 before any Schema-2 writer is enabled (otherwise advance R4's
   Schema-1 worker pin separately ahead of R2's Schema-2 writer). On OVH, pass
   the explicit worker bundle via `tools/ovh-machine-host-release.sh --workers <release>`
   and include Claude + connected Code verification in pin acceptance.
4. **Exact OVH Plugin installation (M20):**
   Install the signed Antigravity Plugin on OVH with **`cowboy operator install`**
   specifying target `ovh`, exact version, exact signed digest, and a stable
   operation ID (never `operator converge` for canary selection). Verify the
   installation receipt, then open a fresh session and confirm it advertises
   both the new worker generation and the new Antigravity Plugin generation
   before running live acceptance.

---

## 9. Verification Plan

### Deterministic Tests

- **Controller (`components/server-agent-api`, `components/server-code`, `components/store`):**
  - `authorize_for_worker_epoch` rejects delayed requests carrying an old
    `worker_epoch` against a replacement worker's grant (H2).
  - `observe_session_read_root` + `session_read_root_is_current` rejects
    replaced remote worktrees at the same path and fails closed on unbound
    remote roots (H3).
  - `acquire_scoped` routes via `observed`'s captured `ConnectionToken` (H8).
  - Entry deadline charged across all awaits + pre-commit check; zero
    `touch_code_observation`, zero `sync_cursor`, zero `deliveries` writes;
    non-blocking `POST_EDIT_INSPECT_SLOTS` shedding under load (M9, M11, M12).
  - `baselines::record_observed`: monotonic `buffer_epoch` across different
    buffer IDs in the same instance, cross-instance `started_at` +
    `retired_instances` protection, and equal-version non-regression of
    `entries` and server sets (H6, M7).
- **Pre-existing fixes (`components/server-runtime`, `components/store`):**
  - `writes.rs`: real wrapped keeper reply, exact `facade.mjs` argv match,
    replayed `process/start` preservation, duplicate chunk `seq` dedup,
    `completed_helpers` replay suppression, and per-session isolation + TTL
    across all pending maps (R1 #8, M17, M18).
  - `gateway.rs`: `AgentInvoke` `> 2 s` completes cleanly through gateway +
    real CLI budget test.
- **Worker (`src/worker_execution`, `src/worker`, `src/account_sandbox`):**
  - Immutable `/nix/store/.../bin/cowboy` + `Ready.scope` check rejecting
    mutable paths; decoupled diagnostics discovery never failing/hiding legacy
    extensions; sidecar cleanup on reopen and termination on init failure (H1,
    M8, M10).
  - Isolated `diagnostics_server` + socket/pending capacity reservation:
    mutating `review.start` / `cowboy_execute` succeeds immediately while
    diagnostics is stuck or saturated (H5, M9).
  - `ReadCallGuard` independent settlement; pre-passthrough refusal without
    ledger; per-frame `claim_send` dropping expired/closed/draining `Invoke`
    across all three `worker.rs` sites; no 1 s retry on fenced `Unavailable`/
    `Capacity`; `Command::Observe` continues for already-pending ops (H3, H4,
    M16).
  - `Done._in_flight = None` for `POST_EDIT_DIAGNOSTICS`, `draining_started()`
    wakeup, silent drop on backpressured socket during drain, while mutations
    keep full `L1c` `RelayGuard` + `cowboy/drained` accounting (H5).
- **Proxy & Connection (`components/provider-runtime`, `tools/execution-probe_test.mjs`):**
  - `Connection`: whole-call deadline covering reconnect/init; `retirePending`
    slot release + active `abandoned` late-reply recognition; zero timer or
    listener leaks on all exits (M12, M14, M15).
  - `Extensions`: `postEditDiagnostics.version === 1` gating; introduced-only
    (truncated count) and escalation-only results preserved; `reserveShown` /
    `commitShown` / `releaseShown` preventing concurrent handoff duplicates
    (M14, M16).
  - `Facade`, `serveMcp`, & `proxy.mjs`: per-file `fileLocks` serializing
    overlapping same-path edits; definite failure preserving `committedRevision`;
    combined `(turn.abort, signal, closingAbort)` cancelling diagnostics
    immediately including on `quiesce()` before `Connection.idle()`;
    `proxy.mjs::toolResult` publishing committed edit diff immediately before
    diagnostic wait and surviving cancellation during wait; `serveMcp` atomic
    handoff reservation/commit/release without dropping clean edit replies
    (H5, M12–M15).
- **Repo gate:** `just check-compact` + exact `retained-worker-interface-compatible`
  assertion (`nix eval .#packages.x86_64-linux.cowboy-machine-host-release.drvPath`).

### Live Acceptance (Post-Rollout, M20, M21)

On a dedicated free-tier Antigravity canary account on OVH (after exact
`cowboy operator install` receipt verification and fresh session verification
of both worker and Plugin generations) against a Hawk worktree with Language
Servers enabled:
1. Preflight a fixture emitting a **versioned `document_current` LSP
   diagnostic** (not save-triggered) where file `b` (never read by the model
   and absent from the prompt) defines a symbol carrying a fresh session-unique
   diagnostic-only nonce in its type/signature mismatch diagnostic.
2. Instruct the model to perform the required `ReadFile` of `a` (permitted per
   `facade.mjs:1090`), then `EditFile` `a` to reference the symbol from `b`
   without reading `b` or running `RunCommand`/`code.diagnostics`.
3. Verify Controller logs `cowboy.code.post_edit_inspect` with
   `matches_expected=true, deliver=true`, and verify the model reports the
   exact fresh nonce from the diagnostic without calling `RunCommand`,
   `ReadFile(b)`, or `code.diagnostics`.
4. In an isolated negative-control session (Language Servers disabled, fresh
   nonce in `b`), perform the equivalent edit and confirm the model does not
   produce the nonce.
5. Verify the Web UI `Edit` card shows clean diff without `<system-reminder>`
   clutter, and verify subagent edits and cancelled-turn edits leave no
   diagnostics and zero `UncertainEffect`.
