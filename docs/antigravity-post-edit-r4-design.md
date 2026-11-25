# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 4)

Status: design, revision 4 (2026-11-25). Incorporates all 10 high and 15 medium
findings from design review `call-f901a15e9dfcc1b559cbcd58` on Revision 3
(which itself superseded Controller-pushed Revisions 1 and 2,
`call-c0e04d3ec2bf7e8fa7c499e0` and `call-36f39f55c54beb427b3ef71b`).

- Baseline: `origin/main` at `960c811b6b03673b984ad97e84172d540dca23b6`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Scope & Complete Disposition of Rev 3 Findings (`call-f901a15e9dfcc1b559cbcd58`)

R4 delivers post-edit diagnostics for root, non-managed Antigravity sessions via
a **dedicated worker-only execution-socket method (`cowboy/postEditDiagnostics`)**
routed over an **isolated built-in `cowboy mcp` server launched strictly from
the keeper-verified immutable `cowboyAgentCli`** to a **new Controller action
`code.post_edit_inspect`**.

### Explicit Scope Narrowing vs Revision 3

- **Generic `cowboy/extensionCall` settlement is completely unchanged.**
  `cowboy_status`, `cowboy_read`, `cowboy_receipt`, `cowboy_execute`,
  `memory_*`, and folder bindings keep their exact existing turn-drained,
  effectful settlement in both directions. Generic read-only exemptions remain
  deferred to §D4.
- **No wire or retained-worker-interface files change.** R4 leaves
  `Cargo.toml`, `Cargo.lock`, `components/store/Cargo.toml`,
  `components/wire/**`, `components/plugin-sdk/**`, and
  `components/provider-sdk/**` byte-identical to the locked `cowboy-workers`
  input.

### Complete Disposition of All 25 Findings from `call-f901a15e9dfcc1b559cbcd58`

