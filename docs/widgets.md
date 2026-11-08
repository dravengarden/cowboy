# Cowboy Widgets: agent-authored structured UI over Cowboy MCP

Status: design for review, revision 2 (2026-11-08); nothing here is built or
activated. Revision 1 received an Astra Max design review ("request changes",
16 findings, 6 blocking); every finding is resolved below and none was
rejected ([§1](#1-review-findings-and-resolutions)). Statements marked
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
authoritative in one Controller table, projected as one anchored transcript
row that older clients still read as a plain notice, rendered by a
Cowboy-owned React + MUI renderer whose components adapt to the width they
are given, and acted on through taps that become user messages composed by
the Controller **from what the person saw**. Agents author data, never code
and never hidden instructions. Phase 1 is deliberately narrow: grouping,
tables, key/value, lists and steps, status and progress, images and typed
references, buttons and choices.

## 1. Review findings and resolutions

Astra Max, 2026-11-08, on revision 1 (`83195d3b4`).

| # | Finding (severity) | Resolution | Section |
|---|---|---|---|
| 1 | Agent-authored instructions acquire human authority: a button label can differ from the hidden `prompt.text`; the confirm text was agent-authored (P1) | `prompt` carries no text or data. The Controller composes the message from the visible label, the selected option labels and ids, the widget id and revision; option values are identifiers, not prose; the confirm sheet is product-owned and previews the exact message, with any agent note labelled as the agent's; widgets never use permission surfaces | §8.1–8.2, §10 |
| 2 | Queued decisions lose the revision and target the person approved (P1) | Every interaction records revision, state version, visible label, selection labels and the composed message immutably; the message names the revision; the Web marks a decision made on an earlier revision | §8.2–8.3 |
| 3 | Stable interaction ids do not give exactly-once admission over a write-behind queue (P1) | A durable `widget_interactions` ledger is the outbox: admission, message and request hash commit in one transaction; a relay moves admitted rows into the session queue under `cmid = widget:<interaction_id>` and re-relays after restart; a replay returns the original outcome before any revision check | §6.3, §8.3 |
| 4 | Start-up repair misses committed updates whose row exists at an older revision (P1) | `state_version` plus a `published_version` watermark per widget; publication is at-least-once from the table; a sweep republishes any row whose watermark lags, without a restart | §6.3 |
| 5 | Repair can resurrect content a transcript Clear removed (P1) | Clear tombstones the session's widgets in the same transaction that deletes events and anchors; tombstoned rows are never published; pending interactions are cancelled; deletion cascades | §6.3 |
| 6 | Artifact retention does not cover the inline carrier or event-image references (P1) | Artifact references are the full `/api/artifacts/<name>` string the collector already extracts; the collector scans `session_widgets` and extracts references inside Markdown text; `event` sources are resolved to artifacts at publish | §5.4, §6.3, §6.4 |
| 7 | Document revision cannot order interaction snapshots (P2) | `state_version` advances on every mutation and orders publication, lookup and client merge; `revision` stays the agent's CAS key | §6.2–6.3 |
| 8 | Closed widgets declared terminal yet reopenable; open-only bootstrap strands offline devices (P2) | `reopen` removed; closure is immutable; bootstrap and lookup cover closed widgets too, bounded by a 500-widget session cap; widget invariants W1–W8 replace the borrowed subagent list | §6.2, §6.6 |
| 9 | Pre-widget clients ignore unknown update kinds and never reach the fallback (P2) | The row is carried as a `notice` (title + text projection) with the widget under `_cowboy.widget`: an older or rolled-back client draws the line, a current client draws the widget | §6.2, §9 |
| 10 | Unknown-node acceptance contradicts the closed union and `assertNever` (P2) | Controller fails closed: unknown type without `fallback` and unknown properties are `invalid`. The Web parses into an explicit `unsupported` runtime node before narrowing; actions inside it are inert | §5.3, §9 |
| 11 | Duplicate keys are collapsed by MCP ingress before the strict parser (P2) | Guarantee narrowed to the bytes each boundary receives: MCP ingress bounds bytes/depth and collapses duplicates (last wins); the inline carrier rejects them; a stricter ingress parser is a separate rollout | §5.3 |
| 12 | A full `widgets.read` can exceed the 16 KiB result budget (P2) | `read` takes `select` and a text window over the compact document JSON with `state_version` consistency; lookup has its own aggregate budget | §6.1 |
| 13 | Expanded controls conflict with Conversation keyboard ownership (P2) | A widget's controls form an entered region: `l`/`Enter` enters, `h`/`Esc` leaves and restores focus, Tab cycles inside, `Mod+Enter` sends; choices never send on selection | §7.5 |
| 14 | Author layout hints can defeat narrow-pane guarantees (P2) | `grid`/`tabs` deferred; `keep_row` and table `fit` are subordinate to measured container width and minimum control geometry; modality and density separated | §7.2 |
| 15 | Node and byte limits do not bound rendering work (P2) | Aggregate budgets (2,000 cells, 1,000 items, 24 images), a 20-row transcript preview, inspector pagination, worst-case fixtures in acceptance | §5.3, §7.1, §13 |
| 16 | Phase 1 contradicted the token-saving goal (P2) | Goal rewritten: publication is UX overhead, savings are per update and per decision; same-language measurements; repeated-table evidence; narrowed catalog; lifecycle measurement plan | §3, §11 |

Rulings on the revision-1 decisions are folded into §14.

## 2. Evidence from history

The numbers come from a read-only copy of the production Controller database
taken 2026-11-08 (304 sessions, 219,120 events, 20,316 agent messages;
queries kept under `$TMPDIR/cowboy-widget-analysis/` during the analysis,
to be committed as `tools/widget-history-measure/` with Phase 1). They
describe what agents already try to express, not what a widget should look
like.

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
- Images already externalize to the content-addressed artifact store
  (`/api/artifacts/<sha>.<ext>`) and render through `ProtectedImage` with a
  lightbox; Calls return verified artifact paths rather than bytes.
- Elicitation (`_cowboy.elicitation`) proves a typed form can be drawn by the
  Web and answered as structured content; it is blocking and one-shot, which
  is right for a question and wrong for a dashboard.
- Draft documents prove the CAS-with-receipt write model and the per-owner
  transaction that admits, writes and records in one step.

## 3. Goals and non-goals

Goals, in priority order:

1. **Read better on every surface.** A status table, comparison, key/value
   summary, progress list or image must be legible at 320 CSS px and dense
   on Desktop without the agent describing layout.
2. **Cost no more per update or per decision than Markdown, and show the
   overhead honestly at publication.** Publishing a widget is UX overhead
   (§11); patching it and answering it must be cheaper than re-sending and
   re-typing, measured on complete exchanges in the same language.
3. **Act without typing, without delegating authority.** A widget can offer
   the next decision as buttons or choices; the answer reaches the agent as
   an ordinary, auditable user message that contains only what the person
   saw and chose.
4. **Extend without a new transport.** New component types are additive; an
   unknown node degrades to its fallback, an older client still gets a
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

Every node may carry `id` (required when a patch, an action, a group or a
value refers to it) and `fallback` (Markdown shown by a client that does not
know the type, and by text-only surfaces). Containers hold `children`. There
are no style, size or breakpoint properties: each component owns its
behaviour at the width it is given (§7.2), which keeps documents short and
rendering decisions in the product.

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
| Input | `button` | the recurring "要不要我…" | 44 px target, product confirm sheet | entered region, `Mod+Enter` | `ConfirmSheet` |
| | `choice` | 1,609 questions with options | radio / checkbox list + Send | same | elicitation fields |

**Candidates for later phases**, each needing its own evidence and a
reviewed addition: `grid` and `tabs` (dashboards), `markdown`, `code`,
`json`, `diff` and `diagram` (these would duplicate renderers prose already
reaches; prose stays Markdown), `metric` and `distribution`, a bounded
inline-SVG `chart`, and `form` (persistent inputs; elicitation covers the
blocking case). Deliberately absent at any phase: conditionals and
expressions (an agent patches instead), colours or spacing values, raw HTML,
remote image addresses, nested widgets, custom icons.

### 5.3 Bounds and validation

`WIDGET_LIMITS` is enforced identically by the Controller (Rust, before a
row is stored) and the Web (TypeScript, before a document is rendered,
including inline-carrier documents the Controller never saw). Bounds measure
the stored compact JSON: UTF-8 bytes, Unicode scalar chars, node depth
(nested nodes, ≤ 12) and JSON depth (nested values, ≤ 32). Tree bounds:
document 24 KiB, 400 nodes, table 200 × 12, text 8 KiB, fallback 2 KiB,
inline image 8 KiB, 64 patch ops. Aggregate budgets bound rendering work,
not only the tree: 2,000 table cells, 1,000 list/kv/step/option items and
24 images per document; the transcript draws at most 20 rows of a table and
the inspector pages the rest; per session, 200 open and 500 total widgets.
A patch applies all ops or none and the result must satisfy every bound.

Validation is **fail-closed at the Controller**: wrong shape, a bound
exceeded, an unknown node type without `fallback`, an unknown property on a
known node, a prose option value, a duplicate or malformed id, a `url` ref
that is not `https:`, an artifact string that does not name an existing
artifact — each is `invalid` with `{path, code, message}` so an agent
repairs in one round trip. The **Web is tolerant** of what the Controller
admitted but the client does not know: an unknown type parses into an
explicit `unsupported` runtime node (type, `fallback`, raw) before the tree
is narrowed to known nodes, so the closed `Node` union and the exhaustive
`assertNever` switch remain honest; it renders the fallback and keeps any
action inside it inert; an unknown property on a known node is ignored.

Parser boundaries are stated, not assumed. MCP ingress parses arguments into
`serde_json::Value` (bytes ≤ 32 KiB, bounded depth, duplicate keys collapse
with the last value winning) before the widget validator runs; the validator
re-serializes and bounds that canonical form. The inline carrier reaches the
Web as text and is parsed with `strictJson` (duplicates rejected). A stricter
ingress parser would change every MCP action and is a separate rollout, not
part of this design. Shared fixture vectors
(`tests/fixtures/widget-v1.json`, following `composition-v1.json`) hold the
two validators to the same answers on both databases.

### 5.4 References, not addresses

Links are typed `Ref` values (`url`, `session`, `file`, `document`, `call`,
`artifact`), never free-form strings the client must classify. `url` accepts
`https:` only and opens through the existing remote-link confirmation.
Images use `ImageSource`: an `artifact` given as the full
`/api/artifacts/<64 hex>.<ext>` string, which is exactly the string form
`artifacts::collect_references` extracts from stored JSON; an `event`
reference to the image block of a tool result stored in the same session,
which `widgets.publish` resolves to an `artifact` source before storing (the
inline carrier resolves it at render time and shows `alt` when the event is
gone); or a small inline `data` image (PNG, JPEG, WebP or SVG, ≤ 8 KiB,
drawn in an `<img>` box of at most 128 px; SVG is never inlined as markup,
GIF is excluded so no frame budget is needed). The Web CSP is `img-src
'self' data: blob:` and treats an agent-chosen remote address as a
zero-click exfiltration channel (`protectedImage.ts`), so a remote image is
not a widget feature; it is a `link`.

## 6. Transport

### 6.1 The `widgets` domain over Cowboy MCP

Reads: `widgets.list({session_id?, after?, limit?, include_closed?})`
(default 50, `page_required` above) and `widgets.read(widget_id, {select:
"meta"|"document"|"interactions", offset?, chars?})`. A valid document can
be 24 KiB and the read budget is 16 KiB, so `document` is a text window over
the compact JSON (default 1,000 chars, max 4,000) carrying `state_version`;
a changed version between pages means restart, as `body_revision` does for
Drafts. Actions: `widgets.publish {widget_id?, document}` and
`widgets.update {widget_id, expected_revision, change}` with `WidgetChange =
replace | patch | close`. Receipts: `cowboy_receipt({domain:"widgets",
operation})`. Help: `cowboy_read({help:"widgets"})` returns the contract as
TypeScript and the catalog the deployed Controller accepts; it must fit the
per-topic budget test, so it carries types, limits and two composition
examples, not this document.

Widget ids are unique within a session, agent-chosen (`WIDGET_ID_PATTERN`)
or generated (`w-<base32>`); publishing an existing id returns `exists`
with the current revision rather than replacing it (use `update`). Writers
use the admission rules of `documents.mutate`: operator role, same-owner
live parent, no system or closing session; Superagent coordinators stay
confined to `superagent.*`; managed Call children have no Agent API and
publish through their parent. Idempotency follows the `agent_api_operations`
ledger: admit `(owner, parent session, operation)` with the request hash
before any effect, finish with the result, replay returns the stored
result, a different request under a reused id is `Conflict`, `unknown` is
never proof of failure. `expected_revision` makes `update`
compare-and-swap; a lost update is `revision_conflict` with `current`.

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
`_cowboy.widget` and draws the widget. This replaces the revision-1
`cowboy_widget` kind, which such clients would have ignored entirely. On an
older client the notice ends the agent's streaming bubble, as any notice
does; a current client treats widget rows as out-of-band (§6.5).

`anchor_key` in `src/transcript_anchors.rs` gains `widget:<widgetId>` for
notices carrying `_cowboy.widget`, so both reducers (the Hub hot log and the
store writer, `src/persistence.rs` `reduce_anchor`) rewrite every later
snapshot to the first row's seq, the durable `transcript_anchors` table
keeps the first anchor, and a Controller restart never starts a second row.
Snapshots are ordered by `stateVersion`, which advances on every mutation of
the widget (publish, update, close, interaction); `revision` advances only
on agent writes and is the CAS key. Newest `stateVersion` wins everywhere:
Hub reduction, store reduction, bootstrap, lookup and client merge.

Closure is immutable: `close` is the last change a widget accepts, so a
closed widget has exactly one final state and a device that cached an open
state is corrected by any later list of `(widget_id, state_version,
closed)`.

### 6.3 Durable store

One new table, `session_widgets`, is authoritative; the transcript row is a
projection:

```sql
session_widgets(session_id REFERENCES sessions ON DELETE CASCADE, widget_id,
                revision, state_version, published_version, closed,
                cleared_at_ms NULL, seq NULL, published_at_ms, updated_at_ms,
                document, decided, PRIMARY KEY (session_id, widget_id))
widget_interactions(session_id, widget_id, interaction_id, request_hash,
                    revision, state_version, node_id, group_id, label,
                    selection, message, state, reason, cmid, created_at_ms,
                    PRIMARY KEY (session_id, interaction_id))
```

- **Writes.** `publish`/`update` validate, write the row (`state_version +
  1`, `revision + 1` for agent writes) and the operation result in one
  transaction, then publish.
- **Publication is at-least-once from the table.** A snapshot push through
  the Hub (hot log, batch writer, fan-out) sets `published_version =
  state_version` only after the batch writer confirms; a sweep (on start and
  every few seconds while any row lags) republishes rows with
  `published_version < state_version`, with origin `repaired`. This covers
  a crash after commit and before publication, and the case revision 1
  found: a row whose transcript copy exists at an older version. No sequence
  reservation is needed because the anchor fixes the row's position and
  `stateVersion` fixes its order.
- **Clear.** `clear_events` already deletes the session's events and anchors
  and dismisses its Calls in one transaction; it also sets `cleared_at_ms`
  on every widget of the session and cancels its `admitted`/`queued`
  interactions (`reason: cleared`). A cleared widget is never published,
  listed only with `include_deleted`-style intent, and refuses `update` and
  `widget_action` with `cleared`. Session deletion cascades both tables.
- **Interactions** (§8.3) append a `widget_interactions` row and bump
  `state_version` without touching `revision`, so a person's tap never makes
  the agent's next CAS write conflict.
- **Artifact retention.** `collect_references` today extracts whole string
  values of the form `/api/artifacts/<name>` from `events` and Call results.
  It gains two inputs and one generalisation: scan `session_widgets.document`
  (so a published widget pins its images for as long as the row lives, 24 h
  grace after tombstoning as today), and extract references that occur
  inside longer strings (so an inline-carrier document embedded in an
  `agent_message_chunk` pins its images for as long as that event lives).
  `event` sources are resolved to `artifact` at publish, so a widget never
  depends on another event's retention **[unverified: the substring scan is
  a GC behaviour change to be measured against the current scan cost]**.

### 6.4 Inline carrier (read-only)

A ```` ```cowboy-widget ```` fence whose body is a `WidgetDocument` renders
through the same renderer, dispatched where the Mermaid fence is dispatched
in `web/src/MarkdownImpl.tsx`, with the same contract: while the fence is
open during streaming the source shows as code; an invalid document shows
the code block with a one-line notice; a valid one renders. It has no
revision, no receipt, is not listed by `widgets.list`, cannot be updated in
place, and its interactive nodes render inert with their labels (the
Controller never saw the document, so it cannot validate an action against
it). Unknown types render `fallback` or the unsupported chip. Artifact
references inside it are retained by the containing event (§6.3). It exists
because it costs no tool round trip and is what a model produces after
reading one example; it is not a second interaction path.

### 6.5 Web client

`derive.ts` gains a `widget` `RenderItem` for notices carrying
`_cowboy.widget`, keyed by the first envelope's seq and indexed by
`widgetId`, merging a later snapshot only when `stateVersion` is newer (the
`newerSubagent`/`mergeSubagent` shape). `lastStreamTarget` skips widget rows
so an arriving widget does not end the agent's bubble; `sameRenderItem`,
`itemProgressSignature` and `ItemView` gain the exhaustive case;
`applyEnvelope` merges a snapshot in place like `tool_call_update`;
`retainedEventCountForRows` keeps `key === String(env.seq)`. The bootstrap
carries `widget_state {epoch, entries, complete}` for the session's widgets
(open and closed, ≤ 500); the client reconciles every cached widget whose
`state_version` is older through `POST /api/sessions/:id/widgets/lookup`
(≤ 200 ids, ≤ 256 KiB aggregate, `complete: false` when truncated). Every
integration file (`MarkdownImpl.tsx`, `TranscriptItems.tsx`, `derive.ts`,
`Transcript.tsx`, `tools/blocks.tsx`) is at its structure-ratchet ceiling,
so each integration edit extracts first and adds zero net lines; the
renderer itself is a new `web/src/widgets/` module, lazy-loaded like
Mermaid, one file per node family under the 800/150/40 limits.

### 6.6 Widget invariants

- **W1 Identity.** `(session_id, widget_id)` names one widget for the life
  of the session; a transcript row exists for it at most once.
- **W2 Order.** `state_version` is total per widget; any reader keeps the
  highest it has seen and never regresses.
- **W3 Authority.** `session_widgets` is the truth; a snapshot, a cached row
  or a history page is a projection and may lag but never contradicts a
  higher `state_version`.
- **W4 Publication.** Every committed `state_version` is eventually
  published while the widget is not cleared; republication is idempotent.
- **W5 Closure.** `closed` is monotonic; a closed widget accepts no change
  and no interaction.
- **W6 Clear.** A cleared widget is never published again and its pending
  interactions are cancelled; repair distinguishes `cleared_at_ms` from a
  missing projection.
- **W7 Interaction.** An interaction is admitted at most once per
  `interaction_id`; its record, message and queue entry are consistent, and
  the message names the revision it was made on.
- **W8 Completeness.** Bootstrap and lookup say whether they are complete;
  a client treats an incomplete answer as "reconcile again", never as
  "nothing changed".

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
The header shows title, the card status chip, "updated 2m ago", **closed**
when interactions are inert, and a glyph marking the row as the agent's
proposal. Row surface, `minHeight` (40 Desktop / 44 Mobile), `overflow:
hidden` and `contain` follow `TaskRow` and `transcriptRowContainment` (rows
with lazy images use `layout`, settled rows `layout paint`). Component
identities are stable across snapshots (mobile invariant 11): a patch
re-renders the changed node, never remounts the widget.

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
controls at every font scale; the product confirm sheet is `ConfirmSheet`'s
inset card; a choice's Send control sits under its options. Text follows the
global font setting; control bounds do not shrink with it.

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
choice's selection (a choice never sends on selection); `Esc` or `h`
**leaves**, restoring focus to the row toggle. A declared command opens the
inspector; `Esc` closes it and restores focus. All keys are declared through
`useDesktopCommand` and classified with `desktopKeyIntent`; components never
read raw keys; menus get no local handlers; no second shortcut grammar.
Hovering a `prompt` button shows the exact message it will send. Tables get
sticky headers, header-click sort when `sort` is set, and `min`/`max` column
hints.

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

### 8.1 Actions

`prompt` sends a message to the agent; `open` navigates to a `Ref`; `copy`
copies text and the toast shows what was copied. A `prompt` carries **no
text and no data**: the message is composed from what the person saw. A
`destructive` button always confirms; `confirm` on a button or a choice's
Send is an agent note that appears, labelled "note from the agent", inside a
product-owned sheet whose title and body are the product's: "Send to the
agent:" followed by the exact composed message. Widgets never use the
permission overlay, its sticky card or the `␣1…9` approval commands; an
agent proposal and a native permission request are visibly different
surfaces.

### 8.2 A tap becomes a user message composed from visible content

The Web sends `widget_action {session_id, widget_id, revision,
state_version, node_id, interaction_id, values?}`. The Controller validates
against the current `session_widgets` row: open and not cleared, `revision`
and `state_version` equal to what the person saw (a stale tap is refused
with `current` so the client redraws, never re-targeted), the node exists
and is interactive, the group is undecided, `values` are option ids present
in the referenced choices. It then composes the message itself:

```
[widget deploy-1b9f121 r1] Activate the built cowboy-web release and verify it
{"widget":"deploy-1b9f121","revision":1,"node":"go"}
```

Line one is the button label, or for a choice "Decision: " followed by the
selected labels. Line two names the widget, the **revision**, the node and,
for a choice, `values` (option ids) and `labels` (what those ids read as).
Nothing else can appear: no agent-supplied text, no data, no hidden fields;
`contracts/widget-v1.test.ts` holds the example to that. The agent is told
in the help topic that a decision applies to the revision it names and that
it must re-read the widget before acting if the revision has moved.

### 8.3 Durable admission and relay

The session queue persists write-behind (`StoreWrite::UpdatePending`), so
the queue cannot be the ledger. Admission is a transaction on
`widget_interactions`: insert `(session_id, interaction_id)` with the
request hash, revision, state version, label, selection, the composed
message and `state: admitted`, bump the widget's `state_version`, and reply
`widget_action_result {outcome: admitted}`. A repeated `interaction_id` with
the same hash returns `replayed` with the original outcome **before** any
revision check; a different hash is `conflict`. A relay then moves
`admitted` rows into the session queue as an ordinary prompt with
`cmid = widget:<interaction_id>` and `promptOrigin {actor:"human",
source:"widget"}` (a new `source`, not a new actor), marks the row `queued`
with that cmid, and after a restart re-relays rows still `admitted`,
checking the queue's known cmids first so a message is never enqueued twice
**[unverified: whether the queue itself refuses a duplicate cmid or the
relay must check `cmids_of`]**. The row becomes `delivered` when the turn
consumes the message (the queue already reports dispatch), `cancelled` when
the person removes it from the queue or a Clear fences it, `failed` when the
session refuses it. The Web shows admitted → queued → delivered on the
control with the unsent → sent → stalled timing used for permission replies.

### 8.4 Decision groups, pending input, the queue

Interactive nodes that share a `group` (and a card's `actions`, implicitly)
are mutually exclusive: the first admitted interaction records the group in
`decided`, siblings render inert with "decided" until a later agent write
changes the nodes. A choice keeps the person's unsent selection locally
across snapshots when the node id and its options are unchanged; when they
change, the selection resets and the control shows "updated". A queued
message is an ordinary queue row: the person may edit or delete it there
before delivery (`cancelled`); there is no expiry beyond the queue's own.
The Web marks a decision whose widget revision has since moved as "made on
r1" so a later reader sees what was approved.

### 8.5 What a tap never does

It never applies a control, starts a Call, writes a file, changes a
preference or touches another session. The message is visible in the
transcript as the person's own message, bounded (label ≤ 200 chars,
selection ≤ 50 options), and previewed before sending.

### 8.6 Inert states

Interactive nodes are inert when the widget is closed or cleared, the
session is exited or deleted, the person's snapshot is stale, the node is
`disabled`, its group is decided, or an interaction for that node is in
flight. Display nodes always render.

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
  on both databases; wire semantics (§6), unsupported-node handling (§5.3)
  and parser boundaries (§5.3) are fixed before the second validator is
  written. Adding a type means: the TS union, the Rust validator arm, a
  renderer file under `web/src/widgets/`, fixture vectors (valid, each bound
  exceeded by one, fallback), a row in §5.2 and a browser conformance case.
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
  bounded inline image drawn as an opaque `<img>`.
- **No hidden instruction reaches the agent with the person's authority.**
  The only agent-to-agent channel through a tap is the node id and option
  ids, both identifiers; labels are what the person read; the composed
  message is previewed and recorded.
- An artifact name is fetchable by any authenticated product user who knows
  it (existing property of `/api/artifacts/`). Widgets do not widen it: the
  Controller admits only names that exist, and an agent can only learn names
  from its owner's sessions.
- Limits are per session: 200 open and 500 total widgets, one publish or
  update per second sustained (burst 5), 30 interactions per minute, one
  in-flight interaction per node. Exceeding them is `invalid`/`refused`,
  never silent truncation.
- Same-owner scope as all session reads; coordinators and managed children
  excluded (§6.1). `text` goes through the inline Markdown renderer that
  already escapes HTML.

## 11. Token budget

Measured on the sampled history and the worked examples
(`contracts/widget-v1.examples.ts`, Markdown and widget in the same
language; "tok" is a rough proxy, one per CJK character and one per four
bytes of other text, until the tokenizer measurement below runs
**[unverified]**):

| Case | Markdown today | Widget | Note |
|---|---|---|---|
| 2,112 historical tables | median 322 chars | **+7%** median (p10 +3%, p90 +14%) | publishing a table is parity |
| Status table example | 458 B, ~115 tok | 855 B, ~214 tok (1.9×) | adds tones, byte formatting and a confirmable restart button the prose only implies |
| Comparison example | 499 B, ~125 tok | 897 B, ~224 tok (1.8×) | the `choice` replaces the trailing question |
| Approval example | 286 B, ~72 tok | 903 B, ~226 tok (3.2×) | `kv` + `steps` + two buttons |
| Update after activation | 197 B, ~49 tok re-sent line | **179 B, ~45 tok patch** | parity with a one-line status; a fifth of re-sending the card |
| The person's answer | a typed sentence, then the agent parses it | **131 B, ~33 tok** composed message | fixed shape; no parsing |

Honest reading: **publishing** a widget costs 1.8–3.2× a terse Markdown
reply in these examples, because it carries the controls and semantics the
prose leaves implicit; a table alone is parity. The contract test guards
against drifting beyond 4×. The saving is structural and per exchange:

1. **Updates are patches.** 10% of historical tables re-send a header
   already sent in the same session (up to seven times); a patch costs what
   changed (~45 tok) instead of the card (~226 tok). A widget that is never
   updated and never answered should have been Markdown, and the help topic
   says so.
2. **References replace content.** An image is an artifact or event
   reference (~70 bytes), a file a `path:line` chip, a session or Call a
   typed link.
3. **Decisions are taps.** The recurring "要不要我…" exchange becomes a
   button whose reply is a fixed-shape message (~33 tok) the agent does not
   parse from prose and the person does not type.
4. **Nothing resident grows.** The MCP tool definitions stay as they are; the
   `widgets` help topic is loaded once per session on demand within the
   existing per-topic budget; `publish`/`update` results are a few dozen
   bytes plus warnings.

Measurement before Phase 1 ships (`tools/widget-token-measure.ts`,
committed with the fixtures): replay the three archetypes as **complete
lifecycles** — help discovery, publish, one receipt read, two patches, one
tap and the agent's read of the resulting message — through the Claude and
Codex tokenizers, against the Markdown lifecycle of the same exchange in the
same language (report, re-sent report, typed answer), and report tokens per
lifecycle. The design is justified only if the lifecycle is cheaper by the
second update or the first decision; if it is not, Phase 1 narrows further
to display-only widgets and the inline carrier.

## 12. Phases

| Phase | Content | Components | Release boundary |
|---|---|---|---|
| 1 | Contract v1 (18 types); `button`/`choice` with `prompt`/`open`/`copy`; `widgets.publish/update/list/read`, receipts, help topic; `session_widgets` and `widget_interactions`, notice-projected anchored rows, publication watermark and sweep, Clear tombstones, bootstrap `widget_state` and lookup; `widget_action` admission, relay and result; inline read-only carrier; Mobile and Desktop renderers with the entered-region keys; `session.events` widget kind; GC inputs | Controller + Web (two new tables) | no Machine release |
| 2 | `form` fields if elicitation proves insufficient; `control` actions behind the product confirm and an explicit allowlist; pin a widget to the Desktop side pane / Mobile dock; `metric`, `distribution`, `grid`, `tabs`; image sources from execution-machine files through the verified artifact delivery used by image Calls | Controller + Web; **Machine release** for file sources | explicit maintenance boundary |
| 3 | `widget_template` Plugin kind; Call children's result documents rendered as parent widgets; live `Ref` badges (a `call` ref shows the Call ledger state); `chart` | Controller, Web, Plugin SDK | separate reviewed contracts |

Phase 1 ships readers before writers: the Web renderer and reducer cases
land first, then the Controller domain, then the help topic and agent
guidance.

## 13. Acceptance (Phase 1)

- Fixture vectors (`tests/fixtures/widget-v1.json`): the examples; each
  bound exceeded by one, including the aggregate budgets; a worst-case valid
  document (twenty 20-row tables, 24 images); duplicate keys at the inline
  boundary; unknown type with and without `fallback`; unknown property;
  prose option value; stale `expected_revision`; a patch into a missing
  node; a patch that breaks a bound. Run by Rust (PostgreSQL and SQLite) and
  Bun.
- Fault injection on both databases: crash after commit before publication
  (sweep republishes); store-writer restart between two snapshots (one
  row); Clear with an admitted interaction (cancelled, never relayed);
  Controller restart with `admitted` rows (relayed once); duplicate
  `widget_action` (replayed, original outcome); stale tap (refused with
  `current`).
- Reducer tests mirroring `backgroundTasksReconcile.test.ts` T1 and
  `persistence_tests.rs` T3/T4: a cached open widget at seq 10 and a
  200-event reconnect tail must show the newest `state_version`; a cached
  open widget whose close fell outside the tail must show closed after
  bootstrap.
- Mixed versions: a pre-widget Web bundle against a widget-publishing
  Controller shows the notice line; a current Web against a rolled-back
  Controller keeps rendering stored rows.
- Browser conformance `widgets` suite in `BrowserProductTheme`, light and
  dark, 320 / 375 / 768 / 1100 px and a 400 px Desktop split pane: every
  node type, table `auto` at three and six columns, the worst-case document
  under the §2.1/§4 compositor rules, fold/Open, inert, decided and
  in-flight states, the product confirm sheet, inline carrier pending →
  rendered → invalid fallback. Registered in `tools/cdp-browser-conformance.ts`
  with its exact test count.
- `just desktop-browser-gate` for enter/leave, Tab cycling inside the
  region, `Mod+Enter`, `Esc` and focus restoration; `just desktop-perf-profile`
  with a 20-widget transcript compared to baseline long-task totals.
- Help budget tests pass with the new topic; `contracts/agent-api-coverage.json`
  maps the new HTTP/WebSocket surfaces; structure and module-layer ratchets
  unchanged; the tokenizer lifecycle measurement (§11) is committed with its
  traces.
- Physical iPhone: swipe across a transcript with mounted widgets without
  dropped frames; the confirm sheet and choice Send at the smallest and
  largest font settings — device evidence reported separately from
  emulation.

## 14. Decisions

| # | Decision | Review ruling (r1) | Status in r2 |
|---|---|---|---|
| 1 | Full snapshots in anchored rows, table-backed reconciliation | conditional on durable projection, ordering and lifecycle | §6.3 watermark + sweep, `state_version`, Clear tombstones, W1–W8 |
| 2 | Inline carrier read-only | approve, with retention and unsupported-content rules | §6.4, §6.3 GC inputs, §5.3 `unsupported` |
| 3 | Table parity | accept as UX overhead, not as demonstrated savings | §3 goal 2 rewritten; §11 lifecycle measurement is a ship gate |
| 4 | Controller-composed messages | request changes: visible intent only | §8.1–8.2: no `prompt.text`/`data`, identifier option values, product-owned preview |
| 5 | No expressions or conditionals | approve | unchanged |
| 6 | Small `data` images; file sources deferred | conditional: bound decoding; SVG opaque | §5.4: ≤ 8 KiB, 128 px box, no GIF, `<img>` only |
| 7 | Two validators, shared fixtures | conditional on identical wire semantics first | §5.3 parser boundaries and unsupported handling fixed; §6 wire types complete in the contract |
| 8 | `control` actions deferred | approve | unchanged; `prompt` safeguards added |
| 9 | Row carried as a `notice` with `_cowboy.widget` (new) | — | §6.2; for review |
| 10 | Closure immutable, `reopen` removed (new) | — | §6.2; for review |
| 11 | Phase 1 catalog narrowed to 18 types; `form`, `grid`, `tabs`, code/data/diagram renderers deferred (new) | — | §5.2, §12; for review |
