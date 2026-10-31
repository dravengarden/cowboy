# Linked session context and fresh agents

Cowboy can link up to 32 owned sessions as context for either an existing agent
or a new session. Provider and subscription changes create a new native agent.
Reasoning, permissions and other supported live settings keep their existing
configuration path.

To keep this session live and branch from it, use **Fork**
([session fork](session-fork.md)). A native fork stays on the source's
subscription and runtime Machine; links never copy native transcripts across
accounts.

## User flow

New Session has an optional **Select sessions** multiselect.

Session Settings answers independent questions, each in one place:

- **How this agent runs**: the Agent block (model, reasoning, permissions)
  changes in place. Nothing below repeats it.
- **What this session has read**: the links list names every session this
  conversation came from or read (below).
- **What the next prompt will read**: **Select sessions** is always visible
  under the Agent block in mobile settings, desktop Run Configuration and the
  empty-session configuration workspace. It behaves like an attachment: every
  selection applies at once, is not a prompt, and starts no reply. Removing a
  chip or **Cancel** in the Composer cue undoes it. Once a prompt carries a
  selection it joins the links.
- **Where the work goes**: **Replace or fork** opens one form with the
  target agent and two actions. **Replace this session** moves the work;
  **Fork to new session** keeps both live.

The form preselects the current installation and subscription, or the staged
target. How history travels follows from the target and is stated under the
pickers, not chosen. **More options** holds what the destination needs before
its first turn: project, the optional preset of the target agent (for the same
Provider an empty preset inherits current model and reasoning) and **Inherit
compatible settings** (on by default). Clearing inheritance is the way past a
target that refuses the source's restrictive permissions; nothing retries
without it automatically.

**Replace this session** saves the target and links the current session
ahead of the selected ones, without creating, stopping or starting a worker,
then closes settings and focuses the Composer. The current session stays a
removable source in the selection, and the action stays available for an
identical target, which starts a fresh conversation in place of the accumulated
one. The current agent and its identity remain intact until the send. A saved
switch shows "Switching to SUBSCRIPTION on your next message · **Back**". Back
(and **Cancel** in the Composer cue) removes the new agent and the current
session's link and keeps other selected sources.

**Fork to new session** ([session fork](session-fork.md)) acts immediately and opens the
new session, where its settings and links are configured. It stays on this
session's project and Machines, so it is disabled for a changed placement, and
it leaves this session's pending links and staged continuation untouched. An
empty session offers only Continue.

The session menu (mobile long press, Desktop session actions with `F`) offers
**Replace or fork**, which opens that form. A continuation keeps its
source's title and links back to it; the mark is the persisted
`session_continued_from:<id>` setting projected as `continued_from` on the
session. Continuations created before that mark are recorded once at Controller
startup from their source's accepted send receipt. Every switch is made in
Session Settings; the Transcript carries no fork, branch or continuation
controls.

A continuation and a fork differ in shape. Continuing moves the work to a new
agent, so there is one line and the original leaves. Forking copies a
conversation, so there are two live lines. An accepted handoff moves the
original to Trash, with Undo on the device that sent it, also when that device
recovers a lost reply. In Trash it stays
readable, by the continuation through Cowboy MCP as well, until retention ends;
its worktree is intact for **Restore** until then. If Trash refuses, the
original stays as read-only history: the session list marks it **History** and
its Composer gives way to a dock. **Open** goes to the continuation, **Fork**
starts a separate session from this agent's conversation (natively where the
Machine supports it, otherwise a linked copy), and **Move to Trash** clears the
original away and opens the continuation. Restoring leads to the same state.
Writing in the original would silently
split one piece of work in two, so the web offers no way to do it;
Select sessions, **Replace or fork** and the session menu item are
hidden for the same reason. While its agent is still busy the
Composer stays, so the turn can be observed and stopped. Deleting or trashing
the continuation returns the original to an ordinary session. The state is
derived from the synced `continued_from` mark; the session API still accepts
messages from agents and scripts.

