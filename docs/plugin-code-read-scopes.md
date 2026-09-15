# Session code-read scopes and immutable diff cursors

Core now carries one typed observation of a Session's code workspace through the
existing Zed call and diff-reader paths. This is a finite read boundary in the
[Plugin design](plugin-spatiotemporal-design.md), not a new Plugin lifecycle,
general graph resolver or execution grant.

## Identity and lifetime

`CodeReadScope` separates a Session observation from an advertised Workspace
snapshot. A Session observation captures the exact Hub incarnation, Session,
Machine, workspace identity, cwd and product owner under the existing session
lock. It has no public constructor, serde implementation or native-worker
ownership. Two Hubs or two Sessions with identical names and paths cannot share
the observation.

Creating/restoring a Session establishes a fresh Controller-local incarnation.
Retargeting its cwd replaces it; changing back does not revive earlier
observations. Deletion/recreation has the same property. Renaming a Session,
changing its status, or writing an unchanged cwd preserves continuity. These
observations neither persist across Controller restart nor stop detached native
workers. The Hub also compares the captured Machine/workspace/principal tuple.

An advertised Workspace snapshot includes Service, Machine, workspace ID and
advertised canonical path. The Service rereads its non-revoked Machine record
when resolving or rechecking that snapshot. It does not canonicalize a remote
path locally. This is snapshot comparison, not a continuous inventory lifetime,
security grant or proof of filesystem identity: an unobserved Workspace
remove/re-add cycle and a same-path filesystem replacement still need the
Machine-owned resolution boundary.

## Existing consumers

- Zed worktree readiness, buffer open/close, language, hover, navigation and
  outline calls retain their original Session observation. They no longer read
  the cwd from one session-list snapshot and the Machine from another. Both
  local socket and authenticated remote response paths check the observation
  before the call and after I/O; an observed change rejects the result. A
  multi-call buffer open cannot continue with its stale original observation.
- Diff first pages and cursor continuations resolve the current context, then
  check it again after asynchronous reading. Cache entries include that typed
  owner instead of just a session name and cwd. Unknown contexts are 404;
  changed or expired contexts/cursors are 410. Existing HTTP authentication and
  Machine trusted-root checks remain independent.
- Each completed diff snapshot has a random 256-bit cursor identity independent
  of its content revision. Equal text in different files/options cannot select
  another entry; eviction and regeneration cannot revive an old cursor. The
  revision still hashes the actual text, concurrent first reads still coalesce,
  and the existing entry/byte/TTL bounds remain unchanged.
- Cursor offsets must be in range and on a UTF-8 boundary before any slicing. A
  crafted offset inside a Chinese character returns 400 instead of panicking.

Cursors retain the opaque `64-hex:offset` wire shape. Web already passes them
through unchanged. They are in-memory observations, not durable state: old
cursors expire at Controller activation as they did on previous restarts. No Web
bundle, Plugin package, Machine protocol, SQL migration, persistent data format,
host policy or native ABI changes.

## Evidence and remaining work

Two regression tests failed before the repair: equal-content files returned the
wrong path on continuation, and a non-character-boundary offset panicked. The
expanded source gate covers all Workspace identity axes, Session cwd ABA,
delete/recreate, independent Hubs/Sessions, metadata drift, cache eviction,
coalescing, exact page reconstruction, and real local sockets/authenticated
Machine channels with a scope change while waiting for the reply.

This does not revoke or compensate a Zed effect already dispatched, release a
pre-existing buffer on a retargeted workspace, atomically fence all Machine
effects against Session edits, or accept an exact new Code/Agent installation.
Discarding a stale reply is not rollback. Other filesystem readers, continuous
Workspace observations, principal/policy admission, typed Code wire contracts,
general state leases and independent post-effect/native recovery remain in the
[completion ledger](plugin-refactor-completion.md). Tests use disposable state
and synthetic transport replies, not a production login or native-generation
acceptance.