| # | Severity | Rev 3 Finding | Rev 4 Closure |
|---|---|---|---|
| 1 | High | Bind inspection to the grant and workspace authorized at admission (`actions::invoke` dropped `connection` & `grant` before awaiting `read_parent` / resolving session) | Thread `connection: &ConnectionToken` and `grant: &Grant` from `coordinator::dispatch` into `actions::invoke` -> `execute` -> `code_reads::post_edit_inspect`. Capture `SessionReadScope` and `LanguageServerTrust` synchronously at admission after verifying `managed_calls::authorize(state, connection, grant)`, and re-verify grant + connection + trust + `session::current` before every native request, before baseline commit, and before reply (§3.1–§3.2). |
| 2 | High | Require keeper-verified immutable executable identity before granting the built-in exemption (PATH fallback `cowboy` is mutable) | Query keeper `Command::Describe` -> `Response::Ready { initialization }` and validate `initialization["cowboyAgentCli"]` via `wire::agent_cli_directory(&cli)` (pinned `/nix/store/<hash>/bin/cowboy`, 64-hex `sha256`, bounded `version`) bound to the keeper's execution session incarnation. Withhold `diagnostics_server` and advertise `postEditDiagnostics: null` whenever verified `cowboyAgentCli` is unavailable or changes across incarnation (§4.1). |
| 3 | High | Give read admissions their own settlement path (calling `settle_call` decrements `state.calls` / `state.admitted` and can release mutation barrier early or underflow) | Introduce distinct `Ledger::admit_read_call(self: &Arc<Self>) -> Result<ReadCallGuard, &'static str>` operating strictly on `state.read_calls` (`MAX_READ_CALLS = 4`). `ReadCallGuard::drop` decrements only `state.read_calls` once on every exit path and never touches `state.calls` or `state.admitted` (§4.3). |
| 4 | High | A memory tool name does not establish read-only semantics | **Eliminated:** dropped `MEMORY_READS` and `COWBOY_READS` exemptions from R4; all `cowboy/extensionCall` invocations remain effectful (deferred to §D4) (§1, §4.1). |
| 5 | High | Separate counters still leave mutations blocked behind reads on shared `Server.io` mutex | Run `cowboy/postEditDiagnostics` on its **own dedicated built-in `cowboy mcp` stdio process** (`diagnostics_server: Option<Arc<Server>>`), completely isolated from the session's main `cowboy` MCP server and hidden from `tools/list`. A stuck diagnostic call can never block `review.start` or any mutating `cowboy_execute` call (§4.1–§4.2). |
| 6 | High | Fence the actual transport send and its retries (`Server::write` `"starting"` loop and `Client::frames` queued resend across reconnect) | Attach an optional `SendFence { turn, deadline, ledger, relays }` to `Pending` in `src/worker_execution.rs` (`invoke_classified_fenced`). Before emitting any `Pending` with `sends == 0` in `Client::frames` (and before rescheduling `Refusal::Unavailable`/`Capacity` in `Client::complete`), drop expired/closed/draining unsent frames as `Err(false)` (`NotSent`). Recheck `SendFence` before every `"starting"` retry in `Server::write`. Once `sends >= 1`, drain normally as `MaybeSent` (§4.3). |
| 7 | High | An older exact snapshot can overwrite a newer baseline (`baselines.rs:130–141` rotates `latest` on arrival) | Expose `BufferSnapshot::version(&self)` and numeric `buffer_epoch_u64()` in `snapshot.rs`. Store `version: Vec<CodeBufferVersion>` on `Capture` in `baselines.rs`. Fence `baselines::record` across all baseline writers: reject lower `buffer_epoch` for the same `(instance, buffer)`, reject older or incomparable CRDT `version` vectors for the same `(instance, buffer, buffer_epoch)` without mutating `record.latest`, and return `unknown("superseded")` (which `code.post_edit_inspect` projects as `deliver: false, matches_expected: false, state: "stale"`) (§3.3). |
| 8 | High | Use the captured Machine connection for native acquisition (`acquire::acquire` calls `zed_session::agent_buffer_request` which re-selects the current Machine connection) | Add `acquire::acquire_scoped(state, observed: &SessionReadScope, path, trust, authorize)` routing `acquireAgentBuffer` directly through `code_reads::session::adapter_request` on `observed`'s captured `ConnectionToken`, re-verifying grant + `trust.current` + `session::current` immediately before issuing each native request (§3.2). |
| 9 | High | Negotiate generic read settlement in both directions | **Eliminated:** generic `cowboy/extensionCall` keeps 100% legacy settlement in both directions; no `readOnly` tool bit is emitted or trusted. `cowboy/postEditDiagnostics` uses a dedicated versioned capability object `"postEditDiagnostics": { "version": 1 }` on `cowboy/extensions` (§4.1, §7). |
| 10 | High | Empty `unread_effects` does not release the `L1c` drain (`RelayQuiet::begin_drain` does not wake `turn_closed`) | Add `drain_notify: tokio::sync::Notify` (and `pub async fn draining_started(&self)`) to `RelayQuiet`, notified inside `begin_drain()`. `ExtensionBroker::post_edit_diagnostics` selects on `client.relays.draining_started()` alongside `turn_closed` and `deadline`, immediately retiring foreground diagnostic waiters and releasing their `RelayGuard` while preserving mutation replies and `L1c` acknowledgement accounting (§4.3). |
| 11 | Medium | Inspection starts a persistent filesystem observer (`touch_code_observation`) | Remove `touch_code_observation` from `code.post_edit_inspect` completely. Test that inspection sends zero observer `Start` or renewal (§3.1, §9). |
| 12 | Medium | Start Controller deadline before authorization and setup | Parse `timeout_ms` synchronously at action entry, establish `deadline = Instant::now() + Duration::from_millis(timeout_ms)` before any `.await` (`read_parent`, `superagent::restricted`, native queries, sleeps), charge every `.await` to `deadline`, and check `Instant::now() < deadline` immediately before `baselines::record` (§3.1–§3.2). |
| 13 | Medium | Gateway change does not establish an end-to-end deadline (queued worker time + `cli.rs` 5 s connect/exchange) | Acquire `diagnostics_server.io.lock()` first (bounded by `send_before`), then compute remaining budget `send_before - Instant::now() - RETURN_MARGIN (500 ms)` right before `Server::write`, refusing unsent if `< 100 ms` and clamping `controller_timeout_ms` to `min(remaining, requested, 3500)` (`< cli.rs` 5 s exchange timeout). Document `cli.rs`'s unmodified 5 s exchange ceiling accurately in §2 and §6 (§2, §4.2, §6). |
| 14 | Medium | Include reconnect and initialization in proxy timeout (`Connection.call`) | Compute `deadline = Date.now() + timeoutMs` at entry to `Connection.call` before `ensureConnected()`, race `ensureConnected()` against both `signal` abort and `deadline` expiry (without cancelling the shared `this.connecting` promise), and pass only remaining positive time to `request()` (§5.1). |
| 15 | Medium | Release abandoned request slots while retaining late-reply recognition | Share one `abandonPending(id, sendStartedAt)` helper for both local timeout and `abortInFlight`. Schedule slot release in `this.abandoned` for `Math.max(0, this.requestTimeoutMs - (Date.now() - sendStartedAt))` from original send. Check `this.abandoned.has(frame.id) || this.retired.has(frame.id)` on message receipt and protect active `abandoned` IDs from `retired` FIFO eviction so an active abandoned read ID can never close the socket (§5.1). |
| 16 | Medium | Refuse worker-only method before no-ledger passthrough (`ledger.rs:1053`) | Intercept `POST_EDIT_DIAGNOSTICS` at the very top of `Client::relay_guarded` **before** the `let Some(ledger) = self.ledger() else { self.invoke(...) }` passthrough, refusing locally whenever either `ledger` or `extensions` is missing (§4.2). |
| 17 | Medium | Distinguish attempted edit from committed revision | Track `committedRevision` (updated only after `publishOnce` succeeds or E1 verifies bytes), `activePublishes` (in-flight publish count per file, decremented in `finally`), and `invalidations` (only on `UncertainEffect`) separately. Definite failures (`conflict` / `ToolError`) leave `committedRevision` and `invalidations` intact (§5.3). |
| 18 | Medium | Record repeat suppression (`shownNow`) strictly at `serveMcp` MCP response handoff | Carry `{ checkFresh, note, commitShown }` from `Facade.call` to `serveMcp` (with automatic direct fallback for direct non-`serveMcp` unit-test calls). Immediately before `send({ id, result })` (after duplicate-ID suppression and stream checks), re-verify freshness, turn state, and cancellation, attach the note, and call `commitShown()` **only** after `await send(...)` resolves `true` (§5.3). |
| 19 | Medium | Publish clean edit result before waiting for diagnostics | Call `this.onResult(call, cleanResult)` immediately after `this.answer(...)` resolves (before awaiting `fetchPostEditNote`), keeping the diagnostic wait and wrap-up attachment strictly on the model-facing MCP reply (§5.3). |
| 20 | Medium | Preserve escalation-only diagnostic results (`comparison.escalated > 0` with no `diagnostics` array) | Accept a `document_current` entry for exact `path` in `Extensions.postEditFindings` when either `(Array.isArray(file.diagnostics) && file.diagnostics.length > 0)` OR `(Number.isInteger(file.comparison?.escalated) && file.comparison.escalated > 0)`, preserving bounded comparison summary even when multiplicity is unchanged (§5.2). |
| 21 | Medium | Make `writes.rs` helper output assembly safe under replay | On repeated `process/start` with matching `(path, expect)`, preserve accumulated stdout and `last_seq` instead of resetting; track `last_seq: Option<u64>` per helper and consume each chunk `seq` once (§6). |
| 22 | Medium | Keep one session from evicting another session's live write evidence in `writes.rs` | Enforce per-session caps (`MAX_PER_SESSION = 32`) and TTL expiry (`5 min`) across all three pending maps (`pending`, `helpers`, `reads`). Prune expired entries first; cap/evict only within the same session so busy session A never evicts quiet session B's live evidence (§6). |
| 23 | Medium | Check retained compatibility against locked `cowboy-workers` input, not `origin/main` | Verify directly via the flake's `retained-worker-interface-compatible` assertion (`nix eval .#packages.x86_64-linux.cowboy-machine-host-release.drvPath` and byte-hash comparison against `flake.lock`'s `cowboy-workers` revision) (§8). |
| 24 | Medium | Activate signed Antigravity Plugin on OVH before running canary | Explicitly converge exact Plugin release + digest on OVH (`cowboy operator converge` + installation receipt check), then verify a fresh session advertises both the new worker generation and the new Antigravity Plugin generation before live acceptance (§8, §9). |
| 25 | Medium | Make live acceptance distinguish delivery from a correct guess | On a dedicated free-tier canary account, edit a file referencing a symbol whose diagnostic message carries a fresh diagnostic-only value from an unread file (absent from prompt, edited file, and all tool output); verify the model cites that exact value without calling shell/read/diagnostics tools, plus a negative control with Language Servers disabled (§9). |

