# Antigravity Post-Edit Diagnostics (R4) — Design (Revision 2)

Status: design, revision 2 (2026-11-25), incorporating all 13 high and 11 medium
findings from design review `call-c0e04d3ec2bf7e8fa7c499e0` on revision 1, and
resolving all 3 high and 8 medium findings carried into round 3 by
[`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) §R4 (from
design review `call-2061c3702f0dafec842f659e`).

- Baseline: `origin/main` at `87d463ed7aaf7c78175281d637e9972e2837f3ef`
  (Machine protocol 53 / Runtime protocol 8, Antigravity Plugin 0.2.38 /
  `cowboy-antigravity-acp` 1.0.28).
- Companion designs:
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md),
  [`antigravity-steering-r1-design.md`](antigravity-steering-r1-design.md),
  [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md).

## 1. Goals, Scope, and Trust Model

After a root, non-managed Antigravity session publishes an inline file write or
edit (`<= 48 KiB`) through a **keeper-controlled `fs/apply` invocation**, the
Controller observes the keeper-authenticated applied outcome with immutable
worker-admission turn attribution and a keeper-assigned monotonic per-path
publication generation, runs `code.post_edit` raw diagnostics for the edited
workspace file(s), filters superseded or digest-mismatched files **before**
projection, and delivers any non-empty bounded untrusted diagnostics summary
into that exact same active turn through a dedicated, authenticated,
non-replaying, reliable-first `DiagnosticNote` channel.

### Trust Boundaries (aligned with R1 `docs/antigravity-steering-r1-design.md:15–17`)

- **Trusted:** Controller, Machine broker (`cowboy-machine`), runtime worker
  (`cowboy-acp-worker`), and target execution keeper (`cowboy-execution-host`).
- **Untrusted:** `agy` and `agy-proxy` (`cowboy-antigravity-acp`, running
  inside `account_sandbox_v1` as `cga-<key>`), tool output, agent text,
  delegated Agent API peers, and legacy components below Machine protocol 54 /
  Runtime protocol 9.
- **Consequences enforced by construction:**
  1. **Keeper authenticates file publication, never proxy argv/env/stdout.**
     The keeper itself resolves `COWBOY_EXECUTION_FILE_HELPER`, runs the
     conditional apply helper without trusting proxy-supplied environment or
     shell wrappers, verifies the written bytes, computes both raw `sha256` and
     LF-normalized `lfSha256`, and stamps a monotonic per-path `generation`.
  2. **Worker authenticates root eligibility and turn attribution.**
     The worker's trusted `Ledger` admits `fs/apply` only inside an active
     non-managed turn when no subagent is running, and stamps immutable
     `(turn_id, publish_seq)` at admission.
  3. **Proxy cannot self-attest authoritative delivery (`Appended`).**
     Because `agy-proxy` runs inside the untrusted account sandbox and the
     worker does not terminate `agy`'s internal MCP socket, the worker and
     Controller **never** promote a proxy-reported write into trusted
     `State::Appended` and **never** emit an authoritative `user_message_chunk`
     transcript row from a proxy claim. Any note handed off to `agy-proxy`
     settles in Controller's `deliveries` ledger (`code.summary`) as
     `State::Unknown` (with diagnostic reason `"proxy_written"` when the proxy
     confirms the MCP socket write callback, or `"proxy_unconfirmed"` /
     `"transport_disconnected"` / `"timeout"` otherwise). Pre-proxy trusted
     refusals and pre-write proxy rejections settle as `State::Refused`.
     `State::Unknown` remains eligible for `deliveries.rs` repeat suppression
     (matching `State::delivered()`), while never forging an unverified
     authoritative transcript row (matching Claude's non-transcript
     `Route::Adapter` model).

### Explicit non-goals

- Shell-made writes (`RunCommand`), edits above `48 KiB` (`--request-file`),
  subagent edits (`session.parent !== undefined` or while any subagent is
  active), managed read-only sessions, background commands (D3), reopening an
  ended turn, attaching a note to a later turn, retrying a refused/expired
  note, or enabling `_session/steering` for Antigravity.

## 2. Keeper-Controlled Publish (`fs/apply`) and Trusted Attribution

### 2.1 Keeper `fs/apply` (`components/machine/src/execution_host.rs`)

Add a native keeper method `fs/apply` (admitted alongside `fs/writeFile` /
`process/start`, refused in read-only managed mode):

- **Request params** (inline `<= 48 KiB` total JSON):
  `{ path: string, base: "absent" | <64-hex>, expect: <64-hex>, splices: [...], staleTempSeconds: number }`.
  Rejects relative paths, non-file URLs, paths outside the keeper's canonical
  workspace root, invalid digests, or payloads exceeding `48 KiB` before
  spawning the helper.
- **Execution:** the keeper spawns its own trusted `COWBOY_EXECUTION_FILE_HELPER`
  with fixed argv `["apply", "--request", <base64>]` and the keeper's own
  closed environment (ignoring any proxy-supplied env).
- **Atomic verified reply:** on helper `{"schema":1,"status":"applied","sha256":expect,"size":n}`:
  - The keeper computes `lfSha256` (SHA-256 of LF-normalized UTF-8 text,
    matching Zed's `bufferSnapshot.content.sha256`, or `null` if non-UTF-8)
    directly from the verified target bytes (or the helper's verified output),
    closing the CRLF/LF digest mismatch for eligible CRLF edits.
  - The keeper increments and attaches a monotonic per-path `generation: u64`
    (held in keeper memory per canonical path, assigned atomically while the
    apply finishes so `generation` reflects true target publication order, not
    Controller arrival order).
  - Returns one atomic JSON-RPC result envelope:
    ```json
    {
      "result": {
        "status": "applied",
        "path": "/canonical/workspace/path",
        "sha256": "<64-hex raw bytes>",
        "lfSha256": "<64-hex LF-normalized UTF-8 or null>",
        "size": 1234,
        "generation": 7
      }
    }
    ```
  - On conflict, returns `{ "result": { "status": "conflict" } }`.
- **Legacy keeper fallback (`facade.mjs`):** when `session.parent === undefined
  && !this.managed` and the request fits in `48 KiB`, `Facade.publishOnce`
  calls `fs/apply`. If a legacy keeper replies JSON-RPC `-32601` (method not
  found, zero side effects), `Facade` caches `supportsFsApply = false` for that
  connection and falls back to the existing `process/start` `APPLY` shell
  helper (emitting no R4 receipt). Subagents, managed calls, and >48 KiB
  file-backed requests never call `fs/apply`.

### 2.2 Negotiated Admission-Time Turn Attribution & Rollback Safety

- **Wire (`components/wire/src/execution_protocol.rs`):**
  `RuntimeRequest` keeps `#[serde(deny_unknown_fields)]` and adds two optional
  fields:
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub turn_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub publish_seq: Option<u64>,
  ```
- **Rollback-safe negotiated serialization on every hop:**
  - Worker (`src/worker_execution.rs` + `src/worker_execution/ledger.rs`):
    When `Ledger::admit` admits `fs/apply` (requiring `!managed`, active
    `turn_id`, and zero active subagents observed by the worker), it stamps
    immutable `turn_id: Some(turn_id)` and monotonic per-epoch
    `publish_seq: Some(next_seq)` onto the internal `Pending` entry **before**
    initial send. Reconnect resends retain that exact immutable stamp.
  - **Worker -> Broker egress (`src/worker.rs`):** serialize `turn_id` and
    `publish_seq` on `Frame::ExecutionRequest` **only when** negotiated broker
    `peer_protocol >= DIAGNOSTIC_NOTE_RUNTIME_PROTOCOL_VERSION (9)`; otherwise
    clone with `turn_id: None, publish_seq: None` so legacy `deny_unknown_fields`
    brokers decode ordinary execution requests without error!
  - **Machine -> Controller egress (`components/machine/src/machine_cli.rs`):**
    strip `turn_id = None, publish_seq = None` whenever negotiated
    `outer_machine_protocol < DIAGNOSTIC_NOTE_PROTOCOL_VERSION (54)` so a
    rolled-back legacy Controller decodes every execution request cleanly!
  - On any downgraded path (`turn_id` or `publish_seq` absent), Controller
    suppresses R4 publish receipts while ordinary execution succeeds.

### 2.3 Controller Write Observer (`components/server-runtime/src/server/execution/writes.rs`)

Fix all existing and new `writes.rs` issues together:

1. **Keeper reply envelope unwrapping:** `execution_host.rs:619–625` retains
   `{ "result": { ... } }` or `{ "error": { ... } }` inside
   `Outcome::Completed { reply }`. Unwrap `reply.get("result")` and reject
   `reply.get("error").is_some()` for BOTH `Invoke` and `Observe` across
   `fs/writeFile`, `process/read` (`result.chunks`, `result.closed`), and
   `fs/apply` (`result.status == "applied"`). Update tests to use the real
   keeper envelope shape.
2. **Idempotent legacy helper tracking & fair partitioning:**
   - For Claude's `process/start` + `process/read` hints: matching starts are
     idempotent (never reset accumulated stdout); reads track `next_chunk_seq`
     to prevent duplicate chunk concatenation on replay.
   - Partition `helpers`, `reads`, and `pending` per session (up to 16 live
     entries per session across up to 256 sessions), raise `HELPER_TTL` to
     `200 s` (exceeding the worker's `180 s` request timeout), and preserve
     correlations across transient same-epoch reconnects (prune only on epoch
     replacement or session removal).
3. **Watermark-based one-shot `fs/apply` publish receipts (no FIFO amnesia):**
   - For `Command::Invoke { invocation, .. }` with `method == "fs/apply"` or
     `Command::Observe { operation_id }` resolving a pending `fs/apply`:
     - Store `pending_applies` keyed by `(session_id, worker_epoch, operation_id)`
       carrying immutable `(turn_id, publish_seq, created_at)` from the initial
       `Invoke`.
     - When `Outcome::Completed { reply }` unwraps to `result.status == "applied"`
       with valid `path`, `sha256`, `lfSha256`, `generation`, and immutable
       `(Some(turn_id), Some(publish_seq))`:
       - Record `(path, lfSha256 ?? sha256, size)` in `writes` for `latest()`.
       - Check per-`(session_id, worker_epoch)` sliding watermark
         `ReceiptWatermark { low: u64, seen_above: BTreeSet<u64> }` (window 256):
         - If `publish_seq <= low || seen_above.contains(&publish_seq)`:
           **duplicate replay -> drop!**
         - Otherwise insert `publish_seq` into `seen_above`, advance `low`
           across contiguous runs (and conservatively if `seen_above.len() > 256`,
           advancing `low` past the oldest gap so old replays can never be
           re-admitted even after >16,384 completions!).
         - Emit one `PublishReceipt { session_id, worker_epoch, turn_id,
           publish_seq, path, raw_sha256, lf_sha256, generation }` to
           `antigravity_push`.

## 3. Dedicated Authenticated `DiagnosticNote` Protocol (Machine 54 / Runtime 9)

### 3.1 Capability & Protocol Constants (`components/wire/src/diagnostic_note.rs`)

- `DIAGNOSTIC_NOTE_PROTOCOL_VERSION: u16 = 54` (`MACHINE_PROTOCOL_VERSION = 54`).
- `DIAGNOSTIC_NOTE_RUNTIME_PROTOCOL_VERSION: u16 = 9` (`RUNTIME_PROTOCOL_VERSION = 9`).
- `ANTIGRAVITY_DIAGNOSTIC_NOTE_MIN_PROVIDER_VERSION: &str = "0.2.39"`.
- `DIAGNOSTIC_NOTE_CAPABILITY_KEY: &str = "cowboy.dev/diagnosticNote"`.
- Tight frame bounds protecting `CancelTurn` latency:
  - `MAX_DIAGNOSTIC_NOTE_TEXT_BYTES: usize = 4 * 1024` (`post_edit` caps total
    diagnostic message chars at 2,000, so serialized untrusted JSON fits well
    inside 4 KiB).
  - `MAX_DIAGNOSTIC_NOTE_FRAME_BYTES: usize = 6 * 1024`.
  - `MAX_DIAGNOSTIC_NOTE_TTL_MS: u64 = 20_000`.
  - `MAX_DIAGNOSTIC_NOTE_PATHS: usize = 5`.
- `supports_current_turn` and `next_checkpoint` stay permanently clamped to
  `false` for Antigravity (`clamp_delivery_capabilities_for_provider`
  unchanged). A rolled-back legacy Controller cannot send `DiagnosticNote` and
  cannot send `_session/steering` to Antigravity.

### 3.2 Wire Frames & Unsequenced ACKs

- **Downstream (ephemeral, non-replaying, reliable-first):**
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct DiagnosticNotePath {
      pub path: String,
      pub generation: u64,
  }

  // CoreCommand::DiagnosticNote & WorkerCommand::DiagnosticNote
  DiagnosticNote {
      #[serde(default)] // CoreCommand carries session_id
      delivery_id: String,
      target: DeliveryTarget,
      text: String,
      paths: Vec<DiagnosticNotePath>,
      ttl_ms: u64,
  }

  // CoreCommand::DiagnosticNoteInvalidate & WorkerCommand::DiagnosticNoteInvalidate
  // Sent when a newer edit or all-clear observation supersedes a pending note.
  DiagnosticNoteInvalidate {
      target: DeliveryTarget,
      paths: Vec<DiagnosticNotePath>,
  }
  ```
- **Upstream (connection-scoped, UNSEQUENCED — never enters `outbox` / `runtime_seq`):**
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum DiagnosticNoteOutcome {
      Refused,
      Unknown,
  }

  // Top-level Frame variant (outside Frame::WorkerEvent / runtime_seq!)
  Frame::DiagnosticNoteAck {
      session_id: String,
      worker_epoch: String,
      delivery_id: String,
      target: DeliveryTarget,
      outcome: DiagnosticNoteOutcome,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      reason: Option<String>,
  }
  ```
  - **Rollback safety:** worker emits `Frame::DiagnosticNoteAck` only when
    `peer_protocol >= 9`; Machine forwards it to Controller only when
    `outer_machine_protocol >= 54`. On rollback, dropping the unsequenced frame
    leaves **zero sequence gap** in `RuntimeEvent` / `runtime_seq`.
  - **Trust boundary enforcement:** notice `DiagnosticNoteOutcome` has ONLY
    `Refused` and `Unknown` — **no `Appended` variant even exists on the wire**!
    Proxy reports `"written"` -> worker translates to
    `DiagnosticNoteOutcome::Unknown` with `reason: Some("proxy_written")`;
    proxy or worker pre-write rejection -> `DiagnosticNoteOutcome::Refused`.

### 3.3 Reliable-First Scheduling & CancelTurn Priority

On EVERY shared transport hop (Controller -> Machine WS, Machine CLI -> Broker
Unix socket, Broker -> Worker Unix socket, Worker -> Proxy ACP stdin):

1. `DiagnosticNote` and `DiagnosticNoteInvalidate` travel on a dedicated
   bounded non-replaying channel (`cap = 8`, drop-on-full -> immediate
   `Refused("backpressure")`), NEVER entering `pending_commands` and NEVER
   replayed on reconnect.
2. Writer loops use **biased reliable-first scheduling** (`CancelTurn`,
   `Cancel`, `Prompt`, `Permission`, etc. always polled before the diagnostic
   lane) and enforce `unix_socket_has_hint_headroom` (>= 8 KiB free buffer
   before non-blocking write) and `<= 6 KiB` encoded frame cap, ensuring
   `CancelTurn` is never stalled behind diagnostic notes even under a stalled
   reader.
3. On the worker, `DiagnosticNoteSlot` never acquires `steering_gate` and never
   shares waiter slots with `CancelTurn`. `CancelTurn` synchronously fences
   `wrap_up` and `diagnostic_note` slots for `target.turn_id`.
4. **End-to-end expiry & single-settlement contract:**
   - Controller computes `ttl_ms = 20_000` and arms a local waiter timer for
     `ttl_ms + 5_000` (5 s grace for network/ACK return).
   - Each hop deducts its local monotonic queue duration from `ttl_ms` before
     forwarding and rejects with `Refused("expired")` if `ttl_ms == 0`.
   - Controller settles each `delivery_id` **at most once** by atomic removal
     from `shared.diagnostic_notes`:
     - Disconnect/epoch-change before wire write -> `State::Refused`.
     - Disconnect/epoch-change/timeout after wire write -> `State::Unknown`.
     - Late `Frame::DiagnosticNoteAck` after removal -> logged and ignored.

## 4. Controller Freshness Batching (`antigravity_push.rs`)

New module `components/server-agent-api/src/server/agent_api/code_reads/antigravity_push.rs`,
spawned alongside `codex_push::run` in `maintain_holdings`:

1. **Per-`(session_id, worker_epoch, turn_id)` keying (no cross-turn relabeling):**
   - `SessionBatch` holds `target: DeliveryTarget` (`worker_epoch`, `turn_id`),
     `pending: HashMap<String, ReceiptSnapshot>` (`rel_path -> { generation,
     lf_sha256 }`), `highest_gen: HashMap<String, u64>`, and `running: bool`.
   - When a `PublishReceipt` arrives for a **different** `(worker_epoch, turn_id)`
     than `SessionBatch.target`:
     - Immediately replace `SessionBatch.target`, **clear** `pending` and
       `highest_gen` (fence any in-flight task from the old turn by comparing
       its captured `DeliveryTarget` before and after every await!).
   - Reject stale receipts where `receipt.generation <= highest_gen[rel_path]`
     or where `session` is not eligible (`provider != "antigravity"`, managed
     child, language servers disabled, `diagnostic_note` capability false for
     `receipt.worker_epoch`, or `path` outside worktree `cwd`) **before**
     mutating `pending`.
   - When a newer `generation` arrives for a path that had an active pending
     note on the same turn, immediately emit `DiagnosticNoteInvalidate` for
     `[(rel_path, receipt.generation)]` so the proxy discards any waiting stale
     note immediately!
2. **Pre-projection raw digest & freshness verification (no budget waste):**
   - After `SETTLE` (`300 ms`), snapshot up to `MAX_PATHS` (5) entries from
     `pending` (along with the captured `DeliveryTarget`).
   - Run **raw** `diagnostics::read(state, observed, relative_paths)` (explicit
     paths only — NEVER advancing `codex_push::CURSORS`!).
   - Re-verify `session::current`, `delivery_target == snapshot_target`, and
     for EVERY file in raw `observation["files"]`:
     - Drop the file if `highest_gen[path] > snapshot_gen` (a newer edit
       started during analysis -> handled by the next loop pass).
     - Drop the file if `snapshot.lf_sha256` is `None` or
       `file["bufferSnapshot"]["content"]["sha256"].as_str() != snapshot.lf_sha256.as_deref()`
       (works identically for LF and CRLF files because `fs/apply` derived
       `lf_sha256` from the exact verified written bytes!).
   - Call `post_edit::project(&filtered_observation)` ONLY on the surviving
     fresh files so stale/mismatched files never consume the `10 errors / 5
     warnings / 2000 chars` budget!
   - If `projected["deliver"] != true`:
     - Emit `DiagnosticNoteInvalidate` for the snapshot's `(path, generation)`
       pairs (so an all-clear fixing edit invalidates any earlier pending note
       waiting on the proxy!) and log `nothing_to_deliver`.
   - If `projected["deliver"] == true`:
     - Record `deliveries::begin(session, Route::AntigravityNote, files,
       deliveries::digest(&projected))`. If suppressed as repeat, stop cleanly.
     - Format the bounded untrusted JSON note using `codex_push::note(&projected)`
       and verify `text.len() <= MAX_DIAGNOSTIC_NOTE_TEXT_BYTES`.
     - Dispatch via `RemoteRuntime::send_diagnostic_note(...)` (never calling
       `send_delivery::begin`, so `config_revert::clear` is never called).
     - Settle `deliveries::finish` once (`State::Refused` or `State::Unknown`
       with reason). Never emit a Hub `user_message_chunk` transcript row.

## 5. Proxy & Facade Delivery & Invalidation State Machine

### 5.1 Out-of-band notifications (`proxy.mjs`)

- Add `"cowboy/diagnosticNote"` and `"cowboy/diagnosticNoteInvalidate"` to
  `RUNTIME_NOTIFICATIONS` and handle both **synchronously out-of-band** before
  `this.queue` (exact R1 `cowboy/wrapUpHint` pattern, never blocking
  `session/cancel` / Stop).
- Strip upstream `result._meta["cowboy.dev/diagnosticNote"]` and
  `result.agentCapabilities._meta["cowboy.dev/diagnosticNote"]` on `initialize`,
  stamping `{ v: 1, adapter: "cowboy-antigravity-acp" }` only when `!this.managed`.
- Validate incoming `cowboy/diagnosticNote`: `!this.managed`, `!this.closing`,
  root session, live non-cancelled non-closed non-compacting turn matching
  `attemptNonce`, `0 < ttlMs <= 20_000`, `text` <= 4 KiB UTF-8, `paths` non-empty
  (<= 5 entries with positive integer `generation`), AND no path in `paths` is
  already superseded by `facade.pathPublishGen(sessionId, path) > generation`.
  - If invalid or already superseded -> emit `cowboy/diagnosticNoteResult`
    with `outcome: "refused"` immediately.
  - Otherwise store as `turn.diagnosticNote` (state `"pending"`, monotonic
    `deadlineMs = performance.now() + ttlMs`), superseding any older `"pending"`
    note with `outcome: "refused", reason: "superseded"`.
- On `cowboy/diagnosticNoteInvalidate`: if `turn.diagnosticNote` is `"pending"`
  and shares any path whose incoming `generation >= notePath.generation`, clear
  it and emit `outcome: "refused", reason: "superseded"`.
- Also tighten `plan-gate.mjs` eviction hygiene: when `list.length >= MAX_WAITING`,
  mark the evicted entry `evicted.ambiguous = true` so `isBindingConsistent`
  always returns `false` for evicted entries.

### 5.2 Single-Owner Call Accounting & Synchronous Pre-Write Guard (`facade.mjs`)

- **Fix double-counted `openCalls`:** track in-flight root MCP calls **once**
  per invocation in `serveMcp` (a `Set` of active call tokens per relay/session;
  do not double-increment in `Facade.call`).
- **Per-path local generation fence:** whenever a root `WriteFile` or `EditFile`
  begins `publishOnce`, increment `session.localPublishGen.get(relPath)` (and
  record keeper's returned `generation` on `fs/apply` success). Starting ANY
  new publish of `relPath` synchronously invalidates any `"pending"`
  `turn.diagnosticNote` referencing `relPath` (`outcome: "refused", reason:
  "superseded_by_edit"`), closing the post-dispatch freshness window so a
  fixing edit never receives obsolete advice!
- **Synchronous pre-write attachment in `serveMcp`:**
  Right before `stream.write(...)` sends a root tool call's MCP response (after
  `onResult` captured the clean un-noted result for UI presentation):
  1. Synchronously check: `!facade.closing`, `!streamEnded`, `!stream.destroyed`,
     `!stream.writableEnded`, root session (`session.parent === undefined`),
     `!managed`, `session.ambiguousRelays.size === 0`, live `turn` matching
     `attemptNonce`, `!cancelled`, `!ended`, `!compacting`, `name !== "UpdatePlan"`,
     `!result.isError`, `Array.isArray(result.content)`, `note.state === "pending"`,
     `performance.now() <= note.deadlineMs`, and every path in `note.paths`
     still matches the latest local/keeper generation.
  2. If all pass, non-mutatingly append `\n\n${note.text}` to the **first text
     block** of `result.content` (or append a text block if image-only,
     composing cleanly with `WRAP_UP_NOTE`).
  3. Transition `note.state = "write_started"` and remove it from
     `turn.diagnosticNote` so no second call can touch it.
  4. Bind the exact delivery's `stream.write(line, cb)` + `close` + `error`
     handlers with a once-only `settle(outcome, reason)`:
     - Synchronous throw before kernel submission -> `settle("refused", "write_threw")`.
     - `cb(null)` -> `settle("written", "proxy_written")` (worker converts to
       `DiagnosticNoteOutcome::Unknown` with `reason: "proxy_written"`).
     - `cb(err)`, `close`, `error`, turn cancel, or shutdown after
       `write_started` -> `settle("unknown", "relay_interrupted")`.
  5. On `startAttemptCleanup`, `cancel`, `beginShutdown`, or `deadlineMs`
     expiry: any remaining `"pending"` note settles `refused` (`"turn_ended"` /
     `"cancelled"` / `"expired"`).

## 6. Release, Conformance, and Acceptance Plan

Because R4 modifies `components/wire/src` (`execution_protocol.rs`,
`runtime_wire.rs`, `machine_protocol.rs`, `diagnostic_note.rs`), it requires an
explicit owner-approved `cowboy-workers` pin advance per
[`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md):

