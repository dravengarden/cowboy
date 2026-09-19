# Claude autonomous execution activity

## Missing loading indicator, 2026-09-15

The `upgrade` session (`sess-1789450485879`) received `turn_end: EndTurn`
at 16:05:47 Asia/Shanghai (event 1446), followed by `lifecycle: running`
(event 1447). Claude subsequently resumed when a background shell task
completed. The screenshot's commands are events 1814–1867, starting at
16:09:19. They arrived without another Cowboy `session/prompt` request or
`busy` lifecycle edge. The session stayed idle while emitting new tools and
assistant messages.

The transcript's loading row and the session status spinner both correctly
consume `busy`. The missing signal was in the ACP client: it forwarded ordinary
`session/update` notifications, but tracked execution only through the lifetime
of a Cowboy-issued `session/prompt` request. Those lifetimes differ when the
Claude SDK autonomously resumes after a background task completes.

## State ownership

For the existing stable Claude session behavior, new, resume, and load requests
opt into `_meta.claudeCode.emitRawSDKMessages`, filtered to
`system/session_state_changed`. `_claude/sdkMessage` carries the SDK's native
`running`, `requires_action`, and `idle` state even outside a pending prompt.
The adapter already enables these SDK notifications.

The worker combines the two inputs:

- A pending prompt remains busy even if an earlier native idle arrives late.
- Native running or requires-action keeps an otherwise idle session busy.
- When both lifetimes are idle, the worker reports running (ready).
- Startup, terminal states, and recoverable prompt errors retain their existing
  ownership. Native activity cannot clear an error.

The extension is restricted to the selected behavior and bound native session
identity. History replay, unrelated messages, and unknown states are ignored.
Raw SDK messages are not persisted or forwarded into the conversation, and
autonomous activity does not fabricate user prompts or turn-completion events.

## Verification and delivery

`src/acp_session_conformance_tests.rs` exercises real ACP byte streams across a
prompt response, native idle, autonomous restart, and final idle, with exactly
one prompt and one turn completion. Further tests cover late idle, permission
waits, failure preservation, replay suppression, and session/provider isolation.

This is a Machine/worker change using the existing runtime status protocol.
Deploy `cowboy-machine-release` through the machine maintenance transaction;
new or safely replaced workers subscribe when establishing their ACP session.
Existing clients consume the corrected status without a Web release.

## Waiting between autonomous stretches, 2026-09-19

The `offline first` session (`sess-1789788450509`) started two `Monitor`
tasks watching a detached gate and release build, then replied "等结果"
and ended its prompt with `turn_end: EndTurn` at 14:23:13 (event 2208).
For the next ten minutes native state alternated: each Monitor event woke
the SDK for a few seconds (`busy`, "No response requested."), then returned
to `idle` (`running`). The spinner therefore showed only during those short
wakes, while the agent was continuously waiting on work it would resume on.

Native execution state is correct here: no model turn is running between
wakes. The missing fact is the live background-task set. The SDK publishes it
as `system/background_tasks_changed`, a level with REPLACE semantics. Its
`ambient` flag excludes housekeeping tasks and live-update watchers; a
`Monitor` (`local_bash`, kind `monitor`) and a backgrounded shell are not
ambient and count as activity.

### Ownership

- The worker adds `background_tasks_changed` to the same raw-message filter,
  counts non-ambient tasks for the bound native session, and reports changes
  through `AgentSink::set_background_tasks`. Attaching a native session resets
  the level because the SDK sends none at startup.
- The count crosses the runtime wire as the Cowboy-owned update
  `cowboy_background_tasks` (not a new `RuntimeEvent` variant, which an older
  peer could not decode) and as the optional `WorkerSnapshot.background_tasks`
  so a Controller reconnect restores it.
- The Controller projects it onto transient `SessionMeta.background_tasks`,
  never into the transcript. Worker restart, exit, and interruption clear it.
- It is presentation only. Busy still means a running turn: a background
  server may never finish, so the level must not hold the queue or block a
  send. The session status indicator spins while an idle session has
  background tasks and names the count in its label.

Deliver Controller and Web before the Machine release. A worker from the new
Machine release talking to an older Controller would record the update as an
unrecognized transcript row.
