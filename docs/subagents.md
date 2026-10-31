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
structural change or root update. Thoughts, plans and configuration of a child
are not kept.

## Persistence and presentation

The Controller's `EventReducer` keeps one durable row per `(session, child)`:
a later snapshot reuses the first row's seq (an UPSERT), survives turn ends,
and does not split a parent message that streams around it. Clients render the
highest `revision` at the row's first position (`web/src/derive.ts`), so live
intermediate snapshots, a Controller restart (new row) or an older Controller
(one row per snapshot) all converge.

`web/src/SubagentCard.tsx` draws the row on Desktop and Mobile: title, state,
counts and a one-line preview, a Stop button while `cancellable`, and a
collapsed activity list with the prompt, the bounded activity and the result.
Its expand toggle is the row's Desktop default action. A worker exit settles a
working row as ended.

## Permissions and elicitations

A child's `session/request_permission` and session-scoped
`elicitation/create` travel the existing permission channel. The worker adds
`_cowboy.subagent: { sessionId, title }` to the presented tool call; the
transcript record, the Desktop approval dock and the Mobile overlay name the
subagent. A subagent's request stays pending across the parent's turn end.
System, Full Access and managed read-only sessions answer child requests
exactly like their own, so a managed child never waits on a person.

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
| `CancelSubagent` routing, protocol 40 | Machine |
| Durable row coalescing, `cancel_subagent` inbound and MCP action, protocol gate | Controller |
| Row, activity, Stop, request attribution | Web |

A Web without the row ignores `cowboy_subagent` updates. A Controller without
coalescing stores one row per snapshot. Without a protocol-40 Machine and a
current worker, Stop is refused.

## Known gaps

- The ACP bridge (Cowboy as an ACP agent for other clients) does not forward
  subagent rows; its clients no longer see the hidden parent tool call.
- A child that never reports `idle` (an adapter defect) stays working until
  its worker exits.