1. **Branch discipline:** keep all changes on `feat/agy-post-edit-r4` until
   ready to land together with the pin advance commit (never break
   `retained-worker-interface-compatible` on `main`).
2. **All 7 bumped Providers published before Controller activation:**
   `components 3.152.0` (`cowboy-antigravity-acp 1.0.29`, `@cowboy/provider-runtime
   1.3.35`, `antigravity 0.2.39`, plus patch bumps of `claude-code`,
   `claude-deepseek`, `codex`, `codex-deepseek`, `deepseek-harness`, `grok`).
   Build, sign, verify, and publish **all 7** Provider releases into
   `/var/lib/cowboy/plugins/catalog` and pass `provider-release-coverage`
   against the exact candidate revision and catalog **before** activating the
   Controller.
3. **Staged worker pin advance & deletion production conformance:**
   - Run independent acceptance (`execution-worker-conformance`,
     `execution-session-conformance`, `agent-worker-conformance`,
     `logs-conformance`) against `.#cowboy-machine-release` of fetchable
     candidate `R`.
   - Advance `inputs.cowboy-workers` to `R` in `flake.nix` / `flake.lock`, then
     build two distinct pinned writer revisions (`cowboy-machine-writer-host-release`
     at `P1` and `P2` with distinct ELFs), the matching recovery reader
     (`cowboy-machine-host-release`), and separate default reader, and obtain a
     passing `just session-deletion-production-conformance` receipt before Hawk
     activation.
   - Activate Hawk Machine (`cowboy-machine-activate`) and OVH full worker
     release (`tools/ovh-machine-host-release.sh` with `.#cowboy-machine-release`)
     separately with explicit owner maintenance approval, then install
     `antigravity 0.2.39` on OVH.
