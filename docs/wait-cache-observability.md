# Wait and prompt-cache observability

This is the Phase 0 audit and Phase 1 source contract for the Columbus
`cowboy-wait-cache-observability` decision at `5c689db8`. Phase 1 observes the
existing dispatch and renewal paths and records the proposed usage gate in
shadow.

## Phase 0 source trace

| Question | Observed path |
| --- | --- |
| Which prompts can renew? | `dispatch_admitted` runs `session_context::rollover::before_dispatch` before claiming the preamble carrier and calling Supervisor. The authored-`cmid` gate excludes `call-notice:` IDs. Slash commands bypass renewal. Owner policy, idle time, context size, ordinary/dormant status, background work and pending context are separate gates. `Supervisor::forget_dormant_context` rechecks Exited, the same native ID, worker absence and no hibernation pending under the wake lifecycle lock. The transcript marker names the committed boundary. |
| What can zero mean? | The worker starts with a `0/0` snapshot. `Hub::set_session_usage` already rejects that seed and persists other readings. Clear and renewal reset the reading and native ID. Older sessions and restores can still have a resumable native ID with no usage record. An observed `0/size` is a distinct fact. Before dispatch, current production still classifies any zero with a native ID as `usage_unknown` before applying the minimum. The hibernation pass still uses the unchanged numeric threshold. |
| What survives a Controller restart? | The Call runner advances the ledger with a CAS, observes the terminal row, then submits `call-notice:<call_id>`. The Hub deduplicates queued or dispatched IDs and transcript echoes. A paused parent keeps its queue. An idle eligible parent can dispatch directly without a persisted queue row. The worker emits a user-message echo before the ACP request and config fences complete. The stored echo includes its bounded `cmid`. Recovery restarts nonterminal Call runners and restores durable pending rows and echoes. A crash between terminal commit and queue admission can leave the notice outside recovery's replay input. A restored queued notice follows existing drain rules and does not itself launch a dormant worker. |
| What does Claude report? | The pinned ACP launch gate in `plugins/claude-code/runtime/idle-compaction.mjs` blocks autonomous idle `/compact`. Native limit-triggered compaction stays enabled. The pinned CLI's cache selection sees `isUsingOverage` and explicit main/subagent TTL settings. Its native traces carry `gen_ai.request.attempt` start events. `api_request` logs carry the total cache creation count and the `ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens` projection as `cache_creation_5m_tokens` / `cache_creation_1h_tokens`. Both split fields can be zero for a positive total. Overage state and actual retention can be absent even when the log reports a split. |
| What does Codex report? | Native `0.169.1` emits HTTP attempts and WebSocket sends separately from `response.completed` usage. Responses `input_tokens` already includes `cached_tokens`. Native code defaults omitted cache-read and cache-write details to zero. The request builder uses `prompt_cache_key` and an optional `prompt_cache_options.ttl`, and WebSocket state can route the follow-up to the same backend. The native usage record leaves requested and granted retention tiers, breakpoint TTLs, billing treatment and cache renewal unknown. |
| What is bounded on a seven-day query? | Each local `otel-*.sqlite` segment has an entry primary key and time, session and trace indexes, with an OTLP BLOB. SQLite `logs analyze --from 7d` calls the same grouped SQL as `metrics` for the requested window and the preceding equal window, each with a five-second SQLite progress deadline. It has a 128-group cap and a bounded evidence-ID lookup. It reads retained evidence from one table per segment. Call reconciliation uses a separate query on the durable ledger. JSONL keeps the 100,000-record and 16 MiB scan limits. SSH RPC has a 20-second and 8 MiB bound. |
| Where do metrics labels come from? | Browser ingress reconstructs client metric resources and point labels. VictoriaMetrics can promote both to series labels. Egress validates the original OTLP bytes against Cowboy's finite canonical projection, including scopes, exemplars, metric metadata and the fixed client histogram layout. The legacy Prometheus lane validates its sample grammar and finite labels before sending the original body. |

