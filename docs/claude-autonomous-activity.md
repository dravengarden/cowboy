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
