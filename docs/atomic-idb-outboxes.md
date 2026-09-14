# Atomic browser outbox deltas

This P3 slice fixes shared snapshot overwrite between updated Web peers. It is
a core state implementation, not an installable Plugin, authorization grant,
cross-tab lifetime lease or distributed compensation executor.

## The race

The product previously persisted every replicated client's complete
`{ base, pending }` over one shared record. Two tabs could both load an empty
queue, independently save A and B, and leave only B on disk. Serializing saves
inside one client did not serialize the other client's read/modify history.
Likewise a stale tab could restore a mutation another tab had acknowledged.

`createIdbPersistenceOwner().outbox<T>(key)` now borrows exactly one replicated
client handle for that record in this owner. Separate tabs own separate handles.
The old `persistence<S>` facade remains a value cache for other consumers, not an
outbox writer. Mixing either facade or borrowing two outboxes for the same key
in one owner fails. Both product registration paths use `outbox`.

Each save reads the current record, applies only the caller's delta against its
last observed/successfully saved snapshot, and puts the result **within one
readwrite transaction**. Updated peers preserve additions they have not seen.
Removing an observed pending id removes that obligation from the shared outbox;
an unchanged stale copy cannot re-add it. Local pending order is preserved within
the positions occupied by that client's mutations; unrelated peer order remains
unchanged. Globally unique mutation ids and server-side dedup remain required.

| Event | Shared pending |
| --- | --- |
| A and B independently load empty state | `[]` |
| A saves its new mutation `a` | `[a]` |
| B saves its new mutation `b` | `[a, b]` |
| A fresh B client actually loads `[a, b]` | `[a, b]` |
| A acknowledges `a` | `[b]` |
| B saves an old observed copy containing `a` | `[b]`, never resurrected `a` |

This is a snapshot-delta protocol, not a permanent dedup/tombstone ledger: reusing an old id
as a supposedly new mutation after its history disappears is not supported.
Explicit transport retries retain their existing id and pending operation.

Overlapping ids must have exactly equal client/name/JSON arguments across all
observed snapshots. Object key order does not matter; a 32-bit convergence hash
is not identity evidence. The decoder rejects malformed envelopes, duplicate ids,
non-JSON arguments, excessive nesting/traversal and more than 4096 pending
mutations. Failures contain only closed codes. `T` and the mutator registry still
belong to the caller; this is not their general runtime codec.

The base remains an offline paint cache. A newer cached version is retained,
except when a caller observes a lower version since its own prior snapshot.
Only a live forced resync establishes current server truth. These numbers are
not Service incarnations or policy epochs.

## Observation, commit and lifetime

`LocalPersistence.acceptLoadedSnapshot` is an optional synchronous handoff used
by the replicated engine. The outbox holds a private baseline and one issued
load result. Only after incorporating that exact result, and before notifying
observers or saving a correction, does the client accept it. A resolved read is
not itself proof the caller incorporated its pending mutations. Empty loads also
require handoff; a fabricated/copied/repeated handoff is rejected. Direct
consumers of `outbox` must implement this contract; `replicatedStore` does so.

Saves while a load awaits handoff fail strictly. A user-authored durable send
therefore leaves its editor content intact and never enters transport on that
failure. A failed or corrupt load is not interpreted as empty storage; this
record stays fenced until a fresh owner is created. A same-id conflict also
fences the record. Other records and owners keep working.

Saves are serialized within each handle and admitted snapshots are cloned.
The delta baseline advances only after transaction `complete`, not request
success. Merge/clone failure aborts; the existing transaction lease remains until
its terminal event. Abort does not advance the baseline and a later explicit
write can retry. There is no automatic transaction replay, timeout-as-success,
lock stealing or timer lease. Dispose drains already-admitted work; client
disposal still precedes database disposal, including failure. Reconnect, view
unsubscription and temporary reauthentication do not end this owner.

The implementation follows IndexedDB's
[overlapping readwrite transaction scheduling](https://w3c.github.io/IndexedDB/#transaction-scheduling)
and [transaction lifecycle](https://w3c.github.io/IndexedDB/#transaction-lifecycle):
the put is enqueued synchronously in the get's success callback, with no await
between them. No Web Locks dependency or main-tab leader election is introduced.

## Compatibility and remaining P3 exits

The database `shared-utils-sync`, store `clients`, schema version 1, record keys
and stored `{ base, pending }` shape are unchanged. Existing records are adopted
in place. No database is deleted, renamed, implicitly upgraded or downgraded.
Older readers can still read the bytes. **Older blind writers are not fenced**;
an already-open pre-upgrade tab, or a reverted old Web bundle, can still exhibit
the original last-writer problem. Refresh all active clients to receive this fix.

The following are deliberately not accepted by this slice:

- exclusive version-fenced lifetime ownership across old/new clients;
- general Service/principal/dataset/codec identity or cross-tab account-change
  fencing (Web storage sharing is not authentication authority);
- arbitrary state migrations, durable deletion tombstones or evidence archival;
- native workspace/worker generation acceptance, Safari/iPhone behavior, or
  physical power-loss guarantees beyond the browser's transaction contract.

State-sync 1.5.0 and state-sync-idb 1.6.0 form component release 3.5.0. The seven
Plugin sources, versions, signatures and exact 2.9.0 component pins are unchanged.
Only Web activation is needed; no Controller/Machine restart or Catalog write.

## Verification

`web/src/idbOutbox.test.ts` covers merge identities and ordering, a seeded
four-writer oracle, strict failures, private load handoff, reentrant/late
hydration, baseline preservation after abort, terminal leases and actual
replicated-client no-send behavior. Existing IDB ownership and state-sync tests
remain part of the gate; compile-only negatives check the borrowed typed API.

Build the pinned optional test browser and run, from the repository root:

```sh
nix develop -c nix build .#cowboy-idb-test-browser --no-link --print-out-paths
nix develop -c just idb-browser-conformance /nix/store/<result>/bin/firefox
nix develop -c just idb-outbox-browser-conformance /nix/store/<result>/bin/firefox
```

The second suite has ten real-engine checks, including independent Workers,
20 simultaneous write rounds, stale acknowledgements, real replicated clients,
concurrent disposal, cold resend, abrupt Worker termination before/after commit,
actual transaction abort, legacy schema readability, incompatible version-change
and independent record keys. Worker termination is not browser-process or host
power loss. Different record keys are not proof of native generation identity.
The runner uses a fresh profile, loopback-only network and disposable data; its
browser/digest receipt never contains product data or credentials.
