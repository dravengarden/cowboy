# Background tasks: unified presentation first, Calls as a capability platform later

Status: living design. **Phase 1 is implemented** (Web + Controller, 2026-10-31,
branch `claude/background-tasks-p1-20261031`); Phases 2–7 are planned and each
needs its own reviewed contract before implementation. This document replaced
the v1 proposal after the Astra Max review ("request changes"); every finding
was resolved, none rejected. Statements marked **[unverified]** are inferred
from code and still need production evidence.

Related: [`subagents.md`](subagents.md) (the ACP subagent snapshot and its
worker side), [`managed-agent-calls.md`](managed-agent-calls.md) (the Call
ledger), [`client-opentelemetry.md`](client-opentelemetry.md) (the
`task.reconcile` client event), [`../web/src/desktop/FOCUS.md`](../web/src/desktop/FOCUS.md)
(keys).

## Summary

**Problem.** A Claude background subagent finished, yet the transcript kept
showing "Working". The database already held the terminal `idle/end_turn`
snapshot; delivery to the client failed:

- every update of a subagent replaced its row in place at the row's first seq;
- a reconnect delivers only the newest 200 events, so a cached `running` row
  older than that tail is never corrected;
- history pages are cached `immutable`, so the stale page also survived reloads
  of the same build;
- two independent reducers (the Hub hot log and the store writer, which starts
  empty on every Controller start) and a clear-all at 4,096 children could
  each start a *second* row for a known child (the duplicate at seq 35526).

**Direction (adopted from the review).**

- Unify presentation before transport and extensibility: a small `TaskSummary`
  view model, one adapter per source, details routed to the existing views.
  Call execution stays ledger-authoritative and its polling is unchanged.
- One compact transcript row per task; details open in an inspector (Desktop
  Dialog, Mobile Sheet), never inline. The terminal chip is always shown and
  distinguishes Done, Failed, Stopped, **Ended** (cessation observed without
  an outcome) and **Not reporting** (contact lost — never terminal).
- A one-line bar replaces the Calls panel, behind a rollback switch.
- One idempotent transcript anchor per accepted Call.
- "Needs you" only for a currently actionable human request.

| Phase | Content | Components | State |
|---|---|---|---|
| 1 | Terminal-state reconciliation, compact rows + inspector, unified list and bar, Call anchors, approval binding, diagnostics | Web + Controller (one new durable table); no Machine release | **implemented** |
| 2 | Notifications: persisted intents, per-account acknowledgement, Web Push accepted on a physical iPhone | Controller + Web | planned |
| 3 | Native background work: ACP v2 Background Work once adapters and the worker speak it; no AIR impersonation | Machine/worker + Provider | planned |
| 4–7 | Capability platform: versioned envelope and implementation binding, trust and injection rules, atomic quota reservation, schema negotiation and rollback readers, discovery | Controller, SDKs, Web | planned |

