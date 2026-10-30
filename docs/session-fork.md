# Session fork

A fork is a new Cowboy session that continues the same native conversation
on a separate branch. The source session and its native thread stay intact;
both can then diverge independently. Unlike
[Continue in new agent](session-context.md), which links context across
accounts or Providers through Cowboy MCP, a fork asks the Provider adapter to
copy its own native transcript with ACP `session/fork`, so the new session
keeps the complete model context and can reuse the Provider prompt cache.

## User flow

- **Fork** in the session menu (mobile menu, Desktop session actions with `F`)
  forks the whole conversation in one step. It does not open a form: the new
  session opens at once (on Desktop with the composer focused; Mobile keeps
  the keyboard down until the user taps the composer). While the source is
  working, the item waits for the turn to end.
- **Fork from here**, shown under each assistant message that ends a turn,
  forks the conversation up to and including that message. Later turns are
  not part of the fork. It needs the Provider message ID that Claude Code and
  Codex attach to their answers.
- `sessions.fork` in Cowboy MCP and `POST /api/sessions/:id/fork` expose the
  same operation to agents and scripts.

The fork keeps its source's Provider, release, subscription, runtime Machine,
execution Machine, project and supported settings (model, reasoning,
permissions, Memory, Calls, send policy and automatic Provider updates). Its
title is the source title with a ` (fork)` suffix, it is placed directly after
the source in the same folder, and its list row shows a **Fork** badge. Its
transcript starts with a "Forked from SOURCE" marker that opens the source.
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
silently starting from the default branch.

## Admission

The Controller admits a fork only when all of the following hold. Each refusal
names its reason and creates nothing.

- The source is owned by the caller, is not a system session, managed child
  or trashed session, and has a native thread that can be resumed.
- **Fork** requires an idle source: no prompt in flight, nothing queued and no
  native background work, because the adapter copies the native transcript as
  it is on disk. **Fork from here** may target a completed turn while a later
  turn is running.
- The message, when given, is an assistant message in the source transcript
  that belongs to a completed turn.
- The Provider is one whose adapter fork has been verified (`claude-code`,
  `codex`). Other Providers refuse until their adapters are verified.
- The runtime Machine speaks Machine protocol 39 or later, which carries the
  native fork launch and the forked workspace base.

The destination is created through ordinary session admission with the
source's account, Provider, Machines and project, and then checked again: the
admitted account, login generation and runtime Machine must equal the
source's, because the adapter reads the source thread from that account's
native home on that Machine.

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
`message_id`. The source records the receipt as the DB-backed
`session_fork_operation:<source>:<operation>` preference; the same operation
with the same request returns the existing destination, and a different
request under the same operation is refused. The destination stores
`session_fork:<id>` (source session, source native ID, message ID) until its
native thread exists. A Controller restart before the first worker launch
therefore still forks rather than starting a blank thread. A restart after the
adapter forked but before the new native ID was saved at most leaves one
unused native thread behind; it never sends a prompt twice.

## Deletion

Deleting a fork removes only its Cowboy session and worktree. Codex
`session/delete` archives native threads rather than erasing them, so native
cleanup follows each Provider's own retention.
