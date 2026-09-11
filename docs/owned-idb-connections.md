# Owned IndexedDB connections

The sixteenth spatiotemporal slice gives the browser persistence backend an
explicit local owner. This is a core implementation component, not an installable
Plugin, a cross-tab lock, or a durable compensation executor.

## Authority and ordering

`createIdbPersistenceOwner` owns one configured database/store and all of its
connection generations. Construction acquires no browser resource. Related
clients borrow `owner.persistence<S>(key, { strictWrites })`; that interface
cannot dispose the database. Unrelated owners do not share a mutable connection
registry, even if they address the same database. Configuration is snapshotted.

The Web product owns one database. Permanent sign-out first fences the socket
and creation of any new sync client, synchronously seals every existing writer,
waits for all final writes to settle, then closes the database. Failure of one
writer does not skip sealing the others or releasing the database. Reentrant
shutdown uses the same barrier. Late queue enumeration cannot create a writer.
The typed local sign-out event accepts cleanup barriers synchronously; the auth
gate awaits them before navigation. This wait has a one-second deadline, after
which navigation may proceed with the explicit `pending` outcome, not a false
successful disposal. Failure is distinct from a clean drain. Server logout is
not delayed by this barrier, and a stuck IDB operation cannot trap logout. Abrupt or
deadline-driven navigation may still interrupt unfinished local cleanup.
Pagehide/visibility flush, view unsubscribe, WebSocket reconnect and temporary
reauthentication do not end this owner. No Machine session or worker is stopped.

`dispose` synchronously rejects new work, but an already-admitted write can
finish opening its database and commit. It is not cancellation/undo authority.
Each actual transaction has an exact lease held through its terminal event.
Reads and writes both wait for `complete`; request success alone is insufficient.
Request errors retain the lease until `abort`/`complete`, including read-only
abort with no request error. A synchronous request/clone failure aborts the
transaction and still waits for its terminal event.

Version-change notifications retire and close that connection synchronously;
existing transactions still drain. Normal `close()` emits no close event, so
cleanup waits for owned transactions, not for a nonexistent notification. Old
events cannot evict a replacement generation. Only `transaction()` throwing
`InvalidStateError` before a transaction exists permits one fresh-open retry.
There is no automatic retry or timeout that reports success after transaction
creation. These boundaries follow the
[IndexedDB transaction lifecycle](https://w3c.github.io/IndexedDB/#transaction-lifecycle)
and [connection-close algorithm](https://w3c.github.io/IndexedDB/#closing-connection).

## Bounded open results, honest cleanup

Opening has a configurable 1–60000ms logical deadline (default 10000ms).
Blocked opens fail immediately. Neither result claims the native request was
cancelled: IndexedDB provides no general open cancellation handle. An abandoned
native request stays owned and prevents an unbounded retry queue. Late success
closes its handle without starting the abandoned operation; late upgrade aborts
without creating schema. A settled failed request permits a fresh later attempt.

The readonly lifecycle snapshot includes phase, admitted tasks, the generation
cleanup holder, cleanup failures, pending native opens, retained connections and
transaction leases. A native open/transaction that never terminates leaves
disposal `draining`. A failed close is retained as `needs_reconcile`, without
automatic retry. Error codes are a closed union and never copy database names,
keys, values or native exception text into diagnostics. Product cleanup warnings
are static.

## Compatibility and limits

The database `shared-utils-sync`, store `clients`, version 1, record keys and
structured-clone values are unchanged. An existing database missing that store
fails closed; no implicit version bump, schema rewrite, data deletion or
downgrade is attempted. Closing does not remove an outbox or send compensation.

`S` types the caller's persistence contract, not untrusted stored bytes. This
slice adds no general snapshot codec, cross-tab single-writer arbitration,
cross-process cleanup proof, or guarantee that abrupt browser termination can
finish asynchronous cleanup. Those are separate concerns. Existing pre-send
durability barriers remain necessary.

The legacy `idbPersistence` convenience function now returns its own disposable
single-record owner; callers must close it after its clients drain. The legacy
one-shot `idbListKeys` waits for its private owner's entire cleanup, which can
remain pending behind an unresolved native open. Both are deprecated in favor of
the explicit owner; product code uses neither. There is no hidden global cache.

Component release 3.3.0 appends state-sync-idb 1.5.0 and its exact state-store
2.1.0 peer edge. State-sync stays 1.4.0. All seven Plugin sources, versions,
2.9.0 pins and signed artifacts remain unchanged. Only Web activation is needed.

## Acceptance

Deterministic event tests cover leases, isolation, shutdown ordering, aborts,
clone failure, closing/retired generations, blocked/late acquisition, deadline
validation, schema mismatch, retained cleanup failure and legacy helper cleanup.
Integration with the actual replicated client proves an admitted outbox survives
shutdown and a new owner resends its original mutation id. Compile-only negative
contracts separate borrowed data access from owner authority.

For actual browser behavior, build `nix build .#cowboy-idb-test-browser --no-link
--print-out-paths`, then run `nix develop -c just idb-browser-conformance
/nix/store/<result>/bin/firefox`. The optional Nix-pinned Firefox is a test tool,
never a product/Plugin runtime dependency. The runner bundles the real component,
uses an exclusively created empty profile and private loopback network, clears
inherited browser environment, and bounds the run to 30 seconds. It removes only
its own temporary profile/fixtures and prints a browser/bundle-digest receipt.
Eight real-engine cases cover cloning/commit, draining admitted opens, write and
read abort, clone failure, upgrade retirement, incompatible schema and a timeout
behind a real held upgrade transaction. This is not Safari, native-shell or
physical-iPhone acceptance. P1's remaining host/security/SDK migrations and
P2–P4 distributed execution and durable compensation remain outstanding.
