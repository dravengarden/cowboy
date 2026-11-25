# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 3)

Status: design, revision 3 (2026-11-25). Supersedes Revision 1
(`call-c0e04d3ec2bf7e8fa7c499e0`, 13 high + 11 medium findings) and Revision 2
(`call-36f39f55c54beb427b3ef71b`, `review_incomplete` + 12 high + 16 medium
findings) following the owner's architectural decision to pivot R4 from
Controller-pushed diagnostics over untrusted `agy-proxy` to a **Claude-parity
pure read-only pull** through a dedicated, worker-validated read-only execution
method (`cowboy/postEditDiagnostics`) backed by a side-effect-free Controller
action (`code.post_edit_inspect`).

- Baseline: `origin/main` at `960c811b6` (Machine protocol 53 / Runtime
  protocol 8, Antigravity Plugin 0.2.38 / `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md),
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Architectural Pivot Rationale & Prior-Review Disposition

### Why Revision 1 and Revision 2 Were Replaced

Two rounds of `astra-max` design review proved that pushing post-edit
diagnostics from the Controller into a sandboxed, untrusted `agy-proxy`
requires invasive cross-layer machinery disproportionate to the feature:

- **Trusted write observation across an untrusted proxy** required inventing a
  new keeper RPC (`fs/apply`), rewriting `file_helper` with fd-anchored
  `openat` anti-TOCTOU handles and `already_current` vs `applied` outcomes,
  persisting `fs/apply` in the worker ledger's `KNOWN_EFFECTS`, gating retained
  v8 workers at the keeper, and authenticating root vs subagent writes over a
  shared execution socket.
- **Cross-host push transport** required bumping Machine protocol 54 and
  Runtime protocol 9 with independent broker egress negotiation for Core vs
  Worker peers, an unsequenced `Frame::DiagnosticNoteAck`, protecting
  `CancelTurn` from Linux 4 KiB `PIPE_BUF` partial writes on ACP stdin, and
  persisting Controller delivery ledgers and replay rows across restarts.
- **Residual UX and trust limitations** remained even after all of that: an
  untrusted proxy still cannot self-attest `Appended` or `Refused` (forcing
  every post-handoff outcome to `Unknown`), and single-edit turns never
  received diagnostics because Controller push only piggybacked onto a later
  tool call.

### Why Earlier Proxy Pull (`3dbb6f727`) Failed — and How Revision 3 Fixes It

The earlier attempt (`3dbb6f727` on unmerged branch `claude/agy-worker-readonly`,
reverted in `11702a645` / `110702d07`) failed for two specific, fixable reasons:

1. **Reliance on `writes.rs` hints:** `3dbb6f727` relied on Controller
   `writes.rs` matching the apply helper's stdout, which failed because
   `facade.mjs` passed `"sh"` instead of `"apply"` and `writes.rs:96` read
   `reply["chunks"]` without unwrapping `reply["result"]`. In Revision 3,
   `Facade` already holds the exact written bytes and sends their
   LF-normalized `expected_sha256` directly; the Controller verifies the
   language engine's `bufferSnapshot.content.sha256` against `expected_sha256`
   directly (eliminating any `writes.rs` dependency and supporting both
   `> 48 KiB` file-form applies and CRLF files).
2. **Reuse of stateful `code.post_edit`:** `code.post_edit` mutated
   `codex_push::sync_cursor` and recorded `deliveries::begin` repeat
   suppression before caller receipt, so `110702d07` removed it from the
   worker's read-only set. In Revision 3, the Controller adds a dedicated
   **side-effect-free** action `code.post_edit_inspect` (no `sync_cursor`, no
   `deliveries::begin`, baseline updated only on exact `document_current` +
   `expected_sha256` match — which is idempotent for identical content), and
   the trusted Worker invokes it only through a locked-down execution-socket
   method (`cowboy/postEditDiagnostics`) with its own read-only concurrency cap.

### Complete Disposition of Prior Findings (`call-c0e04d3ec2bf7e8fa7c499e0` & `call-36f39f55c54beb427b3ef71b`)

Every finding from Round 1 (24 findings) and Round 2 (28 findings) is recorded
below so reviewers can audit full closure directly against this document:

| Review & # | Severity | Prior Finding | Rev 3 Disposition |
|---|---|---|---|
| R1 #1 | High | Stamped execution requests break legacy receivers (`deny_unknown_fields`) | **Eliminated:** zero changes to `RuntimeRequest` / `components/wire/src`. |
| R1 #2 | High | Queued `DiagnosticNoteAck` blocks sequenced outbox after rollback | **Eliminated:** no `DiagnosticNoteAck` or `RuntimeEvent` added. |
| R1 #3 | High | Single visible ACP announcement does not identify MCP request | **Eliminated:** no ACP↔MCP correlation; diagnostics attach directly to the exact `WriteFile`/`EditFile` MCP response. |
| R1 #4 | High | Eviction erases contradiction while leaving cached binding valid | **Eliminated:** no `plan-gate` announcement binding used for diagnostics. |
| R1 #5 | High | Proxy can attest to its own delivery (`Appended`) | **Eliminated:** no Controller delivery receipt or transcript row attested by proxy. |
| R1 #6 | High | Helper argv marker does not authenticate root publish | **Eliminated:** Controller does not observe or authenticate helper argv for R4; `code.post_edit_inspect` is a worktree-scoped read. |
| R1 #7 | High | Normal MCP path cannot satisfy `openCalls === 0` | **Eliminated:** no `openCalls === 0` push gate. |
| R1 #8 | High | Write observer cannot read a real keeper completion (`writes.rs:96`) | **Fixed independently (§6):** `writes.rs` unwraps `reply["result"]`; R4 does not depend on `writes.rs`. |
| R1 #9 | High | Socket callback state reports false refusal and false delivery | **Eliminated:** no Controller-pushed receipt state machine on proxy socket callbacks. |
| R1 #10 | High | Completion FIFO forgets replayable operations | **Eliminated:** no Controller completion watermark/FIFO. |
| R1 #11 | High | Session-level target relabels edits from previous turn | **Eliminated:** pull is synchronous within the exact `WriteFile`/`EditFile` call and fenced by `turn.abort` + worker `turn_closed`. |
| R1 #12 | High | Acknowledged note appears live and disappears on replay | **Eliminated:** no synthetic `user_message_chunk` transcript row; delivered as tool-result `<system-reminder>` matching Claude's `Route::Adapter` model. |
| R1 #13 | High | Freshness filter reads a digest the projected result discards | **Addressed (§3):** `code.post_edit_inspect` checks `bufferSnapshot.content.sha256 == expected_sha256` before projection and returns `matches_expected` + `analyzed_sha256`. |
| R1 #14 | Medium | Independent waiter slots do not give `CancelTurn` transport priority | **Eliminated:** zero diagnostic frames on ACP stdin. |
| R1 #15 | Medium | Subtracting local queue time does not enforce one end-to-end expiry | **Addressed (§2, §3, §4):** strictly nested wall-clock budgets across Controller (`<= 4 s`), `cowboy mcp` (`5 s`), worker (`<= 6 s`), and proxy (`<= 8 s`), plus instant `turn.abort` / `turn_closed` cancellation. |
| R1 #16 | Medium | Duplicate starts and partial reads corrupt receipt assembly | **Eliminated:** R4 does not assemble helper stdout in `writes.rs`. |
| R1 #17 | Medium | Tracker eviction discards another session's live evidence | **Addressed (§6):** `writes.rs` adds TTL/oldest eviction for `helpers` and `reads` as an independent fix; R4 uses no Controller write tracker. |
| R1 #18 | Medium | Controller observation order is not publication order | **Addressed (§5):** per-file monotonic `Facade.revisions` + Controller exact `expected_sha256` match against live disk (`source_snapshot::check`) reject stale/superseded observations. |
| R1 #19 | Medium | Freshness stops at dispatch while note waits 25 s | **Eliminated:** attached immediately to the edit's own tool response (re-checked against `revisions`, `invalidations`, and `turn` state right before returning). |
| R1 #20 | Medium | Helper and analyzer hash different bytes for CRLF files | **Addressed (§5):** `Facade` normalizes `\r\n` to `\n` (matching Zed's `LineEnding::normalize` and `writes.rs`) before hashing `expectedSha256`, and skips lone `\r` / binary / NUL files. |
| R1 #21 | Medium | Web live stream merges note into human message | **Eliminated:** no `user_message_chunk` emitted; clean `WriteFile`/`EditFile` result passed to `onResult` so Web Edit cards stay clean without raw `<system-reminder>` noise. |
| R1 #22 | Medium | Publishing only Antigravity leaves six changed manifests uncovered | **Addressed (§8):** component release bumps `provider-runtime` and all 7 pinning Providers, refreshes `components/registry.json` and `worker-registry-input.json`, and publishes all covered manifests before Controller activation. |
| R1 #23 | Medium | Production deletion conformance cannot run on one pre-pin candidate | **Eliminated for R4 merge (§8):** R4 keeps `retained-worker-interface-compatible = true`; Hawk/OVH worker pin advance is coordinated once with R2 under the full pin-advance runbook. |
| R1 #24 | Medium | Live acceptance identifier is already known to the model | **Addressed (§9):** live acceptance introduces a real compiler error whose diagnostic code/message is emitted by the language server (not present in the model's edit) and verifies the model reports it without running `RunCommand` or `code.diagnostics`. |
| R2 #1–#6 | High | Keeper `fs/apply` worker admission, `openat` anti-TOCTOU, root/child socket auth, durable `KNOWN_EFFECTS` recovery, generation/digest serialization, `already_current` distinction | **Eliminated:** no `fs/apply` keeper RPC; `Facade` retains the existing turn-fenced `publish()` helper unchanged. |
| R2 #7 | High | Broker outgoing frames gated on Core runtime version | **Eliminated:** Machine protocol 53 / Runtime protocol 8 unchanged. |
| R2 #8 | High | Prove recipient MCP call belongs to intended attempt | **Eliminated:** diagnostics attach directly to the originating `WriteFile`/`EditFile` call's own MCP reply. |
| R2 #9 | High | Reserve `Refused` for trusted non-delivery | **Eliminated:** no Controller delivery ledger state transitions driven by proxy. |
| R2 #10 | High | Protect `CancelTurn` against `PIPE_BUF` partial writes on ACP stdin | **Eliminated:** zero diagnostic frames sent on ACP stdin. |
| R2 #11 | High | Reject old turn before replacing pending batch | **Eliminated:** no Controller push queue or batch replacement. |
| R2 #12 | High | Persist Controller receipt identity and delivery outcome | **Eliminated:** matches Claude's `Route::Adapter` model (no synthetic transcript row; structured Controller logs + client-side `shown` dedup after receipt). |
| R2 #13–#19 | Medium | Routable note envelope, `fs/apply` schema-1 translation, child-running fallback, broker launch capability binding, `latest()` ordering, watermark over live ops, abandoned partition reclamation | **Eliminated:** all belonged solely to the Rev-2 keeper `fs/apply` + Controller push architecture. |
| R2 #20 | Medium | Keeper-verified relative path across workspace aliases | **Addressed (§3, §5):** `Facade.root` is canonicalized via `fs/canonicalize`; `Facade.resolve` produces canonical paths inside `this.root`; Zed's `buffer_key` canonicalizes the worktree root and `source_snapshot::identity` rejects in-worktree symlinks (`ResolveFlags::NO_SYMLINKS`), failing closed on aliases. |
| R2 #21 | Medium | Filter freshness before committing diagnostic baselines (`diagnostics.rs:322–332`) | **Addressed (§3):** `diagnostics::inspect_for_expected` separates observation from baseline commit and calls `baselines::record` **only** after verifying trust fence, `session::current`, `identities_match`, `state == "document_current"`, and `snapshot.matches_content(&expected_sha256)`. |
| R2 #22–#24 | Medium | Supersession vs clear floors, in-flight edit tokens vs committed generations, unrelated fresh files | **Addressed (§5):** per-file `Facade.revisions` (incremented at publish start) and `Facade.invalidations` (incremented on `UncertainEffect`) scope freshness per file without suppressing unrelated files. |
| R2 #25 | Medium | Expiry including transport time | **Addressed (§2, §4):** nested wall-clock deadlines across all hops, with `send_before` check before acquiring `Server.io` so an expired read is never even sent to `cowboy mcp`. |
| R2 #26 | Medium | Budget serialized note before opening delivery | **Addressed (§3, §5):** `post_edit::project` enforces 10 errors / 5 warnings / 2000 message chars, and `code.post_edit_inspect` verifies serialized JSON stays bounded (`<= 12 KiB`) before returning; `Extensions.postEditFindings` enforces a matching ceiling (`16 KiB`). |
| R2 #27–#28 | Medium | Include Claude & connected Code in pin acceptance; select OVH worker bundle explicitly with `--workers` | **Addressed (§8):** retained in the coordinated R4 + R2 Hawk/OVH worker pin-advance rollout checklist (`tools/ovh-machine-host-release.sh --workers <release>`). |

---

## 2. End-to-End Architecture, Trust Model & Timeout Nesting

### Trust Boundaries

- **Trusted:** Controller (`cowboy-server`), Machine host (`cowboy-machine` +
  `CallHost` + `Gateway`), runtime worker (`cowboy-acp-worker`), target
  execution keeper (`cowboy-execution-host`), and target `cowboy mcp`.
- **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp` inside
  `account_sandbox_v1`), repository content, LSP diagnostic strings, and legacy
  peers.

