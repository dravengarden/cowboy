# Session code-read scopes and immutable diff cursors

Core now carries one typed observation of a Session's code workspace through the
existing Zed calls and filesystem/Git HTTP readers. This is a finite read
boundary in the [Plugin design](plugin-spatiotemporal-design.md), not a new Plugin lifecycle,
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
- All eleven filesystem/Git HTTP entrypoints share a Controller-owned response
  boundary: reference search, Code search, directory tree, manifest, changes,
  repository history, commit, commit diff, diff pagination, file pages and raw
  file bytes. They resolve once, recheck before invoking the reader closure and
  recheck after the entire buffered response is ready. Local I/O, remote I/O,
  cache hits, early errors and conditional `304` returns cannot skip that final
  observation. Unknown contexts are 404; a changed context replaces the entire
  response with `410` and `Cache-Control: no-store`, without its old ETag or body.
  The reference picker now uses the same `unknown code context` 404 message.
  Existing HTTP authentication and Machine trusted-root checks remain independent.
- Diff cache entries include that typed owner instead of just a session name
  and cwd. Expired cursors are 410. The unified response boundary also covers
  both first pages and cursor continuations, including cache lookup errors.
- Each completed diff snapshot has a random 256-bit cursor identity independent
  of its content revision. Equal text in different files/options cannot select
  another entry; eviction and regeneration cannot revive an old cursor. The
  revision still hashes the actual text, concurrent first reads still coalesce,
  and the existing entry/byte/TTL bounds remain unchanged.
- Cursor offsets must be in range and on a UTF-8 boundary before any slicing. A
  crafted offset inside a Chinese character returns 400 instead of panicking.

Core remote Code requests now serialize the existing closed Rust
`CodeAdapterRequest` / `CodeOperation` types instead of constructing free-form
JSON per HTTP handler. All ten operation variants, optional repository/file
cursors and three diff scopes keep their existing JSON shapes. Response-kind
matching still rejects a wrong remote variant; this is not a new negotiated
protocol or a serialized scope grant. The independent adapter's trusted-root
validation is unchanged. The installed Zed Plugin has a separate protocol and
is not repackaged by this change.

Diff cursors retain the opaque `64-hex:offset` wire shape. Web already passes them
through unchanged. Those cache-entry observations are not durable state: old
cursors expire at Controller activation as they did on previous restarts. No Web
bundle, Plugin package, Machine protocol, SQL migration, persistent data format,
host policy or native ABI changes.

## Evidence and remaining work

Two regression tests failed before the repair: equal-content files returned the
wrong path on continuation, and a non-character-boundary offset panicked. The
expanded source gate covers all Workspace identity axes, Session cwd ABA,
delete/recreate, independent Hubs/Sessions, metadata drift, cache eviction,
coalescing, exact page reconstruction, real local Unix sockets and fixture
channels in the authenticated Machine registry, with a scope change while
waiting for the reply.

The expanded buffered-response source tests use real Hub observations and
deterministic channel handoffs. They cover retarget, cwd ABA, deletion and
recreation crossed with successful, conditional and failed responses; a stale
observation cannot invoke the reader closure, even its synchronous setup. Stable
metadata edits retain the exact directory-cache bytes/headers, conditional
status and ordinary errors. Fourteen request vectors compare the closed
serializer with the historical wire shape and round-trip the adapter reader;
the adapter tests also run in its independent feature graph. These are source
fixtures, not actual HTTP/native Plugin installation acceptance.

This does not revoke or compensate a Zed effect already dispatched, release a
pre-existing buffer on a retargeted workspace, atomically fence all Machine
effects against Session edits, or accept an exact new Code/Agent installation.
Discarding a stale reply is not rollback: already-started reads, manifest Zed
readiness and rebuildable physical-path directory-cache fills are not undone.
The final observation is not an atomic transaction with HTTP delivery or a
future streaming-body fence. File-content digest cursors retain their existing
format; unlike diff entry cursors they do not acquire cross-request Session
lifetime binding here. Continuous Workspace observations, principal/policy
admission, general state leases and independent post-effect/native recovery remain
in the
[completion ledger](plugin-refactor-completion.md). Tests use disposable state
and synthetic transport replies, not a production login or native-generation
acceptance.

The [initial diff/Zed Controller release](releases/plugin-code-read-scopes-2026-09-15.md)
passed the complete gate and retained its 15 observed workers. The subsequent
[buffered-reader release](releases/plugin-buffered-code-reads-2026-09-15.md)
passed the expanded gate, is published and activated, and retained all 16
observed workers, Machine and Victoria in its own bounded deployment window.