4. **Live acceptance on dedicated free-tier account (unpredictable challenge):**
   - After `agy` completes an `EditFile` on a workspace file, inject a **fresh
     randomly generated diagnostic challenge** inside the diagnostic fixture /
     language-server output that NEVER appears in prompts, repository files, or
     other tool outputs.
   - Wait until the `DiagnosticNote` is pending on the proxy, then have `agy`
     run a probe `ReadFile` on an unrelated file and report the exact diagnostic
     from the tool output.
   - Assert: (a) `agy` reproduces the exact unseen challenge token; (b)
     Controller `code.summary` `deliveries` records `route: "antigravity_note"`,
     `state: "unknown"`, `reason: "proxy_written"` (never an untrusted
     `Appended` transcript row); (c) a superseding edit invalidates a pending
     note; (d) CRLF file edits match and deliver; (e) subagent edits and >48 KiB
     edits produce zero R4 receipts.

## 7. Test Plan

- **Wire & rollback (`components/wire`, `worker.rs`, `machine_cli.rs`):**
  already-pending `RuntimeRequest` serialized with `turn_id`/`publish_seq` on
  `>= 54/9` and stripped on rollback `< 54` or `< 9` (testing both rollback
  orders against strict `deny_unknown_fields` legacy decoders); unsequenced
  `Frame::DiagnosticNoteAck` dropped on rollback with zero `runtime_seq` gap;
  `clamp_delivery_capabilities_for_provider` unchanged and header-shaped
  owner/peer sends refused.
