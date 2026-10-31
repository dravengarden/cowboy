# ACP subagents

Cowboy is an ACP client for subagents (`unstable_subagents`, schema 1.13). The
worker declares `clientCapabilities.subagents: {}` in `initialize`, so an
adapter runs each subagent as its own child ACP session instead of an opaque
tool call of the parent. Cowboy shows each child as one live row of the parent
session, routes its permission requests to the person, and can stop it alone.

## What the adapters send

Both production adapters choose the standard form only when the capability is
an object (Claude `dist/acp-subagents.js` `clientSupportsSubagents`, Codex
`subagentProtocol` in `dist/index.js`).

| | claude-agent-acp 0.99.1 | codex-acp 2.5.0 |
|---|---|---|
| Announcement | `subagent_update` on the parent (title, description = role, `capabilities.cancel: {}`, `state: running`) before any child traffic (`announceNativeSubagent`) | `subagent_update` with `capabilities: {}`; `cancel` is toggled later to `{}` / `null` while a turn of the child runs (`publishCancellation`) |
| Child traffic | Ordinary updates whose `sessionId` is the child (`toChild`), including its tool calls, messages and `config_option_update` | Same; the child also gets `session_info_update` and model configuration |
| Parent transcript | The Agent/Task tool call is hidden (`routeRfdLaunch`); the delegation prompt and the reply become linked `session_message`s (`linkedSessionMessages`, `launchReply`) | The `spawnAgent` collab call is consumed (`StandardSubagentEventRouter.handle`); no `session_message` is sent, the reply is the child's last message |
| Work state | `idle` + `end_turn` / `cancelled` / `error` (`rfdEndState`); `requires_action` while a child request is open (`awaitingUser`) | `idle` per child turn end (`turnEndState`); `unknown` when the thread closes |
| Stop one | `session/cancel` with the child id stops only that task (`cancelChildSession` → `stopTask`) | `session/cancel` with the child id cancels it and its descendants (`cancelChild`) |
| Parent cancel | Spares background children (`perTaskStopAffordance: true` in the RFD form) | Cascades to children |
| Nesting | A grandchild is announced on its parent child's session | Same |

Cowboy's Codex `adapter.patch` does not touch subagent routing.

## Routing (worker)

`src/acp_subagents.rs` owns the registry; `handle_session_notification` in
`src/acp.rs` calls it before any root-only projection:

- A notification whose `sessionId` is a known (or evicted) child is folded
  into that child and never reaches the parent timeline, usage, configuration
  options, completion capture or managed final text.
- A root `subagent_update` registers or patches a child (`null` clears a
  field). One from a child session registers a nested child.
- A root `session_message` (or chunk) sent to a known child is its delegation
  prompt; one received from it is its result. Unlinked messages stay ordinary
  root updates.
- Any other root update is unchanged.

Each change publishes a complete `cowboy_subagent` snapshot as an ordinary
`Event::Update` of the parent: `sessionId`, monotonic `revision`,
`parentSessionId`, labels, `state`, `stopReason`, `error`, `cancellable`,
`prompt`, `result` (the linked reply, else the last message once idle),
counts and the latest 24 activity entries (tool title/status, message text).
Text is bounded (600 bytes per entry, 8 KiB for prompt/result); streamed text
alone republishes at most once per second and is flushed by the next
structural change or root update. Thoughts and plans of a child are not kept.

A snapshot also carries the child's `model` and `usage`. The model is the
display name of the child's own `model` configuration option (both adapters
send the child a `config_option_update` with one choice: Claude
`childModelOption`, Codex `subagentConfigurationOptions`), else
`_meta["_claude/model"]` of its `usage_update`, else the only row of an idle
state's `_meta.quota.model_usage`. `usage` holds the counters of an idle
state's ACP `usage` when an adapter reports one; neither production adapter
does today (Claude `rfdEndState`, Codex `turnEndState`), so the row shows tokens
only once one does.

## No longer reporting

