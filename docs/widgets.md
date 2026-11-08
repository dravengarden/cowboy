# Cowboy Widgets: agent-authored structured UI over Cowboy MCP

Status: design closed for implementation planning, revision 13 (2026-11-08);
nothing here is built or activated. Thirteen Astra Max design review rounds
ran (findings and resolutions in [§1](#1-review-findings-and-resolutions));
by agreement with the owner the thirteenth was the last design round. Its
verdict was "request changes" with three blocking findings, all in the
interactive delivery protocol; they and its four P2 findings are recorded
**open** in [§1.13](#113-revision-13-astra-max-fb9cedf2e--open-phase-1-freeze-prerequisites)
as prerequisites that must be resolved, with fixtures, before the Phase 1
interaction contract is frozen. Display widgets (document model,
presentation, transport, storage, inline carrier) are not affected by them.
Twelve decisions (17, 18, 20–24, 26, 27, 30–32) were approved as design
choices. Statements marked
**[unverified]** are inferred from code or history and still need evidence.

Related: [`agent-api.md`](agent-api.md) (the MCP read/execute/receipt surface
this design extends), [`background-tasks.md`](background-tasks.md) (durable
transcript anchors and the compact-row-plus-inspector presentation),
[`tool-results.md`](tool-results.md) (data is displayed, never evaluated),
[`ux-principles.md`](ux-principles.md) and
[`mobile-spatial-presentation.md`](mobile-spatial-presentation.md) (the
Mobile paint budget every widget renders inside), `web/src/desktop/FOCUS.md`
(keys), [`workspace-documents.md`](workspace-documents.md) (the CAS and
receipt model Drafts already use).

## Summary

**Problem.** Agents report through Markdown prose. The transcript renders
tables, code and diagrams well on Desktop, but the dominant report shapes (a
5–8 row status or comparison table, a key/value summary, a progress list, a
question with options, a screenshot) are produced as text that degrades on
a phone, cannot be updated in place and cannot be acted on except by typing.
Images are the clearest gap: tools produced 1,087 image blocks in the
sampled history while agent prose linked an image six times.

**Direction.** One versioned, typed **JSON widget document**
(`cowboy.widget/1`, TypeScript contract in
[`contracts/widget-v1.ts`](../contracts/widget-v1.ts)), published and
patched through a `widgets` domain of the existing Cowboy MCP surface, kept
authoritative in Controller tables, projected as one anchored transcript row
that older clients still read as a plain notice, rendered by a Cowboy-owned
React + MUI renderer whose components adapt to the width they are given, and
acted on through taps that become user messages composed by the Controller
**from what the person saw and nothing else**: the button label, or the
question and the chosen answers, the widget's public handle and the revision
it was decided on. Agents author data, never code, never hidden
instructions, and no agent-chosen identifier is ever attributed to the
person. Delivery of such a message is proven by the turn it started, never
by the transcript echo, and an unproven delivery is shown as uncertain and
retried only by the person. Phase 1 is deliberately narrow: grouping,
tables, key/value, lists and steps, status and progress, images and typed
references, buttons and choices. Its token case is bounded and honest: a
widget costs 2–4× a terse report to publish and about twice a one-line
status to update; it **does not save tokens** and the design does not claim
it does; it buys in-place updates, readable Mobile rendering and
typing-free, auditable decisions within measured bounds.

## 0. Implementation status

- **Slice 1 (Web only): the read-only inline carrier.** A ```` ```cowboy-widget ````
  fence in agent Markdown renders as a widget through `web/src/richFence.tsx`
  and `web/src/widgets/` (validator, MUI renderer for the 18 Phase 1 types,
  width-adaptive tables, fold within the height budget). It follows §6.4:
  an open fence streams as code, an invalid document keeps its source with a
  one-line notice, unknown types render their fallback, inputs render inert,
  and images show their `alt` (event-source resolution is not wired yet).
  Tested by `web/src/widgets/*.test.ts` and the `widgets` browser
  conformance suite (320/375/768/1100 px, light and dark).
- **Slice 2 (Controller + Web): agent-published widgets.** `widgets.publish`
  and `widgets.update` (replace, patch, close; revision CAS) run through the
  idempotent Agent API operation ledger with `widgets` receipts; `widgets.list`
  and `widgets.read` expose current state (`help("widgets")`). The
  `session_widgets` table stores each widget, and one notice row anchored by
  `widget:<id>` projects it into the transcript, replaced in place on every
  write; older clients render its plain-text description. Clear tombstones
  widgets with the transcript. The Web renders widget notices through the
  slice 1 renderer with the handle and a closed chip
  (`web/src/NoticeRow.tsx`, `web/src/widgets/widgetRows.ts`).
- Not built yet: interactions, the inspector, table sort and image sources.

## 1. Review findings and resolutions

### 1.1 Revision 1 (Astra Max, `83195d3b4`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | Agent-authored instructions acquire human authority (P1) | `prompt` carries no text or data; messages are composed from visible labels, the Controller-assigned widget handle and the revision; no agent-chosen identifier appears; product-owned exact-message preview on every sending surface | §8.1–8.2, §10 |
| 2 | Queued decisions lose the revision and target (P1) | Immutable interaction records; the decided revision stays readable (pinned) and the message names it | §6.3, §8.2–8.3 |
| 3 | Stable ids do not give exactly-once admission over a write-behind queue (P1) | Durable `widget_interactions` ledger with write-ahead dispatch marks, echo-based delivery proof and explicit `uncertain` | §8.3 |
| 4 | Start-up repair misses committed updates (P1) | `state_version` + acknowledged `published_version` watermark; republishing sweep | §6.3 |
| 5 | Repair can resurrect cleared content (P1) | Transcript generation fence checked at publication, persistence and dispatch; Clear tombstones and cancels | §6.3, §8.3 |
| 6 | Artifact retention does not cover the inline carrier (P1) | Semantic reference extraction into `widget_artifact_refs` on every write; inline carrier limited to `event` sources | §5.4, §6.3, §6.4 |
| 7 | Document revision cannot order snapshots (P2) | `state_version` | §6.2 |
| 8 | Closed widgets reopenable; open-only bootstrap (P2) | `reopen` removed; bootstrap covers closed widgets; W5 defines what still changes after close | §6.2, §6.6 |
| 9 | Pre-widget clients ignore unknown kinds (P2) | Row carried as a `notice` with `_cowboy.widget`; limited compatibility promise stated | §6.2 |
| 10 | Unknown-node acceptance vs closed union (P2) | Controller fails closed; Web parses into an explicit `unsupported` node | §5.3 |
| 11 | Duplicate keys collapse at MCP ingress (P2) | Guarantee narrowed to the bytes each boundary receives | §5.3 |
| 12 | Full read exceeds the 16 KiB budget (P2) | Windowed document reads, paged interactions | §6.1 |
| 13 | Controls vs Conversation keyboard ownership (P2) | Entered region | §7.5 |
| 14 | Layout hints defeat narrow panes (P2) | Hints subordinate to measured width | §7.2 |
| 15 | Limits do not bound rendering work (P2) | Aggregate budgets, previews, worst-case fixtures | §5.3, §7.1, §13 |
| 16 | Phase 1 contradicted the token goal (P2) | Goal rewritten; same-language, full-envelope, per-prefix measurement | §3, §11 |

### 1.2 Revision 2 (Astra Max, `854b7f48f`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| F1 | The relay still permits lost or repeated delivery; the queue already dedups cmids but is not a ledger (P1) | Ledger states `admitted → dispatched → delivered`, with `queued` derived from the queue, a write-ahead `dispatched` mark before the prompt is sent, delivery proven by the message echo persisted with its cmid, `uncertain` when a dispatch cannot be proven, recovery that re-enqueues `admitted` (the queue dedups `widget:` cmids once `tracks_delivery` knows the prefix) and never replays `dispatched` | §8.3 |
| F2 | Clear has no fence against in-flight publication or dispatch; Clear does not clear the queue (P1) | `sessions.transcript_generation`, advanced by Clear in the Hub and carried in `ClearEvents`; every widget row, snapshot, interaction and queued widget prompt carries its generation; Hub reduction, store reduction and dispatch drop or cancel anything from an older generation; Clear removes queued widget prompts | §6.3, §8.3, W9 |
| F3 | Naming a revision does not preserve the target approved at that revision (P1) | Revisions a decision was made on are pinned in `widget_revisions`; `widgets.read({revision})` returns them; the message names the revision; help tells the agent a moved revision never authorises substitution | §6.1, §6.3, §8.2 |
| F4 | Retention changes do not cover the extraction path; queries filter on string prefix; `event` sources re-enter on patch (P1) | No text scanning: references are extracted semantically from the validated document on publish, replace and patch into `widget_artifact_refs`, which the collector unions as a root; `event` sources are normalised on every write; the inline carrier may use only `event` sources | §5.4, §6.3, §6.4 |
| F5 | Publication acknowledgement needs an explicit version (P2) | `published_version = max(published_version, acknowledged snapshot version)` after the durable write, under the generation fence; W4 promises eventual publication of the latest version | §6.3, W4 |
| F6 | Interactions can exceed read and lookup budgets (P2) | Selection ≤ 12 options, labels ≤ 200 chars, records ≤ 4 KiB by construction; snapshots carry 5 summaries; records are paged with at least one per page; snapshots ≤ 28 KiB so a 256 KiB lookup page always progresses; `groups` ≤ 32 | §5.3, §6.1, §6.5 |
| F7 | Reconciliation lacks deletion and placement information (P2) | Per-id lookup outcomes `current`/`cleared`/`unknown`, epoch and generation on every response, anchor `seq` in snapshots and entries, continuation cursor, absence in a complete bootstrap means removal | §6.5 |
| F8 | Immutable closure contradicts pending delivery updates (P2) | Close freezes the document and forbids new admissions; admitted interactions still settle and advance `state_version` | §6.2, W5 |
| F9 | Queue editing breaks the immutable interaction/message association (P2) | Widget prompts in the queue cannot be edited, only deleted (`cancelled`) | §8.4 |
| F10 | Decision-reset semantics undefined; implicit groups lack identity (P2) | Group identity (explicit `group` or the card `id`, required when a card has actions); a group resets only when its interaction signature changes; a pending selection is invalidated by the same rule | §8.4 |
| F11 | Wire types cannot represent promised outcomes (P2) | `cleared` on update; list and read arguments and results; nullable `seq`; discriminated action results; refusals stateless; `open`/`copy` never admitted; selection keys and cardinality | contract, §6.1, §8.3 |
| F12 | Identifier syntax is not a visible-intent guarantee (P2) | Messages contain no agent-chosen identifier at all: the Controller handle replaces the widget id, labels replace node ids and option values; the exact message is rendered next to every Send control before sending; hostile-identifier vectors in the test | §8.1–8.2, §10 |
| F13 | The image bound limits geometry, not decoding (P2) | `data` images are still PNG/JPEG/WebP with intrinsic size ≤ 256 × 256 verified by the Controller before storing; SVG deferred; the inline carrier cannot carry `data` images | §5.4 |
| F14 | The token comparison overstates savings (P2) | Full envelopes measured: an update costs 2.2× a one-line status and 0.9× a re-sent report; break-even stated per prefix; the gate stops the design on failure instead of narrowing | §11 |
| F15 | Validator parity lacks precise units and vectors; byte boundaries misdescribed (P2) | Units fixed (bytes vs scalar chars, node vs JSON depth, safe numbers); 1 MiB raw frame vs 32 KiB re-serialized arguments; the example checker counts nested items and per-type limits; shared Unicode, numeric, recursive-budget and patch-sequencing vectors | §5.3, §13 |

### 1.3 Revision 3 (Astra Max, `a17877541`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| R1 | The transcript echo is not delivery proof: it is synthesized before the provider request and for cancelled held prompts (P1) | Evidence is the turn the dispatched prompt started: `delivered` only when that turn ends with a non-cancel stop reason or produced agent output, attributed through the Hub's dispatched-prompt bookkeeping; the echo is never evidence; everything else stays `uncertain`; a worker-side `prompt_accepted` signal is a Phase 2 Machine change | §8.3 |
| R2 | Recovery can dispatch a restored queue copy of an already dispatched interaction; the writer can exhaust retries and still persist a mark (P1) | A commit-acknowledged store operation with `committed`/`failed`/`unknown`; handoff only after a successful `admitted → dispatched` CAS under the generation; restored queue entries reconciled against the ledger before drain is enabled; content hash verified at handoff; `unknown` never hands off | §6.3, §8.3 |
| R3 | Clear has no durable serialization point; a publisher can commit against the old generation; a crash loses Clear (P1) | Clear commits first (generation bump, tombstones, cancellations, event deletion in one awaited transaction) and only then clears memory and acknowledges; every widget write, admission and handoff is a conditional update on the session's generation row, so the database serializes them with Clear; handed-off work at Clear becomes `uncertain`, never proven `cancelled` | §6.3, §8.3, W6, W10 |
| R4 | Pinned revisions do not retain their artifacts; inline `artifact` links have no root (P1) | `widget_artifact_refs` is keyed by revision and retains the union of the current and every pinned revision; the inline carrier may not reference artifacts at all (sources or links) | §5.4, §6.3, §6.4 |
| R5 | Choice messages discard the question being answered (P1) | A choice `label` is required, non-empty and unique among the widget's interactive labels, and the composed message carries the question with the selected answers | §5.3, §8.2 |
| R6 | Record and snapshot byte bounds were false; `interaction_id` unbounded (P2) | A per-node worst-case message bound (2 KiB) validated at publish; record ≤ 6 KiB by construction and re-checked at admission; snapshot ≤ 32 KiB; `interaction_id` follows the id pattern; lookup pages fit at least eight snapshots | §5.3, §6.5 |
| R7 | Send-again cannot pass admission (P2) | `widget_action_retry` copies the original pinned intent, records `retry_of`, bypasses revision/closure/group checks, is refused only when cleared, not retryable or already retried; late proof marks the retry a duplicate | §8.3 |
| R8 | Queue-to-draft preserves the cmid and reopens editing (P2) | Delete-only enforced across queue edit, queue-to-draft and resubmission for `widget:` cmids; the dispatcher verifies the queued content against the ledger's message hash | §8.4 |
| R9 | Reconciliation misses widgets absent from the cache (P2) | Bootstrap paginates to a complete index; the client reconciles entries that are missing locally or stale | §6.5 |
| R10 | The interaction signature omitted selection mode (P2) | `interactionSignature` in the contract is canonical: ids, type, label, disabled, `multiple`, effective `max_selected`, option values and labels | §8.4 |
| R11 | Prompt nodes could omit `id` (P2) | `id` is required on every button and choice | §5.3 |
| R12 | State changes could prevent reading an immutable pinned revision (P2) | Pinned reads are bound to the document's `sha256`; only current-document reads restart, and only on `revision` change | §6.1 |
| R13 | Token prefix arithmetic was wrong; tests did not enforce thresholds (P2) | Prefix totals recomputed from complete exchanges; the contract test measures publish and update envelopes with results against the same baselines the gates use | §11 |
| R14 | Field-level domains, units and serialization unspecified (P2) | Units and integer domains fixed in the contract comments and §5.3; uniqueness scopes and serialization rules stated; the checker enforces option-label length, empty labels, message bounds and ids on actionable nodes | §5.3 |

### 1.4 Revision 4 (Astra Max, `251963f05`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| N1 | Delivery attribution unspecified: `dispatched_cmids` records only cmid, dispatch epoch and echo status; the remote runtime discards turn identity (P1) | The runtime already mints `command_id` and `turn_id` per dispatch and the worker reports `TurnStarted`/`TurnEnded` in frames carrying `worker_epoch`; the Controller stops discarding them (`supervisor.rs`, `apply_event`, the `TurnEnded` projection), records the attempt `(command_id, turn_id)` in the dispatch CAS, binds the turn to its worker epoch, persists the turn reference on the `TurnEnd` envelope, and accepts as evidence only a `TurnEnd` carrying that pair with a completing stop reason; outputs, acks, Busy and the echo are never evidence; attempts, lifetimes and restart rules are stated | §8.3 |
| N2 | An unknown Clear commit has no recovery protocol (P1) | Clear is an operation with a stable id and `expected_generation`, applied as a conditional update; on `unknown` the Hub reads the durable generation to learn whether it applied, then reconciles memory, before any effect reopens | §6.3 |
| N3 | The awaited Clear can overtake buffered `AppendEvent` writes (P1) | Clear travels through the store writer as an acknowledged, in-order barrier (the writer flushes the batch first, as today); every envelope carries the session generation and the writer drops older-generation events and anchors | §6.3 |
| N4 | Cancellation versus dispatch is not arbitrated (P1) | A durable `cancel_requested` intent is recorded (acknowledged) before the queue is touched; the dispatch CAS requires it to be false; cancellation is promised only after its commit; a prompt whose intent never became durable may still dispatch, and the matrix says so | §8.3 |
| N5 | Artifact pinning races collection (P1) | Pin admission refreshes the artifact's age before the transaction commits (the `put_bytes` rule), so the collector's 24 h grace protects the in-flight reference exactly as it protects an event's; the collector re-reads `widget_artifact_refs` in the same sweep that enumerates roots | §6.3 |
| N6 | The turn Retry path (`cowboy-retry:` cmid) bypasses the widget ledger (P2) | `retry_turn` for a last prompt whose cmid is `widget:` becomes `widget_action_retry` with the original interaction identity | §8.3, §8.4 |
| N7 | Retry admission incomplete; W5 contradicts the retry exception (P2) | Idempotent on `retry_interaction_id`; complete refusal set; chain of one; links projected in summaries; W5 amended | §8.3, W5, contract |
| N8 | Worst-case message checker ranks by raw UTF-8; metadata unbounded (P2) | `worstCaseMessageBytes` ranks by serialized contribution (raw plus escaped); labels are one printable line; `reason` ≤ 200 chars, handle ≤ 16; the quote/emoji vector is a test | §5.3, contract |
| N9 | Read and lookup pages not packed against the encoded response (P2) | Pages are packed to `readPageBytes` on the complete encoded response with actual continuation offsets; `pinned_revisions` leaves `WidgetMeta` for a paged `revisions` selection | §6.1, §6.5 |
| N10 | Selection domains ambiguous (P2) | Unique option values, valid initial value, unique submitted selections, single-choice cardinality one in the signature, one-line printable labels | §5.3, contract |
| N11 | `PublishResult` lacks the generation-CAS refusal (P2) | `cleared` added with retry semantics | §6.1, contract |
| N12 | The update gate passed against an inflated baseline (P2) | Baselines separated (108-byte sentence; card plus sentence); the update bound is a constant (≤ 512 B for three ops) and the document states that an update costs more than both for a report this small | §11 |
| N13 | Controller acceptance of unknown types with `fallback` contradicted the closed union (P2) | The Controller rejects every unknown type; `unsupported` parsing is reserved for a newer document meeting an older Web client | §5.3 |

### 1.5 Revision 5 (Astra Max, `4ce4140df`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| G1 | Reconnect retransmits pending commands and worker dedupe is per process, so a replacement worker can execute a widget prompt again without a dispatch CAS; a rejected replay does not prove the original unsent (P1) | A widget command is written to exactly one connection and never retransmitted: on reconnect the runtime drops it from `pending` and the attempt becomes `uncertain` unless the old epoch's replayed `TurnEnded` proves it; one epoch per attempt is therefore exact | §8.3 |
| G2 | Treating a startup withdrawal as `returned → admitted` would make the relay re-send a decision the person stopped (P1) | A withdrawal caused by Stop settles the record `cancelled (stopped)` — proven unsent, never relayed; `returned` is reserved for prompts the runtime itself gives back and requeues (rejected first ack, `UndeliveredPrompt`) | §8.3 |
| G3 | Refreshing the artifact's age does not close the collection race: the collector reads an old mtime, pauses, and unlinks after the pin commits (P1) | Pin admission (verify, refresh, commit) and the collector's final age check plus unlink share one in-process lock; the paused-after-metadata vector is a fixture | §6.3 |
| G4 | `LABEL_PATTERN` reported with doubled backslashes (P2) | The committed source is `/^[^\p{Cc}\p{Zl}\p{Zp}\p{Cf}]+$/u` with a single escape (the examples pass); line and paragraph separators and format characters are now excluded too, with positive and negative label tests | §5.3, contract |
| G5 | Late delivery proof is discarded once an attempt is `uncertain`; `retried_by` missing from summaries (P2) | Attempt identities stay searchable; a late `TurnEnd(T, E)` moves `uncertain → delivered` and marks an existing retry `duplicate_of`, serialized through the record's conditional update; summaries carry both links | §8.3, contract |
| G6 | A crash between `cancel_requested` and `cancelled` strands an admitted record (P2) | Startup reconciliation settles every admitted record with a durable intent: queue copies removed, `cancelled (removed)`, snapshot published, before drain is enabled | §8.3 |
| G7 | The 6 KiB record bound fails with five attempts; `stop_reason` unbounded (P2) | `worstCaseRecordBytes` reserves every bounded field at maximum (message, selected labels, five attempts, reason, links) and is validated at publish against 8 KiB (10 KiB since revision 8); `stop_reason` ≤ 64 chars | §5.3, contract |
| G8 | `widgets.list` not packed (P2) | List pages are packed against the complete encoded response with a continuation cursor after the last included entry | §6.1 |
| G9 | 512 B is an example budget, not a three-op bound; §3 contradicted §11 (P2) | The bound is scoped to a benchmark update class (≤ 3 ops, paths and values ≤ 64 chars); general patches are bounded by the patch and document limits; §3 and §11 now state the same thing | §3, §11 |
| G10 | The checker let a single choice carry `max_selected: 13` (P3) | The raw field is validated 1..12 independently of the effective cardinality; vector added | §5.3, contract |

### 1.6 Revision 6 (Astra Max, `b1827e8b1`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| H1 | Dropping a written widget command from `pending` also drops the protection `pending_prompt_for` gives `idle_snapshot_guard`: an idle snapshot could release the dispatch guard while the command is still travelling, two prompts overlap, and the worker's mutable `current_turn_id` lets the first prompt's `TurnEnded` name the second (P1) | Unresolved attempts keep dispatch ownership: the runtime consults them wherever it consults `pending`, so idle reconciliation cannot release the guard until the attempt settles or its worker is revoked. Independently, interactive widgets require a worker generation whose `TurnEnded` carries the prompt task's own turn id (`widget_turn_identity`): the capability becomes a Phase 1 prerequisite with a Machine release, and a session on an older generation renders prompt controls inert with a notice | §8.3, §12 |
| H2 | The record contract still sent startup withdrawals back to the relay (P2) | `InteractionRecord.state` distinguishes runtime returns (`returned → admitted`) from the person's Stop (`cancelled (stopped)`) | contract |
| H3 | `worstCaseRecordBytes` was not an upper bound: ASCII reserved for metadata, single-escape ranking for labels (P2) | Metadata strings are printable ASCII by contract; `recordContribution` ranks labels by their exact serialized contribution (escaped once in `selected`, once inside the message string, twice inside its JSON line); every counter at `maxCounter`; the test constructs every permitted selection independently and checks none exceeds the estimate | §5.3, contract |
| H4 | A maximal document plus 32 decided groups and five linked summaries exceeds 32 KiB (P2) | `worstCaseSnapshotBytes` reserves every metadata field at its maximum around the document at publish; `snapshotBytes` is 36 KiB and a lookup page fits seven | §5.3, §6.5, contract |
| H5 | Bootstrap absence was not ordered against concurrent creation (P2) | A per-session `index_version` advances on every creation and tombstone; bootstrap pages share one; snapshots carry the creator's; absence removes only cached widgets created at or before the index's version | §6.5, contract |
| H6 | The benchmark class maximum exceeds 512 B (P2) | The class is defined with its identifier maxima and its computed maximum is bounded at 1 KiB; `progressPatch` is the measured 438 B instance; §3 and §11 say so | §3, §11, contract |

Also fixed: the stale "≤ 6 KiB" interaction read claim (§6.1) and explicit
wire domains for counters and timestamps (`maxCounter`, `maxTimestampMs`).

### 1.7 Revision 7 (Astra Max, `95123ed3c`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | `uncertain` released ownership while delivery remained possible; "no worker + disconnected" does not revoke broker-held commands; `complete_turn` ignores identity (P1) | Transport **ownership** is a separate per-attempt state from delivery: held from the CAS until a terminal transport event — the attempt's `TurnEnd`, a proven-unsent return or Stop, or the broker's acknowledged `CommandRevoked` (dropped undelivered → the attempt is `returned`; delivered to an epoch that has exited → `uncertain`); an attempt can be `uncertain` for delivery and still hold ownership; while any attempt is held the Hub dispatches nothing and an idle snapshot releases nothing; `complete_turn` releases the guard only for the dispatched turn id | §8.3 step 5, contract |
| 2 | The capability prerequisite did not cover retry or the receiving worker; "stay untouched" contradicted the Machine release (P2) | Fail-closed capability discovery from the Machine handshake; `widget_action`, `widget_action_retry` and the handoff all refuse `capability_missing`; widget commands carry `requires: ["widget_delivery_v1"]` and a broker or worker without it rejects them (`returned`); a generation fallback revokes open attempts; the worker-generation files change in the Machine release and the sentence says so | §8.3, §12, contract |
| 3 | Printable ASCII still escapes (`"`, `\`), so the record estimate was not an upper bound (P2) | Metadata strings use an alphabet with no escaped character (`METADATA_PATTERN`); runtime text is normalized (`normalizeMetadata`) before storage; the independent record test uses adversarial metadata through the normalizer | §5.3, contract |
| 4 | Capability changes had no ordered snapshot update (P2) | `interactive` leaves the widget snapshot; the session-level `cowboy_delivery_capabilities` projection (already used for Steer targets) carries `widget_delivery`, and the renderer combines it with every widget | §6.2, §8.3, contract |
| 5 | The index version had no durable storage or transaction contract (P2) | `sessions.widget_index_version` and `session_widgets.index_version` are written atomically with creation and tombstoning; the bootstrap reads counter and entries in one transaction and filters pages by `index_version ≤ watermark` | §6.3, §6.5 |
| 6 | The benchmark class maximum was computed with unescaped single-byte values (P2) | The class is defined by JSON-escaped UTF-8 bytes (paths and values ≤ 64 bytes escaped) and its maximum, built with quote and emoji values, is bounded at 1 KiB | §3, §11, contract |

### 1.8 Revision 8 (Astra Max, `1cfbd4e6d`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | A `TurnEnded` acknowledged before its ledger transition commits can, after a crash, leave an attempt held forever while the surviving worker is idle (P1) | Durable terminal settlement before the cumulative runtime acknowledgement is mandatory in Phase 1 for tracked `TurnEnded`; independently, the broker's durable command-fate ledger keeps a `terminal` receipt per command, so a held attempt is also settled from that receipt on recovery | §8.3 step 5–6 |
| 2 | The revocation fence had no answer for commands never observed or forgotten across a broker restart; the broker removes a pending command before sending it (P1) | An idempotent **command-fate protocol**: the broker keeps a durable per-command ledger (`held`, `delivered {epoch}` recorded under the routing lock *before* the write to the worker channel, `terminal {epoch, turn_id, stop_reason}`, `rejected`); `RevokeCommand` installs a `rejected` tombstone for a held or never-observed command so a later copy is dropped on arrival; `unknown_history` is a defined answer that is never proven unsent and releases ownership only once the worker shows no active or pending turn for the command | §8.3 step 5 |
| 3 | Direct API prompts (`Supervisor::send` → `prompt_traced`) bypass the ownership gate (P2) | Ownership is enforced at the single runtime admission boundary every prompt producer passes through (`prompt_traced`), which refuses `widget_attempt_held` while any attempt holds ownership; the API maps it to a retryable conflict | §8.3 step 5 |
| 4 | `complete_turn` stayed identity-blind for ordinary turns, so a late widget completion could release an ordinary turn's guard (P2) | Every completion is matched against the active guard's `(worker_epoch, turn_id)`; an unmatched completion updates ledgers and releases nothing; the contradictory exception is removed | §8.3 step 5 |
| 5 | A `requires` field cannot make an older peer reject a command (P2) | The negotiated broker capability is re-checked immediately before every actual write, including a retained unwritten command after reconnect, and an unsupported peer makes the attempt a local proven-unsent return; guarded prompts travel in a new frame variant older peers cannot decode | §8.3 step 5 |
| 6 | Retry admission raced the revocation's automatic requeue (P2) | `in_flight` for retry includes held ownership; a broker `rejected` answer for an original whose retry already exists settles the original `cancelled (superseded)` instead of requeueing it; §8.6 lists held ownership | §8.3 step 7, §8.6 |

### 1.9 Revision 9 (Astra Max, `e9d0dabfd`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | `unknown_history` plus an idle snapshot does not fence future execution; the broker's snapshot omits commands already in the worker channel (P1) | A fate answer is produced only after a durable prohibition on future routing (broker tombstone) **and** a worker fence ordered after every earlier channel write (`FenceCommand` → `CommandFenced` on the same FIFO channel, installing a receipt tombstone when `never_received`); an observational snapshot is never used; every `CommandFate` except `running` is terminal for ownership | §8.3 step 5 |
| 2 | `delivered` had non-terminating recovery paths: broker commit then crash before the write with a surviving idle worker; lost negative acks; salvage without command identity (P1) | The worker keeps a durable per-command journal (`received` before ACP, `finished` after the `session/prompt` response) and answers fences from it, so a surviving idle worker answers `never_received` (fenced) or `finished`; the broker records negative acks before forwarding them; `UndeliveredPrompt` carries the real command id; `received_exited` covers an epoch that died mid-turn; no recovery path waits for an incidental exit | §8.3 step 5, contract |
| 3 | Clear made required terminal settlement impossible under the generation fence; the 30-day purge could remove unresolved ownership evidence (P1) | Transport settlement writes (attempt ownership, outcome, delivery state) are identity-checked, not generation-checked, and never republish or requeue old-generation content; a return on a cleared interaction settles `cancelled (cleared)`; unresolved transport records are exempt from purge until settled | §6.3, §8.3, W10 |
| 4 | The single admission gate would refuse the very attempt that acquired ownership (P2) | The dispatch CAS yields a single-use, attempt-bound **handoff permit** that only the owning command can consume; `prompt_traced` admits a prompt only with the permit matching the session's held attempt and refuses every other producer; the reservation is serialized with ordinary admission under the runtime's session lock and refusals are typed through the Supervisor | §8.3 step 4–5 |
| 5 | Identity-matched completion still had contradictory exceptions (P2) | One rule everywhere: a completion releases the active guard only when its `(worker_epoch, turn_id)` equals the guard's; the wire already carries turn ids, so no legacy exception exists; an unmatched completion updates ledgers only | §8.3 step 5–6 |
| 6 | The Web could not observe held ownership (P2) | Snapshots carry `outstanding`: every unsettled interaction with its state, ownership and `retryable`, bounded by one per interactive node (≤ 32) and reserved in the snapshot estimate; ownership changes advance `stateVersion` | §8.6, contract |
| 7 | Sections still described the superseded `CommandRevoked {dropped, delivered_to}` protocol; no outcome for a never-observed command (P2) | One protocol throughout (`QueryCommandFate`, `RevokeCommand`, `FenceCommand`, `CommandFenced`, `CommandFate`, `GuardedPrompt`), each message carrying session and command ids; a never-observed command is `rejected` once tombstoned at the broker and fenced at the worker | §8.3, §12, §13, contract |

### 1.10 Revision 10 (Astra Max, `ac5a07584`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | `QueryCommandFate` was a read yet was expected to yield the terminal `rejected`, which needs the routing tombstone (P1) | Reads never release ownership unless they return a terminal receipt (`finished`, `aborted`); `rejected` is only ever the answer to an acknowledged `RevokeCommand`; recovery that needs a terminal fate revokes | §8.3 step 5, contract |
| 2 | Fencing the current worker did not prove that every earlier recipient epoch had stopped; a disconnected epoch can survive and reconnect while the broker projects `Crashed` (P1) | The broker keeps a durable **recipient inventory** per command (`routed {epoch}` recorded before each write); revocation fences every inventoried epoch that is still connected on its own channel and requires a confirmed process-tree termination proof (the existing `tree_exited` receipts, which the bundle requires via tree-contained launches) for every epoch that is neither connected nor finished; until then it answers `pending_epochs` and ownership stays held | §8.3 step 5, §12 |
| 3 | A fallback to a worker or broker without the bundle could strand held attempts (P2) | A smaller recovery-reader capability (`widget_delivery_recovery_v1`: answer `RevokeCommand`/`FenceCommand`, report termination proofs) is retained by every build from the bundle's introduction onward, and a cutover or rollback below it is refused for a Machine while any attempt is held | §8.3 step 5, §12 |
| 4 | The journal defined no terminal disposition for exits before the provider response; the matrix used `received` for the broker's pre-write record (P2) | The worker journals exactly one disposition after `received`: `finished` or `aborted {reason}` (cancelled before start, configuration failure, ACP channel failure); the broker's pre-write record is `routed`; the matrix is corrected | §8.3 step 5, contract |
| 5 | Settled-attempt idempotency discarded permitted late proof (P2) | Ownership release is idempotent; completing evidence still upgrades `uncertain → delivered` and marks an existing retry `duplicate_of`, through the retry-admission conditional update | §8.3 step 5–6 |
| 6 | `outstanding` membership conflicted with retry admission and the per-node bound (P2) | Exact membership predicate: `admitted`/`dispatched` records plus retryable `uncertain`/`failed` ones; a record leaves when it settles, when its retry is admitted or when a new decision on a reset group supersedes it; `retryable` is false for records with `retry_of` or `retried_by`; one entry per node by construction | §8.6, contract |
| 7 | Estimators reserved the shorter boolean and state forms; fixtures only re-ran the estimator (P3) | Estimators reserve the longest valid alternatives; the tests construct records and snapshots independently across every boolean and state value and compare them with the estimate | contract, tests |

### 1.11 Revision 11 (Astra Max, `d73b8fc7f`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| F1 | One recipient's terminal receipt could release ownership while another recipient remained active (P1) | Delivery evidence and ownership settlement are separated: a receipt is evidence; ownership is released only when **every** epoch in the attempt's recipient inventory is terminal (receipt, receipt tombstone installed before any receipt, or proven termination) — trivially immediate for the normal single recipient; conflicting or duplicate recipients keep the attempt held until all are terminal, and two `finished` receipts are recorded as `duplicate_execution` | §8.3 step 5, contract |
| F2 | `tree_exited` receipts are keyed by launch UUID, overwritten per launch, and skipped for default-account launches (P2) | The broker records an immutable `worker_epoch → launch (boot, containment)` binding at registration and retains an immutable copy of the launch's termination proof while any inventory references the epoch; the bundle requires contained launches for every capability-enabled session, default accounts included; an unavailable inventory keeps ownership held, releasable only by a Machine reset that proves all worker trees exited | §8.3 step 5, §12 |
| F3 | The rollback prohibition had no authoritative admission barrier (P2) | The component activator enforces a persistent compatibility floor: before activating a build below the recovery reader it asks the Controller, which is the authority on holds, for held attempts on that Machine and drains new reservations; an unreachable Controller refuses the rollback | §8.3 step 5, §12 |
| F4 | The recovery recipe still called the read a fence; no query result for an unobserved command (P2) | Recovery revokes; `QueryCommandFate` answers only observations (`unobserved`, `held`, `routed`, `received`, `running`, `receipts`) and `RevokeCommand` only settlements (`pending_epochs`, `rejected`, `settled`); the wire contract encodes the split | §8.3 step 5–6, contract |
| F5 | No disposition for a request that failed after submission without a response (P2) | The worker journals `submitted_failed {reason}` as the third disposition: terminal for the recipient, delivery `uncertain`, never an automatic return | §8.3 step 5, contract |
| F6 | `outstanding` excluded held-but-uncertain interactions (P2) | Membership includes every record with any held attempt, whatever its delivery state, until ownership settles | §8.6, contract |
| F7 | Retry eligibility contradicted supersession and the one-per-node guarantee; `retry_of` missing from the projection type (P2) | One predicate (`retryable`) shared by retry admission and projection; supersession is persisted (`superseded_by`) when a replacement decision is admitted on a reset group and a retry of a superseded intent is refused `superseded`; `OutstandingInteraction` carries `retry_of` and `superseded_by` with their byte reservation | §8.3 step 7, §8.6, contract |

### 1.12 Revision 12 (Astra Max, `cc9a80438`)

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| R1 | The single-recipient shortcut released ownership without proof that the inventory was complete and closed; step 6 still bound one epoch (P1) | No shortcut: a `TurnEnd` or `finished` is delivery evidence that triggers `RevokeCommand`; ownership is released only by a settlement whose inventory the broker **sealed** under its routing lock (`sealed: true`), the same barrier for returns; receipts are matched per inventoried recipient | §8.3 step 5–6, contract |
| R2 | Controller rollback bypassed the ownership protocol: an older Controller could dispatch from worker snapshots while widget work was queued or held (P1) | A Controller recovery-reader floor enforced by the Controller component activator (consult the database for held or admitted interactions, refuse rollback below the floor) plus a durable admission fence older Controllers already honour — the session's queue hold — set while any attempt is held; cold recovery of a floor-compliant Controller reconstructs holds from the ledger, never from worker snapshots alone | §8.3 step 5, §12 |
| R3 | Settlement dropped the abort cause needed to honour Stop (P2) | Recipient dispositions are discriminated payloads with a required `cause` on `aborted` and `turn_id`/`stop_reason` on `finished`, preserved from journal to attempt | §8.3, contract |
| R4 | Delivery reduction incomplete; duplicate detection disagreed between text and contract (P2) | One exhaustive reduction (`reduceSettlement`): completing `finished` decides delivery; every possibly executed recipient (`finished` any reason, `submitted_failed`, `terminated`) counts toward duplication; the remaining combinations are enumerated and tested | §8.3 'Settlement rules', contract |
| R5 | Supersession did not establish the one-per-node invariant for ungrouped nodes and could drop held records (P2) | Every interactive node without a `group` is its own implicit group; admitting a new decision on a node supersedes that node's released, retryable records; supersession only clears retryability and never removes a held record; `superseded_by` is in the schema sketch | §8.4, §8.6, §6.3 |
| R6 | The record limit no longer left room for read-response overhead (P2) | `readPageBytes` is a target; a page may exceed it only for a single item and never the 16 KiB cap; one record plus ≤ 2 KiB of metadata always fits; the remaining 10 KiB statements are corrected | §6.1, contract |

### 1.13 Revision 13 (Astra Max, `fb9cedf2e`) — open, Phase 1 freeze prerequisites

These are not resolved in this document. Each must be resolved and encoded in
transition or boundary fixtures before the interaction part of the Phase 1
contract is frozen; implementation of display widgets may proceed.

| # | Finding (severity) | Required resolution |
|---|---|---|
| F1 | Controller rollback admission misses held interactions: Clear can turn a held one `uncertain`; Pause still permits manual dispatch and `Supervisor::send → prompt_traced` bypasses the queue hold; additive migrations do not stop an older Controller from starting (P1) | Refuse lower Controller readers whenever an interaction is admitted/dispatched **or any attempt is held**, serialized with admission and enforced on automatic recovery and cold start; specify the exact durable hold key with atomic acquisition/release that every prompt producer of an older Controller honours (a queue hold alone is insufficient) |
| F2 | A sealed `rejected` loses a Controller-local Stop: Stop during configuration withdraws the command before the broker sees it, so no recipient reports `aborted {stopped}` and the reduction yields `returned` (P1) | Persist an attempt-bound Stop intent before withdrawal; apply it when reducing a sealed rejection so the record settles `cancelled (stopped)`; define recovery between the two commits; fix the crash-matrix row that still releases by `turn_end` |
| F3 | `submitted_failed` does not prove execution stopped: the provider may keep running after the response channel fails (P1) | Treat `submitted_failed` as delivery-uncertain but **not** ownership-terminal; keep ownership held until a response, an authoritative cancellation completion or a retained termination proof; add a live-provider/failed-channel vector |
| F4 | `rejected` carries no recipients, so a recovering Controller cannot compute `duplicate_execution` (P2) | Carry the sealed inventory on `rejected`; reduce the full settlement including unavailable history; align the `duplicate_execution` comment |
| F5 | Moving a node to another group bypasses supersession (P2) | Supersede by stable node identity as well as group; one effective-group resolver shared by validator and admission; group-move fixtures; the test validator must model implicit standalone groups |
| F6 | `submitted_failed {reason}` (≤ 200 chars) makes the maximal record 12,684 B, above the 12 KiB bound (P2) | Bound and normalize `reason`; compute the estimator maximum across every disposition variant and attempt history; test those variants independently |
| F7 | The 16 KiB read cap was applied to HTTP lookup pages that carry ≤ 48 KiB snapshots (P2) | Scope the 12/16 KiB target/cap to MCP list/read; keep `lookupBytes` for HTTP lookup; test encoded responses |

Rulings: approve 17, 18, 20, 21, 22, 23 (Controller write-once), 24, 26, 27,
30, 31, 32; request changes 19 (F5), 25 (F2), 28, 34 (F2/F3), 29, 33, 36
(F2–F4), 35, 37 (F1).

Rulings on decisions are folded into §14.

## 2. Evidence from history

The numbers come from a read-only copy of the production Controller database
taken 2026-11-08 (304 sessions, 219,120 events, 20,316 agent messages). The
queries are to be committed as `tools/widget-history-measure/` with Phase 1;
until then they are unverified by a second reader. They describe what agents
already try to express, not what a widget should look like.

### 2.1 Report shapes agents produce today

| Shape | Agent messages | Sessions | Notes |
|---|---|---|---|
| Bold key/value lines (`**key**: value`) | 3,034 | 118 | the most common structured shape |
| Numbered or bulleted lists | 4,561 / 1,855 | 145 / 111 | plans, steps, options |
| Markdown table | 1,814 (2,112 tables) | 107 | avg 1,439 chars; 60% have 5–8 rows; 83% have 2–3 columns |
| Fenced code | 894 | 58 | commands, config, logs |
| `file.ext:line` references | 666 | 110 | navigation targets |
| Question to the user | 1,609 | 95 | mostly free text; 260 elicitation forms in total |
| Mermaid diagram | 22 | 10 | rare |
| Inline image link | 6 | 6 | versus 1,087 tool image blocks and 1,239 user screenshots |

Agent messages are short (p50 105 chars, p90 1,131, p99 2,962). Re-encoding
the 2,112 tables as the v1 `table` node (column labels plus row arrays,
compact JSON) is **7% larger at the median** (p10 +3%, p90 +14%): a table
costs the same to publish either way.

**Repeated reports.** 203 of the 2,112 tables (10%) repeat a header that
already appeared earlier in the same session, in 27 sessions, one header up
to seven times. That is the measurable floor of "the same report sent
again"; status prose without a table was not measured. 4,529 agent messages
mention progress or completion words, which shows status is reported often,
not that the same report is re-sent.

### 2.2 What users ask for

Of 5,884 human messages: review 349, UX/UI 335, "continue" 397, status or
progress 206, table 138, summary/report 138, compare 128, analysis 110,
deploy 109, screenshot 72, disk/memory/CPU 51. Status and comparison are the
recurring reading tasks; approval ("要不要我…") is the recurring decision.

### 2.3 Mechanisms the design can reuse

- Cowboy MCP is already used by agents: 855 `cowboy_read` and 231
  `cowboy_execute` calls in 48 sessions. Help is loaded per topic, results
  are bounded (16 KiB) and writes carry a stable operation id with a receipt.
- Cowboy already owns transcript rows that are replaced in place:
  `cowboy_subagent` and `cowboy_call_anchor` (durable `transcript_anchors`,
  first anchor wins, both reducers resolve through one index).
- The session queue already refuses a duplicate `cmid` for prompts it has
  accepted, queued, dispatched or echoed (`prompt_already_accepted`), and a
  user message's echo persists its `cmid` in `events`.
- The remote runtime already mints a `command_id` and a `turn_id` for every
  dispatched prompt (`RemoteRuntime::prompt_traced`), and the worker reports
  `TurnStarted {turn_id, command_id}` and `TurnEnded {turn_id, stop_reason}`
  in frames stamped with its `worker_epoch`; Steer and CancelTurn already
  address a turn by `(worker_epoch, turn_id)`. The Controller merely drops
  these identities before the Hub.
- Images already externalize to the content-addressed artifact store
  (`/api/artifacts/<sha>.<ext>`) and render through `ProtectedImage` with a
  lightbox; Calls return verified artifact paths rather than bytes.
- Elicitation (`_cowboy.elicitation`) proves a typed form can be drawn by the
  Web and answered as structured content; it is blocking and one-shot.
- Draft documents prove the CAS-with-receipt write model and the per-owner
  transaction that admits, writes and records in one step.

## 3. Goals and non-goals

Goals, in priority order:

1. **Read better on every surface.** A status table, comparison, key/value
   summary, progress list or image must be legible at 320 CSS px and dense
   on Desktop without the agent describing layout.
2. **Bounded, honest token overhead.** Publishing a widget costs more than a
   terse Markdown reply, updating it costs more than a one-line status for
   any report smaller than about 330 bytes, and this design claims no net
   token saving (§11). What it claims is a bound: publication ≤ 4.5× the
   same-language report; a benchmark update (≤ 3 `set` ops whose paths and
   string values are ≤ 64 bytes in JSON-escaped UTF-8, a widget id ≤ 64 and
   an operation id ≤ 128 chars) ≤ 1 KiB of envelope and result at the class
   maximum, with general
   patches bounded only by the patch and document limits; a decision message
   ≤ 2 KiB and fixed in shape. The bounds are measured on complete exchanges with every envelope
   counted, per prefix, and the measurement is a ship gate that stops the
   design if it fails.
3. **Act without typing, without delegating authority.** A widget can offer
   the next decision as buttons or choices; the answer reaches the agent as
   an ordinary, auditable user message that contains only what the person
   saw and chose.
4. **Extend without a new transport.** New component types are additive; an
   unknown node degrades to its fallback; an older client still gets a
   readable line.
5. **Keep the Cowboy trust rule.** Agent content is data; the renderer owns
   all behaviour; a tap never grants the agent more than the person doing
   the same action by hand.

Non-goals:

- Agent-authored React, HTML, CSS or scripts. They would have to be
  evaluated, which `tool-results.md` forbids, and they cost more tokens than
  the data they decorate.
- A second transcript, a dashboard product or a replacement for the
  Markdown renderer. Prose stays Markdown; widgets are for structured
  content that will be updated or answered.
- Replacing elicitation or permission prompts. A blocking question keeps
  AskUserQuestion / `request_user_input`; a widget choice is non-blocking
  and persistent, and persistent forms are not in this contract.
- Live data bindings that make the Web poll Cowboy reads on the agent's
  behalf.

## 4. Format decision: a typed JSON document, not agent-written React

| Option | Verdict | Why |
|---|---|---|
| Agent writes React/TSX (or any code) | rejected | must be evaluated in the client; breaks the display-as-data rule and the plugin model (no plugin code runs in the Web); the largest token cost per rendered pixel |
| Agent writes HTML/CSS | rejected | same trust problem, no MUI semantics, no width adaptation |
| Markdown extensions only (fences, directives) | insufficient alone | updateability, receipts and validated actions come from the transport, not from the shape; kept as the read-only **inline carrier** (§6.4) because it costs no tool round trip |
| **JSON component tree, TypeScript contract, Cowboy renderer** | **chosen** | validated at both boundaries; versioned; each type renders with product MUI components and adapts to its width; patchable; the same model as elicitation forms and the Provider UI IR |

"React and TS" therefore means: the **contract is TypeScript**
(`contracts/widget-v1.ts`, the source of truth, with a derived JSON Schema
for non-TS consumers), the **renderer is React + MUI** owned by Cowboy, and
the **agent emits JSON**. Discovery is on demand:
`cowboy_read({help:"widgets"})` returns the contract as TypeScript the same
way other topics do, within the existing per-topic budget test.

## 5. Document model

### 5.1 Shape

```ts
WidgetDocument { schema: "cowboy.widget/1"; title; summary?; icon?; root: Node;
                 presentation?: { inline?: boolean; height?: "xs"|"sm"|"md"|"lg" } }
Node = LayoutNode | ContentNode | StatusNode | MediaNode | InputNode   // 18 types in Phase 1
```

Every node may carry `id` (required when a patch, a group or a value refers
to it, and on a card with `actions`) and `fallback` (Markdown shown by a
client that does not know the type, and by text-only surfaces). Containers
hold `children`. There are no style, size or breakpoint properties: each
component owns its behaviour at the width it is given (§7.2), which keeps
documents short and rendering decisions in the product.

### 5.2 Phase 1 catalog

| Group | Type | Covers (§2) | Narrow width | Wide width | Reuses |
|---|---|---|---|---|---|
| Layout | `stack` | grouping | row → column unless every child keeps ≥ 120 px | row/column, wrap | MUI `Stack` |
| | `card` | one topic with status and actions | full width, actions wrap | dense card | `Paper`, `activityRowSurface` |
| | `section` | progressive disclosure | collapsed state paint-only | same | `Collapsible` |
| | `divider` | | | | |
| Content | `text` | a line with tone; inline Markdown subset | | | `Markdown` inline |
| | `kv` | 3,034 bold key/value messages | one column | two columns when ≥ 560 px | `MetricFacts` grid |
| | `list` | 4,561 bullet / 1,855 numbered messages | tones, icons, nesting | same | MUI `List` |
| | `table` | 1,814 table messages | fit when it keeps font and target minimums, else one card per row | sticky header, sort, min/max widths, 20-row preview | `markdownTableFit`, `Collapsible` |
| | `link` | 177 messages with links | | | typed `Ref` (§5.4) |
| | `file` | 666 `path:line` messages | opens Code Review | same | `FileChip` |
| Status | `status` | state chips | | | `StatusChip` |
| | `progress` | determinate or indeterminate | `transition: none` bar | same | `CapacityMeter` |
| | `steps` | plans, repeated status | checklist | same | `PlanDock` presentation |
| | `alert` | callouts, recommendations | | | MUI `Alert` |
| Media | `image` | 1,087 tool images, Call artifacts | `ProtectedImage`, lightbox | same | artifact store |
| | `gallery` | several screenshots | two columns, paged lightbox | grid | `ImageLightbox` |
| Input | `button` | the recurring "要不要我…" | 44 px target, exact-message preview, product confirm | entered region, `Mod+Enter` | `ConfirmSheet` |
| | `choice` | 1,609 questions with options | radio / checkbox list + preview + Send | same | elicitation fields |

**Candidates for later phases**, each needing its own evidence and a
reviewed addition: `grid` and `tabs` (dashboards), `markdown`, `code`,
`json`, `diff` and `diagram` (these would duplicate renderers prose already
reaches; prose stays Markdown), `metric` and `distribution`, a bounded
inline-SVG `chart`, `form` (persistent inputs; elicitation covers the
blocking case) and SVG `data` images. Deliberately absent at any phase:
conditionals and expressions (an agent patches instead), colours or spacing
values, raw HTML, remote image addresses, nested widgets, custom icons.

### 5.3 Bounds and validation

`WIDGET_LIMITS` is enforced identically by the Controller (Rust, before a
row is stored) and the Web (TypeScript, before a document is rendered,
including inline-carrier documents the Controller never saw). Units are
fixed in the contract: `*Bytes` are UTF-8 bytes of the stored compact JSON,
`*Chars` are Unicode scalar values (never UTF-16 units), node depth counts
nested nodes (≤ 12) and JSON depth nested values (≤ 32), numbers are finite
doubles of magnitude ≤ 2^53. Tree bounds: document 24 KiB, 400 nodes, table
200 × 12, text 8 K chars, fallback 2 K chars, label 200 chars, 64 patch ops.
Aggregate budgets bound rendering work, not only the tree: 2,000 table
cells, 1,000 list/kv/step/option items counted recursively, 24 images and 32
decision groups per document; the transcript draws at most 20 rows of a
table and the inspector pages the rest; a multiple choice submits at most 12
options; per session, 200 open and 500 total widgets, 200 interactions per
widget. A patch applies all ops in order against the running result, or
none, and the result must satisfy every bound.

Actionable nodes name themselves and read as sentences: every `button` and
`choice` has an `id`; a choice has a non-empty `label` (the question);
interactive labels are unique within a widget and option labels within a
choice, because labels, not ids, identify what the person chose (§8.2);
option values are unique within a choice and are the only accepted
selection keys, a submitted selection has no duplicates, and a single
choice's effective cardinality is one whatever `max_selected` says. The
worst-case composed message of every prompt node, computed by
`worstCaseMessageBytes` (options ranked by their serialized contribution,
raw label plus JSON-escaped copy, the effective cardinality largest
selected), must fit `messageBytes` (2 KiB). The worst-case record
(`worstCaseRecordBytes`) ranks option labels by their exact serialized
contribution — escaped once in `selected`, once inside the message string,
twice inside the message's JSON line — selects the effective cardinality of
them, and places every identifier and counter at its maximum
(`idChars`, `maxCounter`, `maxTimestampMs`), five attempts, a maximal
`reason` and all retry links; metadata strings (`reason`, `stop_reason`,
handles, identifiers) use an alphabet without any JSON-escaped character
(`METADATA_PATTERN`: letters, digits, space, `_ . : -`), and runtime text
such as a provider stop reason is normalized into it (`normalizeMetadata`)
before storage, so their serialized size is their length; the result must
fit `interactionRecordBytes` (12 KiB). The
worst-case snapshot (`worstCaseSnapshotBytes`) reserves 32 decided groups,
five linked summaries and every metadata field at its maximum around the
document and must fit `snapshotBytes` (48 KiB, including the `outstanding`
list). Both are validated at
publish and replace, so no later settlement can grow a record or a snapshot
past its bound; the tests construct every permitted selection independently
and check none exceeds the estimate.

Field domains are part of the contract: `percent` cells are fractions 0..1,
`bytes` integer byte counts, `duration` seconds, `date` epoch milliseconds
or RFC 3339 strings, `progress.value` finite 0..1; `seq` and `index` are
integers ≥ 0, `line` ≥ 1, `max_selected` an integer 1..12; identifiers
(`id`, `group`, option `value`, `widget_id`, `interaction_id`) follow
`WIDGET_ID_PATTERN`; a value of the wrong shape in a formatted cell renders
as text. Serialization is compact JSON (no insignificant whitespace), UTF-8,
object keys in the order given, escapes only where JSON requires them; sizes
are measured on that form.

Validation is **fail-closed at the Controller**: wrong shape, a bound
exceeded, **any** unknown node type (with or without `fallback`), an unknown
property on a known node, a prose or duplicate option value, an initial
`value` that is not an option, a duplicate or malformed id, a duplicate
interactive label, a label that is not one printable line
(`LABEL_PATTERN`: no control characters, no line or paragraph separators,
no format characters such as bidirectional overrides), a `max_selected`
outside 1..12 as a raw field, an initial `value` with duplicates, a card
with `actions` but no `id`, a `url` ref that is not `https:`, an artifact
string that does not name an existing artifact, a `data` image that fails
the decode check (§5.4), a prompt node whose worst-case message exceeds
`messageBytes` or whose worst-case record (`worstCaseRecordBytes`: message,
selected labels, five attempts, reason and retry links at their maxima)
exceeds `interactionRecordBytes` — each is `invalid` with `{path, code,
message}` so an agent repairs in one round trip. The **Web is tolerant** only of what a *newer Controller*
admitted and this client does not know: an unknown type parses into an
explicit `unsupported` runtime node (type, `fallback`, raw) before the tree
is narrowed to known nodes, so the closed `Node` union and the exhaustive
`assertNever` switch remain honest; it renders the fallback and keeps any
action inside it inert; an unknown property on a known node is ignored.
`fallback` therefore serves older clients, never the Controller.

Parser boundaries are stated, not assumed. An MCP frame may be up to 1 MiB;
`cowboy_execute` arguments are re-serialized and capped at 32 KiB, and that
re-serialization is where duplicate keys collapse with the last value
winning and where the 24 KiB document bound is measured. The inline carrier
reaches the Web as text and is parsed with `strictJson` (duplicates
rejected). A stricter ingress parser would change every MCP action and is a
separate rollout. Shared fixture vectors (`tests/fixtures/widget-v1.json`,
following `composition-v1.json`) hold the two validators to the same
answers on both databases: the examples, each bound exceeded by one,
nested-item budgets, Unicode (astral characters, combining marks, invalid
surrogates at the inline boundary), numeric edge cases (−0, 2^53, 1e400,
NaN text), patch sequencing (ops that only succeed in order, a missing path
mid-sequence) and hostile identifiers.

### 5.4 References, not addresses

Links are typed `Ref` values (`url`, `session`, `file`, `document`, `call`,
`artifact`), never free-form strings the client must classify. `url` accepts
`https:` only and opens through the existing remote-link confirmation.

Images use `ImageSource` with rules per carrier:

| Source | Published widget | Inline carrier |
|---|---|---|
| `artifact` (`/api/artifacts/<64 hex>.<ext>`) | verified to exist, recorded in `widget_artifact_refs` for the revision on every write | not allowed as an image source or a `Ref`; renders `alt` or the label as text |
| `event` (image block of a tool result at `seq`) | resolved to `artifact` and recorded on every write (publish, replace, patch) | resolved by the Web at render time; retained by the referenced tool event |
| `data` (PNG, JPEG, WebP ≤ 8 KiB) | Controller decodes the header: intrinsic size ≤ 256 × 256, still image (animated WebP and APNG rejected) | not allowed |

The Web draws a `data` image in an `<img>` box of at most 128 px and never
draws one the Controller did not verify; SVG is deferred until a bounded
policy exists. The Web CSP is `img-src 'self' data: blob:` and treats an
agent-chosen remote address as a zero-click exfiltration channel
(`protectedImage.ts`), so a remote image is not a widget feature; it is a
`link`.

## 6. Transport

### 6.1 The `widgets` domain over Cowboy MCP

Reads: `widgets.list({session_id?, after?, limit?, include_closed?})`
(`session_id` addresses another same-owner session; pages are packed against
the complete encoded response like every other page, so a page may hold
fewer entries than `limit` and `next` continues after the last included
one) and `widgets.read({session_id?, widget_id, select:
"meta"|"document"|"interactions", revision?, offset?, chars?, after?,
limit?})`. A valid document can be 24 KiB and the read budget is 16 KiB, so
`document` is a text window over the compact JSON (default 1,000 chars, max
4,000) carrying the document `revision` and `sha256`. A current-document
read restarts only when `revision` changes between pages; a **pinned** read
(`revision` given) is immutable, bound to its `sha256`, and never
interrupted by interaction settlement or later revisions. A pinned revision
exists for every revision a decision was made on (§6.3). `interactions`
returns records by cursor, at least one per page. Read outcomes: `ok`, `unknown_widget`, `cleared`,
`unknown_revision` (records are ≤ 12 KiB by construction). Pages — document
windows, interaction records, pinned `revisions` — are packed against the
**complete encoded response** including `WidgetMeta` and escaping, to the
`readPageBytes` target (12 KiB); a page may exceed the target only to carry
a single item and never the 16 KiB result cap, which one record plus ≤ 2 KiB
of metadata always satisfies; pages return the actual continuation offset;
`WidgetMeta` carries `pinned_count`, not the list.

Actions: `widgets.publish {widget_id?, document}` and `widgets.update
{widget_id, expected_revision, change}` with `WidgetChange = replace | patch
| close`. A publish that races a Clear returns `cleared` (nothing stored;
re-read and publish again), the same outcome `update` reports. `publish` returns the Controller-assigned public **handle**
(`w17`), which the row header shows and composed messages use; the agent's
`widget_id` is for addressing only. Widget ids are unique within a session,
agent-chosen (`WIDGET_ID_PATTERN`) or generated; publishing an existing id
returns `exists` with the handle and current revision rather than replacing
it. Update outcomes: `committed`, `revision_conflict {current}`, `invalid`,
`unknown_widget`, `closed`, `cleared`, `refused`. Receipts:
`cowboy_receipt({domain:"widgets", operation})`. Help:
`cowboy_read({help:"widgets"})` returns the contract as TypeScript, the
limits, two composition examples and three rules: a widget that will never
be updated or answered should be Markdown; a decision applies to the
revision it names and a moved revision never authorises substitution — read
the pinned revision; labels are the message.

Writers use the admission rules of `documents.mutate`: operator role,
same-owner live parent, no system or closing session; Superagent
coordinators stay confined to `superagent.*`; managed Call children have no
Agent API and publish through their parent. Idempotency follows the
`agent_api_operations` ledger: admit `(owner, parent session, operation)`
with the request hash before any effect, finish with the result, replay
returns the stored result, a different request under a reused id is
`Conflict`, `unknown` is never proof of failure. `expected_revision` makes
`update` compare-and-swap.

Controller touch points follow the pattern for adding a domain:
`READ_OPERATIONS` / `INVOKE_OPERATIONS`, the `sdk!` block, `HELP_TOPICS` and
`revision()` in `src/agent_api.rs`; the read namespace table in
`components/server/src/server/agent_api/runner.js` and `SessionHost::call`;
`execute` and receipt arms in `agent_api/actions.rs`;
`contracts/agent-api-coverage.json` for the new HTTP/WebSocket surfaces; and
`session.events`, whose closed `EventKind` drops unknown kinds today, so an
agent reading history sees widget rows.

### 6.2 One anchored row per widget, readable by every client

A widget is projected into its session's transcript as a `notice` update:
`{sessionUpdate:"notice", severity:"info", title, description, _cowboy:
{widget: WidgetSnapshot}}`. `title` is the widget title; `description` is a
bounded plain-text projection the Controller computes (summary, then each
node's `fallback` or built-in text: `k: v` lines, step labels with state,
`alt` for images, labels for inputs; ≤ 2 KiB). A client that predates
widgets, an offline PWA on an old bundle or a rolled-back Web release draws
the notice line it already knows; a current client recognises
`_cowboy.widget` **before** the generic notice branch (so the cursor is not
reset and ordinary adapter notices keep their behaviour) and draws the
widget. The compatibility promise is limited and stated: a pre-widget client
appends a notice line per live snapshot it receives, because its
`applyEnvelope` does not know the anchor; one anchored server row therefore
guarantees one row in history and on current clients, not on a pre-widget
client's live view until it reloads. That is accepted as strictly better
than showing nothing.

`anchor_key` in `src/transcript_anchors.rs` gains `widget:<widgetId>` for
notices carrying `_cowboy.widget`, so both reducers (the Hub hot log and the
store writer, `src/persistence.rs` `reduce_anchor`) rewrite every later
snapshot to the first row's seq, the durable `transcript_anchors` table
keeps the first anchor, and a Controller restart never starts a second row.
Snapshots are ordered by `stateVersion`, which advances on every mutation of
the widget (publish, update, close, interaction admission and settlement);
`revision` advances only on agent writes and is the CAS key. Newest
`stateVersion` wins everywhere: Hub reduction, store reduction, bootstrap,
lookup and client merge. Every snapshot also carries the session's
`transcriptGeneration`; a reducer drops a snapshot from an older generation
(§6.3).

Whether prompt controls are live is **not** widget state: the session's
`cowboy_delivery_capabilities` projection (the update Steer targets already
use) carries `widget_delivery: "v1" | null` for the session's current worker
generation, is re-emitted on every generation change and reconnect, and the
renderer combines it with every widget row, so an upgrade enables and a
fallback disables controls without touching any widget's `stateVersion`.

Closure freezes the document and forbids new interactions; interactions
already admitted still settle (`dispatched`, `delivered`, `uncertain`,
`cancelled`, `failed`) and each settlement advances `stateVersion`. A closed
widget therefore has one final document and a finite number of later states,
all ordered, and a device that cached an open state is corrected by any
later list of `(widget_id, state_version, closed)`.

### 6.3 Durable store

Four new tables and one column; `session_widgets` is authoritative and the
transcript row is a projection:

```sql
ALTER TABLE sessions ADD COLUMN transcript_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN widget_index_version INTEGER NOT NULL DEFAULT 0;

session_widgets(session_id REFERENCES sessions ON DELETE CASCADE, widget_id,
                handle, transcript_generation, index_version, revision,
                state_version, published_version, closed, cleared_at_ms NULL,
                seq NULL, published_at_ms, updated_at_ms, document,
                document_sha256, decided, PRIMARY KEY (session_id, widget_id))
widget_revisions(session_id, widget_id, revision, document, document_sha256,
                 pinned_by_interaction, created_at_ms,
                 PRIMARY KEY (session_id, widget_id, revision))
widget_interactions(session_id, widget_id, interaction_id, request_hash,
                    transcript_generation, revision, state_version, node_id,
                    group_id, label, selected, message, message_sha256,
                    state, reason, cancel_requested, attempts,
                    retry_of, retried_by, duplicate_of, superseded_by, cmid,
                    created_at_ms, PRIMARY KEY (session_id, interaction_id))
widget_artifact_refs(session_id, widget_id, revision, artifact_name,
                     PRIMARY KEY (session_id, widget_id, revision, artifact_name))
```

- **Generation CAS at the database.** Every widget write, interaction
  admission, dispatch mark and retry runs in a transaction that performs a
  conditional update on the session's row (`UPDATE sessions SET
  transcript_generation = transcript_generation WHERE id = $1 AND
  transcript_generation = $expected`); zero rows means the generation moved
  and the transaction aborts with `cleared`. **Transport settlement is the
  exception**: writes that settle an attempt's ownership or outcome or a
  record's delivery state are identity-checked (`session_id`,
  `interaction_id`, `command_id`) and not generation-checked, so a terminal
  event for an interaction of a cleared generation still settles it and its
  acknowledgement can complete; such a settlement never republishes a
  snapshot of the old generation and never requeues its prompt — a return
  on a cleared interaction settles `cancelled (cleared)`. PostgreSQL serializes these
  with Clear through the row lock; SQLite through its single writer. An
  in-process per-session fence (an async mutex shared by publication,
  admission, handoff and Clear) reduces contention; correctness does not
  depend on it.
- **Commit-acknowledged writes.** The store writer gains acknowledged
  variants of its in-order channel: the caller receives `committed`,
  `failed` or `unknown` (timeout or lost reply) after the writer has applied
  the operation **in channel order**, so an acknowledged operation is
  ordered after every `AppendEvent` already buffered before it. Widget
  admissions, dispatch marks, cancellation intents and Clear use it.
  `unknown` is treated as "possibly committed": the caller never performs the
  effect the mark guards and re-issues the same conditional statement (each
  carries an operation id and an expected value) until it reads a definite
  answer.
- **Generation on every envelope.** The Hub stamps the session's current
  `transcript_generation` on each envelope it pushes; the writer compares it
  with the durable generation and drops older-generation events and anchors
  instead of persisting them, so a buffered event from before a Clear cannot
  recreate cleared history after it.
- **Writes.** `publish`/`update` validate, resolve every `event` image
  source to an `artifact`, verify `artifact` and `data` sources, write the
  row (`state_version + 1`, `revision + 1` for agent writes, new
  `document_sha256`), insert the new revision's `widget_artifact_refs`, drop
  the refs of revisions that are neither current nor pinned, and record the
  operation result, all in one generation-checked transaction. A creation
  additionally increments `sessions.widget_index_version` and stores the
  new value as the row's `index_version` in that same transaction; a
  tombstone increments the counter too. The bootstrap reads the counter and
  the entries in one transaction (PostgreSQL repeatable read; SQLite single
  writer), labels every page with that counter, orders entries by
  `(index_version, widget_id)` and includes only rows with
  `index_version ≤` the label, so continuation pages describe the same
  index and a concurrent creation never appears in an index that does not
  carry its version.
- **Pinned revisions.** The first interaction admitted on a revision copies
  that revision's document into `widget_revisions`; later writes never
  touch it, and its `widget_artifact_refs` rows are kept for as long as the
  widget row exists. `widgets.read({revision})` serves it bound to its
  `sha256`; the Web's "decided on r1 — view" opens it.
- **Publication is at-least-once from the table.** A snapshot carries the
  `state_version` it was built from and the row's generation. Pushing it
  through the Hub (hot log, batch writer, fan-out) and receiving the batch
  writer's confirmation sets `published_version = max(published_version,
  that snapshot's state_version)`. A sweep (on start and every few seconds
  while any row lags) republishes rows with `published_version <
  state_version` from their current state, origin `repaired`, skipping
  tombstoned rows. W4 promises eventual publication of the **latest** state.
- **Clear is durable first and has an identity.** `clear_transcript` takes
  the session fence, reads the current generation `g`, and sends the
  acknowledged `ClearEvents {operation_id, expected_generation: g}` through
  the writer, which flushes the buffered batch (as it does today) and then
  commits one transaction conditional on `transcript_generation = g`:
  `g + 1`, `cleared_at_ms` on every widget row of generation ≤ g, every
  `admitted` interaction of the session `cancelled (cleared)`, every
  `dispatched` one without delivery evidence `uncertain (cleared)`, and the
  event and anchor deletion `clear_events` performs today. On `committed`
  the Hub clears the hot log and reducer, removes queued `widget:` prompts,
  fans out and acknowledges. On `failed` (zero rows: another Clear or a
  restart already moved the generation) it reconciles memory to the durable
  generation and reports that. On `unknown` it reads the durable generation:
  `g + 1` means the Clear applied and memory is reconciled as for
  `committed`; `g` means it did not and the same operation is re-issued; no
  second increment is possible because the statement is conditional on `g`.
  Until the outcome is known the session stays fenced: no publication,
  admission or handoff proceeds. A publisher that read `g` cannot commit
  afterwards (its conditional update finds zero rows); a snapshot built
  before the Clear is dropped by both reducers on its stale generation; a
  dispatcher mid-handoff fails its CAS (§8.3). Tombstoned rows do not count
  against the 500-widget cap.
- **Retention and cascades.** Child rows (`widget_revisions`,
  `widget_interactions`, `widget_artifact_refs`) live exactly as long as
  their widget row. A tombstoned widget and its children are purged 30 days
  after `cleared_at_ms` (interaction records are audit evidence), except
  that a widget with an interaction whose attempt still holds transport
  ownership is not purged until that attempt is settled; deleting a session
  cascades everything at once after its held attempts are fenced. The artifact collector unions
  `widget_artifact_refs` of non-purged widgets with its existing `events`
  and Call-result roots, so an image stays fetchable while any current or
  pinned revision that shows it is still readable; after the purge the usual
  24 h grace applies. Pinning and collection are coordinated, not merely
  staggered: the artifact store gains one in-process pin lock. Publication
  holds it while it verifies that every referenced artifact exists,
  refreshes its age and commits the reference; the collector holds it for
  each candidate across its final reachability check (re-reading
  `widget_artifact_refs` and the other roots for that name), its age check
  and the unlink. A collector that paused after inspecting an old
  modification time therefore re-checks under the lock and finds the new
  reference before it can unlink; the 24 h grace remains a second guard for
  the event path. No text scanning is needed: references are extracted from
  the validated document, and the inline carrier may not reference artifacts
  at all (§6.4).

### 6.4 Inline carrier (read-only)

A ```` ```cowboy-widget ```` fence whose body is a `WidgetDocument` renders
through the same renderer, dispatched where the Mermaid fence is dispatched
in `web/src/MarkdownImpl.tsx`, with the same contract: while the fence is
open during streaming the source shows as code; an invalid document shows
the code block with a one-line notice; a valid one renders. It has no
handle, revision or receipt, is not listed by `widgets.list`, cannot be
updated in place, and its interactive nodes render inert with their labels
(the Controller never saw the document, so it cannot admit an action against
it). Unknown types render `fallback` or the unsupported chip. Its only image
source is `event`; `artifact` and `data` image sources render `alt`, and an
`artifact` `Ref` renders its label as plain text, because nothing roots an
artifact for an inline document. It exists because it costs no tool round
trip and is what a model produces after reading one example; it is not a
second interaction path.

### 6.5 Web client

`derive.ts` gains a `widget` `RenderItem` for notices carrying a validated
`_cowboy.widget`, checked before the notice branch, keyed by the first
envelope's seq and indexed by `widgetId`, merging a later snapshot only when
`stateVersion` is newer (the `newerSubagent`/`mergeSubagent` shape).
`lastStreamTarget` skips widget rows so an arriving widget does not end the
agent's bubble; `sameRenderItem`, `itemProgressSignature` and `ItemView` gain
the exhaustive case; `applyEnvelope` merges a snapshot in place like
`tool_call_update`; `retainedEventCountForRows` keeps `key ===
String(env.seq)`.

Reconciliation: the bootstrap carries `widget_state {epoch,
transcript_generation, index_version, entries, complete, next}` for the
session's widgets (open and closed, each with `seq`), paginated so the
client always obtains the complete authoritative index; every page of one
bootstrap shares its `index_version`, a per-session counter the Controller
advances on every widget creation and tombstone, and every snapshot carries
the `indexVersion` current when its widget was created. Against that index
the client discards a cached widget only when the index does not list it
**and** the widget's `indexVersion` is at or below the index's version — a
widget whose live snapshot arrived while the index was being fetched is
newer and is kept — and asks `POST /api/sessions/:id/widgets/lookup
{widget_ids ≤ 200, epoch, transcript_generation, after?}` for every entry
that is **missing locally or stale** (`state_version` newer than the cached
one), including widgets created while the device was away and already
outside the reconnect tail. The answer carries the same epoch and generation
or is discarded and reconciliation restarts; each id has an outcome:
`current` with the snapshot (placed at its `seq` when the row is not in the
cache or the tail), `cleared` or `unknown` (the cached row is removed).
Pages are packed against the complete encoded response (≤ 256 KiB,
snapshots ≤ 48 KiB by construction and the envelope accounted, so at least
five fit), always
carry at least one outcome, and `next` resumes the rest; progress is
guaranteed.

Every integration file (`MarkdownImpl.tsx`, `TranscriptItems.tsx`,
`derive.ts`, `Transcript.tsx`, `tools/blocks.tsx`) is at its
structure-ratchet ceiling, so each integration edit extracts first and adds
zero net lines; the renderer itself is a new `web/src/widgets/` module,
lazy-loaded like Mermaid, one file per node family under the 800/150/40
limits.

### 6.6 Widget invariants

- **W1 Identity.** `(session_id, widget_id)` names one widget for the life
  of a transcript generation; a transcript row exists for it at most once.
- **W2 Order.** `state_version` is total per widget; any reader keeps the
  highest it has seen and never regresses.
- **W3 Authority.** `session_widgets` is the truth; a snapshot, a cached row
  or a history page is a projection and may lag but never contradicts a
  higher `state_version`.
- **W4 Publication.** The latest committed state of a non-cleared widget is
  eventually published; acknowledgement never exceeds the state actually
  persisted; republication is idempotent.
- **W5 Closure.** `closed` is monotonic; after it the document does not
  change and no new decision is admitted; already admitted interactions
  still settle and an `uncertain` or `failed` one may be retried
  (`widget_action_retry` copies an intent already decided), each change
  advancing `state_version`.
- **W6 Clear.** Clear is durable before it is visible. A cleared widget is
  never published again; its not-yet-dispatched interactions are cancelled;
  its dispatched ones become uncertain, never proven cancelled; repair
  distinguishes `cleared_at_ms` from a missing projection.
- **W7 Interaction.** An interaction is admitted at most once per
  `interaction_id`; its record, message and queue entry agree and the
  dispatcher verifies that agreement; the message names the revision it was
  made on and that revision and its artifacts stay readable; dispatch
  happens only after its durable mark is acknowledged; delivery is proven by
  the turn the prompt started, never by the transcript echo, and otherwise
  reported as uncertain.
- **W8 Completeness.** Bootstrap and lookup say whether they are complete
  and which epoch and generation they describe; a client treats an
  incomplete or mismatched answer as "reconcile again", never as "nothing
  changed".
- **W9 Generation.** Every widget row, snapshot, interaction and queued
  widget prompt carries its transcript generation; nothing from an older
  generation is reduced, published, dispatched or admitted.
- **W10 Serialization.** Every generation-bearing write is a conditional
  update on the session's generation row, so the database, not the process,
  orders it against Clear; an unacknowledged write never guards an effect.
  Transport settlement writes are identity-checked instead, so a Clear can
  never leave an attempt unsettleable (§6.3).

## 7. Presentation

### 7.1 The row

Inline by default: the document renders inside the row within its height
budget (`sm` 24vh, `md` 40vh default, `lg` 60vh) using `Collapsible` with the
existing fade and 44 px Expand, then **Open** shows the whole widget in the
inspector (Mobile `DetentSheet` with the shared island navigation and
context summary; Desktop a Dialog, with the side-pane placement planned for
Call children as the later home). The transcript preview is bounded
independently of the fold: a table draws its first 20 rows, a list or kv its
first 50 items, a gallery its first 6 images; the inspector pages the rest
(50 rows per page). `presentation.inline: false` gives the compact
`TaskRow`-style line (title, summary, status, Open) for reference material.
The header shows the handle (`#w17`), title, the card status chip, "updated
2m ago", **closed** when interactions are inert, and a glyph marking the row
as the agent's proposal. Row surface, `minHeight` (40 Desktop / 44 Mobile),
`overflow: hidden` and `contain` follow `TaskRow` and
`transcriptRowContainment` (rows with lazy images use `layout`, settled rows
`layout paint`). Component identities are stable across snapshots (mobile
invariant 11): a patch re-renders the changed node, never remounts the
widget.

### 7.2 Layout follows the width it is given

Documents carry hints; the renderer measures. Input modality (touch target
size, hover) comes from the surface; density and layout come from the
container's actual width through a `ResizeObserver` on the widget root,
because a Desktop split pane, a tablet split view and a phone can all be
narrow. Rules: a row `stack` stays a row only while every child keeps
≥ 120 px, `keep_row` included; `kv` uses two columns at ≥ 560 px; a table
honours `fit` only while the fitted table keeps the minimum font step and
44 px row targets of `markdownTableFit`, otherwise it becomes one card per
row with the first column as title and the rest as a `kv` list, whatever
the hint says; a gallery has two columns below 560 px and up to four above;
buttons wrap. Nothing in a document can force horizontal overflow.

### 7.3 Mobile (iPhone)

Read above, operate below. Inside the transcript page a widget may not use
inner `transform`, shadows or filters on moving children, horizontal scroll
boxes that really overflow, `will-change` at rest or `setState` on
`touchstart`; selected states are paint-only. Bars are width-based; images
go through `ProtectedImage` at the row's height budget and open in the
lightbox. Interactive nodes keep 44 × 44 CSS px targets inside rounded
controls at every font scale; the exact composed message is rendered as a
caption line directly above every Send control (§8.1); the product confirm
sheet is `ConfirmSheet`'s inset card. Text follows the global font setting;
control bounds do not shrink with it.

### 7.4 Tablet (iPad)

iPad is the Mobile product on a wide surface, never Desktop. The width rules
of §7.2 apply to the pane the widget actually has (full width or split
view); touch targets are unchanged; the inspector is a wide sheet.

### 7.5 Desktop

Dense and keyboard-first, and a widget's controls are an **entered
region**, because the Conversation dispatcher owns `Tab` (next row toggle)
and `Enter` (toggle the containing row). The row is a
`[data-desktop-widget-toggle]` element: `Tab` selects it, `h`/`l` fold and
unfold it as today. `Enter` or `l` on an unfolded widget **enters** the
region: focus moves to its first control, the row becomes the focus owner
and declares that state in the status line; inside, `Tab`/`Shift+Tab` cycle
the widget's controls only; `Mod+Enter` sends the focused button or the
choice's selection after the preview line has been visible (a choice never
sends on selection); `Esc` or `h` **leaves**, restoring focus to the row
toggle. A declared command opens the inspector; `Esc` closes it and restores
focus. All keys are declared through `useDesktopCommand` and classified with
`desktopKeyIntent`; components never read raw keys; menus get no local
handlers; no second shortcut grammar. Tables get sticky headers,
header-click sort when `sort` is set, and `min`/`max` column hints.

### 7.6 Theme

Only product theme tokens (`web/src/theme.ts`, `appearanceThemes.ts`): tones
map to `success`/`warning`/`error`/`info`, `neutral` to `text.secondary`,
`running` to `primary`; sizes through `desktopSize` and `glyphSize`; radius
10; `CircularProgress` animates paint, bars set `transition: none`.
Acceptance renders in both light and dark through `browserProductTheme.tsx`,
never a bare `createTheme()`.

### 7.7 Text-only surfaces

Notifications, search, Explore pages and the notice projection show `title`,
`summary` and, per node, `fallback` or the node's own text (`alt` for
images, labels for inputs). An agent that wants a widget to read well in a
push notification or on an old client writes a good `summary`.

## 8. Interaction

### 8.1 Actions and the unavoidable preview

`prompt` sends a message to the agent; `open` navigates to a `Ref`; `copy`
copies text and the toast shows what was copied. `open` and `copy` are
local and never reach the Controller. A `prompt` carries **no text and no
data**. On every surface that can send, the exact composed message (§8.2)
is rendered as a product-owned caption line directly beside the control
before it can be activated: under a button when it is focused or pressed,
above a choice's Send control as soon as an option is selected, and in the
hover tooltip on Desktop in addition. A `destructive` button, or any
`confirm` note, additionally opens the product confirm sheet whose title and
body are the product's ("Send to the agent:" and the message) with the
agent's note shown beneath, labelled "note from the agent". Widgets never
use the permission overlay, its sticky card or the `␣1…9` approval commands;
an agent proposal and a native permission request are visibly different
surfaces.

### 8.2 The message contains only what the person saw

```
[widget w17 r1] Activate the built cowboy-web release and verify it
{"widget":"w17","revision":1}
```

```
[widget w3 r4] Which cache may be removed? → The stale Cargo target directory
{"widget":"w3","revision":4,"question":"Which cache may be removed?","selected":["The stale Cargo target directory"]}
```

Line one is the button label, or the choice's question followed by the
selected option labels. Line two names the Controller-assigned handle, the
revision and, for a choice, the question and the selected labels again. The
question is part of the message because two choices may both offer "Yes";
`twoQuestions` in the examples is the collision vector. **No agent-chosen
identifier appears**: not the widget id, not the node id, not option values,
not group names. Labels identify the node because interactive labels are
unique within a widget and option labels within a choice (§5.3); the agent
authored them and maps them trivially. `composeInteractionMessage` in the
contract is the normative composer; the Rust composer mirrors it and the
shared fixtures, including the hostile-identifier and collision vectors,
hold both to it. The agent is told in the help topic that a decision applies
to the revision it names, that the pinned revision is readable with
`widgets.read({revision})`, and that a moved revision never authorises
substitution.

### 8.3 Durable admission, relay, acknowledged handoff and proof of delivery

The session queue persists write-behind (`StoreWrite::UpdatePending`), the
transcript echo of a user message is synthesized by the worker **before**
the provider request (`src/acp.rs`, `AgentCommand::Prompt`) and also for a
held prompt that is cancelled before reaching ACP (`src/remote_runtime.rs`
`cancel`), the runtime retransmits every pending command on reconnect while
the worker's command dedup is per process, and the store writer may exhaust
its retries. None of them can therefore be the record or the proof.
`widget_interactions` is the record, the acknowledged conditional write is
the gate, the turn identified by `(worker_epoch, turn_id)` is the proof, and
a widget command is written to exactly one connection.

1. **Admit.** The Web sends `widget_action {session_id, widget_id,
   revision, state_version, transcript_generation, node_id, interaction_id,
   selected?}`. In one generation-checked transaction the Controller checks
   the widget is open and not cleared, revision and state version match what
   the person saw, the node exists, carries `prompt` and is not disabled,
   its group is undecided, the selection is 1 value for a single choice or
   1..`max_selected` for a multiple one with no duplicates and every value an
   option of that choice; composes the message (§8.2) and checks it against
   `messageBytes` and the record against `interactionRecordBytes`; inserts
   the record (`admitted`, request hash, label, selected labels, message and
   its SHA-256, generation, `cancel_requested = false`, no attempts), pins
   the revision if this is its first decision, records the group as decided,
   bumps `state_version`, and replies `widget_action_result {outcome:
   admitted}`. A repeated `interaction_id` with the same hash returns
   `replayed` with the record's current state **before** any staleness
   check; a different hash is `conflict`. Refusals are stateless: nothing is
   recorded, and a retry with the same id is evaluated afresh.
2. **Relay.** A relay keeps every `admitted` record whose `cancel_requested`
   is false present in the session queue as a prompt whose content is exactly
   the record's message, with `cmid = widget:<interaction_id>`,
   `promptOrigin {actor:"human", source:"widget"}` and the record's
   generation. `tracks_delivery` gains the `widget:` prefix, so
   `prompt_already_accepted` refuses a duplicate whether the original is
   queued, dispatched or echoed. The ledger does not transition on enqueue:
   "queued" is a derived display state.
3. **Reconcile before the queue drains.** On Controller start, and whenever
   a session's queue is restored, before drain is enabled: every restored
   entry with a `widget:` cmid stays only if its record is `admitted` in the
   current generation, without `cancel_requested`, and its content hash
   matches `message_sha256`; otherwise it is removed. Every `admitted` record
   with `cancel_requested = true` is settled: queue copies removed, record
   `cancelled (removed)`, snapshot published. Every remaining `admitted`
   record without an entry is re-enqueued. Enqueue dedup protects against
   duplicates; this step protects against a stale persisted copy draining
   after its record moved on and against a cancellation that committed its
   intent but not its completion.
4. **Hand off only after an acknowledged CAS.** When the queue head is a
   widget prompt, the dispatcher takes the session fence, verifies the
   content hash, mints the attempt's `command_id` and `turn_id` (the ids
   `RemoteRuntime::prompt_traced` creates today; the dispatcher supplies
   them instead of letting the Supervisor discard the returned id), and
   issues the acknowledged conditional write `admitted → dispatched` that
   also appends the attempt `{command_id, turn_id, worker_epoch: null,
   outcome: open, ownership: held}`, under the current generation **and**
   `cancel_requested = false`. The CAS is issued under the runtime's
   per-session admission lock and only while that lock shows **no admitted
   prompt still unfinished and no active turn** (`pending` empty, no held
   attempt, no `current_turn_id`), so an ordinary prompt admitted an instant
   earlier cannot be overlapped by the reservation. `committed`: the CAS has
   yielded a single-use **handoff permit** bound to `(command_id,
   turn_id)`; hand the prompt to the runtime with that permit (step 5) and
   `note_dispatched`. `failed` (the record is no longer `admitted`, a
   cancellation intent is recorded, or the generation moved): drop the queue
   entry. `unknown`: hand nothing off, keep the entry, retry the same
   statement; if the retry reads the record as already `dispatched` with
   this attempt while this process knows it has not handed off, it hands off
   exactly once; if the process has restarted in between, the attempt is
   open without a handoff and resolves to `uncertain` (step 5). At most five
   attempts are recorded per interaction; exhausting them makes the record
   `uncertain`.
5. **Write once, never retransmit; ownership is its own state; command
   fate is a protocol.** The runtime keeps a `CoreCommand::Prompt` in
   `pending` until its `CommandAck` and, on reconnect, clears `sent` and
   retransmits every pending command; the worker's `seen_commands` dedup is
   per process; the Machine broker queues a command for a session without a
   worker (`route_worker` → `queue_pending`) and, in `flush_pending`,
   removes queued commands from `pending_commands` before writing them to
   the worker channel, so neither that queue nor the broker's snapshot
   counts (which omit commands already in the channel) can prove
   non-delivery. Widget commands opt out of retransmission: the runtime
   marks a widget command `written` the moment it is handed to a connection
   and, on reconnect, removes a written-but-unacknowledged widget command
   from `pending` instead of resending it; a command never written is still
   pending and is written once to the next connection — after the
   capability check below.

   **Ownership and the single gate.** Each attempt has an `ownership` state
   separate from the record's delivery state: `held` from the dispatch CAS,
   `released` only by a terminal transport event. The runtime keeps the held
   attempts in a per-session set that `pending_prompt_for` and
   `idle_snapshot_guard` consult exactly as they consult `pending`, and
   **every prompt producer** — Hub dispatch, the direct API prompt
   (`client_commands` → `Supervisor::send`), Superagent delivery and
   background wakeups — passes through the one runtime admission point,
   `prompt_traced`, which takes the session's admission lock and admits a
   prompt only if it carries the **handoff permit** matching the session's
   held attempt (consumed on use) or no attempt is held; everything else is
   refused `widget_attempt_held`, a typed refusal the Supervisor returns to
   its callers (the API maps it to a retryable conflict; queue producers
   keep the prompt queued). The owning command therefore passes its own gate
   exactly once and nothing can overlap it. The Hub's dispatch guard records
   the dispatched `(worker_epoch, turn_id)` for every prompt and
   `complete_turn` releases it only for a completion carrying that exact
   pair; an unmatched completion (a late `TurnEnd` for an earlier widget turn
   during an ordinary turn, a replay, an `unknown-turn`) updates ledgers and
   releases nothing. The wire already carries turn ids for every worker
   generation, so there is no identity-less exception.

   **Command fate.** Three parties keep durable, fsync'd records for guarded
   commands, every message carrying the session id and the command id:

   - the **worker** journals `received {command_id}` in the session's
     private state directory before the prompt reaches ACP and then exactly
     one disposition: `finished {command_id, turn_id, stop_reason}` after the
     `session/prompt` response; `aborted {command_id, reason}` when the
     prompt was never submitted — cancelled before it started, configuration
     or managed-round failure, ACP channel delivery failure (today the worker
     unmarks such a command so a reconnect may retry it; a guarded command is
     instead journaled `aborted` and acked negatively); or `submitted_failed
     {command_id, reason}` when `send_request` queued the request and it
     failed through an agent or connection failure without a response —
     execution may have happened, so this is terminal for the recipient but
     never an automatic return. It carries the real command id on every
     `UndeliveredPrompt` and answers `FenceCommand {command_id}` with
     `CommandFenced {state}` — `never_received` (installing a receipt
     tombstone so a later copy is refused), `running {turn_id}`, `finished`,
     `aborted` or `submitted_failed`. An ACP-level re-issue of
     `session/prompt` inside the same prompt task is one attempt and one
     turn for the ledger: `received` once, one disposition, no second
     `TurnStarted`; the worker never re-submits a guarded prompt in a new
     task;
   - the **broker** keeps a per-command ledger in the Machine state store
     (as `session-account-stops/` is kept today): `held` when queued; the
     **recipient inventory** — `routed {worker_epoch}` recorded under the
     routing lock **before** the write to that epoch's channel, one entry per
     epoch ever written to, at most two (a further routing is refused and the
     attempt settles `uncertain` once the rest are terminal); the immutable
     **epoch binding** `worker_epoch → launch UUID, boot id, containment
     identity` recorded at worker registration, together with an immutable
     copy of that launch's `tree_exited` proof once it exists, retained while
     any inventory references the epoch (the session-level receipt is
     overwritten by the next launch, so the copy is what recovery reads);
     `received`/`finished`/`aborted`/`submitted_failed` relayed from worker
     journals; `rejected_by_worker` recorded **before** a negative
     `CommandAck` is forwarded; and the routing tombstone;
   - the **Controller** keeps the attempt with its copy of the inventory.

   `QueryCommandFate` is a read: it installs nothing and answers only an
   **observation** — `unobserved` (no ledger entry on this Machine), `held`,
   `routed`, `received`, `running` or `receipts` — none of which releases
   ownership by itself. `RevokeCommand` installs the routing tombstone,
   forwards `FenceCommand` to **every** inventoried epoch that is still
   connected (on that epoch's own FIFO channel, so the answer is ordered
   after every earlier write to it; a reconnected earlier epoch is a
   recipient too, whatever the broker projected while it was away), requires
   the retained termination proof for every inventoried epoch that is
   neither connected nor terminal, and answers a **settlement**:
   `pending_epochs {recipients}` until every recipient is terminal
   (`finished`, `aborted`, `submitted_failed`, `never_received` or
   `terminated`), then exactly one of `rejected {journals_unavailable}`
   (tombstoned, every recipient fenced `never_received` or proven terminated,
   no receipt anywhere — with intact journals the command never executed;
   with `journals_unavailable` the future is fenced and every recipient
   terminated but the past is unknown) or `settled {recipients}`. A
   never-observed command is `rejected` once revoked. A broker that recorded
   `routed` and crashed before the write is resolved by the fence: the
   surviving idle worker answers `never_received`. If the inventory itself
   is unavailable, ownership stays held; the only release is a Machine reset
   that proves every worker tree of the Machine exited.

   **Settlement rules.** Delivery evidence and ownership are decided
   separately, and ownership has **no shortcut**: the attempt's own
   `TurnEnd(T, E)` or a `finished` receipt is evidence that makes the
   Controller issue `RevokeCommand`; the broker then seals the inventory
   under its routing lock (routing tombstoned, so no recipient can ever be
   added) and answers `settled {sealed: true, recipients}` once every
   recipient is terminal — immediately for the normal single recipient —
   or `pending_epochs` until then. Negative acks and `UndeliveredPrompt`
   returns go through the same barrier before the record is re-admitted.
   Only a sealed settlement (`settled` or `rejected`) releases ownership.
   Delivery and duplication are then reduced exhaustively over the sealed
   recipients (`reduceSettlement`, tested): any `finished` with a completing
   stop reason (`EndTurn`, `MaxTokens`, `MaxTurnRequests`, `Refusal` or the
   snake_case spellings) → `delivered`; otherwise any possibly executed
   recipient (`finished` with any other stop reason, `submitted_failed`,
   `terminated`) → `uncertain`; otherwise an `aborted` whose cause is the
   person's `stopped` → the record settles `cancelled (stopped)`; otherwise
   (only `aborted` for another cause, `never_received`) → the attempt is
   `returned` (record `admitted`), or `cancelled (superseded)` when a retry
   already exists, or `cancelled (cleared)` when the generation was cleared;
   `rejected` reduces as the all-`never_received` case, with
   `journals_unavailable` making it `uncertain`. `duplicate_execution` is
   set whenever two or more recipients are possibly executed, whatever the
   delivery outcome; both messages stay visible. The abort cause and the
   finish identity travel unchanged from the worker journal through the
   broker ledger and the settlement into the stored attempt, so a Stop is
   honoured after any interruption. Release is idempotent — a later answer
   for a released attempt changes ownership nothing — but completing evidence
   still counts: a `finished` with a completing stop reason that arrives for
   a record released as `uncertain` upgrades it to `delivered` and marks an
   existing retry `duplicate_of`, through the same conditional update that
   admits retries (step 7). The Controller revokes whenever it receives a
   terminal receipt for an attempt, whenever it drops a
   written-but-unacknowledged widget command, whenever a bound epoch exits
   without `TurnEnd(T)`, for every held attempt found at start, and before
   any generation change for the session (below).

   **Capability, fail closed, at the write.** Interactions require the
   Machine capability bundle `widget_delivery_v1` (step 6). The Controller
   learns it from each connection's handshake for the session's current
   worker generation, keeps it in the session-level
   `cowboy_delivery_capabilities` projection, refuses `widget_action` and
   `widget_action_retry` with `capability_missing` without it, re-checks it
   before the dispatch CAS, and re-checks the **negotiated capability of the
   connection it is about to write to** immediately before every actual
   write — including a retained unwritten command after a reconnect to a
   rolled-back broker — refusing locally as a proven-unsent return when the
   peer lacks it. Guarded prompts travel in a new frame variant
   (`GuardedPrompt`) that older peers cannot decode, so a peer that was not
   negotiated can never silently accept one. A generation change (fallback,
   cutover or rollback) for a session with held attempts first revokes and
   settles them; the bundle is split so that this is always possible: the
   smaller **recovery-reader capability** `widget_delivery_recovery_v1`
   (answer `RevokeCommand` and `FenceCommand`, report termination proofs,
   serve journals) is retained by every worker and broker build from the
   bundle's introduction onward, and the component activator enforces it as
   a persistent **compatibility floor**: before activating a Machine build
   below the floor it asks the Controller — the authority on holds, since a
   reservation can exist in `widget_interactions` before the broker ever saw
   the command — for held attempts on that Machine, the Controller drains
   new reservations for the maintenance window, and an unreachable Controller
   or an unanswered query refuses the activation; "no Machine record" never
   means "no holds". The **Controller** has the same floor: the Controller
   component activator refuses to activate a build below
   `widget_controller_recovery_v1` (one that knows `widget_interactions`,
   held attempts and the admission gate) while the database holds any
   `admitted` or `dispatched` interaction, and an older Controller that
   starts against such a database anyway must not dispatch for those
   sessions — so while any attempt is held the Controller also sets the
   session's existing durable queue hold (the pause older Controllers
   already honour) and clears it only when the attempt settles; a
   floor-compliant Controller reconstructs holds and the dispatch guard from
   the ledger on cold start, never from worker snapshots alone. The
   projection flips the controls inert as soon as the current generation
   lacks `widget_delivery_v1`.
6. **Proof: the turn, identified.** The turn's identity is the pair
   `(worker_epoch, turn_id)`, exactly what Steer already targets. Three
   Controller-side changes make it usable for display and attribution —
   `apply_event` receives the frame's `worker_epoch` and
   `runtime_seq`; on `TurnStarted {turn_id, command_id}` the Controller
   binds the turn to the inventoried recipient epoch that emitted it — each
   recipient has its own binding, and a `TurnStarted` from an epoch not in
   the inventory is recorded as a new recipient, not ignored; and the
   `TurnEnded {turn_id, stop_reason}` projection stamps `{turn_id,
   worker_epoch}` onto the persisted `TurnEnd` envelope through the reserved
   payload-key mechanism the echo's cmid already uses (`persistence.rs`),
   with a Controller-side envelope round-trip fixture beside
   `contracts/protocol/event.json`, which only covers the bare `Event`. The
   worker-generation files (`agent_model.rs`, `acp.rs`, `worker.rs`,
   `runtime_wire.rs`, the Machine broker) change only inside the
   `widget_delivery_v1` Machine release below; display widgets need none of
   it.

   **Evidence.** A record becomes `delivered` when a `TurnEnd` carrying the
   attempt's `turn_id` and its bound `worker_epoch` ends with `EndTurn`,
   `MaxTokens`, `MaxTurnRequests` or `Refusal` (or their older snake_case
   spellings). Never evidence: the user-message echo (synthesized before the
   provider request), `TurnStarted` and the Busy lifecycle row (emitted
   before the fallible `session/prompt` send), `CommandAck` (the ACP command
   channel accepted a message, nothing more), `acknowledge_prompt_finish`
   (transport cleanup for any matching completion), and **any agent
   output** — the window between a turn's Busy and its `TurnEnd` opens before
   the request is sent and can contain native-cycle or subagent output.
   `Cancelled`, `Error` or `error: …` stop reasons, a `TurnEnd` with a
   missing reference or `unknown-turn`, the bound epoch exiting (Exited,
   Crashed, a Welcome snapshot from another epoch whose `current_turn_id` is
   not this turn) without `TurnEnd(T)`, a lineage reset, a dropped
   written-but-unacknowledged command, or a Clear while dispatched make the
   record `uncertain` once its inventory is sealed. A `TurnEnd` whose pair
   matches no inventoried recipient is ignored for the ledger and never
   releases the dispatch guard (step 5).

   **Late proof.** Attempt identities stay searchable after the record is
   `uncertain`. A `TurnEnd(T, E)` with a completing stop reason that arrives
   later — the old epoch's unacknowledged events replayed after a reconnect,
   or found in the store after a restart — moves the record `uncertain →
   delivered` through the same conditional update that admits retries, so the
   two cannot interleave: if a retry was already admitted, it is marked
   `duplicate_of` the original and stays visible as such; if the late proof
   commits first, a retry is refused `not_retryable`. Both links appear in
   the snapshot's interaction summaries.

   **Returned and stopped prompts.** The runtime gives a prompt back without
   any provider request when the worker rejects the one and only
   `CommandAck` (its workspace check runs before dedup, but with no replay
   there is nothing to confuse it with) or reports `UndeliveredPrompt` from
   cold-start salvage; the runtime already requeues such prompts under their
   cmid, and for a widget prompt the attempt closes as `returned` and the
   record goes back to `admitted`; the next dispatch is a new attempt. A
   prompt the person **stops** during configuration startup is different:
   `RemoteRuntime::cancel` withdraws the held prompt before any send and
   synthesizes `Cancelled` without requeueing it; the attempt closes as
   `cancelled` and the record settles `cancelled (stopped)`, proven unsent,
   and is never relayed again — relaying it would undo the person's Stop. A
   Stop after the command was written is `uncertain`, as any other
   cancellation of a sent prompt. Parking into drafts
   (`park_recovery_prompts`) is refused for `widget:` cmids as any other edit
   path (§8.4).

   **Lifetimes.** interaction ↔ cmid: permanent. cmid ↔ attempt
   `(command_id, turn_id)`: from the CAS until `TurnEnd(T)` arrives, the
   prompt is returned or stopped, the bound epoch dies, or a Clear fences it;
   the identities remain on the record afterwards for late proof. turn ↔
   epoch: from `TurnStarted` to `TurnEnded` or worker exit. After a Controller
   restart the open attempts are resolved from durable evidence: a persisted
   `TurnEnd(T, E)` found in the store; a `TurnEnded` the worker replays
   because it was never acknowledged; a Welcome snapshot whose
   `current_turn_id` is still `T` (keep waiting); otherwise the Controller
   revokes (`RevokeCommand`) and the settlement decides. The remaining gap — a `TurnEnded`
   acknowledged to the worker but lost before the write-behind store
   persisted it — would leave an attempt held while the surviving worker sits
   idle, so **the acknowledgement of a tracked `TurnEnded` is delayed until
   its ledger transition commits** (acks are cumulative, so later events wait
   too); this is mandatory in Phase 1, and the settlement write is
   identity-checked so a Clear cannot block it (§6.3). A crash before the
   ledger commit therefore leaves the event unacknowledged; the worker
   replays it and settlement completes without acknowledgement loss. On
   recovery a held attempt is also settled from the worker journal's
   receipts through the revocation.

   ACP has no "prompt accepted" acknowledgement — `send_request` only queues
   the JSON-RPC request — so the `session/prompt` response is the only
   absolute proof. Today the worker stamps `TurnEnded.turn_id` from its
   mutable snapshot slot (`current_turn_id`), which a second overlapping
   prompt could overwrite; the Controller-side ownership above prevents the
   overlap, but attribution must not rest on that alone. **Interactive
   widgets therefore require a worker generation that stamps `TurnEnded`
   from the prompt task itself** (and tags the active prompt's `Update`
   events with its `turn_id`, and distinguishes "cancelled before send" from
   a provider `Cancelled`), together with the worker command journal and
   `FenceCommand`/`CommandFenced`, the broker's command-fate ledger and
   recipient inventory with `QueryCommandFate`/`RevokeCommand` →
   `CommandFate`, tree-contained worker launches whose termination the
   Machine can prove (`tree_exited`), the `GuardedPrompt` frame and the
   retained recovery reader `widget_delivery_recovery_v1` — advertised in
   the Machine handshake as the capability bundle `widget_delivery_v1`. That is a
   Machine release and an explicit maintenance boundary, and it is a Phase 1
   prerequisite for `button` and `choice`: a session whose current worker
   generation lacks the bundle shows prompt controls inert with the notice
   "interactions need a newer worker" (from the session's
   `cowboy_delivery_capabilities` projection, never from widget state) and
   refuses `widget_action` and `widget_action_retry` with
   `capability_missing`; display widgets work everywhere.
7. **Retry.** `widget_action_retry {interaction_id, retry_interaction_id}`
   is the only Send-again for an `uncertain` or `failed` record. It copies
   the original's pinned intent (label, selection, revision, message)
   regardless of the widget's current revision, closure or group state (the
   decision was already admitted), records `retry_of` on the new record and
   `retried_by` on the original, and is admitted idempotently on
   `retry_interaction_id`: a replay returns the retry's current state. It is
   refused with `unknown_interaction`, `widget_mismatch` (the original
   belongs to another widget), `cleared`, `not_retryable` (the original is
   not `uncertain` or `failed`, including one just proven delivered),
   `already_retried`, `retry_chain` (the original is itself a retry; chains
   have length one), `superseded` (a replacement decision was admitted on
   the original's reset group; supersession is persisted on the record when
   that decision is admitted), `in_flight` (**including an original whose
   attempt still holds transport ownership**), `capability_missing`,
   `interaction_limit`, `rate_limited`, `session_unavailable` or `conflict`.
   Eligibility is the single predicate `retryable` the snapshot projects:
   state `uncertain` or `failed`, every attempt released, no `retry_of`, no
   `retried_by`, no `superseded_by`.
   Retry admission and the settlement of a revocation answer are the same
   conditional update on the record, so they cannot interleave: a
   `CommandFate.rejected` for an original whose retry already exists settles
   the original `cancelled (superseded)` instead of requeueing it, and a
   retry admitted after a `rejected` settlement finds the original
   `admitted` again and is refused `not_retryable`. The session's ordinary turn Retry
   (`retry_turn`, which otherwise re-submits the last user prompt under a
   `cowboy-retry:` cmid) recognises a last prompt whose cmid is `widget:`
   and performs this operation instead, so no path re-sends a widget prompt
   outside the ledger.
8. **Cancel.** Deleting a widget prompt from the queue is arbitrated with
   dispatch through the ledger, not the queue: the Hub first records the
   durable intent `cancel_requested = true` on the record through the
   acknowledged write (conditional on the record still being `admitted`);
   on `committed` it removes the queue entry and marks the record
   `cancelled (removed)`; on `failed` the record was already dispatched and
   the person is told so; on `unknown` the entry stays, the dispatcher will
   not hand it off (step 4 requires the intent to be false, and an unknown
   intent is treated as possibly set until re-read), and the intent write is
   re-issued. A crash between the intent and the completion is finished by
   step 3 at the next start. Cancellation is promised only after its intent
   commits: a prompt whose intent never became durable remains `admitted`
   and may still dispatch after a restart, which the person sees as the
   control returning to "queued".

| Crash or fault point | Ledger | Queue after restart | Outcome |
|---|---|---|---|
| after admission commit, before enqueue | `admitted` | re-enqueued (step 3) | sent once |
| after enqueue, queue state lost | `admitted` | re-enqueued | sent once |
| after enqueue, queue state persisted | `admitted` | kept after hash check | sent once |
| cancellation intent `committed`, completion lost | `cancel_requested` | removed, record settled `cancelled` at start | never sent |
| cancellation intent `unknown` | `admitted`, intent possibly set | entry kept, not dispatched until re-read | sent once or cancelled, never both |
| cancellation intent never durable (crash before commit) | `admitted` | re-enqueued | may still dispatch; shown as queued again |
| CAS acknowledged, process dies before the write | `dispatched`, ownership held | entry removed by reconciliation; revocation issued at start | `rejected` (never observed, never received): attempt `returned`, record `admitted`, sent once later |
| CAS acknowledgement lost, process alive | `admitted` or `dispatched` | entry kept | retry CAS; the owning command passes its permit once |
| command written, connection drops before ack | `dispatched`, held | not retransmitted; revocation issued | `pending_epochs` until every recipient answers or is proven terminated; then a sealed `rejected` → `returned`, or a sealed `settled` reduced by `reduceSettlement` |
| broker recorded `routed`, crashed before the worker write | `dispatched`, held | revocation fences the surviving idle worker | `never_received` → `rejected` → `returned` |
| earlier recipient epoch survives a disconnect and reconnects | `dispatched`, held | revocation fences it on its own channel | its journal decides: `never_received`, `running`, `finished` or `aborted` |
| command never written, connection drops | `dispatched`, held | written once to the next connection after the capability re-check | one execution, or a local proven-unsent return |
| handoff done, worker replaced before `TurnEnd` | `dispatched`, held | revocation | held until the old epoch's termination is proven or it answers; then the sealed settlement reduces to `uncertain` (terminated without receipt) or to its journal's disposition |
| prompt cancelled or failed before the provider request | attempt `aborted` | — | `returned`, or `cancelled (stopped)` for the person's Stop |
| fallback or rollback requested while an attempt is held | held | — | refused until settled; the recovery reader answers revocations |
| turn ends `Cancelled`, `Error` or `error:` | `uncertain`, released by `turn_end` | — | Send-again available |
| turn ends `EndTurn`/`MaxTokens`/`MaxTurnRequests`/`Refusal` with the attempt's `(turn_id, worker_epoch)` | `delivered` | — | proven |
| late completing `finished` after `uncertain` | `delivered`; an existing retry marked `duplicate_of` | — | both messages visible |
| prompt returned (rejected first ack recorded by the broker, `UndeliveredPrompt` with its command id) | attempt `returned`, record `admitted` | requeued by cmid | sent once on a later attempt |
| person stops during configuration startup | attempt `cancelled`, record `cancelled (stopped)` | not requeued | never sent |
| `TurnEnd` for another turn id or epoch | unchanged | — | ignored for the ledger and for the dispatch guard |
| `TurnEnded` arrives, Controller crashes before the ledger commit | `dispatched`, held; event unacknowledged | — | the worker replays it; settled once; no acknowledgement loss |
| echo present without provider request | any | — | ignored as evidence |
| Clear while an attempt is held | ownership kept; generation moved | entries removed | terminal event still settles by identity: `delivered`/`uncertain`, or `cancelled (cleared)` on return; nothing republished or requeued |
| broker restart with lost journals | `dispatched`, held | revocation | `pending_epochs` until every recipient is proven terminated; then `rejected {journals_unavailable}` → `uncertain`, future fenced |
| direct API prompt while an attempt is held | — | — | refused `widget_attempt_held`; the owning command passes with its permit |
| retry admitted, then `rejected` for the original | original `cancelled (superseded)` | one copy queued | no duplicate |
| restored stale queue copy of a dispatched record | `dispatched` | removed before drain | never re-sent |

The Web shows admitted → queued → dispatched → delivered on the control with
the unsent → sent → stalled timing used for permission replies, `uncertain`
with its Send-again, and `cancelled`/`failed` plainly.

### 8.4 Decision groups, pending input, the queue

- **Group identity.** A group is the explicit `group` id, or for a card's
  `actions` the card's `id` (required); an interactive node with neither is
  its own implicit group. An explicit `group` on a card action takes
  precedence over the card's implicit group. At most 32 groups per document.
- **Supersession.** Admitting a decision on a node supersedes every
  released, retryable `uncertain`/`failed` record of that node's group
  (`superseded_by`), so the old intent can no longer be re-sent; a record
  whose attempt still holds ownership is never superseded or removed — it
  stays outstanding until ownership settles, and the new decision is
  refused `in_flight` until then. Supersession and retry admission are the
  same conditional update on the record.
- **Decided and reset.** Admission records `decided[group] = {interaction,
  revision}`. A later agent write (`replace` or `patch`; the rule is
  computed on the resulting document, so both behave alike) **resets** a
  group only when the group's canonical interaction signature changes.
  `interactionSignature` in the contract defines it: the group's actionable
  node ids and, per node, its type, label, `disabled`, and for a choice
  `multiple`, the effective `max_selected` after defaults, and its option
  values and labels in order. Reordering or changing surrounding display
  nodes does not reset a decision; changing what the person would be
  deciding does. Fixtures cover both.
- **Pending selection.** A choice keeps the person's unsent selection across
  snapshots only while the choice's signature is unchanged; otherwise the
  selection resets and the control shows "updated".
- **Queue rows are delete-only on every path.** The Hub refuses
  `queue_edit`, `queued_to_draft` and any resubmission that would carry a
  `widget:` cmid (`edit_refused`); deleting the entry cancels the record
  (§8.3 step 7); the turn Retry routes a widget-originated last prompt to
  `widget_action_retry` instead of minting a `cowboy-retry:` copy (§8.3
  step 6). The dispatcher verifies the queued content against
  `message_sha256` before any handoff, so a prompt that somehow diverged
  from its record is dropped and the record cancelled rather than sent.
- **Decided on an earlier revision.** When the widget's revision has moved
  past a decision's revision, the control shows "decided on r1 · view" and
  opens the pinned revision.

### 8.5 What a tap never does

It never applies a control, starts a Call, writes a file, changes a
preference or touches another session. The message is visible in the
transcript as the person's own message, bounded (label ≤ 200 chars, ≤ 12
selected labels), composed only from visible text, and previewed before
sending on every surface.

### 8.6 Inert states

Interactive nodes are inert when the widget is closed or cleared, the
session is exited or deleted, the person's snapshot is stale, the node is
`disabled`, its group is decided, the session's worker generation lacks the
capability bundle, or an interaction for that node is in flight
(`admitted`, `dispatched`, or any attempt still holding transport
ownership). The renderer learns all of this from the snapshot's
`outstanding` list, whose membership is exact: every record with **any
attempt still holding ownership**, whatever its delivery state; every
`admitted` or `dispatched` record; and every record for which `retryable`
is true (the predicate retry admission shares: `uncertain` or `failed`,
every attempt released, no `retry_of`, no `retried_by`, no `superseded_by`).
A record leaves the list when its ownership is released and it is settled
or not retryable, when its retry is admitted (the retry represents it and
links back with `retry_of`) or when a decision on its node's group
supersedes it (`superseded_by`); a held record never leaves through
supersession. There is therefore at most one entry per interactive node by
construction (≤ 32; the next admission is refused `interaction_limit`);
older records stay readable through `widgets.read(select:"interactions")`. Every ownership change advances
`stateVersion`, so the list is ordered like every other snapshot change.
Display nodes always render.

### 8.7 Relationship to elicitation and permissions

Unchanged. A blocking question still uses the native tool and the permission
channel; a widget choice is non-blocking and persistent. The agent chooses;
the surfaces never look alike.

## 9. Extensibility and compatibility

- **Additive catalog.** New node types and optional properties are minor
  changes under `cowboy.widget/1`; a changed meaning or a new required
  property is `cowboy.widget/2`. The help topic lists what the deployed
  Controller accepts.
- **Older readers get a line, not nothing.** The notice projection (§6.2)
  means a pre-widget, rolled-back or stale-PWA client shows title and text;
  a current client older than its Controller shows `fallback` for a type it
  lacks. Reader-first rollout still applies: renderer and reducer land before
  the domain, the domain before the help topic and agent guidance.
- **Two validators, one fixture set.** `tests/fixtures/widget-v1.json`
  vectors run under the Rust validator tests and `contracts/widget-v1.test.ts`
  on both databases; wire semantics (§6), unsupported-node handling and
  parser boundaries (§5.3) are fixed before the second validator is written.
  Adding a type means: the TS union, the Rust validator arm, a renderer file
  under `web/src/widgets/`, fixture vectors (valid, each bound exceeded by
  one, fallback), a row in §5.2 and a browser conformance case.
- **Plugins contribute data, not renderers.** Every signed Plugin kind
  supplies data the core draws, and the plugin design states a renderer
  cannot be registered by a signed template. A later `widget_template` kind
  may ship bounded documents with parameter slots that an agent instantiates,
  rendered by the same core catalog; it needs its own contract because Plugin
  installation is Machine-scoped while widgets render on the Web.
- **Carriers are additive.** MCP action and inline fence share one document;
  a future carrier (a Call child's result document rendered as a widget in
  the parent) reuses the schema and the anchored row.

## 10. Trust boundary

- Agent content is data at both boundaries: bounded parse, closed unions
  rendered by exhaustive switches ending in `assertNever` after an explicit
  `unsupported` step, no HTML, no styles, no scripts, typed `Ref`s instead of
  URL strings, images only from the artifact store, a stored event or a
  Controller-verified still image drawn as an opaque `<img>`.
- **No agent-chosen text or identifier is ever attributed to the person.**
  A tap sends only labels the person read, the Controller's handle and the
  revision; the message is previewed before every send; identifiers (widget
  id, node id, option values, groups) are addressing that never leaves the
  Controller's tables and the agent's own reads. The hostile-identifier
  vector is a permanent test.
- An artifact name is fetchable by any authenticated product user who knows
  it (existing property of `/api/artifacts/`). Widgets do not widen it: the
  Controller admits only names that exist, and an agent can only learn names
  from its owner's sessions.
- Limits are per session: 200 open and 500 total widgets, one publish or
  update per second sustained (burst 5), 30 interactions per minute, 200
  interactions per widget, one in-flight interaction per node. Exceeding them
  is `invalid`/`refused`, never silent truncation.
- Same-owner scope as all session reads; coordinators and managed children
  excluded (§6.1). `text` goes through the inline Markdown renderer that
  already escapes HTML.

## 11. Token budget

Measured on the sampled history and the worked examples
(`contracts/widget-v1.examples.ts`, Markdown and widget in the same
language), counting the **complete `cowboy_execute` envelope and its
result** for the agent side; the same arithmetic is what
`contracts/widget-v1.test.ts` holds the examples to. Bytes are shown because
tokenizer traces are the ship gate below, not this table **[unverified]**:

| Step | Markdown today | Widget (envelope + result) | Ratio |
|---|---|---|---|
| 2,112 historical tables, publish | median 322 chars, p90 675 | +7% median (p10 +3%, p90 +14%); table only, no envelope | 1.07× |
| Status table, publish | 458 B | 1,085 B | 2.4× |
| Comparison, publish | 499 B | 1,167 B | 2.3× |
| Approval report, publish | 286 B | 1,151 B | 4.0× |
| One status update, three ops | 108 B one sentence · 396 B card re-sent with the sentence | 438 B | 4.1× the sentence · 1.1× the re-sent card |
| The person's answer | a typed sentence the agent then parses | 97 B button · ≈200 B choice with question | fixed shape |

An update's cost does not grow with the report: a **benchmark update** (≤ 3
`set` ops whose paths and string values are ≤ 64 bytes in JSON-escaped
UTF-8, a widget id ≤ 64 and an operation id ≤ 128 chars, counters at their
maxima) measures 438 B for `progressPatch` and is bounded at 1 KiB at the
class maximum (`benchmarkUpdateBytes`, constructed in the test with quote
and emoji values), while a general patch is bounded only by the patch and
document limits (an op may set an 8 K-char text). Patching therefore beats
re-sending a report only when the report is larger than about 330 bytes: the **median historical
table is at that boundary and the top third is above it**; against a
one-sentence status, patching never wins. Prefix totals on the approval
exchange: after publication the widget has cost 1,151 B against 286 B
(865 B behind); after one update 1,589 B against 394 B when Markdown sends
only the sentence (1,195 B behind) or 682 B when it re-sends the card
(907 B behind); for this small card each further update widens both gaps.
For a p90 table (675 B) re-sent with its sentence, the widget gains about
345 B per update and recovers its publication overhead after roughly six
updates **[arithmetic on the measured examples; tokenizer traces pending]**.
**The design therefore claims no token saving.** What it claims is a bound —
publication ≤ 4.5× the same-language report, a benchmark update ≤ 1 KiB at
its class maximum (438 B measured), a decision message ≤ 2 KiB — in
exchange for in-place updates, readable Mobile rendering and typing-free,
auditable decisions. The one measurable token
effect in the design's favour is on the person's side: a decision is a
97–200 byte fixed-shape message instead of a typed sentence plus the agent's
parsing of it.

What keeps the overhead bounded: patches instead of re-sends; references
instead of content (an image is ~70 bytes); fixed-shape answers; nothing
resident grows (the help topic loads once per session within the per-topic
budget, results are a few dozen bytes). Agent guidance in the help topic: a
widget that will never be updated or answered should be Markdown; prefer one
widget per topic, patched; prose stays Markdown.

**Ship gate** (`tools/widget-token-measure.ts`, committed with the fixtures
and its tokenizer traces): tokenize, with the Claude and Codex tokenizers,
the three archetypes and a p50 and a p90 historical table as complete
same-language lifecycles — cold and warm help discovery, publish envelope and
result, receipt read, two patch envelopes and results, one tap message, the
agent's read of it — and two Markdown lifecycles of the same exchange (the
report then one-sentence statuses; the report then the report re-sent with
the sentence; plus a typed answer), reporting each prefix separately against
each baseline. Phase 1 ships only if measured publication is ≤ 4.5× its
report for every archetype, the benchmark class maximum is ≤ 1 KiB of
envelope and result, a decision message is ≤ 2 KiB, and warm discovery plus
one publish
stays under 4 K tokens; otherwise the design stops here and is reworked, not
narrowed automatically. The contract test enforces the byte forms of these
bounds on the committed examples today.

## 12. Phases

| Phase | Content | Components | Release boundary |
|---|---|---|---|
| 1 | Contract v1 (18 types); display widgets on Controller + Web alone; `button`/`choice` with `prompt`/`open`/`copy` **gated on the Machine capability bundle `widget_delivery_v1`** (prompt-owned `TurnEnded` identity, the `GuardedPrompt` frame variant, the worker's durable command journal with `finished`/`aborted` dispositions and `FenceCommand`/`CommandFenced`, the broker's durable command-fate ledger and recipient inventory with `QueryCommandFate`/`RevokeCommand` → `CommandFate`, negative acks recorded before forwarding, real command ids on `UndeliveredPrompt`, tree-contained worker launches for every capability-enabled session including default accounts, immutable epoch→launch bindings with retained termination proofs, the retained recovery reader `widget_delivery_recovery_v1` enforced as a component-activator compatibility floor that consults the Controller, sealed settlements with discriminated recipient dispositions, `submitted_failed`, stop-before-send distinguished); the Controller floor `widget_controller_recovery_v1` with the durable queue hold for held sessions; `widgets.publish/update/list/read`, receipts, help topic; `sessions.transcript_generation` and `widget_index_version`, `session_widgets`, `widget_revisions`, `widget_interactions`, `widget_artifact_refs`; acknowledged in-order store-writer variants, generation stamping on envelopes and durable-first Clear with an operation identity; notice-projected anchored rows, acknowledged publication watermark and sweep, bootstrap `widget_state` (paged, index-versioned) and lookup; `widget_action` admission, relay, queue reconciliation with cancellation settlement, acknowledged CAS handoff with recorded attempts, write-once widget commands with ownership as its own state, the command-fate protocol, the permit-based single admission gate for every producer, identity-matched `complete_turn` for every completion, identity-checked transport settlement across Clear, `outstanding` projection, delayed acknowledgement of tracked `TurnEnded`, pre-write capability checks, capability projection through `cowboy_delivery_capabilities`, turn identity carried from `prompt_traced` through `apply_event` to the persisted `TurnEnd` envelope with its round-trip fixture, late proof, `uncertain` and `widget_action_retry`; the artifact pin lock; inline read-only carrier; Mobile and Desktop renderers with preview lines and the entered region; `session.events` widget kind; collector root | Controller + Web (four tables, three columns) for display; **Machine release** (`widget_delivery_v1`: worker, broker and wire) before interactions are enabled | display: no Machine release; interactions: explicit maintenance boundary |
| 2 | `form` fields if elicitation proves insufficient; `control` actions behind the product confirm and an explicit allowlist; pin a widget to the Desktop side pane / Mobile dock; `metric`, `distribution`, `grid`, `tabs`; SVG `data` images under a bounded policy; image sources from execution-machine files through the verified artifact delivery used by image Calls | Controller + Web; **Machine release** for the worker signal and file sources | explicit maintenance boundary |
| 3 | `widget_template` Plugin kind; Call children's result documents rendered as parent widgets; live `Ref` badges; `chart` | Controller, Web, Plugin SDK | separate reviewed contracts |

Phase 1 ships readers before writers: the Web renderer and reducer cases
land first, then the Controller domain, then the help topic and agent
guidance; the §11 gate runs before the domain is enabled.

## 13. Acceptance (Phase 1)

- Fixture vectors (`tests/fixtures/widget-v1.json`): the examples; each
  bound exceeded by one, including the aggregate and recursive budgets; a
  worst-case valid document (twenty 20-row tables, 24 images); duplicate
  keys at the inline boundary; unknown type with and without `fallback`;
  unknown property; prose or duplicate option value; invalid initial value; duplicate
  interactive label; label with a control character; card with actions and
  no id; the quote/emoji worst-case ranking vector; Unicode and numeric edge
  cases; patch sequencing; the hostile-identifier document, the two-question
  collision and their composed messages. Run by Rust (PostgreSQL and SQLite)
  and Bun.
- Fault injection on both databases at every row of the §8.3 matrix: crash
  after commit before publication (sweep republishes); an older snapshot
  acknowledged after a newer state (watermark stays correct); store-writer
  restart between two snapshots (one row); an exhausted or rejected
  acknowledged write (no handoff, retried); a lost acknowledgement with and
  without a restart (handoff once / `uncertain`); a restored stale queue copy
  of a dispatched record (removed before drain); a synthetic echo without a
  provider request (not evidence); a turn ending cancelled (`uncertain`);
  Clear during each awaited handoff and against a publisher holding a
  pre-Clear snapshot (dropped), an already queued interaction (removed and
  cancelled), an `admitted` one (never relayed) and a dispatched one
  (`uncertain`, not cancelled); Clear whose commit fails (nothing cleared,
  error reported); Controller restart with `admitted` rows (relayed once;
  cmid dedup observed); duplicate `widget_action` (replayed); stale tap
  (refused with `current`); queue edit, queue-to-draft and resubmission of
  a widget prompt (refused), delete (cancelled); retry of an `uncertain`
  record and a late proof afterwards (duplicate marked); a pinned revision
  read while the widget is updated and interactions settle (uninterrupted);
  worst-case record and snapshot fixtures (12 longest labels, 32 decided
  groups) within their byte bounds; a publish racing Clear (`cleared`); an
  unknown Clear outcome resolved from the durable generation in both
  directions; a buffered pre-Clear event arriving after the Clear (dropped by
  the writer); a cancellation intent `committed`, `unknown` and never durable
  (the matrix rows); the turn Retry on a widget-originated last prompt
  (routed to `widget_action_retry`); a retry replay and a retry of a retry
  (`retry_chain`); the collector paused between root enumeration and unlink
  while a widget pins an old artifact (age refreshed, not unlinked). Turn attribution: a `TurnEnd` for another `turn_id` or another
  `worker_epoch` is ignored; agent output before the `TurnEnd` proves
  nothing; a returned prompt reopens the record and the next dispatch is a
  new attempt; a replacement worker's Welcome snapshot with
  `current_turn_id == T` keeps the record open, without it the record is
  `uncertain`; a replayed un-acknowledged `TurnEnded` after a restart
  resolves the open attempt exactly once. Write-once: a reconnect with a
  written-but-unacknowledged widget command does not retransmit it (the
  attempt is `uncertain`) while a never-written one is written once; a Stop
  during configuration startup settles `cancelled (stopped)` and nothing is
  relayed; a late `TurnEnd` after `uncertain` delivers and marks an existing
  retry a duplicate, and a retry admitted after the late proof is refused;
  a crash between cancellation intent and completion is settled at start.
  Artifacts: the collector paused **after inspecting the modification time**
  while publication pins the artifact under the lock does not unlink it.
  Packing: a `widgets.list` page with maximal titles and summaries fits the
  response budget and continues after the last included entry. Labels: the
  positive and negative label vectors; a single choice with
  `max_selected: 13` refused; duplicate initial values refused; the maximal
  record fixture within 12 KiB, constructed independently for every permitted
  selection; the maximal snapshot fixture within 48 KiB; the benchmark class
  maximum within 1 KiB. Ownership: a reconnect that drops a written widget
  command followed by an idle snapshot before `TurnStarted` does not release
  the dispatch guard or dispatch a second prompt; a second `TurnStarted` for
  the same turn from another epoch makes the attempt `uncertain`; a session
  on a worker generation without `widget_turn_identity` renders prompt
  controls inert and refuses `widget_action` with `capability_missing`.
  Index: a live snapshot arriving during a bootstrap fetch is kept because
  its `indexVersion` exceeds the index's, while a widget tombstoned before
  the index is removed; a restart reconstructs `indexVersion` from the row;
  a creation concurrent with a paged bootstrap is absent from every page of
  that index. Ownership: a dropped written command keeps ownership until its
  `CommandFate`; `rejected` with intact journals reopens the record as
  `returned`, `rejected {journals_unavailable}` and a terminated recipient
  without receipt settle `uncertain` with the future fenced, `running` keeps
  it held; a late
  unrelated `TurnEnd` does not release the dispatch guard; a generation
  fallback revokes open attempts, flips the projection inert and makes
  `widget_action` and `widget_action_retry` refuse `capability_missing`; a
  held command routed to a worker without the capability is refused and
  `returned`.
  Metadata: adversarial stop reasons and reasons are normalized into the
  alphabet before storage and the independent record stays within 12 KiB.
  Command fate: a crash between a `TurnEnded` and its ledger commit leaves
  the event unacknowledged, the worker replays it and settlement completes
  once; a held attempt found at start is settled through the fence from the
  worker journal; a never-observed command is tombstoned at the broker and
  fenced at the worker and its late arrival is refused; a broker that
  recorded `routed` and crashed before the write is resolved by the
  surviving idle worker's `never_received`; a lost negative ack is recovered
  from the broker ledger; a broker restart with lost journals answers
  `rejected {journals_unavailable: true}` and the attempt releases as
  `uncertain` with the future fenced; a direct API prompt during a held
  attempt is refused and the owning command passes with its permit; a late
  `TurnEnd` for an earlier widget turn during an ordinary turn updates the
  ledger and leaves the ordinary turn's guard alone, including after a
  fallback; a reconnect to a rolled-back broker makes a retained unwritten
  command a local proven-unsent return and the guarded frame is undecodable
  there; a retry admitted before `rejected` supersedes the original instead
  of duplicating it; duplicate and delayed fate answers across successive
  attempts change ownership nothing while a late `finished` still upgrades
  delivery and marks a retry duplicate; a Clear while an attempt is held
  still lets its terminal event settle (`cancelled (cleared)` on return,
  `delivered` or `uncertain` otherwise) without republishing; the
  `outstanding` list reflects held → released, retry admission (the original
  leaves, the retry enters) and group reset in `stateVersion` order; a query
  never releases ownership for an unreceived command; a revocation with a
  disconnected recipient epoch answers `pending_epochs` until that epoch
  reconnects and is fenced or its tree exit is proven; a prompt cancelled
  before its provider request journals `aborted` and the attempt is
  `returned` or `cancelled (stopped)`; a fallback requested while an attempt
  is held is refused until settled and the recovery reader answers the
  revocation. Aggregate settlement: one recipient `finished` while a second
  inventoried epoch is disconnected keeps ownership held (`pending_epochs`)
  until that epoch is fenced or its retained termination proof exists; two
  `finished` receipts record `duplicate_execution`; a `submitted_failed`
  disposition settles `uncertain` and never requeues; a query of a command
  that never reached the broker answers `unobserved` and releases nothing
  while the revocation answers `rejected`; the activator refuses a rollback
  below the floor while the Controller reports holds or cannot be reached; a
  replacement decision on a reset group persists `superseded_by` and a retry
  of the superseded intent is refused. Sealing: a `TurnEnd` arriving before
  a delayed second-recipient `routed` record does not release ownership —
  the revocation seals the inventory first and the settlement waits for the
  second recipient; a negative ack is settled through the same barrier
  before re-admission. Controller floor: a Controller build below the floor
  is refused activation while interactions are held; an older Controller
  started against the database finds the sessions' queue hold set and
  dispatches nothing; `reduceSettlement` vectors (completing plus
  terminated, cancelled plus submitted_failed, stop abort, startup abort,
  all never-received) are fixtures. Ungrouped nodes: a new decision on a
  node with a released uncertain record supersedes it; a held record on the
  node refuses the new decision `in_flight` and stays outstanding.
- Reducer tests mirroring `backgroundTasksReconcile.test.ts` T1 and
  `persistence_tests.rs` T3/T4: a cached open widget at seq 10 and a
  200-event reconnect tail must show the newest `state_version`; a cached
  open widget whose close fell outside the tail must show closed after
  bootstrap; a cached widget absent from a complete bootstrap is removed;
  a lookup answer from another epoch or generation is discarded.
- Mixed versions: a pre-widget Web bundle against a widget-publishing
  Controller shows the notice line (and the documented repeated lines on
  live updates); a current Web against a rolled-back Controller keeps
  rendering stored rows; interleaved streaming text, widgets and ordinary
  adapter notices keep their cursor behaviour.
- Browser conformance `widgets` suite in `BrowserProductTheme`, light and
  dark, 320 / 375 / 768 / 1100 px and a 400 px Desktop split pane: every
  node type, table `auto` at three and six columns, the worst-case document
  under the §2.1/§4 compositor rules, adversarial media (an 8 KiB PNG that
  decodes to 10,000² is refused by the Controller fixture; the Web never
  receives one), fold/Open, inert, decided and in-flight states, the preview
  line on every sending surface, the product confirm sheet, hostile
  identifiers in a choice submission, inline carrier pending → rendered →
  invalid fallback. Registered in `tools/cdp-browser-conformance.ts` with its
  exact test count.
- `just desktop-browser-gate` for enter/leave, Tab cycling inside the
  region, `Mod+Enter` after the preview, `Esc` and focus restoration;
  `just desktop-perf-profile` with a 20-widget transcript compared to
  baseline long-task totals.
- Help budget tests pass with the new topic; `contracts/agent-api-coverage.json`
  maps the new HTTP/WebSocket surfaces; structure and module-layer ratchets
  unchanged; the §11 tokenizer lifecycle measurement is committed with its
  traces and passes its thresholds.
- Physical iPhone: swipe across a transcript with mounted widgets without
  dropped frames; the preview line, confirm sheet and choice Send at the
  smallest and largest font settings — device evidence reported separately
  from emulation.

## 14. Decisions

| # | Decision | Rulings | Status in r3 |
|---|---|---|---|
| 1 | Full snapshots in anchored rows, table-backed reconciliation | r1 conditional; r2 F4/F5/F7 | §6.3 acknowledged watermark, generation fence, per-id lookup outcomes with placement, W1–W9 |
| 2 | Inline carrier read-only | r1 approve with retention rules; r2 F4 | §6.4 `event`-only sources; no retention change needed |
| 3 | Table parity | r1 accept as UX overhead, not savings; r2 F14 | §3 goal 2 and §11 claim bounded overhead, savings only against re-sends and prose answers; gate stops on failure |
| 4 | Controller-composed messages | r1 request changes; r2 F3/F12 | §8.2: labels, handle and revision only; pinned revisions; preview on every surface |
| 5 | No expressions or conditionals | approve | unchanged |
| 6 | Small `data` images | r1 conditional; r2 F13 | §5.4: PNG/JPEG/WebP, intrinsic ≤ 256², still, Controller-verified; SVG deferred; none in the inline carrier |
| 7 | Two validators, shared fixtures | r1 conditional; r2 F15 | §5.3 units, boundaries and vector classes fixed |
| 8 | `control` actions deferred | approve | unchanged |
| 9 | Row carried as a `notice` with `_cowboy.widget` | r2 approve with limited promise | §6.2 recognised before the notice branch; repeated legacy lines documented |
| 10 | Closure immutable, `reopen` removed | r2 approve with F8 clarification | §6.2/W5: document frozen, admitted interactions settle |
| 11 | Phase 1 catalog of 18 types | r2 approve | unchanged |
| 12 | Controller-assigned public handle replaces every agent identifier in messages | r3 request changes (R5) | §8.2: the choice question is part of the message; collision vector |
| 13 | Interaction ledger with explicit `uncertain` | r3 request changes (R1, R2, R7) | §8.3: turn-attributed proof, acknowledged CAS handoff, queue reconciliation, retry operation, crash-point matrix |
| 14 | Transcript generation as the Clear fence | r3 request changes (R3) | §6.3: Clear durable first; generation CAS at the database; dispatched work becomes uncertain |
| 15 | Decision-pinned revisions | r3 request changes (R4, R12) | §6.3: refs per revision retained for pinned ones; reads bound to `sha256` |
| 16 | Queued widget prompts are delete-only | r3 request changes (R2, R8) | §8.4: every mutation path refused; content hash verified at handoff |
| 17 | Delivery evidence is the turn the prompt started, never the echo | r4–r10 request changes (N1, G1/G2/G5, H1, r7 #1/#2, r8 #1–#4, r9 #1–#3/#5, r10 #1–#5) | §8.3: reads never release; revocation with recipient inventory and termination proofs; journal dispositions for every exit; idempotent release with late upgrades |
| 18 | A commit-acknowledged store operation gates every widget effect | r4 conditional; r5 conditional (G6) | §8.3 step 3 settles committed cancellation intents at start |
| 19 | `widget_action_retry` as the only Send-again | r4, r5, r7, r8, r11, r12 request changes (G5, r7 #2, r8 #6, F7, R5) | §8.3/§8.4: one `retryable` predicate; supersession per node group, never of held records |
| 20 | The design claims bounded overhead and no token saving | r4/r5 claim approved; evidence corrected (G9) | §3 and §11 agree; benchmark class defined |
| 21 | An update's bound is a constant for a benchmark update class, not for arbitrary patches | r5, r6, r7 request changes (G9, H6, r7 #6) | §11: class defined by escaped bytes; maximum built with quote and emoji values, bounded at 1 KiB |
| 22 | Clear is an identified, conditional, acknowledged writer operation | r5 approve | retained |
| 23 | A widget command is written to exactly one connection and never retransmitted | r6–r8, r11, r12 request changes (…, F1, R1) | §8.3: write-once at the Controller; the broker seals the inventory under the routing lock before any release |
| 24 | Artifact pin admission and collection share one lock | r6 approve | retained |
| 25 | A Stop during configuration startup settles the decision as cancelled, never relayed | r6 request changes (H2) | contract aligned |
| 26 | Interactive widgets require the `widget_delivery_v1` Machine capability bundle; display widgets do not | r7, r8 request changes (r7 #2/#4, r8 #5) | enforced at admission, retry, handoff and immediately before every write; `GuardedPrompt` frame; projected through `cowboy_delivery_capabilities` |
| 27 | Bootstrap absence is ordered by a per-session index version | r7 request changes (#5) | §6.3: durable counter and creation version written atomically; consistent paged read |
| 28 | Transport ownership is a per-attempt state distinct from delivery | r8–r12 request changes (…, F1, F6, R1, R5) | §8.3: released only by a sealed settlement; held records never superseded or dropped |
| 29 | The command-fate protocol spans worker journal and broker ledger | r8–r12 request changes (…, F1/F2/F4/F5, R1/R3/R4) | §8.3: sealed settlements, discriminated dispositions with abort causes, exhaustive reduction |
| 30 | Acknowledgement of a tracked `TurnEnded` waits for its ledger commit | r9 approve the ordering (#3 made settlement possible) | §6.3: identity-checked settlement across Clear |
| 31 | `prompt_traced` is the single prompt admission gate for every producer | r9 request changes (#4) | §8.3: attempt-bound single-use handoff permit |
| 32 | Transport settlement writes are identity-checked, not generation-checked, and never republish old-generation content (new) | — | §6.3; for review |
| 33 | The worker keeps a durable per-command journal and answers fences from it | r10–r12 request changes (r10 #2/#4, F5, R3/R4) | discriminated dispositions with abort causes preserved end to end |
| 34 | Ownership release requires a sealed inventory with every recipient terminal; reads never release | r11, r12 request changes (F1, F2, R1) | §8.3: no single-recipient shortcut; seal under the routing lock |
| 35 | Retained recovery-reader capabilities make settlement possible across generation changes | r11, r12 request changes (F3, R2) | §8.3, §12: Machine floor and Controller floor, durable queue hold for held sessions |
| 36 | Delivery evidence and ownership settlement are decided separately over the recipient inventory | r12 request changes (R1, R4) | §8.3: `reduceSettlement` is the exhaustive reduction; sealing is the ownership barrier |
| 37 | The Controller enforces its own recovery-reader floor and fences admission durably through the sessions' queue hold while attempts are held (new) | — | §8.3, §12; for review |
