# Antigravity Post-Edit Diagnostics (R4) — Design

Status: design, revision 1 (2026-11-25), resolving all 3 high and 8 medium
findings carried into round 3 by
[`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) §R4 (from
design review `call-2061c3702f0dafec842f659e`).

- Baseline: `origin/main` at `87d463ed7` (carrying R1 wrap-up steering in
  Machine protocol 53 / Runtime protocol 8, and R3 facade-enforced plan mode in
  Antigravity Plugin 0.2.38 / `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## Goals and Scope

After a root, non-managed Antigravity session publishes an inline file write or
edit (`<= 48 KiB`) through Cowboy's apply helper, the Controller observes the
helper's verified `applied` outcome with immutable admission-time turn
attribution, runs `code.post_edit` for the edited workspace file(s), verifies
that the analyzed text matches the exact published digest and has not been
superseded by a newer edit, and delivers any non-empty bounded untrusted
diagnostics summary into that exact same active turn through a dedicated,
authenticated `DiagnosticNote` channel.

### Explicit non-goals

- Shell-made writes (`RunCommand`) — no baseline for a session's first
  foreground command and out of scope for R4.
- File-backed edits above `48 KiB` (`--request-file`) — out of scope for R4.
- Subagent edits (`session.parent !== undefined`) — excluded, matching Claude
  and Codex.
- Managed read-only calls — excluded.
- Background commands (D3).
- Reopening an ended turn, attaching a note to a later turn, or retrying a
  refused/expired note.
- Enabling owner or peer-agent `_session/steering` for Antigravity (remains
  clamped to `false`).

## Architecture Overview

Rather than reusing `_session/steering` (which caused High #3 and Medium #1,
#3, #4 in the revision-3 design), R4 introduces a dedicated, authenticated
Controller-to-Proxy `DiagnosticNote` lane modeled on R1's `WrapUpHint` channel:

```text
agy WriteFile/EditFile (root, inline <= 48 KiB)
  -> Facade.publishOnce runs ["/bin/sh", "-c", APPLY, "apply", "--request", b64]
  -> Worker Ledger admits process/start under immutable active turn_id
  -> Worker Client stamps RuntimeRequest.turn_id ONCE at admission (never re-sampled on retry)
  -> Controller execution::forward -> execution::writes::observe
       verifies exact APPLY argv + stdout {"schema":1,"status":"applied","sha256":expect}
       dedupes one-shot per (session_id, worker_epoch, process_id)
       emits PublishReceipt { session_id, worker_epoch, turn_id, process_id, path, sha256, seq }
  -> Controller antigravity_push (per-session serialized, key-aware freshness batching)
       runs post_edit read (without advancing shell-change cursor)
       verifies per-file write.observed && write.matches_analyzed && sha256 == receipt.sha256
       verifies no newer write_seq arrived for that path during analysis
       records deliveries::begin(..., Route::AntigravityNote, ...)
       dispatches CoreCommand::DiagnosticNote (non-replaying, single monotonic ttl_ms)
  -> Machine Broker -> Worker (DiagnosticNoteSlot, off steering_gate & CancelTurn)
  -> Worker acp.rs -> Proxy notification cowboy/diagnosticNote (out-of-band, off proxy queue)
  -> Facade attaches untrusted JSON note to the next SOLE unambiguous root MCP tool result
  -> Proxy observes exact 1-to-1 ACP tool_call_update(completed) for that sole toolCallId
  -> Proxy notification cowboy/diagnosticNoteResult -> Worker RuntimeEvent::DiagnosticNoteAck
  -> Controller settles delivery ledger; ONLY on Appended:
       persists PromptDispatchRecord(Cowboy("code_intelligence"))
       emits Controller-authored Event::Update(user_message_chunk, promptOrigin=code_intelligence)
```

## Resolution of Every Round-3 Requirement

### 1. Immutable admission-time turn attribution & one-shot receipts (High #1, Medium #6)

- **Wire (`components/wire/src/execution_protocol.rs`)**:
  - Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub turn_id: Option<String>`
    to `RuntimeRequest`.
  - Backward compatible with older workers (which omit `turn_id` -> `None`).
    `writes::latest()` continues serving Claude/Codex write hints even when
    `turn_id` is `None`; R4 publish receipts strictly require `turn_id: Some(_)`
    and non-empty `worker_epoch`, dropping any operation whose originating turn
    cannot be proven.
- **Worker (`src/worker_execution.rs`, `src/worker_execution/ledger.rs`)**:
  - `Ledger::admit` already returns `Admitted { turn: Option<String>, ... }`
    holding the immutable active turn ID at admission.
  - `Client::call_counted_for_turn(command, turn_id)` constructs
    `RuntimeRequest` with that immutable `turn_id` **before** inserting into
    `Client.pending`. Reconnect retries (`Client::frames`) and Machine bulk-lane
    resends therefore carry the exact original admission-time `turn_id` and
    never re-sample `current_turn`.
- **Facade (`components/provider-runtime/packages/antigravity-acp/facade.mjs`)**:
  - In `publishOnce`, pass `"apply"` as `$0` only for root, non-managed, inline
    publishes:
    `["/bin/sh", "-c", APPLY, "apply", "--request", base64]`
    (`APPLY` is `'exec "$COWBOY_EXECUTION_FILE_HELPER" apply "$@"'`, so `$0`
    does not alter `"$@"`). Subagent (`session.parent !== undefined`), managed,
    and file-backed (`--request-file`) publishes keep `"sh"` and emit no R4
    receipt.
- **Controller write observer (`components/server-runtime/src/server/execution/writes.rs`)**:
  - `observe` takes `(session, worker_epoch, turn_id, command, response)`.
  - Existing `["apply", "--request"]` matching stays for Claude's `latest()`
    write hint. R4 publish receipt emission additionally requires the exact
    Antigravity helper shape:
    `argv.len() == 6 && argv[0] == "/bin/sh" && argv[1] == "-c" && argv[2] == ANTIGRAVITY_APPLY_SCRIPT && argv[3] == "apply" && argv[4] == "--request"`,
    plus `turn_id: Some(turn_id)` on `process/start`.
  - `helpers` is keyed by `(session_id, worker_epoch, process_id)` and stores
    `{ path, expect, turn_id, is_antigravity_root, output, created_at: Instant }`.
    `reads` is keyed by `(session_id, worker_epoch, operation_id)` and stores
    `{ process_id, created_at: Instant }`. `pending` (`fs/writeFile`) similarly
    stores `created_at: Instant`.
  - `process/read` and `Observe` correlate strictly within the same
    `(session_id, worker_epoch)` and NEVER overwrite `turn_id` from a read's
    turn; the originating `turn_id` captured at `process/start` is immutable.
  - **Bounded & expiring state (Medium #6)**: on `observe`, expire entries
    older than `HELPER_TTL` (`120 s`); if still at capacity (`MAX_PENDING = 256`),
    evict the oldest entry instead of stalling admission across all sessions.
    Expose `prune_session_epoch(session, worker_epoch)` and
    `clear_session(session)` invoked by `RemoteRuntime` on epoch change /
    disconnect / session removal.
  - **One-shot receipts (High #1)**: maintain a bounded FIFO `completed_receipts`
    set (`MAX_COMPLETED_RECEIPTS = 1024`, keyed by `(session_id, worker_epoch,
    process_id)`). Once `applied(output, expect)` succeeds, insert the key
    before emitting `PublishReceipt`; any replayed `process/read` or `Observe`
    returning cached stdout for the same process is ignored.

### 2. Strict 1-to-1 delivery & receipt proof at Proxy/Facade (High #2, Medium #8)

The revision-3 failure was correlating ACP stdout `tool_call_update` with MCP
`tools/call` across concurrent or heuristic matches. R4 closes this with a
strict **single-open-call barrier**:

- `Proxy` holds at most one `pending` note (and at most one `inFlight` note
  already written to an MCP reply plus one `queued` replacement) per root turn,
  bound to `(deliveryId, attemptNonce, deadlineMs)`.
- After `Facade.call` finishes a tool and `onResult(call, result)` captures the
  clean un-noted result for UI presentation, `proxy.maybeAttachDiagnosticNote(call,
  result, relayId)` attaches the note **only if ALL of the following hold**:
  1. Root session (`session.parent === undefined`), `!this.managed`, `!this.closing`.
  2. `callTurn` is live (`session.turn === callTurn && !cancelled && !ended &&
     uncertain === undefined`), matches `turn.attempt` and `note.attemptNonce`,
     and `!turn.compacting`.
  3. `name !== "UpdatePlan"`, `!result?.isError`, `Array.isArray(result?.content)`.
  4. Relay is unambiguous (`session.ambiguousRelays.size === 0` and `relayId`
     not marked ambiguous).
  5. Monotonic deadline has not elapsed (`performance.now() <= note.deadlineMs`;
     if elapsed, drop and emit `cowboy/diagnosticNoteResult("not_delivered", "expired")`).
  6. **Strict 1-to-1 barrier**:
     - No other in-flight root MCP call exists (`callTurn.openCalls === 0` after
       decrementing this call in `finally`, and `activeRootMcpCalls === 1` in
       `serveMcp` for this call).
     - `PlanGate.soleOpenBinding(call, currentTurn)` proves there is **exactly
       one** open ACP announcement in `waiting.get(sessionId)` (`list.length === 1`),
       and that single entry matches `call`: same `turn`, same `attempt`,
       `name === "mcp_cowboy_" + call.name`, `!ambiguous`, `!tainted`, AND
       `hasInput === true && digest === callDigest` (never an inputless
       announcement).
- If condition (6) does not hold (e.g., concurrent identical or distinct calls,
  inputless announcement, ambiguous/tainted announcement), the proxy **skips
  handoff for this call** and leaves the note pending for the next unambiguous
  call before `deadlineMs` or turn end.
- **Attachment format (Medium #8)**: non-mutatingly append `\n\n${note.text}` to
  the **first text block** of `result.content` (or append a new text block if
  the result is image-only), matching R1's `WRAP_UP_NOTE` so both real
  `agy 1.5.14` and fake test harnesses read the appended text.
- **Receipt state machine**:
  - Transition note to `inFlight = { deliveryId, attemptNonce, toolCallId, call,
    written: false, completed: false }`.
  - In `serveMcp`, when `send({ id, result })` callback finishes:
    - If `!delivered`: immediately emit `cowboy/diagnosticNoteResult("not_delivered", "relay_write_failed")` and clear `inFlight`.
    - If `delivered`: set `inFlight.written = true`; if `inFlight.completed` is
      already true, emit `cowboy/diagnosticNoteResult("appended")` and clear `inFlight`.
  - In `proxy.track(sessionId, update)` when `TERMINAL.has(update.status)` for
    `inFlight.toolCallId`:
    - If `update.status === "completed"` and `!outcome.contradicted` and
      `this.planGate.isBindingConsistent(inFlight.call, turn)`:
      - Set `inFlight.completed = true`; if `inFlight.written`, emit
        `cowboy/diagnosticNoteResult("appended")` and clear `inFlight`.
    - Otherwise (`failed` or `contradicted`): emit
      `cowboy/diagnosticNoteResult("unknown", "contradicted_or_failed")` and
      clear `inFlight`.
  - On turn end (`startAttemptCleanup`), cancel, shutdown, or `deadlineMs`
    expiry:
    - Any unattached `pending`/`queued` note emits `not_delivered`.
    - An `inFlight` note with `written === false` emits `not_delivered`; with
      `written === true` (bytes left proxy over Unix socket without proven
      completion) emits `unknown` (never falsely claims `appended` or
      `not_delivered`).

### 3. Authenticated dedicated protocol lane & rollback safety (High #3, Medium #1, #2, #3, #4)

- **Protocol versions (`components/wire/src/`)**:
  - `DIAGNOSTIC_NOTE_PROTOCOL_VERSION = 54` (`MACHINE_PROTOCOL_VERSION = 54`).
  - `DIAGNOSTIC_NOTE_RUNTIME_PROTOCOL_VERSION = 9` (`RUNTIME_PROTOCOL_VERSION = 9`).
  - `ANTIGRAVITY_DIAGNOSTIC_NOTE_MIN_PROVIDER_VERSION = "0.2.39"`.
  - `DIAGNOSTIC_NOTE_CAPABILITY_KEY = "cowboy.dev/diagnosticNote"` stamped on
    ACP `initialize` `_meta` as `{ "v": 1, "adapter": "cowboy-antigravity-acp" }`
    and stripped from upstream `agy` `_meta` and `agentCapabilities._meta`.
  - `WorkerSnapshot.diagnostic_note: bool` (default `false`), sanitized by
    `effective_diagnostic_note(provider, raw, outer_machine_protocol, runtime_protocol)`.
  - `supports_current_turn` and `next_checkpoint` remain permanently clamped to
    `false` for Antigravity (`clamp_delivery_capabilities_for_provider`
    unchanged).
- **Wire frames**:
  - `CoreCommand::DiagnosticNote { session_id, delivery_id, target: DeliveryTarget, text: String, ttl_ms: u64 }`
    (`MAX_DIAGNOSTIC_NOTE_TEXT_BYTES = 16 * 1024`,
    `MAX_DIAGNOSTIC_NOTE_FRAME_BYTES = 24 * 1024`, `MAX_DIAGNOSTIC_NOTE_TTL_MS = 30_000`).
  - `WorkerCommand::DiagnosticNote { delivery_id, target: DeliveryTarget, text: String, ttl_ms: u64 }`.
  - `RuntimeEvent::DiagnosticNoteAck { delivery_id: String, target: DeliveryTarget, outcome: DiagnosticNoteOutcome, reason: Option<String> }`
    where `DiagnosticNoteOutcome` is `Appended | Refused | Unknown`.
- **Controller rollback immunity (High #3)**:
  - A rolled-back legacy Controller cannot produce `CoreCommand::DiagnosticNote`
    and cannot send `_session/steering` to Antigravity (because Antigravity
    never advertises `steering` / `next_checkpoint`). Mixed-version tests verify
    that header-shaped owner and peer sends against Antigravity are rejected on
    both new and legacy Controllers.
- **Targeted Interrupt priority & independence (Medium #1)**:
  - `RemoteRuntime::send_diagnostic_note` uses a dedicated
    `shared.diagnostic_notes` map (never `shared.steering`).
  - Worker `DiagnosticNoteSlot` never acquires `steering_gate` and never shares
    admission slots with `CancelTurn`. `CancelTurn` synchronously fences both
    `wrap_up` and `diagnostic_note` slots for `target.turn_id`.
- **Single monotonic deadline, no reconnect resend, off proxy queue (Medium #2)**:
  - Controller sets `ttl_ms = 25_000` (remaining monotonic budget, avoiding
    cross-host wall-clock skew between Hawk and OVH). Each hop subtracts its
    local elapsed queue time before forwarding and rejects immediately if `ttl_ms == 0`.
  - `CoreCommand::DiagnosticNote` is **ephemeral / non-replaying**: sent on a
    bounded direct lane (never inserted into `pending_commands`, never resent on
    Controller/Machine/Worker reconnect). On transport drop, Controller's waiter
    settles as `Unknown` (if written to wire) or `Refused` (if never written).
  - On the proxy, `cowboy/diagnosticNote` is an ACP notification handled
    synchronously out-of-band in `fromClient` BEFORE `this.queue`, so Stop /
    `session/cancel` reaches `agy` immediately without ever queuing behind a
    note.
- **Controller-projected transcript row, never `user_message_chunk` echo (Medium #3)**:
  - Worker never emits a `user_message_chunk` for a diagnostic note.
  - Only when Controller receives `RuntimeEvent::DiagnosticNoteAck` matching an
    admitted pending waiter `(session_id, worker_epoch, turn_id, delivery_id)`
    with `outcome == Appended`:
    1. Persist `PromptDispatchRecord` (`cmid = format!("{CMID_PREFIX}{delivery_id}")`,
       `origin = DeliveryOrigin::Cowboy { source: "code_intelligence" }`) through
       `record_steer_dispatch` (failing closed if store write fails).
    2. Emit Controller-authored `Event::Update` (`sessionUpdate: "user_message_chunk"`,
       `content: { type: "text", text }`, `promptOrigin: { actor: "cowboy",
       source: "code_intelligence" }`) tagged with `cmid` into `Hub` (persisted
       and verified on reload/replay with exact `code_intelligence` origin).
    3. Mark `deliveries::finish(session, &delivery_id, State::Appended, None)`.
  - On `Refused` or `Unknown`: emit no transcript row; finish delivery ledger
    with `State::Refused` or `State::Unknown`.
- **Configuration undo window preserved (Medium #4)**:
  - `antigravity_push` never calls `send_delivery::begin`, so
    `config_revert::clear` is never called by diagnostic notes.

### 4. Key-aware freshness batching (Medium #5)

In `components/server-agent-api/src/server/agent_api/code_reads/antigravity_push.rs`:

- Per-session `Pending` tracks `target: DeliveryTarget` and
  `edits: HashMap<String, (u64, String)>` (`relative_path -> (write_seq, expect_sha256)`),
  plus `highest_seen: HashMap<String, u64>`.
- On receiving a `PublishReceipt`:
  1. Reject immediately if `provider != "antigravity"`, managed child, language
     servers disabled, or `path` is outside the session's execution worktree.
  2. Reject immediately if `(receipt.worker_epoch, receipt.turn_id)` does not
     match the session's live active `delivery_target`, OR if
     `receipt.sequence <= highest_seen[rel_path]` (stale / out-of-order receipt
     rejected before mutating `pending`!).
  3. Update `highest_seen[rel_path] = receipt.sequence` and record
     `(receipt.sequence, receipt.sha256)` in `pending`.
- Worker loop serializes one observation per session (`SETTLE = 300 ms`),
  snapshots up to `MAX_PATHS` (5) `(path, seq, expect_sha256)` entries, runs
  `post_edit_value_paths(state, &info.meta, relative_paths)` (explicit paths
  only, never advancing `codex_push::CURSORS`), then filters `value["files"]`:
  - Drop any file where `highest_seen[path] > snapshot_seq` (a second edit
    arrived during analysis -> handled by the next loop iteration).
  - Drop any file where `write.observed != true || write.matches_analyzed != true`
    or where the analyzed buffer digest (`bufferSnapshot.content.sha256`) does
    not match `expect_sha256`.
  - Recompute `value["deliver"]` from the surviving files only; if false,
    log `nothing_to_deliver` and skip.
  - Re-verify `delivery_target == snapshot_target` before `deliveries::begin`
    and dispatch.

## Release & Deployment Plan (Medium #7)

R4 modifies `components/wire/src` (`execution_protocol.rs`, `runtime_wire.rs`,
`machine_protocol.rs`), which is checked by `retained-worker-interface-compatible`
in `flake.nix`. Per [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md):

1. Keep R4 on feature branch `feat/agy-post-edit-r4` until ready to land
   together with an approved `cowboy-workers` pin advance (never break
   `retained-worker-interface-compatible` on `main` ahead of the pin commit).
2. Component / Plugin release (`components 3.152.0`, `cowboy-antigravity-acp
   1.0.29`, `antigravity 0.2.39`, plus patch bumps of the 6 other providers
   pinning `cowboy.provider-runtime`): build, sign, verify, and publish
   `antigravity 0.2.39` to `/var/lib/cowboy/plugins/catalog` **before**
   activating any Controller revision carrying the bumped manifests.
3. Independent worker/session/agent/logs/deletion conformance acceptance on the
   candidate revision, followed by explicit owner go-ahead for the pin advance
   and Hawk/OVH Machine maintenance rollout, then install `antigravity 0.2.39`
   on OVH.
4. **Live acceptance on OVH with real `agy 1.5.14` (Medium #8)**:
   - Open a disposable `antigravity 0.2.39` session on a workspace with a live
     language server (e.g. Rust/TypeScript).
   - Prompt `agy` to introduce a deliberate diagnostic error containing a unique
     identifier via `EditFile`, then call `ReadFile` on another file and report
     any diagnostic surfaced in the tool output.
   - Verify: (a) `agy`'s reply quotes the unique diagnostic that appeared ONLY
     in the appended block; (b) `code.summary` delivery ledger records
     `route: "antigravity_note"`, `state: "appended"`; (c) persisted session
     history shows the Controller-projected `"Code intelligence after an edit"`
     row with `promptOrigin: { actor: "cowboy", source: "code_intelligence" }`;
     (d) subagent edits and >48 KiB edits emit no note.

## Test Plan

- **Wire & protocol (`components/wire`)**: `RuntimeRequest.turn_id` optional
  serde round-trip (present + absent legacy JSON), `DiagnosticNote` frame
  limits, `effective_diagnostic_note` matrix across protocols `<54`/`54`,
  `<9`/`9`, non-antigravity providers, and `clamp_delivery_capabilities_for_provider`
  unchanged.
- **Write observer (`writes.rs`)**: exact Antigravity apply helper matching vs
  subagent `"sh"` vs random shell command, immutable `turn_id` across
  `process/start` -> `process/read` / `Observe` retries, one-shot receipt dedup,
  TTL expiry + capacity eviction + epoch pruning.
- **Controller push (`antigravity_push.rs`)**: reversed-order stale receipt
  rejection, second edit during analysis superseding stale result, digest
  mismatch suppression, repeat suppression in `deliveries`, `config_revert` undo
  window untouched, Controller-projected `code_intelligence` row only on
  `Appended` (and verified on durable replay), zero row on `Refused`/`Unknown`,
  legacy Controller header-shaped owner/peer send refusal.
- **Remote runtime & Worker (`remote_runtime.rs`, `worker.rs`, `acp.rs`,
  `acp_diagnostic_note.rs`)**: independent admission of `CancelTurn` while a
  diagnostic note waits, non-replaying disconnect -> `Unknown`/`Refused`, TTL
  deduction & expiry, pre-0.2.39 / managed / compacting / stale-turn refusal.
- **Proxy & Facade (`antigravity-acp`)**: strict 1-to-1 single-open-call barrier
  (concurrent identical calls skipped, sole unambiguous call attached to first
  text block), `written && completed` -> `appended`, `written && turn_end` ->
  `unknown`, `!written` -> `not_delivered`, contradicting terminal update ->
  `unknown`, out-of-band `cowboy/diagnosticNote` never blocks Stop, coexistence
  with `WRAP_UP_NOTE`, subagent `"sh"` exclusion.
