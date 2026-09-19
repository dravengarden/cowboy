# Session code-read scopes and bounded continuations

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

The original advertised Workspace snapshot includes Service, Machine, workspace ID and
advertised canonical path. The Service rereads its non-revoked Machine record
when resolving or rechecking that snapshot. It does not canonicalize a remote
path locally. This is snapshot comparison, not a continuous inventory lifetime,
security grant or proof of filesystem identity: an unobserved Workspace
remove/re-add cycle and a same-path filesystem replacement still need the
Machine-owned resolution boundary.

The later [continuous Workspace read scopes](plugin-workspace-read-scopes.md)
replace that string-only snapshot with a private authenticated-registry
observation. Observed remove/re-add and connection replacement can no longer
revive old reads or cursors. Unreported filesystem/configuration changes and
Machine-owned state leases remain outside that finite Controller boundary.

The later [Session read-route binding](plugin-session-read-routes.md) likewise
joins the logical Session observation to the original core registry and Machine
connection for buffered filesystem/Git reads and cache continuations. A logical
Session alone is no longer their cache key; named colocated Machines cannot
fall back to a saved local-route flag after disconnection.

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

## File-page continuations

The Controller now binds each file continuation to its original typed code-read
observation and exact requested path. A bounded process-local registry translates
the opaque public token into the existing native `revision:byte-offset` cursor
only after that match, before local or remote I/O. Session retarget/ABA,
delete/recreate, independent Hubs or Sessions, a changed advertised Workspace
tuple, another path and a changed offset cannot adopt a prior token. The outer
buffered-response guard still rechecks the original observation after I/O.
Neither the public token nor the native cursor is authorization.

The registry retains no file contents. It is limited to 512 entries, 1 MiB of
logical identity-string bytes and ten minutes of idle lifetime, with lazy expiry
and LRU eviction. Concurrent identical first pages reuse a live binding; eviction,
expiry and Controller restart create a different random 256-bit identity. Only
the offset remains visible in the historical `64-hex:offset` shape. Existing
unbound native cursors return `410`, not an implicit import. Web already reloads
file pages on `409/410`; no bundle or native protocol upgrade is required for
the Controller binding cutover.

Projection validates revision shape, byte offsets, progress, page size and
terminal/truncated/limited flags. Malformed tokens are `400`, missing bindings
are `410`, changed revisions are `409`, invalid backend pages are `502` and
oversized retained identities are `503`; these errors have `no-store` and no
ETag. Existing physical-file metadata revisions and content-cache digests remain
separate domains; changing readers cannot silently continue with another revision.
The binding is to the requested path, not an independent filesystem inode proof
or a claim that a remote adapter's contents have been independently attested.

File ETags hash the exact serialized page, including its public continuation.
Two pages of one revision no longer share an ETag, and regenerating an expired
continuation forces fresh JSON instead of `304` with an unusable cached cursor.
Conditional reads accept exact strong/weak tags, lists and `*`, not substring
matches, and keep `private, max-age=0, must-revalidate`.

Both local and content-cache readers now cut 256 KiB pages only at UTF-8
boundaries, preserving the existing newline preference. An incomplete codepoint
at actual EOF is an error; a partial codepoint at the 32 MiB view limit is
trimmed without issuing a cursor into that partial tail. Long two-, three- and
four-byte-character lines reconstruct exactly. This shared reader also builds
in the independent core Code adapter. A Controller release updates colocated
reads and all Controller bindings, but **does not upgrade a running remote
Machine adapter**; its UTF-8 repair needs the separate Machine maintenance lane.
The signed Zed Plugin is not changed or repackaged.

## Zed operation connection lifetime

Each Session Zed call sequence now owns a non-serializable Controller operation.
`ensureWorktree` and `openBuffer` share its original authenticated Machine
connection, or one already-connected local Unix peer. A replacement connection
cannot be adopted even if it repeats the same Machine and epoch strings. The
Machine command registry checks that original token atomically with enqueue and
again after awaiting the reply: completion just before a reconnect does not
make a parked reply current. The existing exact Session observation is still
checked before and after each exchange. Ordinary unbound adapter callers are
unchanged; transport identity does not become a Session authorization grant.