*(Prior Round-1 and Round-2 Controller-pushed findings `call-c0e04d3ec2bf7e8fa7c499e0` #1–#24 and `call-36f39f55c54beb427b3ef71b` #1–#28 remain eliminated/closed by the architectural pivot to pull and the fixes above; R1 #8/#16/#17/#24 and R2 #20/#21/#22–#24/#25/#26/#27/#28 are explicitly closed in §§3–9 below.)*

---

## 2. End-to-End Architecture, Trust Model & Deadline Propagation

### Trust Boundaries

- **Trusted:** Controller (`cowboy-server`), Machine host (`cowboy-machine` +
  `CallHost` + `Gateway`), runtime worker (`cowboy-acp-worker`), target
  execution keeper (`cowboy-execution-host`), and the keeper-verified
  immutable `cowboy mcp` binary.
- **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp` inside
  `account_sandbox_v1`), repository content, LSP diagnostic strings, `PATH`
  fallback binaries on unpinned retained keepers, and legacy peers.

### Key Architectural Invariants

1. **Dedicated worker-only method, isolated built-in CLI server:**
   `cowboy/postEditDiagnostics` is intercepted before the no-ledger passthrough
   in `Client::relay_guarded`, routed **exclusively** to `ExtensionBroker`'s
   dedicated `diagnostics_server` (launched only from keeper-verified immutable
   `cowboyAgentCli`, hidden from `tools/list`, never sharing `Server.io` with
   the session's main `cowboy` MCP server).
2. **Zero change to generic `cowboy/extensionCall`:** Every MCP tool visible to
   `agy` (`cowboy_status`, `cowboy_read`, `cowboy_receipt`, `cowboy_execute`,
   `memory_*`, folder bindings) keeps its exact existing turn-drained effectful
   contract.
3. **End-to-end propagated deadlines (honest about unmodified `cli.rs`):**
   - Proxy starts `proxyDeadline = Date.now() + 7500` **before**
     `ensureConnected()`. If reconnect/init exceeds `proxyDeadline`, the
     request rejects unsent. Remaining time passes to `Connection.request`.
   - Worker computes `send_before = Instant::now() + workerBudget`
     (`min(requestedMs, 3500) + 2000`, max `5500 ms`). It acquires
     `diagnostics_server.io.lock()` racing `send_before`, `turn_closed`, and
     `relays.draining_started()`. Only **after** winning the lock does it
     compute `remaining = send_before - now - 500 ms`, refusing unsent if
     `< 100 ms`, and stamps `timeout_ms = min(remaining, requestedMs, 3500)`
     into the `code.post_edit_inspect` request right before `Server::write`.
   - Target `cowboy mcp` (`cli.rs`, unmodified) has a fixed 5 s connect timeout
     and 5 s exchange timeout (`wait_ms() = 0 + 5000 ms`). Keeping
     `controller_timeout_ms <= 3500 ms` keeps Controller completion strictly
     below `cli.rs`'s 5 s exchange ceiling.
   - Target `Gateway` (`gateway.rs`, fixed in §6) raises its timeout from
     `wait_ms() + 2 s` to `wait_ms() + 12 s` (strictly outside `CallHost`'s
     `wait_ms() + 10 s`). Note: on un-updated Hawk hosts prior to maintenance,
     `Gateway` cuts off at 2 s (warm LSP `< 2 s` still succeeds; cold degrades
     cleanly). Even after `Gateway` updates, unmodified `cli.rs` remains capped
     at 5 s for `AgentInvoke` (which comfortably covers Zed's 4 s
     `ANALYSIS_WAIT` and our `<= 3.5 s` budget).
   - Controller starts `deadline = Instant::now() + Duration::from_millis(timeout_ms)`
     at action entry **before** any `.await`, charging authorization,
     observation, acquisition, refresh, and polling to that one clock, and
     re-checks `Instant::now() < deadline` immediately before `baselines::record`.
4. **Explicit §D4 purity note:** Like Claude's `code.post_edit`,
   `code.post_edit_inspect` may trigger bounded Zed buffer acquisition
   (`acquire_scoped`, 300 s idle release) and causally fenced
   `baselines::record` (only on exact `document_current` + `expected_sha256`
   match). It does **not** touch workspace files, shell cursors, filesystem
   observers (`touch_code_observation` omitted), `deliveries` repeat
   suppression, credentials, or user processes.

---

## 3. Controller Design (`components/server-agent-api`, `components/server-code`)

### 3.1 Admission-Bound Authorization & Entry Deadline (`actions.rs`, `code_reads.rs`)

- Thread `connection: &ConnectionToken` and `grant: &Grant` from
  `coordinator::dispatch` (`Action::AgentInvoke`) through `agent_api::invoke`
  -> `actions::invoke` -> `actions::execute` -> `code_reads::post_edit_inspect`.
- In `actions::execute`, intercept `action == "code.post_edit_inspect"` right
  beside `code.post_edit`:
  1. **Before any `.await`**: parse and validate `args` synchronously
     (`only_keys(&["path", "expected_sha256", "timeout_ms"])`, strict
     `hover::Locator`-grade normalized relative `path`, 64 lowercase hex
     `expected_sha256`, optional integer `timeout_ms` in `1..=3500`, default
     `3500`) and create `deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms)`.
  2. **Before any `.await`**: verify `super::super::managed_calls::authorize(state, connection, grant)`, capture `observed: SessionReadScope` via `session::resolve(...)`, and capture `trust: LanguageServerTrust` via `observe_language_server_trust(...)` pinned to `observed.session()`.
  3. Charge `read_parent` and `superagent::restricted` to `deadline` via
     `tokio::time::timeout_at(deadline, ...)`. If `deadline` expires, return
     `{ deliver: false, files: [{ path, state: "unavailable" }], truncated: false, matches_expected: false }` cleanly.
  4. **Never** call `touch_code_observation`, `codex_push::sync_cursor`, or
     `deliveries::begin`.

### 3.2 Connection-Pinned Acquisition & Bounded Inspection (`diagnostics.rs`, `acquire.rs`, `zed_session.rs`)

- **Connection-pinned acquisition (H8):** Add
  `zed_session::agent_buffer_request_scoped` and `acquire::acquire_scoped(state, observed: &SessionReadScope, path, trust, still_authorized)` which send
  `acquireAgentBuffer` through `code_reads::session::adapter_request` using
  `observed`'s captured `ConnectionToken` (never re-selecting
  `control.operation_connection(machine_id)`), checking `still_authorized()`
  (`managed_calls::authorize(state, connection, grant).is_ok() && trust.current(&state.hub) && session::current(&state.hub, &state.machine_control, observed)`) immediately before issuing the native request.
- **Single-clock observation loop (`diagnostics::inspect_for_expected`):**
  Every `observe_open_buffer`, `acquire_scoped`, `query_open_buffer(RefreshedLanguage)`,
  and 200 ms poll sleep runs under `tokio::time::timeout_at(deadline, ...)`
  with `still_authorized()` checked before and after every `.await`:
  - `not open` -> `acquire_scoped` (RAII `Opening` drop guard prevents lease
    leaks on timeout/cancellation) -> poll until `deadline` or terminal state.
  - already open + `SourceState::Changed` -> `acquire_scoped` refresh -> one
    `Query::RefreshedLanguage { refresh: AgentOwned }` (ignoring `"refused"` on
    shared user Code panes, which reload automatically) -> poll until
    `deadline` or terminal state.
  - `SourceState::Matching` + `snapshot` present + `!snapshot.matches_content(expected_sha256)` -> disk already superseded this edit -> stop immediately (`matches_expected: false`, no baseline mutation).
  - `SourceState::Matching` + `snapshot.matches_content(expected_sha256)` +
    analysis still `Unknown`/`Stale` -> poll every 200 ms until `deadline` or
    `document_current`.
  - Revoked grant / trust / replaced scope -> stop immediately (`restricted` /
    `code_context_changed`).
- **Pre-commit fence & byte bound:**
  - Synchronously check: `still_authorized()`, `Instant::now() < deadline`,
    `identities_match(&diagnostics, Some(snapshot))`,
    `state == "document_current"`, and `snapshot.matches_content(expected_sha256)`.
  - Only if ALL hold: call `baselines::record` (§3.3). If `baselines::record`
    returns `reason == "superseded"`, return `{ deliver: false, files: [{ path, state: "stale" }], truncated: false, matches_expected: false }`!
  - Otherwise project with `post_edit::project(&observation)`, attach
    `matches_expected: true` and `analyzed_sha256: expected_sha256`, verify
    `serde_json::to_vec(&projected).len() <= 12 * 1024` (fail closed on
    overflow), re-check `still_authorized()`, emit structured tracing log
    `cowboy.code.post_edit_inspect` (no repository text), and return.

### 3.3 Causal Version Ordering in `baselines.rs` & `snapshot.rs` (H7)

- Add `pub fn version(&self) -> &[CodeBufferVersion]` and
  `pub fn buffer_epoch_u64(&self) -> Option<u64>` to `BufferSnapshot` in
  `components/server-code/src/server/code_context/snapshot.rs` (note:
  `BufferSnapshot::bounded()` already guarantees `version` is strictly sorted
  by `replica_id` with length `<= 64` and `buffer_epoch` is canonical `u64`).
- Store `version: Vec<CodeBufferVersion>` and `buffer_epoch: u64` on `Capture`
  in `components/server-agent-api/src/server/agent_api/code_reads/baselines.rs`.
- Define `dominates(newer: &[CodeBufferVersion], older: &[CodeBufferVersion]) -> bool`:
  every `(replica_id, timestamp)` in `older` has a matching entry in `newer`
  with `newer_ts >= older_ts` (absent replica = 0).
- In `baselines::record(scope, path, pending, diagnostics)` (protects ALL
  baseline writers — `code.diagnostics`, `code.post_edit`, and
  `code.post_edit_inspect`):
  - When an existing record exists for `(scope, path)`:
    1. Same `(instance, buffer)` and `capture.buffer_epoch < record.latest.buffer_epoch`:
       do **not** mutate `record`; return `unknown("superseded")`.
    2. Same `(instance, buffer, buffer_epoch)`:
       - If `capture.version == record.latest.version`:
         - If `capture.sha256 != record.latest.sha256 || capture.utf8_bytes != record.latest.utf8_bytes`:
           do **not** mutate `record`; return `unknown("superseded")`.
         - Same content and same exact version: replace `record.latest` only if
           `capture.servers.len() >= record.latest.servers.len()` (never
           regress to weaker evidence), keeping `record.baseline` untouched.
       - Else if `!dominates(&capture.version, &record.latest.version)`:
         (older or concurrent-incomparable version) do **not** mutate `record`;
         return `unknown("superseded")`.
       - Else (`capture.version` strictly dominates `record.latest.version`):
         if `(sha256, utf8_bytes)` changed, rotate
         `record.baseline = Some(replace(&mut record.latest, capture))`; else
         update `record.latest = capture` keeping `record.baseline`.

---

## 4. Worker Design (`src/worker_execution/`, `src/account_sandbox.rs`)

### 4.1 Keeper-Verified CLI Identity & Dedicated `diagnostics_server` (H2, H4, H5, H9)

- In `ExtensionBroker::discover(client, plan)`:
  - All planned servers (`cowboy` + folder bindings) and `memory` continue to
    discover and run over `cowboy/extensionCall` **unchanged** (zero `readOnly`
    flags on `cowboy/extensions` tools).
  - Separately, query `client.call(Command::Describe)` to obtain the current
    keeper incarnation's `Response::Ready { session_id, initialization, .. }`.
  - Parse `initialization.get(wire::AGENT_CLI_INITIALIZATION_KEY)`
    (`"cowboyAgentCli"`) as `wire::Executor` and validate via
    `wire::agent_cli_directory(&cli)` (requires pinned
    `/nix/store/<hash>/bin/cowboy`, 64-hex `sha256`, bounded `version`, no
    `..`).
  - **Only** when validated `cli` is present:
    - Launch a dedicated private `diagnostics_server: Arc<Server>` with
      `argv = [cli.command, "mcp"]` (recorded in ledger via `record_server` /
      `server_started` and terminated on session close, identical to other
      worker-owned `SERVER_PREFIX` processes).
    - Initialize it (`initialize`, `notifications/initialized`, `tools/list`) and
      confirm its `tools/list` includes `"cowboy_execute"` (proving
      `grant.agent_actions == true`).
    - Record `diagnostics: Some(DiagnosticsTarget { server, cli, keeper_session_id })`
      in `Discovery` (never added to `discovery.tools`!).
  - When `Describe` fails, lacks valid `cowboyAgentCli`, or `tools/list` omits
    `cowboy_execute`: `discovery.diagnostics = None`.
- `ExtensionBroker::list()` (`cowboy/extensions`):
  - Emits `"postEditDiagnostics": { "version": 1 }` **only** when
    `found.diagnostics.is_some()` (otherwise omits the key / `null`).
  - Does **not** emit `readOnly` on any tool in `tools[]`.

### 4.2 Early Interception, Validation & Dynamic Budget Stamping (M13, M16)

- Constant `pub const POST_EDIT_DIAGNOSTICS: &str = "cowboy/postEditDiagnostics";`.
- **Intercepted at the very top of `Client::relay_guarded` (`ledger.rs`)**,
  strictly **before** the `let Some(ledger) = self.ledger() else { self.invoke(...) }`
  no-ledger passthrough:
  - If `self.ledger()` is `None` or `self.extensions.get()` is `None`, return
    `(refused("post-edit diagnostics unavailable"), None)` immediately (never
    forwarded to keeper).
- `ExtensionBroker::post_edit_diagnostics(&self, client, ledger, params)`:
  1. Strict `only_keys(params, &["path", "expectedSha256", "timeoutMs"])`,
     strict normalized relative `path` (`1..=4096`, `Component::Normal` only),
     exact 64 lowercase hex `expectedSha256`, optional integer `timeoutMs`
     (`1..=3500`, default `3500`).
  2. Confirm `discovery.diagnostics` is `Some(target)` and `ledger.turn_open` /
     `!client.relays.draining()`.
  3. Acquire **`ReadCallGuard`** via `ledger.admit_read_call()` (§4.3).
  4. Compute `send_before = Instant::now() + Duration::from_millis(requested_ms + 2000)` (at most `5500 ms`).
  5. Spawn background task owning `ReadCallGuard` (which calls
     `diagnostics_server.request_fenced(...)` -> acquires `diagnostics_server.io.lock()`
     racing `send_before`, `ledger.turn_closed(&turn)`, and
     `client.relays.draining_started()`).
  6. **After acquiring `diagnostics_server.io.lock()`**:
     - Compute `remaining_ms = send_before.saturating_duration_since(Instant::now()).saturating_sub(Duration::from_millis(500))`.
     - If `remaining_ms < Duration::from_millis(100)` or `!fence.allows_send()`:
       return `Failure::NotSent("the diagnostic read expired before sending")`
       immediately without writing to `cowboy mcp`!
     - Construct the locked-down MCP `tools/call` payload right before
       `Server::write`:
       ```json
       {
         "name": "cowboy_execute",
         "arguments": {
           "action": "code.post_edit_inspect",
           "arguments": {
             "path": "<validated path>",
             "expected_sha256": "<validated sha256>",
             "timeout_ms": "<min(remaining_ms, requested_ms, 3500)>"
           },
           "operation": "post-edit-<uuid>"
         }
       }
       ```
     - Execute `Server::write` using `client.invoke_classified_fenced(...)` (§4.3)
       and read the JSON-RPC reply bounded by `CALL_TIMEOUT`.
     - If `Failure::MaybeSent` occurred (write may have reached `cowboy mcp`
       but reply was lost), hold `ReadCallGuard` until `started + CALL_TIMEOUT`
       before dropping it; otherwise drop `ReadCallGuard` immediately on task
       exit!
  7. Foreground `tokio::select!`:
     - `tokio::time::timeout_at(send_before, answered)`
     - `ledger.turn_closed(&turn)`
     - `client.relays.draining_started()`
     Returns `(reply, None)` immediately on any branch — never holds
     `SettlementGuard`, never calls `mark_uncertain`.

### 4.3 Dedicated Read Settlement, Transport Send Fencing & `L1c` Drain Wakeup (H3, H6, H10)

- **`ReadCallGuard` (`ledger.rs`, H3):**
  - `LedgerState` adds `read_calls: usize` (capped at `MAX_READ_CALLS = 4`).
  - `Ledger::admit_read_call(self: &Arc<Self>) -> Result<ReadCallGuard, &'static str>`
    requires an active non-closed turn and `state.read_calls < MAX_READ_CALLS`,
    increments ONLY `state.read_calls`, and returns `ReadCallGuard { ledger, turn }`.
  - `Drop for ReadCallGuard` decrements ONLY `state.read_calls` once and
    notifies `self.changed`. Never touches `state.calls`, `state.admitted`, or
    `settle_call`.
  - `Ledger::fence` drops its lock and calls `self.changed.send_modify(|v| *v += 1)`
    so `turn_closed(&turn)` wakes immediately.
