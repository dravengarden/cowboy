# Machine-owned Session filesystem observations

Protocol 25 adds a private core `session-code` adapter envelope with three
operations: observe a cwd directory, verify its opaque incarnation, and read
through that incarnation. It extends the [Session read-route contract](plugin-session-read-routes.md)
for the eleven buffered filesystem/Git readers. Native Zed queries, buffer
ownership, Session lifecycle, security domains and state leases retain their
separate contracts.

The Controller first checks the original product credential and Session
visibility. It observes the remote cwd through the original captured connection
before any read cache or page/diff continuation lookup. The Machine-minted
32-character incarnation participates in equality, hashing and cache string
budgets alongside the logical Session and original connection. Machine root
verification surrounds the complete buffered response, including errors, cache
hits and 304. A failed verification returns HTTP 410 with no-store and strips
the complete prior body and ETag. A fresh HTTP request may observe the replacement
object; it cannot reuse a continuation from the old object even when file bytes
and the pathname are identical.

The Machine admits only absolute roots under the current configured workspaces
or its managed worktree root, preserving core Code's trusted-root boundary.
Admission and observation are serialized with configuration retirement; a
queued request cannot mint from a withdrawn snapshot. Withdrawing a configured
root or alias retires Session observations beneath it without depending on an
intermediate inventory advertisement. Unrelated roots retain their observations.
The execution-binding target Machine owns the observation, rather than the
Session's possibly different runtime Machine.

A separate process-local registry retains at most 256 open directory handles.
Handles pin the original inode; device/inode and optional creation time describe
that exact object. Replacement, disappearance, accepted configuration withdrawal,
Machine restart or FIFO capacity eviction ends its incarnation. Re-observation
mints a new random value. A stale token cannot retire a newer one. These
incarnations are neither advertised workspace identities nor persisted tokens,
Plugin grants or filesystem authority.

Directory opens use `O_DIRECTORY`: a FIFO, socket or regular-file replacement
is refused before opening can wait for a writer or touch a device.

The Machine verifies a carried incarnation before core Code dispatch and again
before producing its result. It consumes the envelope itself and forwards only
the existing Code request to the isolated core adapter. It never forwards this
private envelope to a Plugin or native Zed. Observation/verification waits are
bounded to three seconds and cancellation removes only the original waiter;
actual Code reads retain their existing timeout and cannot switch connections.
This is finite observation and result suppression, not an atomic filesystem
snapshot or cancellation/rollback of dispatched work.

Machines below protocol 25 retain the explicit legacy remote read path and do
not claim this fence. Local and colocated reads retain the Controller-owned
physical root observation. Negotiating protocol 25 alone grants no authority.
Deployment requires both Machine and Controller adoption; upgrading just one
leaves the pair on its lower negotiated protocol.
