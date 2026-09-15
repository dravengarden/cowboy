# Original-owner native buffer references

Status: Machine/Zed implementation and conformance candidate. The ordinary
Controller/Web buffer API is **not yet switched**. This is a prerequisite for
cross-request ownership, not its production acceptance or a generic DAG executor.

## Why a new reference is needed

The legacy HTTP API sends a browser lease string with a path. Each request
resolves the current Session workspace; the native adapter also canonicalizes
the file on close. A retarget, deletion or missing open reply therefore cannot
be repaired by sending the same path to whichever runtime is current.

The optional Zed 1.3.0 private interface allocates a reference **before** opening
the native buffer. Core can then retain that original reference across an
ambiguous open reply. Communication, authorization, installation, resource
ownership and Machine routing remain core mechanisms, not another Plugin.

| Private request | Input | Effect / observation |
| --- | --- | --- |
| `prepareBuffer` | Original absolute worktree and relative file | Validate a ready worktree; allocate an effect-free reservation |
| `openBufferLease` | Original `lease` only | Open at most once; duplicate reads never repeat an ambiguous native open |
| `queryBufferLease` | Original `lease` only | Read saved process-local state without reopening or resolving paths |
| `releaseBufferLease` | Original `lease` only | Release that owner of the recorded native buffer, without filesystem lookup |

Replies are closed `bufferLease` / `api_version: 1` values with the exact
`lease` and one state: `prepared`, `open`, `released`, or `unknown`. A lease has
an unpredictable 128-bit adapter `instance` (32 lowercase hex characters) and a
non-recycled monotonically increasing `id` (16 lowercase hex characters).
Strings avoid JSON's unsafe integer range. Zero, future, malformed and foreign
IDs fail. A missing **issued** ID is retired; it can never open again. No
unbounded native tombstone history or recycled slot ID is necessary.

These references are not bearer permissions, serialization of authority, or
restart records. The installed runtime's private socket is the native boundary;
the future Controller consumer must separately own the original Service,
principal, Session incarnation and Machine connection. A guessed or restored
path, Plugin digest, PID, instance string or browser correlation ID cannot mint
that ownership.

## Native ownership

Preparation captures the canonical target and the process-local worktree
incarnation. Open checks the same target and incarnation while holding its
worktree observation through I/O; it rejects a changed symlink or close/reopen
ABA. This does not prove continuous filesystem inode identity or fence unrelated
filesystem writers. Active owners are a typed union: legacy strings and native
reference IDs cannot collide. Closing one owner preserves every other owner of
the same buffer. Closing a worktree does not destroy its still-open buffers.

Before native open is attempted, the slot becomes `unknown`. Cancellation or a
native failure cannot replay it or pretend to release a buffer whose native ID
was never acquired. If native open completes but the socket observer goes away,
the saved `open` record remains queryable using the previously returned handle.
Open/ambiguous slots never expire or disappear under LRU pressure.

Release uses the stored buffer key and owner, not `canonicalize` or a new
worktree lookup. Deleting or renaming the file/worktree therefore cannot redirect
it. A missing expected active owner is an error, not fabricated completion. The
native Zed `CloseBuffer` protocol has no ACK: `released` proves local ownership
removal and successful close enqueue when needed, **not** a verified native
recovery transaction. Native transport rejection retains local ownership. It
never undoes file edits, deletes source data or restores an Agent session.

Native capacity is 1,024 outstanding references. Only effect-free preparations
expire, after 30 seconds; expiry is checked on subsequent lease operations.
Each supplied and resolved path is limited to 4,096 bytes. Exhaustion rejects
before a native open. Unknown effects consume capacity until independent
resolution or process teardown; they are not silently discarded. The socket
reader/writer also enforces 4 MiB newline frames and a 35-second write deadline.

## Machine ownership

The Machine reserves capacity before preparing and retains the exact
`RunningCodeRuntime`, original worktree route and native reference. It supports
these commands only on installed Code Plugin generations, not mutable legacy
socket paths. Subsequent open/query/release bypass filesystem resolution and
installation selection entirely. An old process's death stays unavailable even
after the same worktree selects a replacement process. Other worktrees and
generations can continue independently; uninstall allows retained leases to
drain without selecting a new package or re-enabling legacy execution.
Observed adapter death also consumes its original process-group cleanup owner
once, so retaining unknown evidence cannot retain its orphan descendants or
signal the group again on later queries.

Pending preparations count toward the Machine's 1,024-reference limit. A
failed/cancelled preparation drops an otherwise unleased runtime; existing
worktree/buffer leases remain intact. Once open may be dispatched, its expiry
is removed before any await. Missing open/release replies retain their original
runtime and evidence for a read-only query. Expiry and terminal cleanup take
the route lock before removing the record, so cancellation cannot strand the
only cleanup owner. Successful local-release receipts retain no process and are
bounded to 1,024 entries; older evicted evidence becomes unavailable, never
permission to replay. These are process-local observations, not durable Machine
operation receipts.

## Compatibility and remaining integration

The Zed Plugin and private adapter independently advance from 1.2.4 to 1.3.0.
The exact Zed server remains 1.13.0 at revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`; no runtime dependency is upgraded.
The adapter directly consumes the already-locked `getrandom` 0.4.3 for instance
identity. Existing API-1 worktree/buffer/language replies and the signed package,
Code payload, SDK and Catalog reader formats remain compatible. The adapter's
health adds `buffer_lease_api: 1`; unknown new commands fail on older adapters.

Health is not Machine capability negotiation. The new core-only, pathless
`bufferLeaseSupport` request returns `{ "type": "bufferLeaseSupport",
"api_version": 1 }` without selecting a Plugin or touching resource state. An
older Machine's generic dispatcher rejects the absent worktree instead of
forwarding this probe to a native Plugin. A future consumer must validate that
reply on its original authenticated Machine connection; it proves host support,
not installation or execution authority. Then it must accept the exact Code
generation and add core original-principal/Session ownership with an admitted
continuation, wire cancellation and stale-owner release, and switch the Web
consumer. Language/hover/navigation reads also need that resource lifetime.
The legacy HTTP API remains unchanged in this candidate and retains its known
cross-request limitations. Never enable the new consumer merely because an
adapter health response advertises this optional API, or restore a handle after
a Controller/Machine/native restart.

Unit/socket/process fixtures cover identity, duplicate/unknown outcomes,
cancellation, capacity, expiry, filesystem changes, wrong replies and independent
native generations. `just zed-plugin-conformance` additionally installs a
temporary signed exact adapter/server package, mixes legacy and owned leases,
uninstalls, removes/renames original paths, drains references and reactivates
verified retained bytes. This does not install on a registered Machine, use
Service credentials, grant production maintenance or accept native recovery.

Publication, Machine maintenance and actual Zed installation remain separate
release steps. A Controller-only deployment cannot activate these Machine/native
mechanisms. General graph resolution, state leases and independently authorized
post-effect recovery remain in the [completion ledger](plugin-refactor-completion.md).