- **No `UncertainEffect` on loss or drain (`ledger.rs`):**
  - `Client::carries_result`: returns `false` for `POST_EDIT_DIAGNOSTICS`.
  - `unread_effects(method, carries_result, reply)`: returns empty for
    `POST_EDIT_DIAGNOSTICS`; `EXTENSION_CALL` remains unchanged (effectful).
- **Transport `SendFence` (`worker_execution.rs`, H6):**
  - Add `SendFence { turn: String, deadline: Instant, ledger: Arc<Ledger>, relays: Arc<RelayQuiet> }`
    with `allows_send(&self) -> bool { self.ledger.turn_open(&self.turn) && !self.relays.draining() && Instant::now() < self.deadline }`.
  - Store `send_fence: Option<SendFence>` on `Pending` (`call_counted_fenced` /
    `invoke_classified_fenced`).
  - In `Client::frames(&self, reconnect: bool)`: before emitting any entry with
    `pending.sends == 0`, check `pending.send_fence`. If `!fence.allows_send()`,
    remove it from `self.pending` and drop its `sender` (causing
    `call_counted_fenced` to return `Err(false)` / `NotSent`) — never
    transmitting expired/closed/draining writes even after reconnect.
  - In `Client::complete` on `Refusal::Unavailable | Refusal::Capacity`: if
    `pending.sends == 0` and `!fence.allows_send()`, remove immediately instead
    of scheduling the 1 s retry timer.
  - In `Server::write`: re-check `fence.allows_send()` before every retry
    attempt (including `"starting"` retries, where `"starting"` proves bytes
    were not yet accepted).
  - Once `Client::sending` increments `pending.sends >= 1`, the write is
    in-flight and drains normally (a lost reply classifies as `MaybeSent`
    without `UncertainEffect`).
