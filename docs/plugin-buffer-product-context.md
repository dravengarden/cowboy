# Code owners in the core product lifetime

The [typed browser owner](plugin-buffer-client-owner.md) now has a core product
entry point, `productCodeBuffers.ready()`. Communication, identity and resource
admission remain core mechanisms, not installable Plugins. Ordinary Review still
uses its existing API; this integration is a prerequisite, not that cutover.
The [verified Web release](releases/plugin-buffer-product-context-2026-09-16.md)
is active on Hawk with service worker `cowboy-v1695`; it restarted no process.

## Identity and time

The auth root binds the immutable user ID before product consumers mount.
`ready()` borrows the actual `productSyncDatabase` descriptor and lifetime; it
does not import the socket store, open IndexedDB or prepare/open a native buffer.
An unauthenticated root cannot discover a dataset. The descriptor is public
identity evidence, not a native reference or an authorization grant. The
Controller still validates the actual credential, role, Session and connection.

The facade retains one buffer registry across view mounts. Concurrent readiness
calls share the dataset owner's discovery; cancelling one view only detaches its
observer. Before ownership is established, transient discovery failure can be
retried. Once bound, same-Service reconnect and temporary outages preserve the
registry. There is no extra discovery on every buffer call or new heartbeat.

The core `productSessionSignal()` is one irreversible lifetime for the page
root. `announceProductSessionEnd()` aborts it synchronously **before** dispatching
cleanup observers. The dataset owner also ends its signal on observed identity
replacement, permanent socket-root abandonment and disposal. It cannot resume
when the old identity returns. A mismatch during initial discovery permanently
seals the original context; a late response cannot bind it to a different user.

Service replacement is detected by the existing dataset/transport admission
checks, not continuous identity monitoring. The production principal is frozen;
an actual user switch ends the page graph and reloads instead of rebinding it.
Reading the frozen principal after sign-out is only needed for old local writes,
not permission to create a new consumer. The binder rejects all post-end binds.

Old registries reject new reserves and operations after context end. Pending
readiness/read observers reject promptly even if their underlying discovery or
transport does not cooperate. Late replies are ignored. No cleanup DELETE is
sent with ended or replacement credentials: unresolved native resources remain
explicitly `retained`, not falsely reported released. Reload does not recover
those owners. Abandoned-browser and independent native recovery remain open.

## Stop admission, then drain local persistence

Ending remote authority and disposing local persistence are different actions:

1. End the core/dataset signal; stop new remote operations and local borrowers.
2. Seal existing replicated clients and await their final original-dataset
   outbox writes. No new dataset is discovered or adopted for that drain.
3. Dispose the shared IndexedDB owner only after those writers finish.

`stopAdmission()` supplies the first boundary for permanent product-store
abandonment. Already-borrowed writers can drain against their adopted, unchanged
identity; they cannot discover a new namespace after end. An observed actual
principal/dataset mismatch remains a hard refusal, not permission to migrate
pending data. New listing, export, deletion and reconnect continuations recheck
admission after readiness, including the microtask gap before I/O.

The bounded session-end result `drained` describes the registered local cleanup
barriers only. It does not assert native release, cancel unknown effects, delete
retained records or guarantee completion after the page is killed. Existing
`pending` and `failed` outcomes remain honest. No storage schema, key format,
mutation ID or Plugin version changes are needed.

## Acceptance

Fifteen focused tests cover readiness, shared cancellation, transient outages,
Service/principal ABA fencing, late discovery, same-stack end, the local I/O
admission gap, final outbox drain and permanent-root abandonment. The core
session-end tests also verify stable per-root signals and fence-before-callback
ordering. Existing lifecycle/transport tests remain unchanged.

From the pinned shell, run `just code-buffer-context-browser-conformance` with
the absolute Firefox executable from `.#cowboy-idb-test-browser`. Its six cases
use the real production binder, default dataset singleton, session-end
rendezvous, React StrictMode and native IndexedDB. They cover pre-auth refusal,
shared readiness, view replay, transient/same-Service reconnect, Service
replacement and sign-out while a read and final local save are outstanding.
The last case reopens the synthetic dataset and verifies the pending mutation
survived. HTTP responses are fixture-owned; no real account, auth UI or native
Plugin is exercised. The accepted Firefox 151.0.1 bundle SHA-256 is
`b101962e584d7791470855f454a3ec056ed6df64098b1bc3510baef0e6467dc8`.

The actual product-store send-admission fixture also passes changed-dataset and
missing-protocol cases with native IDB, retaining each pending prompt and
delivering none. Existing buffer, Settings recovery, IDB-owner and outbox browser
suites pass separately. These are not physical iPhone, actual Review or deployed
native-generation acceptance.

The [core Settings cleanup surface](plugin-buffer-cleanup-surface.md) now
observes this same registry without readiness discovery or new resource effects.
Remaining work includes Review integration, positional content and
anchor semantics, independent Machine/Code activation, supported-device
acceptance and independently authorized post-effect recovery. See the
[completion ledger](plugin-refactor-completion.md); the overall Plugin refactor
is not complete.