While choices are pending, a row directly above the Composer, in the same frame
as the Plan panel, says what the next message does ("Next message · Starts
Claude · Mink", or how many sessions it links) with **Cancel**, and the input
placeholder reads "Message the new agent…" so the cue survives the mobile
keyboard hiding auxiliary rows. After a handoff the link is history, not a
next-send choice, so it leaves the Composer. The switch itself stays in Session
Settings. While a continuation prepares, or any session with its first message queued,
its page shows only the progress: the setup form is for a session nothing has
been sent to yet, and Session Settings stays available. A continuation opens with one link, "Continued from SOURCE · OLD → NEW" (also in
its empty and preparing state), with `+N` for its other links.

The links list in Session Settings starts folded under "N links" and has one row per link: "Forked from",
"Continued from" or "Linked", with the title and subscription. A link is
recorded when its send is accepted (`links` in the context) and survives later
edits, with the history boundary and title captured then. Following a row opens
the source at that row, paging history back in and highlighting it. A source can
go away at any time, so the row follows the live session lists: in Trash it
stays named with **Restore**, and deleted it stays named from the stored title,
marked Deleted, with nothing to open. The record of what this conversation read
cannot be removed. Where the work continued keeps its own row, "Continued in a
new agent · Open" (with **Hibernate** when available).
Sessions without announced context or a continuation mark render and fetch
nothing for this cue.

On mobile, **Select sessions** opens a separate sheet above Settings or
New Session. It opens without focusing an input; search raises the keyboard
only when chosen. Search stays above the scrolling results and **Done** stays
above the keyboard. Selection, including **Clear**, edits only the owning form;
closing the picker retains those edits. Desktop retains searchable keyboard
multiselect. Both show selected sessions with account labels, enforce the
32-source limit and let users remove unavailable sources.

The next explicit send checks the saved revision and current authorization. A
fresh-agent transition requires the source to be idle with no queued prompts or
native background work. It creates a separate Cowboy session through ordinary
account and placement admission, then sends the message once and opens that
destination. The original conversation, native identity, account binding and
files remain available. Linking context alone does not replace an agent.

The continuation keeps its source's title and effective sidebar folder, including
explicit Global placement and project-bound folders. It is inserted immediately
after the source in both the session order and the mixed session/Draft order.
These changes use the shared Service sync state and persist across devices. The
folder is copied as placement, not passed as new directory defaults that could
override the source's selections.

At send time, inheritance copies compatible Provider configuration, explicit
Memory/Calls/reviewer choices, the send policy, automatic Provider updates and
the Zed browsing-directory override. A source override naming its own
worktree (`session::SOURCE`) becomes the destination's own worktree; a project
or another session stays the explicit choice. Same-Provider options must be declared by
the target's signed contract, including its versioned permission interface; a
different release only accepts values present in its signed defaults/presets.
Restrictive permissions require the same Provider release and permission
contract: an incompatible target refuses the send rather than resetting them.
Cross-Provider model and permission identifiers are never guessed. Explicit
target presets override model/reasoning without widening inherited permissions.
Unset Memory/Calls fields keep following the destination's agent defaults,
including later Owner changes. Calls with explicit targets are filtered by the
destination's ordinary family restrictions without introducing new targets;
the destination Provider's call limits still apply. If no explicit target remains
compatible, Calls are disabled until reconfigured. Disabling inheritance keeps
the destination's defaults while retaining title, folder and adjacent placement.
Queue contents, Pause, one-shot approvals, pending operations, native identities
and credentials are never copied. Later source edits do not change the new agent.

Compatibility and folder access are checked before a destination or worktree is
created, then checked again against its actual admitted generation. A concurrent
change can still fail after creation; the failed receipt explicitly identifies
the unsent destination for inspection or deletion, and never retries the message.

A continuation in the source's project on its execution Machine (protocol 39 or
later), from a source that has run, picks up the work where it stands: its new worktree starts from the source's `HEAD` with the
source's uncommitted changes, through the same copy as a
[fork](session-fork.md), including its limits and the 512 MiB refusal. The source
worktree is never written, so Undo and Restore return it unchanged. Another
project or Machine follows normal isolated-worktree creation and the changes
stay with the source. Other registered directories retain their existing sharing
rules. Selected source sessions are not stopped, modified or granted new access by a link.

## Back for in-place changes

Model, reasoning, permissions and other live options still apply at once.
The first change of each option since the last sent message records the value
it replaced (`session_config_revert:<id>`, announced to every device). The
Agent block then shows "Changed from Opus · High · **Back**"; Back re-applies
those values through the same configuration path. Setting an option back to
its recorded value forgets it. Sending any message to the agent (typed,
queued, scheduled, steered or a wakeup) clears the record once it reaches the
agent, so Back is never available
for a conversation that already used the new values. Agents use
`sessions.config_revert` and `sessions.config_revert_restore`; the HTTP
contract is `GET /api/sessions/SESSION/config-revert` and
`POST /api/sessions/SESSION/config-revert/restore` (409 once a message was
sent, on a trashed session or during a Provider change). Restore applies values
in the agent's option order and keeps Back for any value it could not apply.

## What changes in place