A snapshot's `reporting` is `false` while a child is `running`, the parent has
no turn in progress, and the child sent nothing for ten minutes since the
parent's turn ended (`QUIET_AFTER`, checked once a minute). A child waiting on
the person (`requires_action`) is never quiet. Any later traffic of the child
publishes it as reporting again. Clients also treat a working row as not
reporting after a worker exit, crash or a new worker's `starting` lifecycle,
until a newer snapshot arrives. The row then reads "Not reporting" in a
neutral tone: Cowboy claims neither success nor failure, and Stop is offered
only while the worker still grants it. An `idle` snapshot without a stop
reason reads "Ended: outcome unreported", never Done
([background-tasks.md](background-tasks.md#32-display-groups-presentation-only)).

## Persistence and presentation

The Controller's `EventReducer` keeps one durable row per `(session, child)`:
a later snapshot reuses the first row's seq (an UPSERT), survives turn ends,
and does not split a parent message that streams around it. That position is
the child's durable transcript anchor (`transcript_anchors`), shared by the Hub
and the store writer across restarts and eviction. The Controller stamps each
snapshot with `observedSeq` and the worker `incarnation`; clients render the
snapshot with the highest `observedSeq` (else `revision`) at the row's first
position (`web/src/derive.ts`), and the focused bootstrap plus a lookup repair
any cached row whose terminal snapshot the reconnect tail did not carry. See
[background-tasks.md](background-tasks.md) §4.

`web/src/TaskRow.tsx` draws one compact row on Desktop and Mobile: title,
model and counts, a Stop button while `cancellable`, and the state chip last.
The row never expands: its default action (a Desktop Conversation widget,
`Tab` selects it) opens the Background tasks inspector, where
`SubagentDetail` (`web/src/SubagentCard.tsx`) shows the prompt, the bounded
activity and the result. Stop asks first with the same `ConfirmSheet` as the
session's Stop (Mobile's inset card, Desktop's centered dialog); `Keep running`
sends nothing. On Desktop, `x` on the selected row opens that confirmation
(`conversation.stopSubagent`, see `web/src/desktop/FOCUS.md`).

## Per-model usage

Both adapters end a prompt with `_meta.quota.model_usage`, one row per model
with the turn's `token_count`:

- claude-agent-acp 0.99.1 `turnQuotaMeta`/`quotaMeta` (`dist/acp-agent.js`):
  rows from the SDK's `result.modelUsage` increments, which also count Task
  subagents, sidechains and internal calls; its `usage_update` names the
  model in `_meta["_claude/model"]` (`attachUsageModel`, since 0.81).
- codex-acp 2.5.0 `buildQuotaMeta` (`dist/index.js`): one row for the current
  model (`currentModelId` without a `[…]` suffix); its `usage_update` has no
  model, and a subagent's `usage_update` reports only its last turn's context.

The worker (`src/acp_model_usage.rs`) adds the rows of every turn it ran and
carries the totals on the session usage as `raw._cowboy.modelUsage` (model,
total/input/output/cached-input tokens, turns), republishing the last usage
when a turn adds rows. Session info lists one row per model when there is
more than one, and names the context's model when the adapter does. The totals
cover the current worker: a new worker starts again from zero, like the
context figures. A Codex subagent on another model shows that model on its
own row only; codex-acp reports no per-model tokens for it.

## Permissions and elicitations

A child's `session/request_permission` and session-scoped
`elicitation/create` travel the existing permission channel. The worker adds
`_cowboy.subagent: { sessionId, title }` to the presented tool call; the
transcript record, the Desktop approval dock and the Mobile overlay name the
subagent. A subagent's request stays pending across the parent's turn end.
System, Full Access and managed read-only sessions answer child requests
exactly like their own, so a managed child never waits on a person. An
`elicitation/complete` names only the elicitation, so it resolves a child's
pending URL elicitation exactly like the parent's.

## Stopping the whole session

A parent Stop still stops everything. After cancelling the parent, the worker
also sends `session/cancel` to every child that is working and currently
cancellable, because claude-agent-acp spares background children on a parent
cancel in the subagents form. Codex cascades on its own; the extra cancel is
harmless there.

## Stopping one subagent

Web sends `cancel_subagent { session_id, subagent_session_id }`; Cowboy MCP
exposes the same command as `sessions.cancel_subagent`. The
Controller refuses it unless the Machine speaks protocol 40
(`SUBAGENT_CANCEL_PROTOCOL_VERSION`) and the session's live worker reports
`supports_subagent_cancel`; it never queues the stop for a later worker. The
Machine forwards `WorkerCommand::CancelSubagent` only to such a worker and
otherwise acknowledges a refusal, which the Controller shows as an error. The
worker sends `session/cancel` for the child only while that child is working
and its adapter grants `cancel`. All wire additions are new variants or
`serde(default)` fields; no existing command changes meaning.

## Turn lifecycle

Child traffic counts as turn progress for the idle watchdog and marks the
current prompt as having visible output, so a parent waiting on a long
subagent is never cancelled as silent or replayed. Working subagents raise the
session's background level (`max` with the SDK's own count), so a subagent
that outlives the prompt keeps the session unsettled and blocks unattended
Provider reloads. The parent's own turn completion, steering and queue are
unchanged.

## Releases

| Part | Component |
|---|---|
| Capability, routing, snapshots, child stop | worker generation (Machine release) |
| Child model/usage, `reporting`, per-model usage totals | worker generation (Machine release) |
| `CancelSubagent` routing, protocol 40 | Machine |
| Durable row coalescing, `cancel_subagent` inbound and MCP action, protocol gate | Controller |
| Bridge forwarding (`cowboy serve-acp`) | the `cowboy` CLI the ACP client starts |
| Row, activity, Stop confirmation and key, request attribution, not reporting, model breakdown | Web |

A Web without the row ignores `cowboy_subagent` updates. A Controller without
coalescing stores one row per snapshot. Without a protocol-40 Machine and a
current worker, Stop is refused. Snapshots from an older worker carry no
`model`, `usage` or `reporting` (read as reporting) and its usage has no
breakdown; the Web then shows neither, and still marks a row not reporting
after a worker change. Every addition is an optional field.

## ACP bridge clients

`cowboy serve-acp` (`src/acp_bridge_subagents.rs`) projects each
`cowboy_subagent` snapshot for its client. A client whose `initialize`
declared `clientCapabilities.subagents` gets the standard form: a
`subagent_update` on the parent (or on the child that launched it) with the
labels, `capabilities.cancel` while Stop is possible and the state (`unknown`
while not reporting); the delegation prompt and result as linked
`session_message`s; and the child's tool calls and message text as updates of
the child session, each delivered once. Its `session/cancel` for a child
session becomes `cancel_subagent`, and a child's permission request is asked in
the child session. Any other client gets one `tool_call` per subagent
(`cowboy-subagent:<child>`, kind `think`, title `Subagent: <title>`), updated in
place with a status (`in_progress` while working or not reporting, `failed`
when stopped or failed, else `completed`) and a text naming the prompt,
progress, model and result; a child's permission request is asked in the
parent with `Subagent <title>:` in its title. The projection follows the
bridge's replay and catch-up order, ignores older revisions and restarts when
the client reloads the thread.

## Known gaps

- Per-model totals live in the worker and restart with it.
- A bridge client sees a child's tool titles and statuses and its message
  text, not tool output or thoughts; the snapshot does not keep them.
