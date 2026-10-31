# Session fork

A fork is a new Cowboy session that continues the same native conversation
on a separate branch. The source session and its native thread stay intact;
both can then diverge independently. Unlike
[Continue in new agent](session-context.md), which links context across
accounts or Providers through Cowboy MCP, a fork asks the Provider adapter to
copy its own native transcript with ACP `session/fork`, so the new session
keeps the complete model context and can reuse the Provider prompt cache.

## User flow

**Duplicate…** in Session Settings (Mobile settings and Desktop Run
Configuration, under **Switch agent or link context**; the session menu item
and Desktop `F` open it) copies a session without changing it. Choose the
installation and subscription; on the same Provider the form shows the live
model and reasoning to keep or change, otherwise a preset.

- Same Provider and subscription on a fork-capable Machine: a native fork
  (below) of the whole conversation, with the chosen model/reasoning saved on
  the copy before its first launch (`config`). It waits for the current turn.
- Another subscription or Provider, or no native fork on the Machine: a new
  session on the source's Machines and project that inherits compatible
  settings and reads the source as linked context on its first message
  (`POST /api/sessions/SESSION/duplicate`, MCP `sessions.duplicate`). A
  stable `operation` returns the same copy on retry, also after a dropped
  request, and refuses a different request; the form starts a new operation
  whenever a choice changes. **Inherit compatible settings** can be turned
  off when the target cannot accept the source's settings. It is placed beside
  the source as "SOURCE (copy)"; if a later step fails, the error names the
  created copy to inspect or delete.

The form states which of the two it will do.

- **Duplicate…** in the session menu (mobile menu, Desktop session actions
  with `F`) opens Session Settings at Duplicate; forking from there needs an
  idle source and opens the new session.
- Forking through an earlier answer is available to agents and scripts with
  `message_id` (below). The Transcript shows no fork action: switches and
  branches are made in Session Settings.
- `sessions.fork` in Cowboy MCP and `POST /api/sessions/:id/fork` expose the
  same operation to agents and scripts.

The fork keeps its source's Provider, subscription and login, runtime Machine,
execution Machine, project and supported settings (model, reasoning,
permissions, Memory, Calls, send policy and automatic Provider updates). Its
title is the source title with a ` (fork)` suffix, it is placed directly after
the source in the same folder, and its list row shows a **Fork** badge. Session
Settings shows "Forked from SOURCE · Open"; the Transcript itself has no
marker.
To use another agent, subscription or project, use **Continue in new agent…**
instead; native transcripts never move between accounts or Machines.

Queue contents, Pause, one-shot approvals, pending permissions, drafts and
scheduled wakeups are never copied.

## Files

A Git project fork gets its own isolated worktree on a new `cowboy/<id>`
branch that starts at the source worktree's `HEAD`. The Machine then copies
the source's uncommitted state into it: staged and unstaged changes to tracked
files (through `git stash create`, which does not modify the source) and
untracked files that are not ignored. Ignored files, such as build outputs, are
not copied. The source worktree is never written. Non-Git roots remain shared,
as for every session.

If the source worktree no longer exists, the fork is refused instead of
silently starting from the default branch. Copying refuses any path that
crosses a symlinked directory in either tree. A Machine records each fork
copy in progress and refuses to reuse a worktree whose copy was interrupted,
so a partial copy is never presented as the source state. Untracked files
above 512 MiB in total refuse the fork; ignore build output first.

## Admission

The Controller admits a fork only when all of the following hold. Each refusal
names its reason and creates nothing.

- The source is owned by the caller, is not a system session, managed child
  or trashed session, and has a native thread that can be resumed.
- **Fork** requires an idle source: no prompt in flight, nothing queued and no
  native background work, because the adapter copies the native transcript as
  it is on disk. **Fork from here** may target a completed turn while a later
  turn is running.
- The message, when given, is the last assistant message of a completed turn
  in the source transcript. A message steered into a running turn does not
  end it. A message older than the Controller's loaded transcript tail is
  validated by the adapter at the fork's first launch instead.
- The source is not in Trash.
- The Provider is one whose adapter fork has been verified (`claude-code`,
  `codex`). Other Providers refuse until their adapters are verified.
- The runtime Machine speaks Machine protocol 39 or later, which carries the
  native fork launch and the forked workspace base.

The destination is created through ordinary session admission with the
source's account, Provider, Machines and project, and then checked again: the
admitted account, login generation and runtime Machine must equal the
source's, because the adapter reads the source thread from that account's
native home on that Machine. Each auth generation has its own runtime home,
and a Service-managed (default) account advances its generation on every
token refresh (about every eight hours) without a new sign-in. Admission
therefore keeps the source's older generation for a fork: the Service
readiness check still holds the current generation, and every retained home
is re-projected with the newest credentials, exactly as when the source
resumes. A Service re-login also advances the generation, and a fork across
it likewise runs on the newest credentials in the source's home, the same as
resuming the source. A named subscription's generation advances only on a
real sign-in whose old tokens may be revoked, so a fork across one is refused
with "use Continue in new agent". A Provider upgrade that changes the auth
contract makes the source's generation unlaunchable, so such a fork fails at
its first launch, as the source itself would. The fork
uses that login's currently admitted
release of the same Provider; inheritance keeps only settings that release
declares. If the admitted session no longer matches, it is kept but marked so
that it never forks, and the request reports it for deletion.

## Native contract

The destination's first worker launch carries `native_fork` instead of a
resume identity. The worker performs ACP `initialize` and requires
`sessionCapabilities.fork`; it then sends `session/fork` with the source
native session ID, the destination's own working directory and the same
Provider `_meta` as `session/new`. A message fork adds
`_meta.jetbrains.air.fork = {"version": 1, "messageId": ID}`. Both pinned
adapters implement this form (`claude-agent-acp` resolves the message to its
SDK `upToMessageId`; `codex-acp` resolves it to `thread/fork.lastTurnId`).

The fork is strict. A missing capability or a failed `session/fork` fails the
worker start with the adapter's reason; it never falls back to
`session/new`, which would present an empty agent as a fork.

The forked native thread already has history, so the worker reports its ID as
materialized. The destination's transcript begins with the fork marker, which
counts as context for the resume rule (`agent_session_id_for_resume`), so the
ID is persisted at once and every later launch resumes it like any other
session. Clearing context discards the fork like any other history.

## Persistence and retries

`POST /api/sessions/:id/fork` takes an `operation` ID and an optional
`message_id`. The destination's DB-backed `session_fork:<id>` record (source
session, source native ID, requested message, fork point, operation and
state) is committed before the destination is created, so a restart never
restores the destination without its fork intent, and workspace preparation
reads its base from it. The same operation with the same request returns the
existing destination while it exists; a different request under the same
operation is refused.

A whole fork pins the source's newest answer at admission as its fork point,
so turns the source completes before the fork's first launch are not copied.
The record stays `pending` until the worker reports the forked native ID. That
ID is saved before the record becomes `forked`; from then on the session
resumes it like any other, even before its first prompt. A restart after the
adapter forked but before the new ID was saved at most leaves one unused
native thread behind; it never sends a prompt twice. Clearing the fork's
context marks the record `discarded`, before or after its first launch, so a
cleared fork never brings the source conversation back.

Controllers send a fork launch only to a Machine connection at protocol 39 or
later and refuse it otherwise; an older Machine would ignore the field and
open an empty thread. Execution Machines before protocol 39 reject the fork
base in workspace preparation.

## Deletion

Deleting a fork removes only its Cowboy session and worktree. Codex
`session/delete` archives native threads rather than erasing them, so native
cleanup follows each Provider's own retention.
