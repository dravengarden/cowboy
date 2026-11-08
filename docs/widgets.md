# Cowboy Widgets: agent-authored structured UI over Cowboy MCP

Status: design for review (2026-11-08); nothing here is built or activated.
The Astra Max design review and its resolutions are recorded in
[Review findings](#review-findings) once the review has run. Statements marked
**[unverified]** are inferred from code or history and still need evidence.

Related: [`agent-api.md`](agent-api.md) (the MCP read/execute/receipt surface
this design extends), [`background-tasks.md`](background-tasks.md) (durable
transcript anchors and the compact-row-plus-inspector presentation the widget
row reuses), [`tool-results.md`](tool-results.md) (data is displayed, never
evaluated), [`ux-principles.md`](ux-principles.md) and
[`mobile-spatial-presentation.md`](mobile-spatial-presentation.md) (the
Mobile paint budget every widget renders inside), `web/src/desktop/FOCUS.md`
(keys).

## Summary

**Problem.** Agents report through Markdown prose. The transcript renders
tables, code and diagrams well on Desktop, but the data below shows the
dominant report shapes (a 5–8 row status or comparison table, a key/value
summary, a progress list, a question with options, a screenshot) are produced
as text that is expensive to emit, degrades on a phone, cannot be updated in
place and cannot be acted on except by typing a reply. Images are the clearest
gap: tools produced 1,087 image blocks in the sampled history while agent
prose linked an image six times.

**Direction.** One versioned, typed **JSON widget document** (`cowboy.widget/1`,
TypeScript contract in [`contracts/widget-v1.ts`](../contracts/widget-v1.ts)),
published and patched through the existing Cowboy MCP surface
(`widgets.*`), stored as one anchored transcript row per widget, rendered by a
Cowboy-owned React + MUI renderer with per-component Mobile and Desktop
behaviour, and acted on through actions that resolve to ordinary user
messages, deep links or confirmed controls. Agents author data, never code;
every node carries a Markdown fallback so an unknown component or an older
client still shows readable content.

## 1. Evidence from history

The numbers come from a read-only copy of the production Controller database
taken 2026-11-08 (304 sessions, 219,120 events, 20,316 agent messages;
queries kept under `$TMPDIR/cowboy-widget-analysis/q1–q4.sql` during the
analysis). They describe what agents already try to express, not what a widget
should look like.

### 1.1 Report shapes agents produce today

| Shape | Agent messages | Sessions | Notes |
|---|---|---|---|
| Bold key/value lines (`**key**: value`) | 3,034 | 118 | the most common structured shape |
| Numbered or bulleted lists | 4,561 / 1,855 | 145 / 111 | plans, steps, options |
| Markdown table | 1,814 | 107 | avg 1,439 chars; 60% have 5–8 rows; 83% have 2–3 columns |
| Fenced code | 894 | 58 | commands, config, logs |
| `file.ext:line` references | 666 | 110 | navigation targets |
| Question to the user | 1,609 | 95 | mostly free text; 260 elicitation forms in total |
| Mermaid diagram | 22 | 10 | rare, but used |
| Inline image link | 6 | 6 | versus 1,087 tool image blocks and 1,239 user screenshots |

Agent messages are short (p50 105 chars, p90 1,131, p99 2,962). The 1,814
table messages alone total 2.6 M characters. Re-encoding the 2,112 tables
they contain as the v1 `table` node (column labels plus row arrays, compact
JSON) is **7% larger at the median** (p10 +3%, p90 +14%): a table costs the
same to publish either way, so the token saving has to come from updates,
references and decisions (§10).

### 1.2 What users ask for

Of 5,884 human messages: review 349, UX/UI 335, "continue" 397, status or
progress 206, table 138, summary/report 138, compare 128, analysis 110,
deploy 109, screenshot 72, disk/memory/CPU 51. Status and comparison are the
recurring reading tasks; approval ("要不要我…") is the recurring decision.
4,529 agent messages mention progress or completion words, so a status report
is re-sent as a new message rather than updated.

### 1.3 Mechanisms the design can reuse

- Cowboy MCP is already used by agents: 855 `cowboy_read` and 231
  `cowboy_execute` calls in 48 sessions. Help is loaded per topic, results are
  bounded (16 KiB) and writes carry a stable operation id with a receipt.
- Cowboy already owns transcript rows that are replaced in place:
  `cowboy_subagent` and `cowboy_call_anchor` (durable `transcript_anchors`,
  first anchor wins, both reducers resolve through one index).
- Images already externalize to the content-addressed artifact store
  (`/api/artifacts/<sha>.<ext>`) and render through `ProtectedImage` with a
  lightbox; Calls return verified artifact paths rather than bytes.
- Elicitation (`_cowboy.elicitation`) proves a typed form can be drawn by the
  Web and answered as structured content; it is blocking and one-shot, which
  is right for a question and wrong for a dashboard.
- Managed Calls (`review` 293, `design_review` 76, `image_generate` 18) show
  that agents delegate work whose result is a document; a findings table or
  a generated image is a natural widget payload.

## 2. Goals and non-goals

Goals, in priority order:

1. **Read better on every surface.** A status table, comparison, key/value
   summary, progress list or image must be legible at 320 CSS px and dense on
   Desktop without the agent describing layout.
2. **Spend fewer tokens than Markdown for the same information**, both when
   publishing and when updating; an update must not re-send the document.
3. **Act without typing.** A widget can offer the next decision as buttons,
   choices or a small form; the answer reaches the agent as an ordinary,
   auditable user message.
4. **Extend without a new transport.** New component types, plugin-provided
   components and new carriers are additive; an unknown node degrades to its
   fallback, never to a broken row.
5. **Keep the Cowboy trust rule.** Agent content is data; the renderer owns
   all behaviour; a click never grants the agent more than the person doing
   the same action by hand.

Non-goals:

- Agent-authored React, HTML, CSS or scripts. They would have to be
  evaluated, which `tool-results.md` forbids, and they cost more tokens than
  the data they decorate.
- A second transcript, a generic dashboard product, or a replacement for the
  Markdown renderer. Prose stays Markdown; widgets are for structured
  content.
- Replacing elicitation or permission prompts. A blocking question keeps
  AskUserQuestion / `request_user_input`; a widget form is non-blocking and
  persistent.
- Live data bindings that make the Web poll Cowboy reads on the agent's
  behalf (considered for a later phase in §8; not in the first contract).

## 3. Format decision: a typed JSON document, not agent-written React

| Option | Verdict | Why |
|---|---|---|
| Agent writes React/TSX (or any code) | rejected | must be evaluated in the client; breaks the display-as-data rule and the plugin sandbox model; the largest token cost per rendered pixel; one bug per agent per message |
| Agent writes HTML/CSS | rejected | same trust problem (sanitizer treadmill), no MUI semantics, no Mobile adaptation |
| Markdown extensions only (fences, directives) | insufficient alone | cannot be updated in place, no receipts, no actions, no typed validation; kept as a read-only **inline carrier** (§4.4) because it costs no tool round trip |
| **JSON component tree, TypeScript contract, Cowboy renderer** | **chosen** | validated with `strictJson` at the trust boundary; versioned schema; each type renders with product MUI components and its own Mobile/Desktop behaviour; patchable; same model as elicitation forms and Call result documents |

"React and TS" therefore means: the **contract is TypeScript** (the source of
truth for the document shape, exported as `contracts/widget-v1.ts`, with
a derived JSON Schema for non-TS consumers), the **renderer is React + MUI**
owned by Cowboy, and the **agent emits JSON** conforming to the contract.
Discovery is on demand: `cowboy_read({help:"widgets"})` returns the contract as
TypeScript the same way other topics do, so nothing resident grows.

## 4. Document model

The contract is [`contracts/widget-v1.ts`](../contracts/widget-v1.ts);
worked examples with their Markdown equivalents are in
[`contracts/widget-v1.examples.ts`](../contracts/widget-v1.examples.ts) and
are held to the bounds by `contracts/widget-v1.test.ts`.

### 4.1 Shape

```ts
WidgetDocument { schema: "cowboy.widget/1"; title; summary?; icon?; root: Node;
                 presentation?: { inline?: boolean; height?: "xs"|"sm"|"md"|"lg" } }
Node = LayoutNode | ContentNode | StatusNode | MediaNode | InputNode   // 29 types
```

Every node may carry `id` (required when a patch, an action or a field
refers to it) and `fallback` (Markdown shown by a client that does not know
the type or a required property, and by text-only surfaces). Containers hold
`children`. There are no style, size or breakpoint properties: each
component owns its narrow-width and Desktop behaviour (§6), which is what
keeps documents short and keeps rendering decisions in the product.

### 4.2 Component catalog (v1)

| Group | Type | Covers (history §1) | Mobile | Desktop | Reuses |
|---|---|---|---|---|---|
| Layout | `stack` | grouping | row → column below 480 px unless `keep_row` | row/column, wrap | MUI `Stack` |
| | `grid` | dashboards | `columns.xs` (default 1) | `sm`/`md` columns | CSS grid |
| | `card` | one topic with status and actions | full width, actions wrap | dense card | `Paper`, `activityRowSurface` |
| | `section` | progressive disclosure | collapsed state paint-only | same | `Collapsible` |
| | `tabs` | alternatives | `SegmentedTabs` | MUI `Tabs` | existing primitives |
| | `divider` | | | | |
| Content | `text` | prose line with tone | inline Markdown subset | same | `Markdown` inline |
| | `markdown` | anything already expressible | existing renderer | same | `Markdown` |
| | `code` | commands, config | wrap on touch, copy | copy, highlight | `CodeBlock` cache |
| | `kv` | 3,034 bold key/value messages | one column | two columns | `MetricFacts` grid |
| | `list` | 4,561 bullet / 1,855 numbered messages | tones, icons, nesting | same | MUI `List` |
| | `table` | 1,814 table messages | `auto`: fit ≤ 3 short columns, else one card per row | sticky header, sort, min/max widths | `markdownTableFit`, `Collapsible` |
| | `json` | structured results | collapsible tree | same | `StructuredResult` |
| | `diff` | edits | | ± stats | `DiffView` |
| | `metric` | disk, quota, counts (51 resource asks) | large number, delta | same | `InfoMetricVisuals` |
| | `link` | 177 messages with links | | | typed `Ref` (§4.4) |
| | `file` | 666 `path:line` messages | opens Code Review | same | `FileChip` |
| Status | `status` | state chips | | | `StatusChip` |
| | `progress` | determinate or indeterminate | `transition: none` bar | same | `CapacityMeter` |
| | `steps` | plans, 1,722 plan updates | checklist | same | `PlanDock` presentation |
| | `alert` | callouts | | | MUI `Alert` |
| | `distribution` | parts of a whole | width-based stacked bar | same | `MetricDistribution` |
| Media | `image` | 1,087 tool images, Call artifacts | `ProtectedImage`, lightbox | same | artifact store |
| | `gallery` | several screenshots | two columns, paged lightbox | grid | `ImageLightbox` |
| | `diagram` | 22 Mermaid messages | source fallback | same | `MermaidDiagram` |
| Input | `button` | the recurring "要不要我…" | 44 px target, confirm sheet | `Mod+Enter` confirm | `ConfirmSheet` |
| | `choice` | 1,609 questions with options | radio / checkbox list | same | elicitation fields |
| | `form` | small structured answers | opens in the inspector sheet above the keyboard | inline | `ElicitationFields` kinds |

Deliberately absent from v1: conditionals and expressions (the Provider UI
`visible_when` family; an agent patches the document instead), colours or
spacing values, raw HTML, remote image addresses, execution-machine file
paths as image sources (§11, needs a Machine release), nested widgets,
custom icons, and a `chart` node (a bounded inline-SVG sparkline/bar is a
candidate for the second phase once `distribution` and `progress` prove
insufficient; there is no chart library in the Web bundle and none is added
by this design).

### 4.3 Bounds and validation

`WIDGET_LIMITS` is enforced identically by the Controller (Rust, before a
snapshot is stored) and the Web (TypeScript, before a document is rendered,
including inline-carrier documents the Controller never saw). Both parse with
the `strictJson` rules already used at the composition boundary (no
duplicate keys, bounded bytes and depth, valid Unicode). The main bounds:
document 24 KiB serialized (the `cowboy_execute` argument cap is 32 KiB),
400 nodes, depth 12, table 200 × 12, text 8 KiB, Markdown 16 KiB, fallback
2 KiB, inline image 8 KiB, 64 patch ops, prompt text 2 KiB, form values
4 KiB, 200 open widgets per session.

Validation is fail-closed for the shape (`invalid` with `{path, code,
message}` per error, so an agent can repair in one round trip) and
tolerant for the content: unknown node types and unknown properties are
accepted with a `warnings` entry, stored, and rendered through `fallback`.
That split is what lets a newer agent talk to an older Controller/Web pair
without losing the row (§8). Shared fixture vectors
(`tests/fixtures/widget-v1.json`, following `composition-v1.json`) hold the
two validators to the same answers.

### 4.4 References, not addresses

Links are typed `Ref` values (`url`, `session`, `file`, `document`, `call`,
`artifact`), never free-form strings that the client must classify. `url`
accepts `https:` only and opens through the existing remote-link
confirmation. Images use `ImageSource`: an artifact name the owner can
already fetch (a tool image or a Call result: `calls.result` already
returns `artifacts[].url`), an `event` reference to the image block of a
tool result stored in the same session (resolved by the Web from the stored
envelope, so no bytes travel through the agent), or a small inline `data`
image. The Web CSP is `img-src 'self' data: blob:` and treats an
agent-chosen remote address as a zero-click exfiltration channel
(`protectedImage.ts`), so a remote image is not a widget feature; it is a
`link`.

## 5. Transport

### 5.1 The `widgets` domain over Cowboy MCP

Reads (`cowboy_read`): `widgets.list({session_id?, after?, limit?,
include_closed?})` and `widgets.read(widget_id)` (document, revision,
recent interactions). Actions (`cowboy_execute`): `widgets.publish` and
`widgets.update` with `WidgetChange = replace | patch | close | reopen`.
Receipts: `cowboy_receipt({domain:"widgets", operation})`. Help:
`cowboy_read({help:"widgets"})` returns the contract as TypeScript and the
catalog the deployed Controller accepts; it must fit the existing help
budget tests (a topic under half the full SDK), so it carries the node
types and limits, not this document.

Controller touch points, from the existing pattern for adding a domain:
`READ_OPERATIONS` / `INVOKE_OPERATIONS`, the `sdk!` block, `HELP_TOPICS`
and `revision()` in `src/agent_api.rs`; the read namespace table in
`components/server/src/server/agent_api/runner.js` and `SessionHost::call`;
`execute` and receipt arms in `components/server/src/server/agent_api/actions.rs`;
`contracts/agent-api-coverage.json` for the new HTTP/WebSocket surfaces;
and `session.events`, whose `EventKind` is closed today and drops unknown
kinds, gains the widget kind so an agent reading history sees its own rows.
Writers use the same admission rules as `documents.mutate`: operator role,
same-owner live parent, no system or closing session; Superagent
coordinators stay confined to `superagent.*`; managed Call children have no
Agent API and publish through their parent.

Idempotency follows the `agent_api_operations` ledger: admit `(owner,
parent session, operation)` with the request hash before any effect, finish
with the result, `Existing` replays the stored result, `Conflict` refuses a
different request under a reused id, `unknown` is never proof of failure.
`expected_revision` makes `update` compare-and-swap; a lost update is a
`revision_conflict` carrying `current`, as with Draft documents.

### 5.2 One anchored row per widget

A published or updated widget is pushed as `Event::Update {sessionUpdate:
"cowboy_widget", widgetId, revision, observedSeq, document, interactions,
closed}` into the parent session. `anchor_key` in `src/transcript_anchors.rs`
gains `widget:<widgetId>`, so both reducers (the Hub hot log and the store
writer, `src/persistence.rs` `reduce_anchor`) rewrite every later snapshot
to the first row's seq, the durable `transcript_anchors` table keeps the
first anchor, and a Controller restart never starts a second row. Anchored
kinds also skip the reducer step that closes the streaming assistant text
slot, so a widget published mid-message does not split the bubble (a
non-anchored kind would).

The stale-row problem solved for subagents applies unchanged: a reconnect
delivers the newest 200 events, history pages are immutable and the
IndexedDB replica keeps what it saw. Widgets therefore follow the
**subagent** model rather than the stateless Call pointer: the Controller
stamps `observedSeq`, the bootstrap carries a `widget_state` list of open
widgets `(widgetId, revision, observedSeq)`, and a bounded
`POST /api/sessions/:id/widgets/lookup` resolves cached open widgets whose
newest revision fell outside the tail. A closed widget is terminal and is
never reconciled again. Invariants I1–I8 of `background-tasks.md` §4.2 apply
with "child" read as "widget".

### 5.3 Durable store

One new table, `session_widgets (session_id, widget_id, revision, closed,
seq, published_at_ms, updated_at_ms, document, interactions)`, is the
authoritative copy: `widgets.read`, `widgets.list` and the lookup read it;
the transcript snapshot is a projection. `publish`/`update` validate, write
the table row and the operation result in one transaction, then push the
snapshot through the Hub (hot log, batch writer, fan-out). If the process
dies between the two, start-up repair re-emits snapshots for table rows
whose `seq` is missing from `events` (origin `repaired`), the same repair
Call anchors run today. Interactions append to the table row and emit a new
snapshot without advancing `revision`, so a person clicking never makes the
agent's next `update` conflict. Artifact garbage collection, which today
keeps artifacts referenced from `events` and Call results, must also scan
`session_widgets.document`; relying on the event projection alone would
drop an image after a repair re-emission reorders references **[unverified
until the GC scan is read against the new table]**.

### 5.4 Inline carrier (read-only)

A ```` ```cowboy-widget ```` fence whose body is a `WidgetDocument` renders
through the same renderer, dispatched where the Mermaid fence is dispatched
in `web/src/MarkdownImpl.tsx`, with the same contract: while the fence is
still open during streaming the source shows as code; an invalid document
shows the code block with a one-line notice; a valid one renders. It has no
revision, no receipt, is not listed by `widgets.list`, cannot be updated in
place, and its interactive nodes render inert with their labels (the
Controller never saw the document, so it cannot validate an action against
it). It exists because it costs no tool round trip and works for any
Provider that renders Markdown through Cowboy, and because the inline shape
is what a model will produce when it has read one example; the review should
confirm the read-only rule rather than a second interaction path.

### 5.5 Web client

`derive.ts` gains a `widget` `RenderItem` keyed by the first envelope's seq
and indexed by `widgetId`, merging a later snapshot only when `observedSeq`
(else `revision`) is newer, exactly as `newerSubagent`/`mergeSubagent`.
`lastStreamTarget` skips widget rows so an arriving widget does not end the
agent's bubble; `sameRenderItem`, `itemProgressSignature` and `ItemView` gain
the exhaustive case; `applyEnvelope` merges a snapshot in place like
`tool_call_update`; `retainedEventCountForRows` keeps `key ===
String(env.seq)`. Any unknown update already counts as turn activity. Every
integration file (`MarkdownImpl.tsx`, `TranscriptItems.tsx`, `derive.ts`,
`Transcript.tsx`, `tools/blocks.tsx`) is at its structure-ratchet ceiling,
so each integration edit extracts first and adds zero net lines; the
renderer itself is a new `web/src/widgets/` module, lazy-loaded like
Mermaid, with one file per node family under the 800/150/40 limits.

## 6. Presentation

### 6.1 The row

Inline by default: the document renders inside the row within its height
budget (`sm` 24vh, `md` 40vh default, `lg` 60vh) using `Collapsible` with the
existing fade and 44 px Expand, then an **Open** control shows the whole
widget in the inspector (Mobile `DetentSheet` with the shared island
navigation and context summary; Desktop a Dialog, with the side-pane
placement planned for Call children as the later home). `presentation.inline:
false` gives the compact `TaskRow`-style line (title, summary, status, Open)
for widgets that are reference material rather than reading content. The
header shows title, status chip when the root is a card with status,
"updated 2m ago", and **closed** when interactions are inert. Row surface,
`minHeight` (40 Desktop / 44 Mobile), `overflow: hidden` and `contain`
follow `TaskRow` and `transcriptRowContainment` (rows with lazy images use
`layout`, settled rows `layout paint`). Component identities are stable
across snapshots (invariant 11 of the mobile contract): a patch re-renders
the changed node, never remounts the widget.

### 6.2 Mobile (iPhone)

Read above, operate below. Inside the transcript page a widget may not use
inner `transform`, shadows or filters on moving children, horizontal scroll
boxes that really overflow, `will-change` at rest or `setState` on
`touchstart`; selected states are paint-only. So: tables fit (`fit`: clip,
compact font, break-anywhere, the `markdownTableFit` ladder) when they have
at most three short columns, otherwise become one card per row with the
first column as the card title and the rest as a `kv` list; row stacks
become columns; grids use `columns.xs`; bars are width-based; images go
through `ProtectedImage` at the row's height budget and open in the
lightbox; galleries are two columns. Interactive nodes keep 44 × 44 CSS px
targets inside rounded controls at every font scale; `form` opens in the
inspector sheet so fields stay above the software keyboard and the sheet,
not the transcript, owns scrolling; `confirm` uses `ConfirmSheet`'s inset
card. Text follows the global font setting; control bounds do not shrink
with it.

### 6.3 Tablet (iPad)

iPad is the Mobile product on a wide surface, never Desktop. Grids use
`columns.sm`, tables stay tables, the inspector is a wide sheet, touch
targets are unchanged, and `keep_row` stacks stay rows.

### 6.4 Desktop

Dense and keyboard-first. The row is a `[data-desktop-widget-toggle]`
element: `Tab` selects it in the Conversation pane, `h`/`l`/`Enter` toggle
`aria-expanded` (fold ↔ full height), and a declared command opens the
inspector. Inside an expanded widget, controls are ordinary focusables in
document order; a form submits and a consequential button confirms with
`Mod+Enter`, never plain `Enter`; `Esc` closes the inspector. Hovering a
`prompt` button shows the exact message it will send. All keys are declared
through `useDesktopCommand` and classified with `desktopKeyIntent`;
components never read raw keys, menus get no local handlers, and no widget
may introduce a second shortcut grammar (`web/src/desktop/FOCUS.md`). Tables
get sticky headers, header-click sort when `sort` is set, and `min`/`max`
column hints; `kv` uses two columns; `grid` uses `columns.md`.

### 6.5 Theme

Only product theme tokens (`web/src/theme.ts`, `appearanceThemes.ts`):
tones map to `success`/`warning`/`error`/`info`, `neutral` to
`text.secondary`, `running` to `primary`; sizes through `desktopSize` and
`glyphSize`; radius 10; `CircularProgress` animates paint, bars set
`transition: none`. Acceptance renders in both light and dark through
`browserProductTheme.tsx`, never a bare `createTheme()`.

### 6.6 Text-only surfaces

Notifications, search, Explore pages and older clients show `title`,
`summary` and, per node, `fallback` or the node's own text (`alt` for
images, labels for inputs). An agent that wants a widget to read well in a
push notification writes a good `summary`.

## 7. Interaction

### 7.1 Actions

`prompt` sends a message to the agent; `open` navigates to a `Ref`; `copy`
copies text. A `confirm` on a button or form submit shows the title and text
in the product confirm sheet first. `destructive` only changes the button's
appearance; it does not change what the action may do, because no action may
do anything beyond those three verbs in v1.

### 7.2 A tap becomes a user message, composed by the Controller

The Web sends one WebSocket command, `widget_action {session_id, widget_id,
revision, node_id, interaction_id, values?}`. The Controller validates it
against the current `session_widgets` row: the widget is open, `revision`
matches what the person saw (a stale tap is refused with the current
snapshot, never re-targeted), the node exists and is interactive, `values`
match the form's fields and bounds. It then composes the user message
itself, so a client cannot forge the origin or the format:

```
[widget deploy-1b9f121] Activate
{"widget":"deploy-1b9f121","node":"go","values":{}}
```

The first line is the action's `text`; the JSON line is present when the
node carries values or `data`. The message enters the same durable queue as
`submit` (a busy session queues it for the next turn), stamped
`promptOrigin {actor:"human", source:"widget"}`, which is a new `source`
string and not a new actor. The interaction is appended to the widget row
(`queued`), flips to `delivered` when the turn consumes it (the queue
already reports delivery), or `failed`, and the Web shows those three states
on the control with the unsent → sent → stalled timing used for permission
replies. `interaction_id` is client-generated and reused on retry, so a
double tap or a reconnect never sends twice; one in-flight interaction per
node.

The agent learns about a tap by reading its next user message; there is no
callback API to poll. `widgets.read` also returns the recent interactions
for an agent resuming after compaction.

### 7.3 What a tap never does

It never applies a control, starts a Call, writes a file, changes a
preference or touches another session. The message it sends is visible in
the transcript as the person's own message, so a mislabelled button is
auditable after the fact and the composed text is visible before the fact
(Desktop hover, Mobile long-press, the inspector). Elicitation and
permission prompts are unchanged: a blocking question still uses the native
tool; the agent chooses which to use.

### 7.4 Inert states

Interactive nodes are inert when the widget is closed, the session is
exited or deleted, the person's snapshot is stale, the node is `disabled`,
or an interaction for that node is in flight. Display nodes always render.

## 8. Extensibility and compatibility

- **Additive catalog.** New node types and optional properties are minor
  changes under `cowboy.widget/1`; a changed meaning or a new required
  property is `cowboy.widget/2`. The help topic lists what the deployed
  Controller accepts, so an agent discovers the catalog instead of assuming
  it.
- **Unknown degrades, never breaks.** An unknown type renders `fallback`
  (or a one-line "unsupported `type`" chip when absent); unknown properties
  are ignored with a publish-time warning. Web and Controller release
  separately (a Web release restarts nothing), so a Web older than its
  Controller sees exactly this path, and the reader-first rollout rule of
  `background-tasks.md` §7 applies: ship the renderer before any writer
  depends on a new type.
- **Two validators, one fixture set.** `tests/fixtures/widget-v1.json`
  vectors are run by the Rust validator tests and `contracts/widget-v1.test.ts`;
  adding a type means: the TS union, the Rust validator arm, a renderer file
  under `web/src/widgets/`, fixture vectors (valid, invalid, oversize,
  fallback), a row in §4.2 and a case in the browser conformance suite.
- **Plugins contribute data, not renderers.** Every signed Plugin kind
  supplies data the core draws (Provider UI IR, workspace-extension views,
  editor-plugin panels), and the plugin design states that a renderer cannot
  be registered by a signed template. Widgets keep that rule: a later
  `widget_template` Plugin kind may ship bounded documents with parameter
  slots that an agent instantiates (`widgets.publish({template, params})`),
  rendered by the same core catalog. It needs its own contract because
  Plugin installation is Machine-scoped while widgets render on the Web.
- **Carriers are additive.** MCP action and inline fence share one
  document; a future carrier (a Call child's result document rendered as a
  widget in the parent) reuses the schema and the anchored row.

## 9. Trust boundary

- Agent content is data at both boundaries: `strictJson`, closed unions
  rendered by exhaustive switches ending in `assertNever`, no HTML, no
  styles, no scripts, typed `Ref`s instead of URL strings, images only from
  the artifact store, a stored event, or a bounded inline `data` image.
- An artifact name is fetchable by any authenticated product user who knows
  it (existing property of `/api/artifacts/`). Widgets do not widen it: an
  agent can only name artifacts it learned in its owner's sessions, and the
  Controller rejects names that do not exist in the store with a warning so
  the image shows its `alt`.
- The person's authority is the ceiling: a tap produces a user message or a
  navigation the person could perform by hand. The composed message is
  bounded (2 KiB text, 4 KiB values) and recorded as the person's message.
- Limits are per session: 200 open widgets, one publish or update per
  second sustained (burst 5), 30 interactions per minute, one in-flight
  interaction per node. Exceeding them is `invalid`/`refused`, never silent
  truncation.
- Same-owner scope as all session reads; coordinators and managed children
  excluded (§5.1). Nothing is evaluated: `markdown` nodes go through the
  renderer that already escapes HTML; `diagram` through Mermaid's strict
  mode with source fallback.

## 10. Token budget

Measured on the sampled history and the worked examples
(`contracts/widget-v1.examples.ts`; "tok" is a rough proxy, one per CJK
character and one per four bytes of other text, until the tokenizer
measurement below runs **[unverified]**):

| Case | Markdown today | Widget | Note |
|---|---|---|---|
| 2,112 historical tables | median 322 chars | **+7%** median (p10 +3%, p90 +14%) | publishing a table is parity |
| Status table example | 449 B, ~117 tok | 935 B, ~234 tok (2.0×) | adds tones, byte formatting and a confirmable restart button the prose only implies |
| Comparison example | 617 B, ~187 tok | 933 B, ~233 tok (1.25×) | the `choice` node replaces the trailing question |
| Approval example | 291 B, ~81 tok | 985 B, ~246 tok (3.0×) | `kv` + `steps` + two confirmable buttons |
| Update after activation | 185 B, ~52 tok re-sent line | **179 B, ~45 tok patch** | parity with a one-line status; a fifth of re-sending the card |

Honest reading: **publishing** a widget costs 1.25–3× a terse Markdown reply
in these examples, because it carries the controls and semantics the prose
leaves implicit; a table alone is parity. The contract test only guards
against drifting beyond 4× (`widget-v1.test.ts`). The saving is structural
and comes from what happens after the first message:

1. **Updates are patches.** 4,529 agent messages in the sample mention
   progress or completion words; a status widget is patched (three `set`
   ops ≈ 45 tok) instead of re-sent (the card ≈ 246 tok, a table message
   median 322 chars plus the prose around it). One widget per topic,
   updated in place, is the intended pattern; a widget that is never
   updated and never tapped should have been Markdown.
2. **References replace content.** An image is an artifact or event
   reference (~60 bytes), not a description and not bytes; a file is a
   `path:line` chip; a session or Call is a typed link.
3. **Decisions are taps.** The recurring "要不要我…" exchange becomes a
   button whose reply is a Controller-composed message (~80–150 bytes) the
   agent does not have to parse from prose, and the person does not have
   to type.
4. **Nothing resident grows.** The MCP tool definitions stay as they are;
   the `widgets` help topic is loaded once per session on demand and must
   stay within the existing per-topic budget test; `publish`/`update`
   results are a few dozen bytes plus warnings.

Agent guidance (for the help topic and `AGENTS.md`): prose stays Markdown;
use a widget for a table or status the agent will report again, for an
image or screenshot the person should see, and for a decision with fixed
options. Do not wrap a one-line answer in a widget. Measurement plan before
Phase 1 ships: replay the three archetypes through the Claude and Codex
tokenizers on real history samples (`tools/widget-token-measure.ts`) and
report tokens per published widget, per patch and per interaction next to
the Markdown baseline **[unverified until run]**.

## 11. Phases

| Phase | Content | Components | Release boundary |
|---|---|---|---|
| 1 | Contract v1; display, status and media nodes; `button` and `choice` with `prompt`/`open`/`copy`; `widgets.publish/update/list/read`, receipts, help topic; `session_widgets` table, anchored `cowboy_widget` rows, bootstrap `widget_state` + lookup, start-up repair; `widget_action` command and Controller-composed messages; inline read-only carrier; Mobile and Desktop renderers; `session.events` widget kind; GC scan | Controller + Web (one new table) | no Machine release |
| 2 | `form` fields and values; `control` actions behind a confirm sheet and an explicit allowlist; pin a widget to the Desktop side pane / Mobile dock; `chart` (inline SVG sparkline/bar) if `distribution`/`progress` prove insufficient; image sources from execution-machine files through the verified artifact delivery used by image Calls | Controller + Web; **Machine release** for file sources | explicit maintenance boundary |
| 3 | `widget_template` Plugin kind (bounded documents with parameter slots); Call children's result documents rendered as parent widgets; live `Ref` badges (a `call` ref shows the Call ledger state) | Controller, Web, Plugin SDK | separate reviewed contracts |

Phase 1 ships readers before writers: the Web renderer and `cowboy_widget`
reducer cases land first, then the Controller domain, then the help topic
and agent guidance.

## 12. Acceptance (Phase 1)

- Fixture vectors (`tests/fixtures/widget-v1.json`): valid examples, each
  bound exceeded by one, duplicate keys, unknown type with and without
  `fallback`, unknown property, stale `expected_revision`, patch into a
  missing node, patch that breaks a bound. Run by Rust and Bun.
- Reducer tests mirroring `backgroundTasksReconcile.test.ts` T1 and
  `persistence_tests.rs` T3/T4: a cached open widget at seq 10 and a
  200-event reconnect tail must show the newest revision; a store-writer
  restart between two snapshots must not create a second row.
- Browser conformance `widgets` suite in `BrowserProductTheme`, light and
  dark, 320 / 375 / 768 / 1100 px: every node type, table `auto` at three
  and six columns, fold/Open, inert and in-flight states, inline carrier
  pending → rendered → invalid fallback. Registered in
  `tools/cdp-browser-conformance.ts` with its exact test count.
- `just desktop-browser-gate` for the row toggle, inspector, `Mod+Enter`
  confirm and `Esc`; `just desktop-perf-profile` with a 20-widget transcript
  compared to baseline long-task totals.
- Help budget tests pass with the new topic; `contracts/agent-api-coverage.json`
  maps the new HTTP/WebSocket surfaces; structure and module-layer ratchets
  unchanged.
- Physical iPhone: form fields above the keyboard inside the sheet, swipe
  across a transcript with mounted widgets without dropped frames — device
  evidence reported separately from emulation.

## 13. Decisions for review

1. **Anchored full-snapshot row with table-backed reconciliation** (subagent
   model) rather than a pointer row read on demand (Call model). Recommended:
   history pages and the offline replica keep rendering widgets; the table is
   authoritative.
2. **Inline carrier is read-only.** Recommended; a second interaction path
   that the Controller cannot validate is not worth the convenience.
3. **Table parity accepted.** Recommended; the contract test guards against
   drifting into a markedly more verbose shape.
4. **Interactions are Controller-composed user messages**, not elicitation
   answers and not a callback API. Recommended for auditability and because
   it needs no Provider or Machine change.
5. **No expressions or conditionals in v1.** Recommended; the agent patches.
6. **`data` images ≤ 8 KiB; file-path sources deferred** to the Machine
   release that reuses image-Call artifact delivery.
7. **Two validators with shared fixtures** (the Provider UI / Provider SDK
   precedent) rather than generating one from the other.
8. **`control` actions deferred** until an allowlist and confirm contract
   exist; `prompt`, `open`, `copy` only in v1.

## Review findings

Pending: the Astra Max design review runs on this revision; findings and
resolutions are recorded here before any implementation starts.