### Security & Isolation Properties

1. **No arbitrary action or JS smuggling into read-only slots:** `agy-proxy`
   cannot invoke `cowboy_execute` or `cowboy_read` as read-only via
   `cowboy/extensionCall`. Instead, the trusted Worker handles a dedicated
   execution-socket method **`cowboy/postEditDiagnostics`** accepting strictly
   `{ path, expectedSha256, timeoutMs? }`, validates every field, and itself
   synthesizes the `cowboy_execute` MCP request for `action: "code.post_edit_inspect"`
   on its own `builtin: true` `cowboy` MCP process (never a folder binding).
2. **No privilege escalation from a compromised proxy:** `code.post_edit_inspect`
   is strictly scoped to the parent session's own worktree (`SessionReadScope`),
   gated by `read_parent`, `superagent::restricted`, `grant.agent_actions`, and
   `LanguageServerTrust`. It returns bounded diagnostics only when the file on
   disk matches `expected_sha256`, which is strictly a subset of what the
   session can already observe via `cowboy.code.diagnostics`.
3. **Separate read-only concurrency bound (§D4):** Read-only extension calls
   (`cowboy_status`, `MEMORY_READS`, and `cowboy/postEditDiagnostics`) consume
   a dedicated `MAX_READ_CALLS = 4` pool in `Ledger`, completely separate from
   mutating extension calls (`MAX_CALLS = 8`). Even a burst of slow or stuck
   reads can never starve mutating calls (`review.start`, folder bindings,
   `memory_put`).
