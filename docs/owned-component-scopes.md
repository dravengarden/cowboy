# Owned component lifetimes

This is the fifteenth spatiotemporal refactor slice: a process-local ownership
primitive and its first state-sync consumers. It is **not** a distributed DAG
executor, grant, durable Operation journal, or automatic compensation mechanism.

## Core component, not an installable Plugin

`@cowboy/state-store/scope` exports `OwnedResourceScope`, `ScopeSnapshot`,
`ScopeClosedError` and `createOwnedResourceScope`. It is framework-neutral and
imports neither React, the Plugin runtime nor native capabilities. Construction
acquires no ambient listener, timer, connection or authority.

An owner registers cleanup before calling a resource-acquiring API. For an
asynchronous acquisition, register its holder first, then admit the task with
`run`; the task must either populate the holder or fail without retaining a
resource. Subscriptions must acquire atomically. `guard` fences observation
callbacks, not user-submitted operations. Each instance has its own identity and
resource/task counts; there is no mutable global registry.

`dispose()` synchronously seals new entry points and requests cooperative abort.
It returns the same promise on every call, waits for admitted tasks to settle,
then awaits finalizers in reverse registration order. Register providers before
consumers. Explicit early release is also idempotent. A throwing finalizer does
not prevent other releases; it stays counted and produces `needs_reconcile` with
an `AggregateError`. Cleanup is not automatically retried. Pending work stays
`draining`, not a false success. Snapshot counts contain no resource data or
exception text; the owning caller receives the actual cleanup error.

This primitive expresses local ordering, not a dependency scheduler. It cannot
prove that an arbitrary finalizer released everything, terminate a stuck task,
detect every cleanup dependency cycle, enforce cross-process budgets, or make an
opaque external effect reversible. A finalizer must not await its own scope's
disposal. Future DAG execution still needs explicit dependencies, verified
resource identities, authorization, timeouts and durable recovery evidence.

## State-sync owner boundaries

`createClient`, `replicatedStore` and `mirroredStore` expose an owner-only
`dispose(): Promise<void>` and a readonly `lifecycle` snapshot. Reads retain the
local recovery state; new mutations, subscriptions, hydration and connection
attempts fail after sealing. A successful close leaves zero owned resources and
tasks. A final local persistence failure rejects and remains visible. Callers
must await the barrier before replacing a writer of the same persistence key.

Replicated clients:

- Drain admitted durable mutations/confirmations and serialize the final local
  snapshot behind earlier saves. Never delete an outbox or send a compensating
  mutation during cleanup. Closing an unchanged, unhydrated client does not
  overwrite its unread persisted record with an empty initial state.
- Suppress sends after owner closure, including a durable write that finishes
  later or an observer that closes the owner reentrantly. The persisted mutation
  remains resumable by a new owner using its original id.
- Keep the UI's optimistic `pending()` separate from `pendingForSend()`. A
  reconnect or reentrant observer cannot bypass an outstanding durable admission
  barrier by resending a row that is visible but not yet safely stored.
- Share one hydration promise. Late cached data cannot publish into a retired
  instance; live patches still outrank cached bases. Failed durable confirmation
  restores only removed, still-unconfirmed rows, not facts acknowledged during
  that write. In-progress/successful confirmation also fences late hydration.
- Give duplicate callback registrations independent unsubscribe handles;
  throwing render observers cannot suppress persistence or transport delivery. A
  throwing mutator cannot leave an unapplied ghost mutation in the outbox.

Mirrored clients:

- Make `connect()` idempotent for each current connection. `disconnect()`
  retires that observation, including pending loads and queued callbacks; it
  leaves the writable store usable. Reconnecting creates a new observation
  identity.
- Pre-register subscription cleanup, including when a subscription synchronously
  calls back into disconnect/close before returning its unsubscribe handle.
- Serialize local and remote writes independently. A slow older save cannot
  become the final value after a newer save. Hydration cannot overwrite a newer
  local edit or remote observation.
- On disposal, cancel unsubmitted remote debounce/throttle timers, drain already
  submitted writes, and persist the local value. This is not an outbox that
  promises eventual remote delivery; explicit `flush()` is the write barrier and
  surfaces failures. Cleanup does not invent a remote write or undo a prior one.

The browser persistence adapter is **borrowed**, not closed by each store. Its
existing shared IndexedDB connection cache has not yet migrated to explicit
connection leases; its lifetime and cross-tab ownership remain a follow-up. The
scope does not validate arbitrary `LocalPersistence<T>` bytes or retrofit a
schema codec onto IndexedDB. The current preference codecs are unchanged.

## Product integration and release

Product sign-out synchronously seals the Web title/order and per-session queue
clients. Late IndexedDB key enumeration cannot create new clients afterwards.
Failure reporting is static and never prints authored content. An abrupt browser
termination cannot guarantee that asynchronous finalization completes; authored
prompts still depend on their existing pre-send durability barrier.

React unsubscribe, background/pagehide flush, temporary reauthentication and
WebSocket reconnect do **not** dispose the shared sync owner. No Machine
session, worker, worktree, credential, preference or persisted history is
removed.

Component registry 3.2.0 appends state-store 2.1.0, state-sync 1.4.0, and the
transitive state-sync-idb 1.4.0 peer update. The seven Plugin sources, versions,
2.9.0 pins and signed artifacts are unchanged. Web binds the new framework-free
package subpath explicitly in its Deno/TypeScript resolution and Vite peer
deduplication; source remains
owned by the same Cowboy component. Only a Web release is needed. No native ABI,
Controller protocol or Machine maintenance change is included.

Regression gates cover scope failure/reentrancy/order/drain/instance isolation,
real state-sync stores under delayed reads/writes and failed confirmations,
strict TypeScript negative cases, existing durable delivery integration, and
seeded lossy/reordered multi-client convergence. These are local deterministic
tests, not physical-device, cross-process cleanup or cross-Service/Machine
compensation acceptance. General host migration, production CoreSecurity
cutover, the public SDK/native retirement and P2–P4 remain outstanding.