- **`L1c` `RelayQuiet` drain wakeup (`account_sandbox.rs`, H10):**
  - Add `drain_notify: tokio::sync::Notify` to `RelayQuiet`.
  - In `RelayQuiet::begin_drain(&self)`: after setting `draining = true`, call
    `self.drain_notify.notify_waiters()`.
  - Add `pub async fn draining_started(&self)`: register `notified()`, re-check
    `if self.draining() { return; }`, then `notified.await`.
  - Foreground `post_edit_diagnostics` wakes immediately on `begin_drain()`,
    exits with a clean refusal (empty `unread_effects`), and drops its
    `RelayGuard` so `L1c` shutdown proceeds without delay while preserving all
    mutating replies and `cowboy/drained` accounting.

---

## 5. Proxy & Facade Design (`components/provider-runtime/`)

### 5.1 `execution-connection.mjs` (M14, M15)

- **Whole-call deadline including reconnect/init (M14):**
  - `Connection.call(method, params, options = {})`:
    - If `options.signal?.aborted`, reject immediately unsent.
    - When a finite positive `options.timeoutMs` is supplied, capture
      `deadline = Date.now() + options.timeoutMs` **before** `ensureConnected()`.
    - Race `this.ensureConnected()` against `options.signal` abort AND
      remaining time until `deadline` (without aborting/closing the shared
      `this.connecting` promise for other callers!).
    - Compute remaining time `remainingMs = deadline - Date.now()`. If
      `remainingMs <= 0` or `options.signal?.aborted`, reject unsent before
      calling `this.request(...)`. Otherwise pass `{ ...options, timeoutMs: remainingMs }`.