4. **Zero `UncertainEffect`, zero turn-drain hold:** Read-only calls never
   increment `state.admitted[turn]`, have `carries_result = false`, produce
   empty `unread_effects` on disconnect or `L1c` drain, and never call
   `ledger.mark_uncertain`. Turn cancellation or completion (`Ledger::fence`)
   wakes `turn_closed(&turn)` immediately, ending the worker's foreground wait
   and cancelling queued reads before `Server::write`.
5. **§D4 purity scope (explicitly accepted):** Like Claude's `code.post_edit`,
   `code.post_edit_inspect` may trigger bounded Zed language-server buffer
   acquisition (`acquire::acquire`, capped per-session/per-Machine with 300 s
   idle release) and content-matched baseline caching (`baselines::record`,
   which is idempotent for identical `(sha256, utf8_bytes)` and runs only on
   exact `document_current` + `expected_sha256` match). Neither mutates
   workspace files, shell cursors, delivery suppression ledgers, credentials,
   or target processes.

### Strictly Nested Timeout Hierarchy

Every hop's timeout nests strictly outside the hop below it so the Controller
finishes and returns a clean JSON response first, releasing `cowboy mcp` and
`Server.io` cleanly without orphaned background work:

| Hop | Budget | Behavior on Expiry |
|---|---|---|
| Controller `code.post_edit_inspect` | `timeout_ms`: `1..=4000 ms` (default `3500 ms`) | Stops polling, skips `baselines::record`, returns `{ deliver: false, matches_expected: false, files: [{ path, state }] }` |
| Target `cowboy mcp` (`cli.rs`) | `5000 ms` (`wait_ms() = 0` + `5000 ms`) | Unmodified CLI; returns timeout JSON-RPC reply, releasing `Server.io` |
| Target `Gateway` (`gateway.rs`, fixed in §6) | `12000 ms` (`wait_ms() + 12 s`, matching `CallHost` `10 s + 2 s`) | Replaces accidental `2000 ms` cliff (on un-updated Hawk hosts before maintenance, warm LSP `< 2 s` still succeeds and cold `> 2 s` degrades cleanly) |
| Worker `ExtensionBroker::read` | `controller_timeout_ms + 2000 ms` (default `5500 ms`, max `6000 ms`) | Refuses unsent if `send_before` expires while queued on `Server.io`; otherwise returns typed read-only error (`outcome: "none", readOnly: true`); background task drains `cowboy mcp` reply (bounded by `CALL_TIMEOUT = 170 s`) |
| Proxy `Connection.call` | `worker_deadline_ms + 2000 ms` (default `7500 ms`, max `8000 ms`) + `abortInFlight: true` on `signal` | Retires request ID so late replies never close socket; aborts in 0 ms on `CancelTurn` / `endTurn`; edit result returned unchanged |