Audited against the pinned adapters (`claude-agent-acp` 0.93.0,
`codex-acp` 2.2.2) and Cowboy's execution binding. Live settings use the
existing session configuration path (`set_config_option`, MCP
`sessions.configure`, `cowboy api`); a saved value is also replayed on the next
launch of a dormant agent.

| Setting | Claude | Codex | Path |
| --- | --- | --- | --- |
| Model, reasoning/effort, fast mode | Applied to the running query | Stored; next `turn/start` | In place |
| Permission/approval mode | Applied to the running query | Stored; next `turn/start` | In place |
| Memory, Calls, reviewers, send policy, auto-update, Zed browsing folder | Cowboy preferences | Cowboy preferences | In place |
| Subscription (for example Claude · Lynx → Claude · Mink) | — | — | New agent |
| Provider (Claude ↔ Codex) | — | — | New agent |
| Project, execution Machine, isolated worktree | — | — | New agent |
| Runtime Machine | — | — | New agent |

Credentials and native session state are never moved between subscriptions or
runtime Machines. A renewed login of the *same* subscription on the same
Machine is the explicit account transition (`accounts.transition_plan` /
`accounts.transition_reconcile`, see [agent API](agent-api.md)); it stops the
worker and rebinds the session to the newer login without starting a worker,
and is not a subscription switch. The native agent cwd is a private runtime entry, while tools
execute in the bound target worktree; the binding is created once per session.
Re-binding a live session to another project would need tool fencing, a new
binding revision and re-projected project guidance at the worker's construction
boundary (see [execution environments](execution-environments.md#session-identity-and-routing)).
That is Machine/worker-generation work, so project changes use a new agent. A
shell `cd` or editing the Zed browsing folder changes neither binding. Choosing a
new agent whose Provider, subscription, project and Machines all match the
current agent is allowed for a fresh conversation; the editor states that
model, reasoning and permissions can change in place instead.

## Context and token costs

The Service adds one bounded preamble to the admitted message, containing only
source IDs, history cursor boundaries and instructions for reading Cowboy MCP.
The user message remains a normal user message in the UI. Sources are
inspectable in Context; full native transcripts are never copied into another
account.

The destination discovers session APIs on demand and reads relevant recent
history, decisions, outstanding work and tool evidence. Full-event continuation
and server search remain available when needed. It does not automatically walk
links recursively, summarize the entire history or invoke another model.
Subsequent messages do not repeat the preamble. Cross-account/provider prompt
cache reuse is not assumed.

Historical content supplies evidence, not authorization. The current user
message wins. Source ownership is checked at selection, send, prompt dispatch
and the existing MCP history-read boundary. A deleted or inaccessible source
cannot silently become another owner's context. Titles never enter the
bootstrap.

## Persistence and concurrency

The DB-backed `session_context:<id>` preference stores a monotonically
increasing revision, ordered source IDs, an optional closed target specification
and the last send receipt. It is restored across Controller restarts and
announced only to the owner's connected clients. A stale edit or send is
rejected. Native credentials, auth homes, cookies, native session IDs and
environment overrides are not accepted as target settings.

Accepting a send advances the preference revision as well as its applied
revision. Another device cannot save a pre-send editor over that result. Later
edits retain the last terminal receipt, so a lost response can still be
observed. Returning to the original session permits ordinary messages there; the
accepted send opens the destination. After a lost reply or page reload, the
first attempted resend observes that receipt and opens the destination once,
with an explanatory notice, instead of duplicating the message. Returning to the
original session then permits a deliberate new message there.

A slash command cannot start a new agent: it needs a conversation, and the
source would go to Trash for nothing. The send is refused with the staged choice
intact, so the user writes the first message or goes Back.

A send reserves its operation and destination before creation. A duplicate with
the same operation and payload observes that receipt; another payload cannot
reuse the operation. Source and destination mutation locks protect first-send
admission. The destination is fenced before its session becomes visible, so
another tab cannot race its bootstrap. Ordinary WS/API submission and steering
cannot bypass a pending context choice.

Links hold nothing: saving them records a waiting list and marks the context
applied, whether the agent is idle, busy or has queued prompts. The first
authored prompt dispatched turns that list into its injection (below), which
is when the selection becomes links and every device drops the cue. A claimed
injection is never replaced while its prompt is queued for retry, and a session
deleted while waiting is skipped rather than refusing the prompt. A Web send
waits for link edits still being saved. `POST .../context/send` still accepts
waiting links, for callers that want a receipt. Only a staged new agent is
pending. The queue holds scheduled wakeups, activated drafts and manual drain
requests until that send. Saving it closes queue admission
under the same Hub lock before awaiting storage. Messages arriving during that
write remain queued. Clearing context is allowed with a held queue and releases
it under the normal Pause policy. A bootstrap precedes messages queued during
its preparation, while continuing to respect Pause.

`accepted` means admitted to Cowboy's normal prompt/queue lifecycle, not that
the model answered. A failure during preparation preserves the old session.
`preparing` or `unknown` after a lost response or restart is observation-only;
it never automatically retries a creation or sends the message again. Inspect
the destination and its transcript. **Resolve interrupted send** acknowledges
the exact operation and clears only the pending preference, preserving the
destination and its files. It does not cancel a running operation or resend.
Known preparation refusals are recorded as `failed`, with a readable reason, and
release any dormant destination's context fence. Failures after submission
remain uncertain until their existing receipt is observed. Unreadable stored
context remains closed to sending: restore the compatible Controller or verified
storage rather than discard an operation whose effects cannot be established.

Asynchronous execution-environment preparation may still fail after a context
send has been accepted into the queue. Its existing recovery loop observes the
same session identity. The Controller retains the Machine's typed refusal reason
in diagnostics and reports the affected Machine and retry state. A later
`running` or `busy` event folds the earlier runtime error into **Session recovered**
with its original detail available; `starting` alone does not prove recovery.
Failed turns and interrupted tools retain their failure state. Folding a
recovered error does not retry a message or transfer it to another session.

Editing the selection before a prompt carries it replaces it; a session already
waiting keeps its history boundary, including across staging a new agent and
going Back.

The links belong to the first authored prompt dispatched to the agent, not only
to the queued bootstrap row. Before handing a prompt to its runtime, the
dispatcher records that prompt as the injection's carrier with a
compare-and-set under a separate `session_context_carrier:<id>` setting; only
the carrier receives the preamble, including on retries of the same prompt. If
the bootstrap row leaves the queue undispatched (removed, cleared by an edit) or
a front insert overtakes it, the next dispatched prompt becomes the carrier and
the overtaken row is later sent as an ordinary message. Scheduled wakeups,
retries, managed-call prompts and slash commands such as `/compact` never carry
or consume the links; current-turn
steering bypasses dispatch and leaves them for the next queued prompt. If the
record cannot be written, the prompt stays queued instead of being sent without
durable evidence, so a Controller restart cannot apply the links twice. A prompt
that became the carrier but then failed dispatch keeps the links for its retry;
deleting that requeued row drops them, and the context can be linked again.
Receipts recorded before this rule keep exact binding. A runtime that returns an
undelivered prompt requeues it without the exact server-generated preamble
block, so its retry receives one copy and similar user text is preserved.

This is account isolation, not a guarantee against Provider enforcement. There
is no quota-triggered account rotation, credential rewriting, native-thread
reuse across accounts, or imitation of another account's identity.

## HTTP, MCP and CLI

- `GET /api/sessions/{id}/context` reads links, revision and the last receipt.
- `PUT /api/sessions/{id}/context` takes `{revision,sources,next_agent?}`.
  `next_agent` contains `provider`, `provider_account_id`, `machine_id`, trusted
  project ID `cwd`, optional `runtime_machine_id`, `preset` and
  `inherit_settings` (defaults to `true`).
- `POST /api/sessions/{id}/context/send` takes
  `{revision,operation,text,content?}`. Keep `operation` stable after
  uncertainty.
- Both session creation APIs accept `context_sources`.

The existing four-tool MCP surface exposes `sessions.context`,
`sessions.context_set`, `sessions.context_send` and the extended
`sessions.create` in its on-demand control catalog. Use
`cowboy.control.read("sessions.context", {session_id})` and `control.apply`
through `cowboy_execute`. The authenticated CLI uses the same HTTP contracts:

```sh
cowboy api GET /api/sessions/SESSION/context
cowboy api PUT /api/sessions/SESSION/context --data - < context.json
cowboy api POST /api/sessions/SESSION/context/send --data - < message.json
```

For explicit recovery, `context_set` additionally accepts `resolve_operation`
with empty sources and no target after inspecting the exact receipt. There are
no new resident MCP tools.

## Ownership and rollout

Native agents continue to own execution and native memory. Cowboy owns the
residual cross-provider UI, immutable account/placement admission and access to
its own stored conversations. The narrow extension is one prompt preamble plus
existing MCP session reads. Claude and Codex use this same adapter-backed
contract; no follower-specific transcript importer is introduced. Native
cross-provider context links with equivalent ownership and receipt semantics
would replace the local bootstrap.

This change belongs to Controller and Web. It does not change the Machine or
worker wire format and must not trigger worker-generation maintenance. Activate
the Controller before the Web that consumes these endpoints. Source checks and
browser fixture checks are distinct from deployment acceptance.