- **Shared abandonment & safe late-reply recognition (M15):**
  - Extract `retirePending(id, sendStartedAt)` called by both local `timeoutMs`
    expiry and `abortInFlight: true` signal abort:
    - Deletes `id` from `this.pending`, clears its timer, removes any abort
      listener (zero listener leaks on `turn.abort.signal`).
    - Schedules slot release in `this.abandoned` for
      `Math.max(0, this.requestTimeoutMs - (Date.now() - sendStartedAt))`
      (bounded from original send time so aborted slots always free even if
      the server never replies).
    - Records `id` in `this.retired` without ever evicting an ID still present
      in `this.abandoned`.
  - On incoming socket message, treat `connection.abandoned.has(frame.id) || connection.retired.has(frame.id)` as an expected late reply (clear its
    `abandoned` timer, delete from both maps, ignore cleanly without closing
    the WebSocket).
  - Mutating calls never pass `abortInFlight: true`.

### 5.2 `antigravity-acp/extensions.mjs` (H4, H9, M20)

- Constructor reads `this.postEditVersion = postEditDiagnostics?.version === 1 ? 1 : 0`
  (false/0 on older workers or when keeper lacked verified `cowboyAgentCli`).
- `Extensions.call(...)` stays **unchanged** (all extension calls remain
  effectful; no `readOnly` bit trusted).
- `postEditInspect(connection, { path, expectedSha256, timeoutMs = 3500, signal })`:
  - Returns `null` immediately when `this.postEditVersion !== 1` or
    `signal?.aborted`.
  - Calls `connection.call("cowboy/postEditDiagnostics", { path, expectedSha256, timeoutMs }, { timeoutMs: timeoutMs + 4000, signal, abortInFlight: true })`.
  - Parses MCP result (`reply?.isError !== true`, `JSON.parse(reply.content[0].text)`,
    unwraps `parsed?.value ?? parsed`); returns `null` on any error/malformed
    payload (never throws `UncertainEffect`).
- `postEditFindings(value, { path, expectedSha256 })` (M20):
  - Fails closed unless:
    - `value?.deliver === true`,
    - `value?.matches_expected === true`,
    - `value?.analyzed_sha256 === expectedSha256`,
    - `Array.isArray(value?.files)` contains an entry with `file.path === path`
      and `file.state === "document_current"` that has EITHER
      `(Array.isArray(file.diagnostics) && file.diagnostics.length > 0)` OR
      `(Number.isInteger(file.comparison?.escalated) && file.comparison.escalated > 0)`
      (preserving escalation-only results with unchanged multiplicity!).
  - Projects only `{ files: [matchedEntry], truncated: value.truncated === true }`,
    JSON-escapes `/[<>&\u2028\u2029]/g` to `\uXXXX`, wraps in Claude-parity
    `<system-reminder>...</system-reminder>`, and rejects notes exceeding
    `16 KiB` UTF-8 bytes.
  - Computes SHA-256 dedup key over the canonical projected payload, checks
    `this.shown` (TTL `600_000 ms`, bounded `32` entries), and returns
    `{ key, note }` without mutating `this.shown` yet (`shownNow(key)` is
    deferred until `serveMcp` handoff confirms delivery).

### 5.3 `antigravity-acp/facade.mjs` & `serveMcp` (M17, M18, M19)

- **Separate attempted vs committed revisions (M17):**
  - `this.commitSeq = 0`
  - `this.committedRevision = new Map()` (bounded 2048, FIFO eviction):
    updated **only** after `publishOnce()` succeeds (or E1 verifies bytes on
    disk). Definite failures (`conflict` / `ToolError`) never modify it.
  - `this.activePublishes = new Map()`: incremented on entry to `publish()`,
    decremented in `finally` (deleted when 0).
  - `this.invalidations = new Map()`: unchanged (incremented only on
    `UncertainEffect`).