---

## 3. Controller Design (`components/server-agent-api`)

### 3.1 New Action: `code.post_edit_inspect`

Registered in `components/server-agent-api/src/server/agent_api/actions.rs`
directly beside `code.post_edit` (after `read_parent` and
`superagent::restricted`, before requiring a durable operation receipt):

```rust
if action == "code.post_edit_inspect" {
    return Ok(success(
        super::code_reads::post_edit_inspect(state, meta, &args).await?,
    ));
}
```

#### Request Schema (strict `only_keys`)

```json
{
  "path": "src/lib.rs",
  "expected_sha256": "<64 lowercase ASCII hex digits>",
  "timeout_ms": 3500
}
```

- `path`: validated with the exact strict relative-path rule used by
  `hover::Locator::parse` (`1..=4096` bytes, no `\0`, no `\`, no control
  characters, every `Path::components()` is `Component::Normal(_)`, normalized
  round-trip equals `path`).
- `expected_sha256`: exact 64 lowercase ASCII hex characters (`[0-9a-f]{64}`).
- `timeout_ms`: optional integer `1..=4000`, default `3500`. Non-integers,
  `0`, or values `> 4000` return `"invalid_argument"`.

#### Side-Effect Guarantees

- **Never** calls `codex_push::sync_cursor`.
- **Never** calls `deliveries::begin` (no pre-receipt repeat suppression).
- **Never** commits `baselines::record` unless `state == "document_current"`
  AND `identities_match` AND `snapshot.matches_content(&expected_sha256)` AND
  post-I/O `trust.current(&state.hub)` AND `session::current(...)` all hold.

### 3.2 Bounded Observation (`diagnostics::inspect_for_expected`)

Implemented in `components/server-agent-api/src/server/agent_api/code_reads/diagnostics.rs`:

1. Resolve `SessionReadScope` via `session::resolve` and call
   `touch_code_observation(state, observed).await`.
2. Observe `LanguageServerTrust` pinned to `observed.session()`. If absent or
   revoked, immediately return `{ deliver: false, files: [{ path, state: "restricted" }], truncated: false, matches_expected: false }`.
3. Compute one wall-clock `deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms)`.
   Every async sub-step (`observe_open_buffer`, `acquire::acquire`,
   `query_open_buffer(RefreshedLanguage)`, and 200 ms poll sleeps) runs under
   the remaining time until `deadline`.
4. **Observation state machine:**
   - First `observe_open_buffer(&state.code, observed, path)`:
     - If `not open`: call `acquire::acquire(state, session, path, trust)`
       (RAII `Opening` drop guard guarantees cancellation safety without
       leaking holding capacity). If `acquire` succeeds, poll
       `observe_open_buffer` every 200 ms until `deadline` or a terminal
       condition below.
     - If already open: call `acquire::acquire` (refreshes agent holding idle
       time within share).
       - If `source_state == SourceState::Changed`: if agent holding succeeded
         and refresh has not been sent yet, issue one
         `Query::RefreshedLanguage { refresh: AgentOwned }` (ignoring `"refused"`
         when a user Code pane shares the buffer, since the Code pane reloads
         from disk automatically). Poll `observe_open_buffer` every 200 ms
         until `deadline` or a terminal condition below.
       - If `source_state == SourceState::Matching` and `buffer_snapshot`
         exists and `!buffer_snapshot.matches_content(expected_sha256)`: disk
         already moved on to different text (another edit superseded this one)
         — stop polling immediately!
       - If `source_state == SourceState::Matching` and
         `buffer_snapshot.matches_content(expected_sha256)` and `analysis` is
         still `Unknown` / `Stale` (LSP still running on the reloaded text):
         poll every 200 ms until `deadline` or `document_current`.
   - **Terminal stop conditions** (checked on every poll iteration):
     - `!trust.current(&state.hub)` -> stop immediately (`restricted`).
     - `!session::current(&state.hub, &state.machine_control, observed)` ->
       stop immediately (`Err("code_context_changed")`).
     - Permanent error (`outside this session` -> `refused`, other error ->
       `unavailable`).
     - `source_state == SourceState::Matching && snapshot.is_some_and(|s| !s.matches_content(expected_sha256))` -> stop immediately (`stale` / superseded).
     - `identities_match(&diagnostics, snapshot) && file_state(analysis, source_state) == "document_current" && snapshot.is_some_and(|s| s.matches_content(expected_sha256))` -> **exact match, stop immediately!**
5. **Final fence, conditional baseline commit, projection & byte cap:**
   - Re-check `trust.current(&state.hub)` and `session::current(...)`.
   - If exact match holds:
     - Capture `pending = baselines::capture(&diagnostics, snapshot, analysis)`.
     - Commit `comparison = baselines::record(observed.session(), path, pending, &diagnostics)` synchronously right before serialization (idempotent for identical `(sha256, utf8_bytes)`).
     - Project with existing `post_edit::project(&observation)` (caps: 10
       errors, 5 warnings, 200 chars/message, 2000 total chars).
     - Attach `"matches_expected": true` and `"analyzed_sha256": expected_sha256`.
     - Verify `serde_json::to_vec(&projected)` fits `<= 12 * 1024` bytes (fail
       closed to `{ deliver: false, files: [{ path, state: "unavailable" }], truncated: false, matches_expected: false }` if ever exceeded).
     - Emit structured tracing log `event_name = "cowboy.code.post_edit_inspect"`
       (session, path, state, matches_expected, deliver — never repository text).
   - Otherwise (timeout, `stale`, `last_observed`, `not_open`, `refused`,
     `unavailable`, mismatched SHA-256):
     - Do **not** call `baselines::record`.
     - Return `{ deliver: false, files: [{ path, state }], truncated: false, matches_expected: false }`.

---

## 4. Worker Design (`src/worker/sandbox_extensions.rs`, `src/worker_execution/`)

### 4.1 Built-In Server Identity & Capability Advertisement

- `ServerPlan` gains `pub builtin: bool`, set `true` **only** for the worker's
  own `"cowboy"` server derived from its launch template in
  `src/worker/sandbox_extensions.rs`, and `false` for every folder binding.
- `Tool` in `extension_broker.rs` records `builtin: bool` from its `ServerPlan`.
- `ExtensionBroker::list()` (`cowboy/extensions`) emits:
  - `builtin: "cowboy"` on tools where `tool.builtin == true`;
  - `readOnly: true` on tools where `read_only(tool)` holds (`cowboy_status` on
    `builtin == true`, or `MEMORY_READS`: `memory_search`, `memory_get`,
    `memory_read`, `memory_receipt` on `Route::Memory`);
  - top-level `"postEditDiagnostics": true` **only** when discovery found a
    tool with `tool.builtin && tool.original == "cowboy_execute"` (otherwise
    `false`; older workers omit the field entirely).

### 4.2 Dedicated Read-Only Method: `cowboy/postEditDiagnostics`

- Exported constant `pub const POST_EDIT_DIAGNOSTICS: &str = "cowboy/postEditDiagnostics";`.
- Dispatched in `Client::relay_guarded` (`src/worker_execution/ledger.rs`)
  right alongside `EXTENSION_CALL` — **never** forwarded to the target keeper.
- `ExtensionBroker::post_edit_diagnostics(&self, client, params)` validates:
  1. `params` is a JSON object containing only keys `path`, `expectedSha256`,
     and optional `timeoutMs`.
  2. `path` passes the exact strict normalized relative-path validator.
  3. `expectedSha256` is exact 64 lowercase ASCII hex characters.
  4. `timeoutMs` (optional) is an integer `1..=4000` (default `3500`).
  5. Discovery succeeded and includes `tool.builtin && tool.original == "cowboy_execute"`.
  6. Admits a read-only call slot via `ledger.admit_call(false)` (requires an
     active turn and `state.read_calls < MAX_READ_CALLS = 4`).
- Synthesizes the locked-down MCP `tools/call` payload:
  ```json
  {
    "name": "cowboy_execute",
    "arguments": {
      "action": "code.post_edit_inspect",
      "arguments": {
        "path": "<validated path>",
        "expected_sha256": "<validated sha256>",
        "timeout_ms": 3500
      },
      "operation": "post-edit-<uuid>"
    }
  }
  ```
- Executes through the shared `ExtensionBroker::read` path with foreground
  deadline `Duration::from_millis(controller_timeout_ms + 2000)` (at most
  `6000 ms`) and `send_before = Some(Instant::now() + deadline)`.

### 4.3 Read-Only Execution, Settlement & `L1c` Drain Guarantees

Adapts the proven `claude/agy-worker-readonly` (`952d42790`..`110702d07`)
design onto current `origin/main` (including `4fdc05c5a` `SettlementGuard` and
`unread_effects`):

- **Separate read-only slot pool (`ledger.rs`):**
  - `MAX_CALLS = 8` for mutating calls (`drains == true`, increments
    `state.calls` and `state.admitted[turn]`).
  - `MAX_READ_CALLS = 4` for read-only calls (`drains == false`, increments
    `state.read_calls` only; never increments `state.admitted[turn]`).
- **Immediate turn-close wakeup (`ledger.rs`):**
  - `Ledger::fence` drops its mutex and calls
    `self.changed.send_modify(|v| *v += 1)` so `Ledger::turn_closed(&turn)`
    resolves immediately on turn end, cancel, or session close.
- **No `UncertainEffect` on loss or drain (`ledger.rs`, `relay.rs`):**
  - `Client::carries_result`: returns `false` for `POST_EDIT_DIAGNOSTICS` and
    `broker.effectful(params)` for `EXTENSION_CALL` (fails closed when broker
    absent or tool unknown).
  - `unread_effects(method, carries_result, reply)`: pushes `"tools/call"` for
    `EXTENSION_CALL` **only when `carries_result && reply.get("result").is_some()`**
    (fixes unconditional `"tools/call"` push for read-only calls during `L1c`
    drain or socket drop). Returns empty for `POST_EDIT_DIAGNOSTICS`.
- **No wasted send after deadline (`extension_broker.rs`):**
  - `Server::request` accepts `send_before: Option<Instant>`. While awaiting
    `self.io.lock()`, it selects across `self.io.lock()`,
    `ledger.turn_closed(turn)`, and `tokio::time::sleep_until(send_before)`.
    If the turn closes or `send_before` expires before `Server::write`, it
    returns `Failure::NotSent` without writing to `cowboy mcp`, freeing its
    `read_calls` slot immediately.
  - Once written, the background task awaits the reply up to `CALL_TIMEOUT`
    (`170 s`); on `Failure::MaybeSent`, it holds the `read_calls` slot until
    `started + CALL_TIMEOUT` before calling `backend.settle_call(&admitted)`.
  - `ExtensionBroker::read` returns `(reply, None)` (no `SettlementGuard`
    holding `admitted[turn]`) as soon as the answer arrives, `deadline`
    expires, or `turn_closed` fires.

---

## 5. Proxy & Facade Design (`components/provider-runtime/`)

### 5.1 `execution-connection.mjs`

- `Connection.call(method, params, options)`:
  - If `options?.signal?.aborted`, rejects immediately unsent before
    `ensureConnected()`.
  - Races `this.ensureConnected()` against `options.signal` abort (without
    disturbing `this.connecting` for other callers) so a request abandoned
    during reconnect is never sent.
- `Connection.request(method, params, { timeoutMs, signal, abortInFlight = false })`:
  - Rejects unsent if `signal?.aborted`.
  - When `abortInFlight === true` and `signal` is provided (used **only** for
    read-only calls), attaches a `{ once: true }` abort listener that clears
    `timer`, deletes `id` from `pending`, adds `id` to `abandoned` + `retired`
    (so any late reply is safely consumed rather than closing the WebSocket),
    and rejects immediately. Removes the listener on normal reply, timeout, and
    `disconnect()` so long-lived `turn.abort.signal` objects never leak
    listeners.
  - Mutating calls keep `abortInFlight = false` (never abandoned in flight).

### 5.2 `antigravity-acp/extensions.mjs`

- Constructor records `builtin: tool.builtin === "cowboy" ? "cowboy" : undefined`
  and `readOnly: tool.readOnly === true` per tool, plus
  `this.postEditSupported = postEditDiagnostics === true`.
- `readOnly(name)` returns `Boolean(this.routes.get(name)?.readOnly)`.
- `call(connection, name, args, { timeoutMs, signal } = {})`:
  - When `this.readOnly(name)` is true, passes
    `{ timeoutMs: localTimeout, signal, abortInFlight: true }` and treats lost
    or timed-out replies as a normal MCP error `{ content: [...], isError: true }`,
    never throwing `UncertainEffect`.
  - Non-read-only calls keep existing `UncertainEffect("tools/call")` behavior.
- `postEditInspect(connection, { path, expectedSha256, timeoutMs = 3500, signal })`:
  - Returns `null` immediately if `!this.postEditSupported` or `signal?.aborted`.
  - Calls `connection.call("cowboy/postEditDiagnostics", { path, expectedSha256, timeoutMs }, { timeoutMs: timeoutMs + 4000, signal, abortInFlight: true })`.
  - Parses MCP result (`reply?.isError !== true`, `JSON.parse(reply.content[0].text)`,
    unwraps `parsed?.value ?? parsed`); returns `null` on any error or invalid
    JSON (never throws `UncertainEffect`).
- `postEditFindings(value, { path, expectedSha256 })`:
  - Fails closed unless `value?.deliver === true`,
    `value?.matches_expected === true`,
    `value?.analyzed_sha256 === expectedSha256`, and `value.files` contains an
    entry with `file.path === path`, `file.state === "document_current"`, and a
    non-empty `Array.isArray(file.diagnostics)`.
  - Projects only bounded fields `{ files: [{ path, state, comparison?, diagnostics }], truncated: value.truncated === true }`.
  - Escapes `/[<>&\u2028\u2029]/g` to `\uXXXX` inside JSON and wraps in
    `<system-reminder>...</system-reminder>` matching Claude's `postEditNote`
    (`plugins/claude-code/runtime/context-mod.js:1501`), refusing notes whose
    UTF-8 byte length exceeds `16 KiB`.
  - Computes SHA-256 repeat-suppression key of the canonical projected payload
    and checks `this.shown` (TTL `600_000 ms`, bounded `32` entries). Returns
    `null` if already shown within the window; otherwise returns `{ key, note }`.
  - `shownNow(key)` records `Date.now()` in `this.shown` only right before
    returning the tool result to `agy`.

### 5.3 `antigravity-acp/facade.mjs`

- Adds `this.publishSeq = 0` and bounded `this.revisions = new Map()` (capped
  at `2048` entries, FIFO eviction). `publish()` increments `revision = ++this.publishSeq`
  and records `this.revisions.set(file, revision)` **before** invoking
  `publishOnce()` so any overlapping publish on the same path immediately
  supersedes older in-flight diagnostic lookups.
- **Clean separation of `turn.inflight`, `onResult`, and model-facing note:**
  - `Facade.answerOnce` passes `readOnlyExtension = Boolean(extension && this.extensions?.readOnly(name))`:
    - Plan mode (`this.authorize`) remains completely unchanged (all agent-initiated
      extension calls still require R3 Plan-mode authorization).
    - Read-only extension calls are NOT added to `turn.inflight` or
      `parentInflight` and pass combined `readSignal(turn, session.parentTurn, signal)`.
  - `writeFile` and `editFile` record `{ file, revision, invalidation, bytes }`
    in an internal `postEdit` out-slot supplied by `Facade.call` once `publish()`
    succeeds (including `verified === true` E1 recovery where `bytes` is
    confirmed on disk).
  - Inside `Facade.call(sessionId, name, args, relayId, { signal })`:
    1. `callTurn.openCalls += 1` (decremented in `finally`).
    2. `const cleanResult = await this.answer(call, { signal, postEdit })` —
       by the time `answer()` resolves, `publish()` has already completed and
       exited `turn.inflight`, so `settled()` never waits on diagnostics.
    3. If `postEdit.target` is set, run
       `note = await this.fetchPostEditNote(session, callTurn, postEdit.target, signal)`:
       - Eligibility gates (all required): `!this.managed`,
         `session.parent === undefined`,
         `!(callTurn.sessionId !== undefined && this.subagents?.owns?.(callTurn.sessionId))`,
         `this.extensions?.postEditSupported === true`,
         `file !== this.root && inside(this.root, file)`,
         `bytes.length <= 4 * 1024 * 1024`, `!bytes.includes(0)`, valid UTF-8
         `text = decodeText(bytes)` with no lone `\r` after `\r\n` -> `\n`
         normalization, and turn still open/non-uncertain.
       - Computes `expectedSha256 = sha256(Buffer.from(normalized))` and calls
         `this.extensions.postEditInspect(this.connection, { path: posix.relative(this.root, file), expectedSha256, timeoutMs: LIMITS.postEditMs, signal: combinedSignal })`
         where `combinedSignal` aborts when either `callTurn.abort.signal` or
         per-call `signal` aborts.
       - After await, re-verifies `session.turn === callTurn`,
         `!callTurn.cancelled && !callTurn.ended && !callTurn.uncertain`,
         `!signal?.aborted`, `this.revisions.get(file) === revision`, and
         `(this.invalidations.get(file) ?? 0) === invalidation`.
       - Extracts `{ key, note }` via `this.extensions.postEditFindings(...)`.
    4. `this.onResult(call, cleanResult)` runs on the **clean** edit result so
       Web Edit/Write cards keep their crisp diff and summary without raw
       `<system-reminder>` JSON clutter.
    5. If `note` is present and the turn is still live/un-cancelled/non-uncertain
       and `(relayId === undefined || !session.ambiguousRelays.has(relayId))`:
       - Records `this.extensions.shownNow(note.key)` and appends
         `\n\n${note.note}` to the first text block of a non-mutating copy of
         `cleanResult`.
    6. Passes the result to `this.maybeAttachWrapUpNote(...)` as today.

---

## 6. Independent Pre-Existing Bug Fixes Included

1. **`components/server-runtime/src/server/execution/writes.rs`:**
   - Unwrap `let body = reply.get("result").unwrap_or(reply);` in
     `helper_output` (only when `reply.get("error").is_none()`), fixing the
     real keeper response shape `{ "result": { "chunks": [...], "closed": true } }`.
   - Record timestamps on `helpers` and `reads` entries and prune entries older
     than 5 minutes (plus oldest eviction at `MAX_PENDING = 256`) so an
     abandoned helper proxy cannot permanently saturate write tracking.
2. **`components/store/src/managed_calls/gateway.rs`:**
   - Change line 158 from `Duration::from_millis(request.action.wait_ms() + 2000)`
     to `Duration::from_millis(request.action.wait_ms()) + Duration::from_secs(12)`
     so `Gateway` stays strictly outside `CallHost::forward`'s
     `wait_ms() + 10 s` deadline instead of cutting off `AgentInvoke` /
     `AgentStatus` / `Inspect` / `Result` / `Cancel` at 2.0 s. This also fixes
     cold-file `code.post_edit` timeouts for Claude sessions once Hawk Machine
     updates.

---

## 7. Mixed-Version Compatibility Matrix

| Controller | Hawk Machine (`gateway.rs`) | OVH Worker | Proxy Plugin | Behavior |
|---|---|---|---|---|
| Old | Old (2 s) | Old | New | `postEditDiagnostics` absent -> clean no-op; zero RPCs, zero `UncertainEffect`. |
| Old | Old (2 s) | New | Old | Old proxy ignores new `cowboy/extensions` fields -> unchanged. |
| Old | Old (2 s) | New | New | `code.post_edit_inspect` refused as unknown action -> proxy ignores refusal, returns edit result unchanged; worker never marks uncertain. |
| New | Old (2 s) | New | New | Warm LSP (`< 2 s`) delivers diagnostics immediately; cold LSP (`> 2 s`) times out cleanly without failing edits or marking uncertain. |
| New | New (12 s) | New | New | Full cold + warm post-edit diagnostics up to 4 s Controller budget. |

---

## 8. Release & Rollout Plan (Coordinated with R2)

1. **`retained-worker-interface-compatible` stays `true`:**
   - Zero changes to `Cargo.toml`, `Cargo.lock`, `components/store/Cargo.toml`,
     `components/wire/Cargo.toml`, `components/wire/src/**`,
     `components/plugin-sdk/**`, or `components/provider-sdk/**`.
   - R4 merges cleanly to `origin/main` without blocking
     `cowboy-machine-host-release` or requiring an emergency pin advance.
2. **Component / Provider release:**
   - `cowboy-antigravity-acp` bumps patch (1.0.28 -> 1.0.29),
     `provider-runtime` bumps patch, Antigravity Plugin bumps patch
     (0.2.38 -> 0.2.39), and all 6 other pinning Providers bump patch with
     refreshed closures (`components/registry.json` +
     `components/worker-registry-input.json`).
   - All changed manifests are signed and published before Controller
     activation so exact-release coverage stays satisfied.
3. **Coordinated Hawk/OVH worker pin advance (with R2):**
   - Updating `cowboy-acp-worker` on OVH (enabling `cowboy/postEditDiagnostics`)
     and `cowboy-machine` on Hawk (enabling the 12 s `gateway.rs` outer
     timeout) is combined with **R2 (User MCP servers)** into a single
     owner-approved `cowboy-workers` pin advance following
     `docs/machine-worker-pin-advance-runbook.md` (including Claude +
     connected Code verification and explicit
     `tools/ovh-machine-host-release.sh --workers <release>` selection on OVH).

---

## 9. Verification Plan

### Deterministic Tests

- **Controller (`components/server-agent-api`):**
  - `code.post_edit_inspect` strict key/path/sha256/timeout validation.
  - Exact `expected_sha256` match commits baseline (idempotently on repeat
    calls for identical content) and returns projected diagnostics with
    `matches_expected: true, analyzed_sha256`.
  - Mismatched `expected_sha256`, `stale`, `last_observed`, timeout, or
    revoked `LanguageServerTrust` does **not** call `baselines::record`, does
    **not** move `codex_push::sync_cursor`, does **not** touch `deliveries`,
    and returns `deliver: false, matches_expected: false`.
  - Byte-budget overflow fails closed.
- **Pre-existing fixes (`components/server-runtime`, `components/store`):**
  - `writes.rs` unwraps real keeper `{"result": {"chunks": [...], "closed": true}}`
    replies, rejects `error` replies, and evicts stale `helpers`/`reads`.
  - `gateway.rs` allows `AgentInvoke` taking `> 2 s` to complete cleanly.
- **Worker (`src/worker_execution`, `src/worker/sandbox_extensions`):**
  - `cowboy/extensions` emits `builtin: "cowboy"`, `readOnly: true` for
    `cowboy_status` and `MEMORY_READS`, and `postEditDiagnostics: true` only
    when built-in `cowboy` discovered `cowboy_execute` (never for folder
    bindings or when `cowboy_execute` is absent).
  - `cowboy/postEditDiagnostics` strict input validation; rejects outside turn;
    uses separate `MAX_READ_CALLS = 4` pool without starving mutating
    `MAX_CALLS = 8`; does not block `endTurn` drain; `send_before` aborts
    queued reads unsent; `turn_closed` aborts foreground wait immediately; lost
    reply / disconnect / `L1c` drain produces empty `unread_effects` and never
    sets `uncertain_sequence`.
- **Proxy & Connection (`components/provider-runtime`, `tools/execution-probe_test.mjs`):**
  - `Connection`: aborted `signal` rejects unsent (including during reconnect);
    `abortInFlight: true` retires request ID so late replies are ignored
    without closing the WebSocket and removes abort listeners cleanly.
  - `Extensions`: capability gating on `postEditDiagnostics`; read-only
    extension calls treat lost replies as ordinary errors; untrusted JSON
    escaping (`<`, `>`, `&`, `\u2028`, `\u2029`), byte cap, and client-side
    `shown` repeat suppression (only recorded after receipt).
  - `Facade`: root `WriteFile` and `EditFile` (including `> 48 KiB`, CRLF, and
    `verified === true` E1 recovery) attach diagnostics to model result while
    `onResult` receives clean edit result; subagent edits, managed calls,
    binary/lone-CR files, superseded `revisions`, `invalidations`, and
    cancelled/ended turns suppress diagnostics without failing edits;
    compatibility with `maybeAttachWrapUpNote` and R3 Plan mode.
- **Repo gate:** `just check-compact` + `retained-worker-interface-compatible`
  diff check (`git diff origin/main -- Cargo.toml Cargo.lock components/store/Cargo.toml components/wire components/plugin-sdk components/provider-sdk` empty).

### Live Acceptance (Post-Rollout)

On a real Antigravity session on OVH against a Hawk worktree with Language
Servers enabled:
1. Perform a root `EditFile` introducing a genuine syntax/type error whose
   compiler diagnostic code/message is generated by the language server (not
   written in the prompt or file text).
2. Verify Controller logs `cowboy.code.post_edit_inspect` with
   `matches_expected=true, deliver=true`.
3. Verify the model cites that exact compiler-generated diagnostic code/message
   in its reply without calling `RunCommand` or `code.diagnostics`.
4. Verify Web UI `Edit` card renders clean diff without `<system-reminder>`
   clutter, and verify subagent / disabled-LSP / cancelled-turn edits leave no
   diagnostics and zero `UncertainEffect`.