Native source references are pinned to [Codex 0.169.1 request construction](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/core/src/client.rs),
[Responses usage conversion](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/codex-api/src/sse/responses.rs)
and the [native request diagnostics](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/otel/src/events/session_telemetry.rs).
See also [context renewal](context-rollover.md) and
[completion notices](agent-api.md#completion-notices).

## Phase 1 evidence

- `cowboy.provider.request_usage` records a native upstream attempt where the
  producer supplies evidence. For Claude, each start event in the LLM span
  owns one attempt. The final attempt can own the span's usage. A matching
  native log supplements individual token fields and can supply the cache
  split and estimated API-equivalent cost. A known error remains an error.
  For Codex, the transport record owns the attempt, including retries
  and transport failures. The observer joins the bounded, ordered stream for
  the same native conversation to a usage record where available. An unmatched
  WebSocket send can be connection prewarm or a stream without usage and
  stays out of the parent clock. Open attempts expire as incomplete after
  ten minutes. DeepSeek reuses the existing gateway usage
  spool's event identity with separate keepalive attribution, alongside the
  existing gateway diagnostic. A gateway's unspecified role stays unknown.
- The worker keeps an allowlisted projection from native OTLP on a private
  loopback listener. Claude sends logs and traces to explicit signal URLs on
  that listener. Codex requires a signed first-party adapter to declare
  `COWBOY_PRIVATE_CODEX_REQUEST_OBSERVATION = "codex-otlp-http-v1"` in its
  runtime contract. The worker passes the adapter a private environment
  descriptor with `schema`, `logs_endpoint`, `header_name` and `header_value`.
  The adapter may consume it only when its native startup can respect the
  effective exporter policy across user, project and managed config
  layers. The receiver checks the header before accepting a body. The signed
  Codex runtime (`plugins/codex/runtime/otel-observation.mjs` and
  `otel-headers.mjs`) inspects effective config across user, project, system
  and managed layers fail-closed and supplies the auth header exclusively via
  `otel.headers_helper` (never in `-c` argv or on disk). Antigravity workers
  project content-free turn and step-timing diagnostics
  (`producer = "antigravity_proxy"`) from `cowboy-antigravity-acp`'s
  `PromptResponse._meta.cowboy` without opening additional sockets or altering
  upstream `agy` requests. Worker request records also carry a 16-hex-char
  structural `prefix_fingerprint` and `is_first_in_worker_epoch` locally
  (never exported as metric series labels). Explicit telemetry environments,
  ambient proxies and sandbox-owned OTLP launches keep their own policy.
  Coverage and source fields record native loss, including at process death.
- `request_index_in_turn` uses the native interaction or prompt boundary
  when available. The three first-after flags are independent. The local
  parent clock uses observed request starts on the same host and Provider,
  including across a worker restart. Late observations can correct a
  successor's daily idle cohort within the seven-day clock horizon.
  Finalized gap fields stay with the retained request detail and its exports
  after the clock entries expire or the rollup retires their cohort.
  Explicitly attributed DeepSeek requests use a source-scoped fingerprint
  and an inferred start. Codex reconstructs the start from the
  native transport timestamp and duration. Keepalives, subagents and unknown
  roles never update that clock. The rollover hint uses the reserved renewal
  block delivered to a fresh worker. Compaction observes a successful typed
  ACP boundary. Unsupported fields and cross-host gaps stay unknown.
- `cowboy.wait.dispatch` and `cowboy.wait.settled` cover Call admission and
  settlement and observed top-level native subagent waits. A successful
  native turn boundary provides positive evidence for `parent_ended_turn`.
  A run rediscovered after a gap is partial until a proven start. A terminal
  snapshot can also yield one partial settlement when the start was missed.
  Its canonical terminal sequence identifies the occurrence across replays.
  `wake_needed` describes the observed need for a dormant worker
  launch for an opted-in Call. The pause flag is separate.
- `cowboy.wait.notice` distinguishes a queued row, a dispatch handoff,
  a duplicate or (before 2026-11-22) an Antigravity draft path.
  `cowboy.wait.receipt` is
  independently recorded on the worker, with a successful ACP completion
  distinguishable from a cancellation or unconfirmed result. The Controller
  also rechecks seven UTC Call cohorts against the receiver's durable echo
  by parent and Call ID. This denominator comes from the terminal Call
  ledger, including records missed by the runner and terminal rows that use
  the ledger's update time as their finish time. A missing echo can
  reflect a hold, a Clear, or a crash gap. Native completion has a separate
  worker receipt.
- `cowboy.cache_opt.decision` carries the production outcome and its
  shadow result. The fallback uses only observed usage from the same native
  ID and signed generation. It can report a genuine zero separately from
  a missing reading. An unversioned restored usage value is insufficient
  for the fallback. The production path owns renewal admission.
  `cowboy.context_rollover.history_read` correlates recall with an observed
  committed rollover boundary, including after hot-tail eviction or
  Controller restore.

## Query and export

```sh
cowboy logs --directory "$LOG_DIRECTORY" query --from 7d --event cowboy.provider.request_usage
cowboy logs --directory "$LOG_DIRECTORY" wait-cache-daily --from 30d
cowboy logs --directory "$LOG_DIRECTORY" wait-cache-metrics --day 2026-11-19 > wait-cache.otlp.pb
```

The daily report reads compact UTC cohorts recorded with tracked detail
before time or capacity eviction. The rollup and its recovery files share
the log store capacity. It keeps up to 45 completed days plus the current
day. At capacity, the writer retires the oldest cohort and retries within a
fixed bound. Retiring an active day marks its coverage incomplete and lets
ingestion continue that day. Rollup failures leave primary log writes
available and mark coverage for the affected event days, including late
arrivals. Closed-day metrics require complete rollup coverage. Each Call
cohort includes the last complete counts and their covered end, alongside
the latest reconciliation attempt status, including truncated attempts.
Only the owning Cowboy service can contribute an event to a rollup.
SSH uses the same bounded read protocol. Read exact event detail with
`query` or export it on the independently authorized VictoriaLogs lane
with `logs export --signal logs`.

The Controller offers one compact previous-day OTLP counter snapshot to its
existing configured exporter. Machine counters can be extracted from a
closed UTC day with `cowboy logs wait-cache-metrics --day YYYY-MM-DD`,
including across multiple `--source` Machines in one invocation so the CLI
aggregates them fail-closed into one single-writer `cowboy.source="machine"`
OTLP snapshot without colliding host series. The existing binding governs delivery.
Every signed metrics lane validates its encoding and admits only finite
instruments, labels and resource attributes independently of its route.
Request series retain
distinct interactive, keepalive, auxiliary and unknown roles. Session/Call IDs,
fingerprints, release digests, model names and paths stay out of metric
series. Inspect `cowboy_observability_failed_metric_batches_total`,
`cowboy_observability_rejected_export_items_total` and
`cowboy_observability_dropped_export_batches_total` alongside a synthetic
query-back. A configured exporter receipt and a queryable sample are
different evidence.

## Phase 2A durable completion notices

Opted-in Managed Calls (`labels.notify = "parent"`) own one durable row in
`managed_call_notices`, inserted atomically in the same transaction as the
terminal `managed_agent_calls` CAS transition. Each Call carries one immutable
`completion_id` (`call-notice:<call_id>`) and moves through explicit states:

- `enqueued`: committed with the terminal Call. Before handing the notice to a
  worker, the dispatcher atomically stamps `dispatch_started_at_ms` (releasing
  the stamp if synchronous handoff fails before leaving the Controller). Queue
  admission and dispatch fencing absorb repeated submissions for the same
  `completion_id`.
- `accepted`: the receiver's durable transcript persisted a `user_message_chunk`
  echo carrying that `completion_id`.
- `consumed`: confirmed either by a completed `TurnEnd` following the echo (with
  no intervening user turn, including a replayed `TurnEnd` after worker
  reconnect) or by an authorized `cowboy.calls.result(...)` read (`cowboy_read`)
  after the sandbox write+flush succeeds, which marks `consumed` (`result_read`)
  and withdraws any still-queued notice. Direct Machine gateway `Action::Result`
  (`cowboy call result`) stays non-consuming without a wire-level post-delivery
  receipt so a transport drop cannot lose an unread notice.
- `unknown`: delivery or consumption could not be confirmed (`parent_unavailable`,
  `dispatch_unconfirmed` after a post-send Controller restart, `worker_stopped`,
  `turn_unconfirmed`, `turn_not_completed`, `superseded_by_user_turn`, or
  `transcript_cleared`).

On Controller startup, `managed_calls::notice_delivery::recover` reconciles
unsettled (`enqueued` / `accepted`) rows against durable transcript events:
pre-send notices (`dispatch_started_at_ms IS NULL`) re-submit or wake a cleanly
dormant (`Exited`) parent (including across native session rollover), while
post-send unconfirmed notices fail closed to `unknown` without blindly
duplicating the prompt. Call inspection, listing, and observation envelopes
expose `notice_delivery`, and transcript clear keeps any Call whose notice is
not `consumed` visible rather than auto-dismissing it. All Providers follow the
same owner-paced delivery path.

## Phase 2B pre-dispatch triage

Once Phase 2A durable completion notices are verified, Cowboy and Columbus
guidance decouple long waits from open parent turns before dispatch (without
modifying signed Plugin manifests outside a signed Plugin release or altering
in-flight native turns):

- **Long (>3–5 min heuristic) read-only work**: dispatch via `calls.start` or
  `review.start` with `notify:true` (CLI: `--notify`) and end the parent turn
  cleanly instead of polling `calls.wait`; read `calls.result` when the durable
  `[Cowboy]` notice arrives.
- **Long write work**: route to an independent isolated session in its own
  worktree (`sessions.create` + `queue.submit` with `notify:true`, or
  `cowboy session prompt --notify`) and end the parent turn.
- **Short critical-path work**: reserve native subagents (Codex
  `spawn_agent`+`wait_agent`, Claude `Task`) for work that fits inside the
  active turn.
- **Hard guardrails**: never abort, reset, or force-compact an in-flight native
  turn. Measure decoupling under ordinary resume first before deciding whether
  Phase 2C (`rollover_on_notify`) is justified.

Canonical rules live in [`AGENTS.md`](../AGENTS.md) and
[`docs/agent-api.md`](agent-api.md#pre-dispatch-triage-long-waits-and-prompt-cache),
paired with the shared Columbus harness contract in `conventions/review.md`.

## Rollout checklist

1. Record the verified main commit. Build `.#cowboy-controller-release` from
   a clean worktree and rehearse the additive observation migrations on an
   isolated database. Record the current component receipts for recovery.
2. At the authorized Controller maintenance window, pass the immutable
   release to the component activator. Verify its receipt, `/healthz`,
   `/version`, the Web cache headers, and Machine presence. Check the
   Controller's local wait and decision events and Call echo reconciliation.
3. Arrange separate Machine maintenance acceptance for worker coverage. Host
   release outputs retain the accepted worker bundle. Use the worker pin
   advance procedure where the deletion writer is active. Verify the
   intended generation in Machine inventory and preserve active sessions at
   the normal lifecycle boundary.
4. Check a disposable query-back against the selected Victoria export route
   before relying on production totals. After the first closed UTC day,
   compare the local rollups with VictoriaMetrics and check the observability
   failure and drop counters. Keep per-Call and per-attempt evidence in the
   local store or the authorized VictoriaLogs lane.

See [component builds and activation](architecture/10-deploy-build.md) and
the [worker pin advance runbook](machine-worker-pin-advance-runbook.md).