- **Keeper `fs/apply` & `writes.rs`:** keeper executes its own
  `COWBOY_EXECUTION_FILE_HELPER` ignoring proxy env, returns canonical path +
  raw `sha256` + `lfSha256` + monotonic `generation` for both LF and CRLF
  files; `writes.rs` unwraps real keeper `{ "result": ... }` envelopes for
  `Invoke` and `Observe`, rejects `{ "error": ... }`, deduplicates `fs/apply`
  via sliding `publish_seq` watermark even after >16,384 completions, makes
  legacy starts idempotent and sequences legacy read chunks, partitions per
  session with 200 s TTL.
- **Controller `antigravity_push.rs`:** `(session, epoch, turn)` keying
  prevents turn A edits from relabeling into turn B; reversed read completions
  ordered by keeper `generation`; pre-projection raw digest & generation
  filtering preserves diagnostic budget; CRLF matching; all-clear edit emits
  `DiagnosticNoteInvalidate`; `config_revert` untouched; single-settlement on
  timeout/disconnect/late ACK; zero Hub `user_message_chunk` emitted.
- **Transport priority & Worker:** `CancelTurn` overtakes repeated 6 KiB
  `DiagnosticNote` frames under a stalled reader; backpressure drops with
  `Refused("backpressure")`; worker translates proxy `"written"` to
  `DiagnosticNoteOutcome::Unknown("proxy_written")`; `fs/apply` denied
  `root_publish` when managed or subagent active.
- **Proxy & Facade (`antigravity-acp`):** end-to-end attachment through real
  `serveMcp` socket write (verifying single-counted active calls, first-text-block
  attachment, coexistence with `WRAP_UP_NOTE`), pre-write cancellation/expiry/
  local-publish-generation invalidation -> `refused`, post-`write_started`
  close/error/cancel -> `unknown`, legacy keeper `-32601` fallback to `APPLY`
  helper, `plan-gate.mjs` eviction marking `ambiguous = true`.