User decisions (2026-10-31): accept the §9 decisions; add the durable
`transcript_anchors` table in Phase 1; keep the Calls rollback switch for two
Web releases or 14 days, whichever is longer (see
[Rollback switch](#rollback-switch)); include the approval-binding fix in
Phase 1.

## 1. Review findings and resolutions

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | Child identity, execution and observation health conflated (P1) | Three keys: stable `entity`, `execution`, `observation`. No "linked reply → done". "Ended" only when cessation is observed. Load/resume fencing. Legacy rows declare incomplete history | §3 |
| 2 | A bounded projection can recreate the bug (P1) | No newest-N list. Every client-known non-terminal id is reconciled by authoritative lookup; complete vs partial bootstrap; transcript fallback only for unsupported servers | §4 |
| 3 | Reducer repair incomplete: two reducers, `MAX_SUBAGENT_ROWS` (P1) | One durable anchor table read by both reducers, LRU plus lookup instead of clear-all; reproduced before the fix | §2, §4.1 |
| 4 | Projection consistency (P1) | Invariants I1–I8 | §4.2 |
| 5 | Pending approval is not a running call's input (P1) | Pending admission is its own item, keyed by approval request; approval bound to the exact presented set | §5 |
| 6 | Similar status names hide different contracts (P1) | Display grouping is not a state machine; adapters keep source fidelity; "Needs you" requires a request | §3.2 |
| 7–9 | Capability mechanism, trusted descriptions, quota (P1, capability rollout) | Later phases P4–P6 | §8 |
| 10 | Native-task counting and retention (P1) | Legacy count is aggregate activity, never added; itemization waits for ACP v2 | §6.3, §8 |
| 11 | Notifications and seen state (P2) | Phase 2 contract | §8 |
| 12 | Interaction contracts (P2) | Focus restoration, stable selection, IME suppression, Mobile Back and deep links, text scaling, nested roll-up, read-only view, explicit Stop outcomes | §6 |
| 13 | Discovery budgets (P2) | P7 | §8 |
| 14 | Compatibility beyond optional fields (P1, capability rollout) | Reader-first rollout, cross-version matrix with both databases | §7, §8 |

## 2. Diagnosis and reproduction

The terminal signal is produced and stored. The adapter sends
`subagent_update {state:{state:"idle",stopReason:"end_turn"}}`
(`finishTask` → `reportState` → `rfdEndState`); the worker publishes it as a
`cowboy_subagent` snapshot; the durable row holds it.

What failed is delivery to a client that holds a cached row:

- `EventReducer` rewrote every snapshot to the child's first seq. The Hub
  reducer was rebuilt only from the hot tail; the store writer's reducer
  started **empty** on every Controller start; at 4,096 children the map was
  **cleared**. Any of these started a new anchor row. A Controller restart
  while the worker replays unacknowledged frames re-delivers a snapshot with
  the *same* revision to an empty writer, which matches the duplicate row.
- A live client gets the raw envelope at a new seq; a reconnect gets
  `SNAPSHOT_TAIL = 200` events; `mergeCanonicalTimeline` keeps cached rows
  outside the tail; immutable history pages and the IndexedDB replica keep
  the stale snapshot.

**Reproduction (before the fix).** Kept as permanent tests:

- Web `backgroundTasksReconcile.test.ts` T1: a cached `running` row at seq 10,
  then a 200-event reconnect tail (seq 301–500). Pre-fix the merged timeline
  rendered `{a: "Working"}` (expected `Done`) — the tail-miss path.
- Controller `persistence_tests.rs` T3/T4: a store writer restart between a
  child's first row and its terminal snapshot produced durable rows
  `[(5, "running"), (400, "idle")]`; more than 4,096 children produced
  `[(1, "running"), (9000, "idle")]` — the duplicate-anchor paths.
- T18, live: a real Controller binary on a disposable SQLite stack served the
  real Web bundle to a disposable hawk Chrome context; three subagents
  followed by 300 later events were cached as Working, the tab went offline,
  the three finished across a Controller restart, and the tab came back
  online. Because the reconnect tail overlapped the cached window, nothing
  refetched the anchors: the pre-fix build stayed Working without a reload;
  the fixed build shows Done (reconciled by lookup). When the offline gap is
  larger than the tail, the existing gap fill already repaired the rows, so
  the stale state needs an overlapping tail. See §7.

## 3. Identity, status and display

### 3.1 Three identities

| Key | Subagent | Call | Native work (Phase 3) |
|---|---|---|---|
| `entity` | child ACP `sessionId` | `child_session_id` | terminal / tool call id (ACP v2) |
| `execution` | **none in Phase 1**: snapshots carry only the latest state | `call_id` (immutable outcome in the ledger) | the item itself |
| `observation` | `{incarnation, source_revision, observed_seq, reporting}` | `{ledger revision, fetched_at}` | `background` list freshness |

A subagent row is an *entity* row: its chip shows the latest observed
foreground state, never an immutable run outcome. Once a resume is observed
(`running` after `idle`) the details say "Showing the latest run only".
Execution numbering needs the worker (Phase 3). A Call row is an *execution*
row; continuing a conversation creates another row.

### 3.2 Display groups (presentation only)

| Group | Chip | Subagent (ACP RFD v1 snapshot) | Call (ledger) |
|---|---|---|---|
| active | Working / Starting / Queued | `running` | `queued`, `starting`, `running` |
| waiting | Waiting | `requires_action` without a request this client can answer | `waiting_input` |
| needs you | Needs your answer | a pending permission/elicitation tagged `_cowboy.subagent` with this child | — (pending admissions are §5 items) |
| stopping | Stopping | Stop accepted, not yet idle | `stopping` |
| done | Done | `idle` + `end_turn` | `completed` (verdict shown separately) |
| failed | Failed[: reason] | `idle` + `error` / `refusal` / `max_tokens` / `max_turn_requests` | `failed` with `error.code` |
| stopped | Stopped | `idle` + `cancelled` | `cancelled` |
| ended | Ended: outcome unreported | `idle` **without** stopReason | — |
| not reporting | Not reporting | `unknown`, `reporting:false`, or an older incarnation | non-terminal while its Machine is unreachable: **state kept**, badge "Reconnecting" |
| unavailable | No longer available | lookup `not_found` (cleared or retired) | — |

R1 no linked-reply shortcut; R2 not-reporting is never terminal; R3 a Call
keeps its execution state while disconnected; R4 "Needs you" needs a request
(several per entity are counted); R5 MCP, A2A and ACP v2 adapters keep their
own semantics when they arrive.

Implementation: `web/src/subagents.ts` `subagentStatus`,
`web/src/backgroundTasks.ts` `callSummary`.

### 3.3 Load/resume fencing

A worker change (new incarnation, `starting`, exit or crash) fences every
snapshot observed before it: not reporting, no Stop, until the new
incarnation reports the child again. A browser reconnect to the same
incarnation is not a load boundary. Implemented by the Controller's
`incarnation` stamp plus the bootstrap's current incarnation, and in `derive`
by comparing a snapshot's `observedSeq` with the last worker boundary seq (a
durable row sits at its old anchor, before the boundary, but was observed
after it — position alone used to fence it wrongly after a reload).

## 4. Reconciliation contract (Phase 1, implemented)

### 4.1 Durable anchors

Migrations `migrations/0076_transcript_anchors.sql` (PostgreSQL) and
`migrations/sqlite/0050_transcript_anchors.sql`:

```sql
transcript_anchors(session_id REFERENCES sessions ON DELETE CASCADE,
                   anchor_key, seq, created_at_ms,
                   PRIMARY KEY (session_id, anchor_key))
```

- Keys `subagent:<child>` and `call:<call_id>`; `backfill:v1` (seq 0) marks a
  session whose pre-table rows were scanned once.
- Written in the same transaction as the event batch
  (`Store::upsert_event_batch_with_anchors`); the first anchor wins and never
  moves. Clearing a transcript deletes its anchors and writes the sentinel.
- `src/transcript_anchors.rs` `AnchorIndex`: a bounded LRU (16,384 keys) used
  by both reducers. Eviction forgets one key; the clear-all is gone.
- The Hub adopts the newest 16,384 anchors from the table at start, then the
  restored hot log. The store writer resolves a key it does not know through
  the batch's own pending anchors, then the table, then the one-time session
  backfill (the earliest `cowboy_subagent` row per child), and only then
  treats it as new. Counters record hits, misses, evictions, store lookups
  and backfills.
- A Hub miss after eviction (more than 16,384 recent anchors) may place the
  next live row at a new hot-log seq; the durable row still resolves to the
  original anchor and clients merge by child id. **[accepted limitation]**
- Rollback: an older Controller ignores the table (SQLx `ignore_missing`) and
  keeps today's behavior.

### 4.2 Invariants

- **I1** Call rows come from committed ledger reads (`/calls`, still polled).
- **I2** The Controller stamps every `cowboy_subagent` it ingests with
  `observedSeq` (the live envelope seq) and `incarnation` (the Machine worker
  epoch of the frame). Clients keep the snapshot with the highest
  `observedSeq`; unstamped rows fall back to `revision`.
- **I3** A snapshot whose incarnation is not the session's current one renders
  not reporting, without Stop.
- **I4** Per child and incarnation: an identical `(incarnation, revision,
  payload)` is dropped (`subagent_duplicates`); an older revision is dropped
  (`subagent_revision_stale`); an equal revision with a different payload is
  accepted and counted (`subagent_revision_conflicts`).
- **I5** The focused bootstrap (`GET /api/sessions/{id}/bootstrap?since_seq=N`)
  carries a sibling `subagent_state` (not an `Outbound`, so protocol bridges
  and older clients are unaffected):
  `{session_id, epoch, watermark_seq, complete, incarnation, items:[{anchor_seq, update}]}`.
  Items are every child the Controller knows as non-terminal plus every child
  observed after `since_seq`. `complete` is false when the store or backfill
  failed. A new `epoch` (Controller process) replaces cached ordering; each
  bootstrap replaces the previous authority.
- **I6** After a bootstrap the Web looks up every cached child it renders as
  non-terminal that the bootstrap did not list:
  `POST /api/sessions/{id}/subagents/lookup {ids ≤ 200}` (paged) →
  `found` with the latest snapshot or `not_found`, rendered "No longer
  available", never finished.
- **I7** Without `subagent_state` (an older Controller) cached rows render
  as-is (`reconciled_by: fallback`).
- **I8** After the admission commit the Controller pushes
  `cowboy_call_anchor {callId, requestId, origin, approvalRequestId?}` into the
  parent, keyed `call:<id>`; a repeat replaces the same row. Controller start
  repairs missing anchors of non-dismissed Calls of open sessions that are
  active or were admitted after the database first ran an anchor-writing
  Controller (setting `transcript_anchors.calls_since_ms`) — older history is
  not appended to the end of transcripts. Metadata updates come from the
  ledger, never add a row. `origin` is `{kind:"mcp", action, operation}` for
  `cowboy_execute` `calls.start` / `review.start`, `{kind:"gateway"}` for the
  private gateway (`cowboy call` / `cowboy review` in a shell), `{kind:"repaired"}`
  after repair. The Web links only a tool card whose structured `rawInput`
  carries that operation; output text is never parsed.
- **Timestamps** Call durations come from ledger times; subagents expose only
  observation times.

### 4.3 Diagnostics

Each task's details have a collapsible **Diagnostics** section: `source`,
`entity`, `execution`, `incarnation`, `source_revision`, `observed_seq`,
`anchor_seq`, `reconciled_by` (`live|bootstrap|lookup|fallback|not_found`),
`reconciled_at`. The Web logs one client event `task.reconcile` per bootstrap
with ids checked, changed, not found and the oldest stale age. The Controller
exports on `/metrics`:

```
cowboy_transcript_anchor_{cache_hits,cache_misses,evictions,store_lookups,backfills,call_repairs}_total
cowboy_subagent_{revision_conflicts,revision_stale,duplicates,epoch_changes,lookup_requests,lookup_ids}_total
```

## 5. Pending admissions and approval binding (Phase 1, implemented)

- A pending admission (`call_approval`) is an attention item (`approval:*`),
  not a task: no Call, row or anchor. Its "Answer" jumps to the existing
  `CallApprovalOverlay` / `DesktopApprovalDock`.
- Every waiting request has a stable `approval_request_id` (`apr-…`). The
  prompt carries `request_ids` (every waiting request) and a
  `prompt_revision` that advances when the set changes.
- `POST …/calls/approval {decision, presented, prompt_revision}` applies only
  to the presented requests still waiting; a later arrival stays pending and
  receives a refreshed prompt; if nothing presented still waits the answer is
  `409` with a refreshed broadcast. Limits, policy and capacity are rechecked
  at admission as before. A decided request's id links into its Call anchor
  (`approvalRequestId`).
- A Web client without `presented` keeps the old aggregate behaviour for one
  release (compatibility only; remove with the rollback switch).
- An expired prompt (`ttl_ms`) disappears and is never actionable; a recorded
  decision older than 30 minutes is not applied.
- Out of Phase 1: capability-scoped digest binding and disabled/ask/auto
  (P4–P5).

## 6. Presentation (Phase 1, implemented)

### 6.1 `TaskSummary` and the detail router

`web/src/backgroundTasks.ts`: `subagentSummary`, `callSummary`,
`approvalSummary`, `buildTaskSummaries` (nested attention roll-up and `(+N)`),
`groupTasks` (Needs you / Active / Ended), `barSummary`.
`web/src/useBackgroundTasks.ts` assembles the live list from the timeline, the
reconciliation authority, the shared Call poll (one poll per parent now feeds
the bar, rows, inspector and legacy dock) and pending requests. Details:
subagent → its former card body (`SubagentDetail`); call → `CallConversation`
+ `CallDetailView`; approval → the approval surface.

### 6.2 Transcript row and inspector

- `web/src/TaskRow.tsx`: icon, title, subtitle, optional Stop, chip last;
  minimum (not fixed) height; nested `(+N)`; warning dot when descendants need
  attention. Opening never expands the transcript (T14).
- `web/src/BackgroundTasksInspector.tsx`: the Tool inspector's frame (Desktop
  Dialog with a rail; Mobile cover Sheet), grouped list, stable selection by
  key, a read-only view always, Stop only with a current grant, Diagnostics.
