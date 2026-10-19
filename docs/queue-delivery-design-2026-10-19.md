# Queue delivery and send interaction — design proposal

Status: implemented and activated, 2026-10-19. See section 12 for acceptance
evidence and remaining physical-device verification limits. Scope: Cowboy
Composer, Queue, Session Settings, shared scheduling/API contract. Revision:
revised after the requested Claude Fable Max review; see section 11. Sections
1–11 retain the reviewed proposal; section 12 records implementation decisions
and the exact native evidence. This is an external Cowboy product design, not a
Columbus execution-kernel addition.

## 1. User constraints and decisions

The user wants technical terminology and complete mobile/desktop support. Idle +
empty queue sends immediately without a picker. Running + send opens a picker to
explicitly select delivery. Queue and Session Settings both configure delivery
even when the queue is empty. Interrupt & send joins the send surface. Desktop
uses the existing keyboard/leader/Vim system; mobile keyboard behavior must be
intentional. A default preference highlights a choice; it never bypasses the
requested running-state picker.

## 2. Native evidence and capability boundary

Checked on 2026-10-19:

- Codex CLI: Enter steers the active turn; Tab queues for the next turn. App
  Server turn/steer requires expectedTurnId and does not start a new turn. The
  public API does not expose a selectable strict next-tool-call boundary.
- Claude Code: ordinary Enter enqueues. A message queued during tool execution
  can be consumed after those tools finish, inside the same turn. Remaining
  messages dispatch after the turn. Ctrl+Enter sends now; work may move to the
  background, or the turn may be interrupted depending on the active operation.
- Inspected Claude SDK 0.3.300 declares priority now/next/later. ACP 0.93.0
  chooses now, or later during permission/elicitation, and does not expose the
  priority choice. A priority name alone does not establish consumption timing.
- Cowboy's shipped current_turn path is capability- and turn-fenced, with
  durable receipts. It is not proof of a strict tool boundary.

Sources:

- [Codex commands](https://learn.chatgpt.com/docs/developer-commands)
- [Codex App Server](https://learn.chatgpt.com/docs/app-server#steer-an-active-turn)
- [Claude queue](https://code.claude.com/docs/en/interactive-mode#when-claude-code-sends-what-you-queued)
- [Existing delivery contract](agent-api.md)
- [Accepted runtime release](releases/agent-runtimes-send-2026-10-18.md)

Candidate policies: Steer (default), Next tool boundary, Next turn.
Tool-boundary support is an implementation research gate, not preemptively
rejected or claimed shipped. Probe native extension surfaces first. If a
Provider's Steer and tool-boundary behavior are identical, display one choice
with the precise semantics. Unsupported choices retain a short capability reason
in Settings, never silently become interrupt. A running picker offers only
executable actions plus a concise reason for a configured unavailable mode.
Interrupt is a one-shot action, never a stored policy.

## 3. Orthogonal state and ownership

| Dimension         | States / owner                                                      |
| ----------------- | ------------------------------------------------------------------- |
| Execution         | idle, running, awaiting_input, recovering; server/runtime authority |
| Queue scheduler   | active, paused; server-persisted user intent                        |
| Policy preference | steer, tool_boundary, next_turn; versioned session preference       |
| Connectivity      | connected, reconnecting, offline; client observation                |
| Delivery receipt  | queued, dispatching, runtime_accepted, not_delivered, unknown       |
| Composer surface  | editing, choosing_delivery, confirming_interrupt, submitting        |

An awaiting permission/elicitation is active, not idle. Provider callbacks keep
their own authority. Queue pause controls subsequent automatic dispatch only.
Runtime accepted is receipt acknowledgement, not proof that the model consumed
the input. Do not label that acknowledgement Delivered. Durable queue admission
and any observed model consumption are separate evidence.

Default policy and manual pause are stored in DB with session preferences,
shared across clients and editable without starting/waking a worker. Do not add
a TOML or localStorage policy copy. Policy and batch edits use CAS; conflicts
are explicit. Pause is an idempotent absolute set, so a newer queue item does
not make a safety pause fail. Resume is explicit against the observed pause
epoch: a stale resume cannot undo a later pause. Stable operation receipts
suppress replayed writes. Show Pause requested until server acknowledgement;
offline clients cannot claim a pause succeeded. Current code already persists
manual pause; retain that behavior. Maintenance/capacity holds are independent
read-only reasons. Existing sessions keep next_turn and their manual pause.
Capability comes from the exact connected worker and negotiated host protocol.

## 4. One send gateway

| Observed state at activation             | Primary action                                      |
| ---------------------------------------- | --------------------------------------------------- |
| Idle, empty queue, scheduler active      | Send directly                                       |
| Running or awaiting_input                | Open delivery picker, including with an empty queue |
| Idle, queue paused                       | Enqueue into paused queue; no auto-resume           |
| Idle, pending queue items                | Append in order; never jump ahead implicitly        |
| Recovering / offline / reconnecting      | Preserve draft; do not submit runtime actions       |
| Same draft revision has ambiguous submit | Observe that operation; do not resend that revision |

Execution is derived, not the raw Status name: Status::Running means ready/idle;
Status::Busy means processing. Map Starting to preparing: only durable next-turn
enqueue, no active-turn actions. Exited/dormant with no worker uses ordinary
send-to-wake when the scheduler permits; settings alone do not wake it.
Crashed/Interrupted use the existing recovery admission and expose its outcome,
never pretend they are ordinary idle or remove the existing recover/send path.
Busy without an addressable active turn (including autonomous continuation)
still opens the picker, but offers Next turn and independent Stop with an
active-turn-unavailable reason. Awaiting permission/elicitation stays active;
only native-supported delivery is allowed, with no implied approval.

The idle paused Composer keeps Enqueue as its primary action and exposes Send
once (queue stays paused) in its disclosure menu/palette. Queue row Send/S is an
explicit send of that existing item: idle sends once even while paused; busy
opens the picker. It never means enqueue the item again. The explicit Send once
preview names any overtaken next-turn items.

The admission gateway is shared by pointer activation, Mod+Enter, fullscreen
Composer, command palette and Queue row Send, with source=compose or existing
item and explicit once intent. A queued-item send uses its existing identity and
cannot clone a second item. Explicitly named commands such as Interrupt & send
may go directly to their required confirmation.

The running button is labelled Send… on desktop, with a visible disclosure
indicator and accessible name Choose delivery on mobile. Idle is Send. Paused
idle is Enqueue (Paused). Do not rely on tooltip alone to expose a state change.
Keep button placement stable; attachment processing/composition/submission
disables activation with a specific reason. Remove the standalone current-turn
and force icons; retain Save Draft, attachments, scheduling and queue priority
where they are useful without adding another competing Send action.

## 5. Running delivery picker

Anchor to the composer action, not a settings page. Three logical regions:

- Send this message: Steer / Next tool boundary / Next turn. A row activation
  submits once. Highlight the persisted default if executable; otherwise
  highlight safe Next turn with a visible unavailable-default explanation.
- Queue state: an independent Pause/Resume control and current queue count. This
  control cannot accidentally submit the draft.
- Interrupt & send…: destructive one-shot action, separated from normal modes.

The mode label explains the actual Provider timing, without promising that a
generation cannot be preempted. Next turn is always the safe held policy;
current-turn actions require exact worker + turn capability and readiness.

Each invocation captures the source session, composer revision, attachment set,
worker epoch and turn identity. Any draft/content/attachment revision change
while the popup is open closes it without sending and preserves the edit; the
next activation captures a fresh snapshot. Composition commit is also a revision
change. The popup never changes target when tabs switch. A switch closes it and
preserves the source draft. Live state can disable an action, but cannot
relocate focus onto a different action or trigger a submit. Enter/Mod+Enter on a
now-disabled focused row is a no-op with a live-region reason. Never select the
next executable row automatically. When the original turn ends, current-turn and
interrupt actions become unavailable; keep Next turn available as explicit
ordinary enqueue/send. If an idle direct-send races a new running turn, refuse
without effect and open the picker; do not silently steer or interrupt. If the
native turn changes after a selection, return not_delivered/unknown as
appropriate, never retarget a new turn. One input gesture has at most one
admission.

Paused means automatic dispatch stops. Selecting Next turn enqueues held. All
explicit once actions (Steer, Next tool boundary, Send once, Queue row Send,
Interrupt & send) admit only this message while leaving the scheduler paused.
While paused, their labels include Once; queue remains paused. For tool
boundary, this grants one dispatch when the selected native boundary arrives,
not a bulk resume. A later pause does not revoke that explicitly armed one-shot;
its row shows Armed once and offers withdrawal only if the provider can prove
that no effect occurred. Pause acknowledgement includes already-claimed/armed
items; pause never promises to recall them. Automatic submissions using the
session default have no such bypass and remain held until explicit
resume/release.

Interrupt confirmation names the target session/turn and the message preview. It
interrupts that turn, runs this draft once, retains other queued items and
preserves pause. Confirmation is explicit and turn-fenced. No onClick/key event
can propagate from the picker into the confirmation. Stop without a draft
remains the independent task-control action; it is not replaced by Interrupt &
send.

## 6. Queue and Session Settings

Queue header: Queue · N | Default: Next turn v | Pause/Resume. The word Default
distinguishes the preference from policies already captured by queued items;
each active-turn row always names its target and delivery policy. When empty, a
compact delivery control stays available without an empty list taking vertical
space. Settings -> Session -> Queue exposes the same preference and pause state,
with capability reasons. A sleeping/new session is configurable without starting
a worker. There is one authoritative preference, three views.

Choosing a per-send mode does not change the default. The picker links to the
same small Default delivery editor (no checkbox that couples submit with a
preference mutation). Preferences are explicit, independently acknowledged.

Changing the default affects new items only. Existing waiting items retain their
captured policy. A general Apply policy operation is not allowed to convert an
unbound future prompt into a current-turn injection. Applying Next turn to
selected unclaimed items is a revision-checked batch edit. Steer selected / Send
selected at tool boundary is a separate explicit batch delivery: preview
selected ids/count, capture the current target and queue revision, preserve
selection order, and reject if any item is claimed, unknown or the target
changes. No partial implicit fallback. Default edits alone never release a
backlog. Resume/release shows count and lanes before dispatch; a lane with
expired targets cannot be silently retargeted.

One Queue UI represents two delivery lanes, with FIFO within each lane:

- Current turn: Steer and tool-boundary items bound to a specific turn/epoch.
  They may run before Next turn items; the selected mode explicitly authorizes
  that. A tool-boundary item ahead of a Steer item blocks that later item until
  its boundary, unless the user explicitly moves/releases the later item first.
  The picker and row show Waiting behind N current-turn items in that case; they
  must not promise immediate injection while appending behind a held item.
- Next turn: ordinary prompts, automatically drained in queue order as turns
  complete. Reorder and Move to front affect this lane, never imply
  interruption.

Show Current turn and Next turn sections only when both contain entries; one
lane stays compact. Item counts and mode selection remain in one Queue header.
This resolves the impossible global FIFO that would make tool-boundary items
miss their target behind a next-turn head.

Queue rows reuse edit/delete/reorder/move behavior. Policy edits are disabled
while claimed/uncertain. Only one admission owner can claim any item; active
turn injections are serialized per target, and turn-start dispatch has a
separate exclusive session reservation.

If a bound target ends while its item is provably still unclaimed, move that
item to Needs decision, retaining its content and provenance. This is no longer
in either drain lane and does not block unrelated ordinary work. Explicit Send
as next turn appends it to the next-turn tail; show that destination. Cancel/
move to Drafts is also available. No auto-retarget or implicit fallback.

An unknown effect is different: the session's automatic dispatcher is held
pending reconciliation, since starting another turn could duplicate or reorder
an unobserved effect. Display Receipt unknown — auto-dispatch held. This is a
safety hold, not the user's manual pause. Explicit new user actions remain
available with that hold disclosed and separate operation/content identity; they
cannot replay the uncertain item. Reconciliation clears the hold only after
proving an outcome. Resume clears only manual pause, never a receipt or
maintenance hold. Settings and editing remain usable.

## 7. Desktop keyboard design

Use the central command registry and current FOCUS.md primitives, with current
browser/OS collision audits. Never implement component-local global hotkeys.

| Context                | Key                                         | Action                                                                                               |
| ---------------------- | ------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Composer               | Mod+Enter                                   | Idle send / running open picker / paused enqueue                                                     |
| Composer               | Enter, Shift+Enter, Tab                     | Keep existing text, newline, completion and focus semantics; do not copy Codex's global Tab-to-queue |
| Picker                 | Up/Down; j/k when Vim navigation owns focus | Move among executable rows; disabled reasons remain accessible                                       |
| Picker                 | Home/End; gg/G with Vim navigation          | First/last executable row                                                                            |
| Picker                 | Enter                                       | Activate focused normal delivery choice, or open Interrupt confirmation                              |
| Picker                 | fresh Mod+Enter                             | Activate focused normal delivery choice, same explicit selection                                     |
| Picker                 | Esc                                         | Close only picker, restore source selection, scroll and Vim mode                                     |
| Composer               | Alt/Option+Enter                            | Open Interrupt & send confirmation; disabled with reason when idle                                   |
| Interrupt confirmation | Mod+Enter                                   | Confirm; plain Enter cannot confirm                                                                  |
| Interrupt confirmation | Esc                                         | Return to picker if opened there, otherwise Composer                                                 |
| Runtime task or picker | Mod+.                                       | Explicit Stop command closes picker and runs Stop; never submits draft                               |
| Anywhere eligible      | Mod+Shift+P                                 | Search Choose delivery, Default delivery, Pause/Resume queue, Interrupt & send                       |
| Workspace              | existing leader Q / comma                   | Queue region / Settings; empty Queue focuses delivery control                                        |

Opening activation must be consumed on keydown and keyup as appropriate.
Repeats, double-clicks and held Mod+Enter/Alt+Enter cannot both open and submit.
A second fresh key press can choose the highlighted default quickly. Menu
ownership blocks background leader and session-digit shortcuts while the menu is
active. Mod+. is an explicitly registered picker-owned Stop command, not leaked
background handling; show its hint. Tab/Shift+Tab follow native popup focus
behavior and never enqueue. A text field inside settings owns characters; j/k or
digit labels never hijack typing.

Preserve insertion selection, Vim Insert/Normal state, undo history and scroll.
Opening a menu is not a synthetic Esc that changes editor mode. IME composition,
candidate commit guard and paste/attachment preparation outrank send shortcuts.
An event that ends composition cannot also open or execute a delivery action. No
auto-focusing another pane after successful send; restore Composer ownership
without resurrecting a dismissed overlay or overwriting a newer draft.

Keep the existing Alt+Enter chord, but make the confirmation-only contract
normative across all source surfaces and update stale Force push documentation.
Current Composer already opens forceAnchor for that chord; FOCUS.md's terse
force-push wording is not evidence that Composer currently sends without a
confirmation. Idle Alt+Enter never becomes Send once silently; use that named
command or disclosure instead. Do not add bare s/b/n/i shortcuts over native
menu typeahead; arrows/j/k plus Enter are already a fast, scoped selection path.

## 8. Mobile, keyboard and accessibility

Choose one executable focus contract: opening the mobile delivery picker
transfers DOM focus into an accessible popup and intentionally dismisses the
software keyboard. Do not claim that a focused menu and focused CodeMirror can
coexist. This revises the earlier keep-keyboard suggestion. Idle direct Send
does not open a picker or deliberately dismiss the keyboard.

Use the existing compact Obsidian-style sheet on phones; keyboard collapse and
sheet layout follow the shared visual-viewport/native inset owner. Reserve the
composer's draft/selection and never replace its editor. No synthetic keyboard
height, hidden focus proxy, repeated focus() or new IME workaround. Controls
have

> =44px hit targets, safe-area bounds and scrolling in short landscape
> viewports. Inter-button empty space follows the shared sheet's
> hit-transparency contract.

Dismiss/cancel returns focus and selection in the initiating user gesture, using
the existing editor restore primitive without changing Vim state. Restore the
software keyboard if it was open and the user did not explicitly dismiss it;
where the browser refuses restoration, keep content/selection and provide a
visible Continue editing action. Success after a network response must not
programmatically reopen the software keyboard or steal focus. On failure,
content remains and Continue editing is available. No async focus loops.

Outside taps only dismiss and cannot click through; opening pointer-up cannot
activate an option. Fullscreen Composer keeps the same behavior. With a hardware
keyboard attached (including iPad on the mobile shell), Mod+Enter invokes the
same gateway; Up/Down, Home/End, Enter and Esc work inside the popup. Mod+Enter
confirms an Interrupt dialog; plain Enter never does. Mobile hardware support
uses shared input/overlay commands, not the desktop pane/leader controller.

VoiceOver/TalkBack use actual focusable controls, announced title/current
choice, disabled reasons and explicit close. No assistive-technology detection
switches the architecture. Focus restoration, viewport motion, external
keyboards and screen readers require device acceptance before shipping. A future
keyboard- retaining accessory would be a separate proven accessible primitive,
not a fallback that breaks this contract. Durable enqueue acknowledgement never
claims runtime delivery.

## 9. Persistence, receipts and API

Keep separate stable identities for message, submit operation, preference edit
and queue revision. A server-side claim transaction arbitrates queue drain,
policy edit, delete, pause and delivery. A paused/claimed/unknown row cannot be
re-sent by another client or drain. Settings CAS conflicts do not submit
messages.

For Next turn enqueue, durable queue acknowledgement may clear only the exact
submitted draft revision. For Steer/tool-boundary/Interrupt current-message
actions, preserve the existing no-clear-before-runtime-acceptance contract. An
armed boundary item has a linked source draft; it is not a second sendable copy.
Native acknowledgement is called Runtime accepted until consumption is actually
observed. Late acknowledgements never erase newer text/attachments.

Pending/unknown locks only that operation and draft revision, not the entire
Composer. Move to Drafts stores the uncertain content plus receipt in a visible
read-only Pending delivery item and frees the Composer for different content.
That item offers Inspect receipt and Copy text, not Retry send. Copying is not
proof of non-delivery; an intentional resend must disclose duplicate risk and
cannot masquerade as receipt retry. Proven not_delivered allows Restore to
Composer (without overwriting a nonempty draft) or explicit enqueue at the tail.
Delete/edit/reorder cannot drop or replay an in-flight operation.

Every runtime action carries an explicit request precondition:
expected_worker_epoch, expected_turn_id (and native thread identity), and the
observed queue/pause revision when that state participates in the action.
Existing-item operations also carry item id/revision. Idle direct Send carries
expect=idle_empty_active; otherwise it is rejected without effect and opens the
busy picker or exposes the new queue state. The Controller checks these with
durable admission/claim atomically. A later worker/runtime check repeats the
exact turn fence. An interrupt must use the native target-specific cancel or a
worker-owned serialized cancel reservation; never check a turn and then call
session-wide cancel. Cancel and next-prompt handoff retain exclusive ownership
so an unrelated turn cannot slip in between. Native inability to guarantee this
is a capability gate, not permission to weaken the contract.

These are proposed HTTP/MCP/CLI contract additions; current prompt and force
endpoints do not carry all these client expectations. Existing legacy paths need
explicit migration/compatibility behavior and must not be advertised as the new
fenced UI API. Reconnect/reload/provider swap/second-tab receipt recovery always
observes the original operation. Client timeout is not proof of failure.

Expose policy/pause discovery, versioned configuration, per-item policy and
receipts in the existing MCP control/read entrances and CLI. Omitted delivery on
existing automation retains next_turn for compatibility; do not silently change
scripts to follow the interactive default. New explicit session-default
admission resolves and snapshots the current policy atomically. Authorize after
resolution: self-targeted MCP steer/tool_boundary/interrupt stays rejected,
including through session-default and batch actions; no silent next-turn
fallback. Preserve all existing actor and cross-session checks. MCP/CLI never
open a popup. Document requested policy, target, once/automatic scheduling and
native acknowledgement versus consumption separately.

## 10. Acceptance before implementation release

This design-only task needs link/index/diff checks and review, not a product
build. Implementation requires:

- Native deterministic probes: generation in flight, serial/parallel tools,
  permission/elicitation, no subsequent tool, turn-end race, stale
  expectedTurnId, background tools, disconnect, timeout and old/new worker
  capability; accepted steering at turn completion (consumed/retained/dropped),
  in-turn compaction and identity changes, parent-versus-subagent destination,
  autonomous Busy without turn id, cancel/promptRequired/new-prompt handoff,
  selectable Codex boundary extensions, and pre/post-protocol-34 workers.
- State-machine coverage: idle/running race, paused+empty, resume+backlog, mixed
  FIFO policies, explicit overtaking, target end, two tabs/CAS and duplicate
  input.
- Receipt recovery: effects before lost response, Controller/worker restart,
  in-flight edits, duplicate force action, attachments and new draft revisions.
- Desktop browser gate (Firefox/Chromium both themes), trusted Chrome input and
  screenshots; macOS and Linux/Windows chord checks, Vim on/off, real IME
  checks.
- Physical iOS/Android acceptance: keyboard open/closed, fullscreen, image
  paste, rotation, viewport shortage, touch cancellation, selection restore and
  screen readers. Browser synthetic IME does not prove native-keyboard
  acceptance.
- No delivery capability enabled from metadata alone; exact Provider profile and
  installed-worker conformance gate. Update FOCUS.md, API docs and PITFALLS.md
  when implementation changes those contracts.

## 11. Review disposition

The requested managed Claude Fable Max design review completed on 2026-10-19:
call-b9468d38f6af62bfee1453cc, preset fable-max, Claude Provider 3.19.46,
runtime OVH / read-only execution snapshot on Hawk. Both result windows were
read. The reviewer judged the initial draft not ready for implementation, with
nine blocking findings and three medium findings. This revision addresses the
design findings below; it has not received a second external review and is not a
claim that native/mobile acceptance has passed.

| Finding                                                           | Disposition in this revision                                                                                                      |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Global FIFO makes a current-turn boundary unreachable             | Two delivery lanes, FIFO per lane; explicit target-bound batch release instead of ambiguous policy conversion                     |
| Client-only turn checks cannot prevent interrupting the next turn | Server admission predicates, worker/native target fencing and exclusive interrupt-to-prompt handoff added to the proposed API     |
| Pause loses once-only/manual-send behavior                        | Explicit Send once for idle-paused and existing rows; all one-shot modes preserve manual pause                                    |
| Expired/unknown rows have undefined drain behavior                | Expired unclaimed items leave lanes as Needs decision; unknown effect holds automatic dispatch, with separate receipt recovery    |
| Draft ownership and ambiguous send can lose/block content         | Native-acceptance clearing for current actions, exact draft-revision lock, retained uncertain Draft item and explicit restoration |
| Raw lifecycle statuses are not the UX states                      | Running/Busy/Starting/Exited/Crashed/Interrupted and unaddressable autonomous continuation mapped                                 |
| Keyboard retention conflicts with accessible popup focus          | One mobile contract: transfer focus and dismiss software keyboard intentionally; mobile hardware-keyboard path specified          |
| Session-default may bypass MCP self-steering prohibition          | Authorization after resolution, including batch and once routes                                                                   |
| Disabled focused choice may fall through to another mode          | No-op plus announced reason, stable focus and explicit new selection                                                              |
| Pause should not fail on unrelated revision conflicts             | Idempotent absolute Pause; Resume still guards its observed pause epoch against undoing a later pause                             |
| Review status was premature                                       | Actual completed initial verdict and revision boundary recorded here                                                              |
| Missing native probes                                             | Consumption, compaction identity, subagents, autonomous work, cancellation handoff and protocol generations added                 |

The review's suggestion that Alt+Enter currently sends without confirmation was
checked against Composer.tsx: it already opens forceAnchor. Keep the chord and
reconcile FOCUS.md's abbreviated wording when implementing; do not invent a
behavioral regression from that wording alone. Its bare-key mnemonic suggestion
is not adopted because existing scoped navigation is sufficient and native
typeahead should remain predictable. Its accessible Stop suggestion is adopted
as an explicit picker-owned command, not a global-event exception.

Design verification: changed local links, index entry, whitespace and index
coverage nonregression pass. The broader Columbus docs-index-check stops on
pre-existing data-transfer-toolkit front matter; the Cowboy base itself has 190
pre-existing unindexed Markdown pages. Neither baseline was modified. That
checkpoint contained no product-code diff. Implementation evidence follows.

## 12. Implementation and native findings

The candidate uses one busy-state picker and one versioned, DB-backed default
shared by Queue and Session Settings. Idle unpaused sends remain direct. Pause
is independent from the delivery preference, and Interrupt is an explicit
one-shot confirmation. Current-turn outbox rows have a separate Queue lane;
ordinary server queue rows retain Next turn. A default edit does not rewrite
previously admitted messages or implicitly batch-release them.

Native experiments on the exact pinned CLIs refined the proposed label: Claude
2.1.302 `next` consumes after foreground tools, but also consumes after
generation ends when no tool checkpoint remains. The public label is therefore
**Next checkpoint**, with the fallback stated in the picker. The persisted
preference remains `tool_boundary`; send intent is `next_checkpoint`. Codex
0.160.1 `turn/steer` consumed the marker in the same captured turn in both tool
and generation fixtures, rejecting an incorrect or idle expected turn. It does
not expose a distinct checkpoint priority, so only Steer is offered.

Claude's extension delegates admission, UUID tracking and settlement to ACP
0.93.0. It changes only the queued SDK user-message priority from `now` to
`next`, retaining upstream permission/elicitation `later` protection and
restoring the input method on failure. Delete this adapter when upstream accepts
the priority directly. Native CLI probes alone do not establish packaged ACP
acceptance; the worker fixture additionally checks the actual built launcher.

ACP `injected` acknowledges runtime admission, not model consumption. Stop may
discard an admitted checkpoint before consumption without rewriting that
receipt. The packaged cancellation fixture checks both the original prompt's
cancelled result and the absence of subsequent tool effects/model consumption.
The UI retires the local outbox after admission; it does not claim execution
completion.

Native-only evidence lives in the append-only receipts
`/srv/storage/fast0/agent/tmp/queue-claude-delivery-20261019-03.json` and
`/srv/storage/fast0/agent/tmp/queue-codex-delivery-20261019-01.json`. The Claude
`now` generation case observed a connection reset from preemption; it is not an
all-cases-clean acceptance result. The tool batch could serialize Bash calls, so
these measurements do not claim parallel-tool scheduling.

Target-bound Interrupt uses durable admission and a persistent drain hold,
checks the worker epoch and active turn at the runtime boundary, observes turn
settlement, and submits exactly one replacement while preserving manual Pause
and other queued messages. Unknown acknowledgement/settlement retains the hold
across restart. Explicit hold release requires the matching operation and a
connected idle worker; release never replays the replacement. Protocol 35 and
worker delivery-controls version 1 gate the new control against older peers.
After target-bound Interrupt has been admitted, a Controller rollback must
retain the persistent delivery-hold reader and dispatch guard. An older binary
that ignores this setting cannot safely reconcile an unknown interrupt. Use a
descendant revert retaining that compatibility bridge, as required by the
component activation contract; never clear holds to make rollback proceed.

The local outbox now owns pending content after durable save. Only the captured
draft revision may clear; native acknowledgement cannot erase later typing.
Reload/reconnect/Check receipt are observation-only. A missing receipt remains
unknown rather than proving that the message was never consumed.

Firefox and Chromium light/dark suites and Chrome trusted-keyboard acceptance
passed under `/srv/storage/fast0/agent/tmp/queue-send-desktop-acceptance4`. IME
and held-Return browser tests found and fixed MUI's implicit Enter click: the
picker must intercept at capture before MenuItem handles the event. These are
browser results, not physical iPhone third-party keyboard acceptance; the
independent image-adjacent caret issue remains open in PITFALLS.md.

The packaged Claude candidate passed 89 isolated worker checks (receipt
`queue-send-runtime-20261019/claude-receipt-03.json`), and the unchanged Codex
candidate passed 25 native worker checks. Managed Claude read-only execution
also passed. The real HTTP session-maintenance fixture passed 18 checks,
including anonymous policy refusal, stale-revision conflict, workerless
configuration, and persistence across Controller restart. Its second receipt
uses a short temporary root because the Nix shell's nested temporary directory
exceeded the Unix socket pathname limit; that environment failure did not
exercise product admission. All receipt paths here are below
`/srv/storage/fast0/agent/tmp/`.

The full immutable Machine bundle at commit
`3d05d10e1c5eab4af199f28f826da7f18dc0735d` is accepted as
`worker-13566979dce45f73d545`. Its actual release worker passed old/new Provider
coexistence (`queue-send-release/coexistence-nix-02.json`); the immutable
Controller/Machine/SDK pack passed the same 18 HTTP lifecycle checks
(`queue-send-release/session-nix-01.json`). macOS arm64 runtime probing passed
without credentials or inference (`queue-send-release/macos-01.json`), and its
temporary artifacts were removed. The retained-worker pin advances to this
accepted bundle so Hawk can retain its schema-one deletion/incarnation writer
capability while selecting the new worker protocol. These are pre-activation
receipts; production activation is recorded separately below.

### Production activation, 2026-10-19

Subsequent UI refinement consolidates **Queue first** into the delivery picker
on Mobile and Desktop. It submits the current composer content ahead of other
queued messages without interruption or resuming Pause, using the existing
durable front-submission path and captured-editor check. The row is unavailable
when the queue is empty. A nonempty queue also opens the picker while idle;
idle with an empty queue still sends directly. Desktop's explicit Queue-first
shortcut remains supported. The mobile icon uses a refined MUI-style filled
plane without a dropdown decoration or persistent background in either state.
Unconfigured sessions default to Steer. Saved explicit delivery preferences
remain authoritative; this does not change the legacy API's omitted-delivery
next-turn behavior or silently bypass the running-session picker.

This refinement activated Controller and Web source
`76a6e44babcb31770b8c59145878d8a042cb19a4`, with service worker
`cowboy-v2020`. The live MCP read of an unconfigured session changed from
`next_turn, revision: 0` to `steer, revision: 0`. Both component transactions
committed; public health and all four connected Machines were healthy.
Acceptance includes 2,303 frontend tests, the three send-intent Rust tests,
Firefox/Chromium in both themes and trusted Chrome keyboard/shortcut checks.
The picker regression covers Queue first on an idle paused queue and refusal
after that queue drains. Evidence is retained under
`/srv/storage/fast0/agent/tmp/`: `queue-first-postflight.json`,
`queue-first-browser-gate2.log`, `queue-first-desktop-acceptance2/`,
`queue-first-release-gate.log`, and `queue-first-steer-gate.log` (the Rust result
passed; its earlier frontend failure is superseded by the final release gate).

Controller and Hawk/Falcon/OVH Machine hosts activated clean source
`58eb2128fa37ea05667643d3cfab631bf90acc4f`. Linux hosts select the accepted
`worker-13566979dce45f73d545`; Hawk preserves its schema-one deletion writer
with an explicit same-generation read-only recovery closure. macOS built the
same source natively and refreshed its existing enrollment; the running host's
SHA-256 matches the new bootstrap binary. No identity or credential reset was
performed.

OVH's guarded maintenance receipt records 13 original workers: 12 retained their
processes and one completed an identity-preserving native generation handoff. It
does not claim that every busy session already uses the new worker. The
maintenance transaction was accepted and its rollback timer disarmed.

Claude Plugin `3.20.0`, composite digest
`sha256:516921fadeca9f7957344c406adaf5ea6be848bdd11f78ed3cc906510192cf47`,
passed the actual Catalog reader matrix and five published artifact digest
checks. Its OVH installation operation `queue-send-claude320-ovh-20261019`
completed with an Applied Machine receipt; inventory reports that exact active
generation. Other Machines did not have this Plugin installed and were not given
a new installation.

Web source `48a01cc14c800bc1daf5fce27af259974be8b76c` activated independently,
with service worker `cowboy-v2013`. Public health/version, app shell and service
worker responses were checked after activation. The final frontend gate passed
2,303 tests, type checking and lint. The complete merged native gate and
Firefox/Chromium light/dark plus trusted Chrome keyboard gates passed earlier;
these still do not establish physical iPhone keyboard acceptance.

Receipts and bounded postflight evidence are retained under
`/srv/storage/fast0/agent/tmp/queue-send-release/`:
`activation-postflight.json`, `falcon-current.json`, `ovh-activate.log`,
`macos-activate.log`, `macos-live.json`, `catalog-before.json`,
`catalog-after.json`, `served-artifacts.json`, and `ovh-plugin-install.json`.
The OVH root-owned maintenance receipt is
`/var/lib/columbus/ovh-machine-58eb2128fa37-20261019T062833Z`.