Local calls retain the connected socket rather than resolving its pathname for
the next step. Replacing the socket pathname cannot redirect that sequence.
Connect has a two-second deadline; each write-and-read exchange has a combined
35-second deadline, a 4 MiB serialized request limit and a 4 MiB newline-framed
response limit. EOF without the newline is incomplete, not an accepted reply.
Failure or cancellation consumes the operation's transport, so a subsequent
request cannot reuse an unread response or implicitly reconnect. Pending remote
correlations are removed independently; no cleanup RPC or retry is invented.

Opening a buffer requires the exact `worktree`, API-1, `ready` response before
the second request. Non-ready states and wrong response variants now return
the existing HTTP `503` readiness failure without opening a buffer. Normal
open/close payloads and responses are unchanged. Closing does not acquire a
new worktree readiness lease.

This is **one operation's transport continuity**, not a native-generation lease
or complete buffer ownership. Separate HTTP close requests still carry only
the browser's lease ID and resolve their current Session target. The protocol
does not yet carry an original core-owned resource handle. A retargeted Session,
deleted/renamed file, Controller restart or lost open reply may therefore leave
an old native buffer uncollected. Reusing client lease strings is not proof of
resource identity. Fixing that needs original-owner release semantics, bounded
cleanup/unknown evidence and an exact native-generation protocol; hashing the
current cwd or following a replacement connection is not a substitute.

The [native buffer-reference candidate](plugin-native-buffer-leases.md) now
implements pre-effect preparation, non-recycled native handles, path-free
release/query and exact retained Machine process routing. It does not switch
this HTTP API or establish its original principal/Session resource owner.
Machine capability-floor acceptance, the Controller/Web consumer and separate
Machine/Code Plugin activation are still required; Controller activation alone
cannot close this gap.

The additive [Controller buffer-owner API](plugin-controller-buffer-owners.md)
now retains that original principal/Session/connection and admits bounded
continuations independently of their HTTP observers. It does not replace the
legacy endpoints described above or bind their language reads to a resource.
Native rollout and the actual Review consumer switch remain separate.

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

Three file-page regression tests failed before the repair: cached and uncached
long multibyte lines failed decoding, and incomplete UTF-8 at EOF returned a
nonprogressing continuation. The expanded tests cover all scope axes above,
byte/count/idle bounds, concurrent issue, offset tampering, invalid native pages,
empty and limited terminal pages, representation ETags and a scope change after
projection. A real local Unix adapter fixture round-trips opaque-to-native
translation and reconstructs the whole long Unicode file. The standard complete
gate now runs the independent Code adapter library tests as well as its build
check. These fixtures do not prove a deployed remote Machine upgrade.

Two Zed sequence regressions failed against the prior implementation: a reply
completed immediately before same-epoch reconnect allowed the next request on
the replacement channel, and non-ready worktree states still opened buffers.
Sixteen new source tests cover these, normal open/close wire shapes, wrong
variants/versions, cancellation, late completion, foreign registries, pending
waiter isolation, response framing and limits, local write/read deadlines and a
real Unix socket pathname replacement between calls. They use disposable peers
and authenticated-registry fixtures, not an installed native-generation or
production lease-recovery acceptance.

This does not revoke or compensate a Zed effect already dispatched, release a
pre-existing buffer on a retargeted workspace, atomically fence all Machine
effects against Session edits, or accept an exact new Code/Agent installation.
Discarding a stale reply is not rollback: already-started reads, manifest Zed
readiness and rebuildable physical-path directory-cache fills are not undone.
The final observation is not an atomic transaction with HTTP delivery or a
future streaming-body fence. Continuous Workspace observations, principal/policy
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
The [file-page Controller release](releases/plugin-file-page-scopes-2026-09-15.md)
passed both complete gates, is published and activated, and retained its 16
observed workers. Its independent immutable core adapter passed seven disposable
process cases; a running remote adapter upgrade remains separate.
The [Zed operation Controller release](releases/plugin-zed-operation-scopes-2026-09-15.md)
passed the complete gate and exact 72-release reads by candidate, predecessor
and cold Controllers. It is published and activated with its 12 observed workers
retained; cross-request buffer recovery and native-generation acceptance remain
separate.