- Stop outcomes: Accepted → Stopping → the source confirms; otherwise
  "Stop was rejected" (the Controller's refusal), "Timed out, still running"
  (30 s, state kept) or "Disconnected: Stop was not sent". Session Stop is
  unchanged; there is no "stop all background tasks".

### 6.3 Bar

`web/src/BackgroundTasksBar.tsx` replaces the Calls panel in the Composer
stack: `◐ 2 working · ⚠ 1 needs you · ✕ 1 failed — Review: auth fallback 1m40s ›`.
It counts only TaskSummary items. The legacy `SessionMeta.background_tasks`
(`max(native, working subagents)`) is never added; a neutral "+ background
activity" mark appears only when it exceeds the working subagent rows. It
hides when nothing is active or needs attention; `␣G` still opens the
inspector. "Failed unseen" uses a device-local acknowledgement (opening the
details); per-account acknowledgement is Phase 2.

#### Rollback switch

`cowboy:ui.calls_dock_legacy` (device preference; "Use the previous Calls
panel" at the bottom of the inspector list, or `localStorage`) restores the
old Calls dock. **Removal deadline: the later of 2026-11-14 (14 days) and the
second Web release after the release that ships the bar.** Parity checklist
before removal: list, every call detail section, Stop, open conversation,
Clear ended with Undo, `[`/`]` stepping, Mobile drill-in. Clear ended with
Undo is still reached through the legacy dock only — port it into the
inspector before removing the switch.

### 6.4 Keyboard, focus, Mobile

- Desktop: `Tab` selects a row (Conversation widget), `Enter`/`l` opens the
  inspector at its key, `x` opens Stop; `␣G` opens the inspector. Inside it:
  `j/k` and `[`/`]` move the selection, `o` opens a Call's conversation,
  `Shift+L` locates the row, `x` Stop, `Esc`/`h` closes. Closing restores
  focus to the originating row, or the composer from the bar. Single keys are
  ignored during IME composition and in inputs.
- Mobile: list → detail → conversation, one history entry each, so Back walks
  up a level and returns to the transcript at its scroll; `#task=<key>` deep
  links into details; the sheet respects safe-area insets.

## 7. Acceptance (Phase 1)

| ID | Covered by | Result |
|---|---|---|
| T1 | Controller `background_tasks_tests` (SQLite + PostgreSQL); Web `backgroundTasksReconcile.test.ts` (bootstrap and lookup paths) | pass; pre-fix failure recorded in §2 |
| T2 | same (61 children, no window) | pass |
| T3 | `persistence_tests` writer restart (both DBs) + `background_tasks_tests` Hub/writer restart | pass; pre-fix failure recorded |
| T4 | `persistence_tests` >4,096 children, small-LRU eviction and legacy backfill (both DBs) | pass; pre-fix failure recorded |
| T5 | `transcript_anchors::tests`, `background_tasks_tests`, Web T5 | pass |
| T6 | `remote_runtime` T6, `background_tasks_tests`, Web T6 | pass |
| T7 | Web T7 + "latest run only" in the inspector | pass |
| T8/T9 | browser `background-tasks` suite | pass |
| T10 | `approval.rs` T10 tests; Web T10; browser suite (expired prompt not actionable) | pass |
| T11 | `background_tasks_tests` (idempotent anchor, crash repair, MCP vs gateway origin); Web T11 | pass |
| T12 | Web T12; browser suite | pass |
| T13 | Web T13 + `subagents.test.ts` | pass |
| T14 | browser suite at 100% and 200% text scale | pass |
| T15 | browser suite (Desktop keys, IME, focus return); `just desktop-browser-gate` | pass |
| T16 | browser suite in Chrome Mobile emulation (Back ×3, scroll, `#task=`) | pass; **iOS Simulator not run** |
| T17 | Controller legacy-row decode/backfill (both DBs); Web fallback (I7); old Web ignores `subagent_state`, `observedSeq`, `cowboy_call_anchor` | pass |
| T18 | live seeded stack, pre-fix vs fixed build | pre-fix Working ×3, fixed Done ×3 |

T18 procedure (`tools/background-tasks-live-repro.ts <controller> <web-root>
<cdp-endpoint> <out-dir>`): build `cowboy-controller` and the Web bundle from
the pre-fix commit and from this branch; for each, start the Controller on a
fresh SQLite database behind a loopback TLS proxy (Option 1 requires an HTTPS
origin), create the only user through the product's setup and login routes,
seed one session with three `running` `cowboy_subagent` rows followed by 300
events, open it with `?session=` in a disposable hawk Chrome context and read
back to the start (the device caches the whole window), take the tab offline
(CDP network emulation), stop the Controller, write the three terminal
snapshots in place at their anchors (the durable state the writer produces)
plus 20 events, restart the Controller and bring the tab online. Results
(2026-10-31, Chrome 151): pre-fix build `Working ×3`, fixed build
`Done ×3`, no reload in either; new Web + old Controller and old Web + new
Controller both render without errors and stay `Working` (the repair needs
both components). Limits: no real Claude agents (a disposable stack cannot
run signed Provider plugins with live credentials), so the adapter → worker
→ ingest path is covered by the worker conformance and the Controller tests
above, not by T18.

Rollout order: Controller first (adds the table, stamps, bootstrap field,
lookup, anchors, binding), then Web. Old Web + new Controller: ignores the new
fields and update kind; approval keeps the legacy aggregate. New Web + old
Controller: no `subagent_state` → fallback rendering, no lookup, approval
without `request_ids` → legacy body. Controller rollback after anchors exist:
the table is ignored. No Machine release.

## 8. Later phases (each needs its own contract; none blocks Phase 1)

| Phase | Contract |
|---|---|
| **P2 Notifications and acknowledgement** | Persisted intents with dedupe keys (`exec:<call_id>:terminal`, `entity:<child>:<observed terminal seq>`, `req:<approval or permission id>`); bootstrap/history replays never notify; completion bursts coalesce; execution failure distinct from review findings; defaults actionable input + unresolved failure on, completion opt-in; a short-lived focused-device presence lease; per-account acknowledgement; push as retryable delivery with duplicate suppression, accepted on a locked physical iPhone per `conventions/pwa.md`. |
| **P3 Native background work** | ACP v2 Background Work (`state_update.background`, `terminal/cancel`, `tool_call/cancel`, `paused`) once adapters and the worker speak v2; no AIR `asyncTasks` impersonation; correlate native ids with subagent and tool ids; keep known items across worker restart as unresolved; worker generation numbering gives subagents an execution identity. |
| **P4 Capability envelope and implementation binding** | `{schema, capability:{id,version}, request_id, inputs, implementation:{provider, generation, preset, runtime_profile}}`; persisted manifest digest; contract, implementations and session policy separate; `same_agent` becomes a review preference; readers (Controller, Machine, worker) ship before writers; rollback tested on both databases; existing settings preserved; new capabilities disabled by default; unknown kinds inspectable. Agent-callable `tools.session_set` must never widen its own authority. |
| **P5 Trust and injection rules** | Application-owned pointer; descriptions are data; reserved core ids and namespaced third-party ids; explicit trust for capability/authority changes; independent enforcement of filesystem, network, credential, context and artifact access. |
| **P6 Resource control** | Atomic reservation at admission, rechecked after approval; idempotent retries; account/provider concurrency; unit budgets; recursion limits; advisory budgets labelled. |
| **P7 Discovery** | Small stable pointer; `calls.capabilities({query?, cursor?})` with bounded search and explicit truncation; separate catalog and availability revisions; invalidation signal; task-level measurements. |

## 9. Decisions on the v1 open questions

| Question | Decision |
|---|---|
| Ended vs Done | "Ended: outcome unreported" when cessation was observed without an outcome; observation loss stays "Not reporting" |
| Call transcript position | One idempotent anchor at admission, linked to the originating tool only through structured MCP identity; Bash `cowboy review` is not linked |
| Shell/monitor rows | Bar/list only (P3); existing tool rows open the same details |
| Seen state | Per-account acknowledgement (P2); device-local interim in Phase 1 |
| Push defaults | Actionable input + unresolved failure on, completion opt-in (P2) |
| Capability defaults | Preserve existing settings; new capabilities disabled (P4) |
| Implementation choice | Model picks the capability and may pick an allowed implementation; `auto` resolves deterministically and is recorded (P4/P6) |
| MCP Tasks | Later; observe already-authorized tasks through the owning integration |
| Write-enabled capabilities | Out of the first release |
| Calls dock removal | Replaced now; rollback switch until the deadline above |