- **Publish clean `onResult` immediately before optional diagnostic wait (M19):**
  - Inside `Facade.call(sessionId, name, args, relayId, { signal, handoff } = {})`:
    1. `const cleanResult = await this.answer(call, { signal, postEdit })`
       (`publish()` has now left `turn.inflight`).
    2. **Immediately** call `this.onResult(call, cleanResult)` so the completed
       edit's Web diff card is published even if cancellation occurs during the
       optional diagnostic lookup.
    3. If `postEdit.target` is set, run `candidate = await this.fetchPostEditCandidate(session, callTurn, postEdit.target, signal)`:
       - Eligibility: `!this.managed`, `session.parent === undefined`,
         `!(callTurn.sessionId !== undefined && this.subagents?.owns?.(callTurn.sessionId))`,
         `this.extensions?.postEditVersion === 1`,
         `file !== this.root && inside(this.root, file)`,
         `bytes.length <= 4 * 1024 * 1024`, `!bytes.includes(0)`, valid UTF-8
         `decodeText(bytes)` with no lone `\r` after `\r\n` -> `\n`
         normalization, and turn open/non-uncertain.
       - Freshness predicate `isFresh()`:
         `session.turn === callTurn && !callTurn.cancelled && !callTurn.ended && !callTurn.uncertain && !signal?.aborted && this.committedRevision.get(file) === revision && (this.activePublishes.get(file) ?? 0) === 0 && (this.invalidations.get(file) ?? 0) === invalidation && (relayId === undefined || !session.ambiguousRelays.has(relayId))`.
       - Returns `{ note, key, isFresh, commitShown }` if `isFresh()` holds and
         `extensions.postEditFindings(...)` returns a non-suppressed candidate.
- **Commit repeat suppression strictly at `serveMcp` handoff (M18):**
  - `serveMcp` passes a `handoff` object into `facade.call(...)`.
  - Inside `serveMcp`, right before `send({ id, result })` (after verifying
    `!entry.suppressed && !streamEnded && !stream.destroyed && !stream.writableEnded && !entry.controller.signal.aborted`):
    - Re-run `candidate.isFresh()`. If still fresh, append `\n\n${candidate.note}`
      to a non-mutating copy of `result` (ahead of `WRAP_UP_NOTE` if present).
    - `delivered = await send({ id, result })`.
    - Call `candidate.commitShown()` (`extensions.shownNow(candidate.key)`)
      **only** when `delivered === true`!
  - Direct non-`serveMcp` callers (such as unit tests calling `facade.call(...)`
    without `handoff`) re-check `isFresh()`, attach the note, and call
    `commitShown()` synchronously right before returning.

---

## 6. Independent Pre-Existing Bug Fixes Included (R1 #8, M21, M22, M13)

1. **`components/server-runtime/src/server/execution/writes.rs`:**
   - Unwrap `let body = reply.get("result").unwrap_or(reply);` in
     `helper_output` (only when `reply.get("error").is_none()`), matching real
     keeper completions `{ "result": { "chunks": [...], "closed": true } }`.
   - **Replay-safe helper assembly (M21):** on repeated `process/start` for the
     same `(session, process)` with matching `(path, expect)`, preserve
     existing accumulated `output` and `last_seq` instead of resetting. Track
     `last_seq: Option<u64>` per helper and skip chunks with `seq <= last_seq`
     so replayed/overlapping `process/read` replies never corrupt split JSON
     receipts.
   - **Per-session isolation & expiry across all pending maps (M22):** track
     `created_at: Instant` on `pending` (direct writes), `helpers`, and `reads`
     with a 5-minute TTL and a per-session cap (`MAX_PER_SESSION = 32`) inside
     the global `MAX_PENDING = 256` ceiling. Prune expired entries first; cap
     or evict only within the same session so one busy session can never
     discard another session's live write evidence.
2. **`components/store/src/managed_calls/gateway.rs`:**
   - Change line 158 from `Duration::from_millis(request.action.wait_ms() + 2000)`
     to `Duration::from_millis(request.action.wait_ms()) + Duration::from_secs(12)`
     so `Gateway` stays strictly outside `CallHost::forward`'s
     `wait_ms() + 10 s` timeout rather than prematurely cutting off
     `AgentInvoke` / `AgentStatus` / `Inspect` / `Result` / `Cancel` at 2.0 s.
   - Note: unmodified `cli.rs:200` retains its `5 s` exchange ceiling for
     `AgentInvoke` (`0 + 5000 ms`), so fixing `gateway.rs` removes the 2 s
     cliff up to that 5 s CLI ceiling (sufficient for Zed's 4 s `ANALYSIS_WAIT`
     and R4's `<= 3.5 s` budget).

---

## 7. Mixed-Version Compatibility Matrix

| Controller | Hawk Machine (`gateway.rs`) | OVH Worker | Proxy Plugin | Behavior |
|---|---|---|---|---|
| Old | Old (2 s) | Old | New | `postEditDiagnostics` absent in `cowboy/extensions` -> clean no-op; zero RPCs; `extensionCall` unchanged. |
| Old | Old (2 s) | New | Old | Old proxy ignores `postEditDiagnostics` -> unchanged; `extensionCall` unchanged. |
| Old | Old (2 s) | New | New | `code.post_edit_inspect` refused as unknown action -> proxy ignores refusal, returns edit result unchanged; worker never marks uncertain. |
| New | Old (2 s) | New | New | Warm LSP (`< 2 s`) delivers diagnostics immediately; cold LSP (`> 2 s`) times out cleanly without failing edits or marking uncertain. |
| New | New (12 s) | New | New | Full cold + warm post-edit diagnostics up to 3.5 s Controller budget (within 5 s `cli.rs` exchange ceiling). |
| Any | Any (unpinned keeper without `cowboyAgentCli`) | New | New | Worker withholds `diagnostics_server` and `postEditDiagnostics` -> clean no-op (zero PATH fallback). |

---

## 8. Release & Rollout Plan (Coordinated with R2)

1. **Retained-worker interface check against locked `cowboy-workers` input (M23):**
   - Verify **directly against the locked worker input** (never `git diff origin/main`
     alone) by evaluating `nix eval .#packages.x86_64-linux.cowboy-machine-host-release.drvPath`
     and verifying byte-identity of every file in `retained-worker-interface-files`
     against `flake.lock`'s `cowboy-workers` revision.
   - Because `retained-worker-interface-compatible` stays `true`, R4 merges
     cleanly to `origin/main` without blocking host-only Machine releases.
2. **Component / Provider release:**
   - Bump `cowboy-antigravity-acp`, `provider-runtime`, Antigravity Plugin
     (`0.2.38` -> `0.2.39`), and all 6 other pinning Providers; refresh
     `components/registry.json` and `components/worker-registry-input.json`;
     sign and publish all 7 updated manifests before Controller activation.
3. **Coordinated Hawk/OVH worker pin advance & OVH Plugin convergence (M24, R2 #27–#28):**
   - Advance `inputs.cowboy-workers` once together with **R2 (User MCP servers)**
     after explicit owner approval, following
     `docs/machine-worker-pin-advance-runbook.md` (including Claude + connected
     Code acceptance on Hawk and explicit
     `tools/ovh-machine-host-release.sh --workers <release>` selection on OVH).
   - Explicitly converge the signed Antigravity Plugin release + exact digest
     on OVH (`cowboy operator converge` + installation receipt check), then
     verify a fresh session advertises both the new worker generation and the
     new Antigravity Plugin generation before running live acceptance.

---

## 9. Verification Plan

### Deterministic Tests

- **Controller (`components/server-agent-api`, `components/server-code`):**
  - Admission-bound grant + connection + trust + scope enforcement before
    native calls, before `baselines::record`, and before reply (H1, H8).
  - Entry-time deadline charged across authorization and all awaits + pre-commit
    expiry check (M12).
  - Zero `touch_code_observation` / observer `Start` or renewal, zero
    `codex_push::sync_cursor` movement, zero `deliveries` writes (M11).
  - Causal `baselines::record` version & `buffer_epoch` fencing preventing
    older exact snapshots from overwriting newer baselines across all writers
    (H7).
  - Exact `expected_sha256` match vs mismatch / stale / timeout / trust
    revocation / byte-cap overflow.
- **Pre-existing fixes (`components/server-runtime`, `components/store`):**
  - `writes.rs`: real wrapped keeper `{"result": {"chunks": [...], "closed": true}}`,
    error rejection, replayed `process/start` preservation, duplicate chunk
    `seq` deduplication, per-session isolation & TTL expiry across `pending`,
    `helpers`, and `reads` (R1 #8, M21, M22).
  - `gateway.rs`: `AgentInvoke` taking `> 2 s` completes cleanly, plus CLI
    exchange test confirming end-to-end budget nesting (M13).
- **Worker (`src/worker_execution`, `src/account_sandbox`):**
  - `Describe` / `cowboyAgentCli` verification: immutable store CLI enables
    dedicated `diagnostics_server` + `postEditDiagnostics: { version: 1 }`;
    missing/PATH/changed-incarnation CLI withholds it cleanly (H2).
  - Dedicated `diagnostics_server`: admitted mutating `cowboy_execute`
    completes immediately while `diagnostics_server` is stuck (H5).
  - `ReadCallGuard`: independent `read_calls` counter (`MAX_READ_CALLS = 4`),
    zero effect on `calls`/`admitted`, no early barrier release or underflow
    across normal/error/timeout/abort exits (H3).
  - `POST_EDIT_DIAGNOSTICS` refused locally when ledger is absent (before
    no-ledger passthrough) (M16).
  - `SendFence` in `Client::frames`, `Client::complete`, and `Server::write`:
    expired/closed/draining unsent requests (including across reconnect or
    `"starting"` retries) drop as `NotSent` and are never transmitted;
    in-flight `sends >= 1` drains cleanly as `MaybeSent` without
    `UncertainEffect` (H6).
  - Dynamic remaining-time stamping right after acquiring
    `diagnostics_server.io.lock()` (M13).
  - `RelayQuiet::begin_drain()` wakes `draining_started()` immediately,
    releasing foreground diagnostic `RelayGuard` without delaying `L1c` drain
    or losing mutation reply/acknowledgement accounting (H10).
  - Mixed-version tests confirming unchanged `cowboy/extensionCall` settlement
    and versioned `postEditDiagnostics` gating (H4, H9).
- **Proxy & Connection (`components/provider-runtime`, `tools/execution-probe_test.mjs`):**
  - `Connection`: whole-call deadline covering reconnect/init without cancelling
    shared `this.connecting`; shared `retirePending` releasing `abandoned`
    slots from original send time while preserving late-reply recognition
    across `retired` FIFO churn (M14, M15).
  - `Extensions`: versioned `postEditDiagnostics.version === 1` gating;
    untrusted JSON escaping + 16 KiB cap; escalation-only (`comparison.escalated > 0`
    without `diagnostics[]`) preserved; `shownNow` deferred until handoff (M18, M20).
  - `Facade` & `serveMcp`: `onResult` emitted immediately after `answer()`
    before diagnostic wait (M19); definite conflict/failure on Edit B leaves
    Edit A's `committedRevision` intact while active/committed/uncertain Edit B
    suppresses Edit A (M17); `serveMcp` rechecks freshness/cancellation right
    before `send()` and commits `shownNow` strictly after `send()` returns
    `true` (suppressed duplicate or failed write never hides unseen note) (M18);
    `> 48 KiB`, CRLF, E1 `verified === true`, subagent/managed/Plan-mode/wrap-up
    compatibility.
- **Repo gate:** `just check-compact` + exact `retained-worker-interface-compatible`
  flake assertion (`nix eval .#packages.x86_64-linux.cowboy-machine-host-release.drvPath`) (M23).

### Live Acceptance (Post-Rollout, M24, M25)

On a dedicated free-tier Antigravity canary account on OVH (fresh session
verified to run the converged signed Antigravity Plugin generation and updated
worker generation) against a Hawk worktree with Language Servers enabled:
1. Prepare a fixture where file `b.rs` (never read by the model and absent from
   the prompt) defines a type/signature carrying a fresh, session-unique
   diagnostic-only identifier (e.g., a unique struct/type name emitted only in
   the compiler's type-mismatch diagnostic), and instruct the model to make a
   root `EditFile` in `a.rs` triggering that type mismatch without calling any
   shell/read/diagnostics tools.
2. Verify Controller logs `cowboy.code.post_edit_inspect` with
   `matches_expected=true, deliver=true`.
3. Verify the model cites that exact fresh diagnostic-only identifier in its
   reply without calling `RunCommand`, `ReadFile` on `b.rs`, or
   `code.diagnostics`.
4. Run a negative control (Language Servers disabled) on an equivalent edit and
   verify the model cannot produce that identifier.
5. Verify Web UI `Edit` card renders clean diff without `<system-reminder>`
   clutter, and verify subagent / cancelled-turn edits produce no diagnostics
   and zero `UncertainEffect`.
