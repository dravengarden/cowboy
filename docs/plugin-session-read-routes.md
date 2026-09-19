# Session read routes

Core now separates a logical Session observation from an executable filesystem/
Git read binding. This extends the finite
[Code read scopes](plugin-code-read-scopes.md) and
[Workspace observations](plugin-workspace-read-scopes.md), not a new Plugin
lifecycle or generic graph executor.

## Resolution and continuity

`SessionCodeScope` remains the Hub-owned logical Session incarnation. A distinct
`SessionReadScope` is constructed only by the actual core Service/Machine
registry, after resolving that original logical scope. Its private fields bind
the core registry incarnation and, for a named Machine, the original
authenticated connection. It has no serde/string constructor. Equal Machine
names, epoch strings or paths cannot mint or renew it; clones borrow the same
observation. Logical scopes alone no longer satisfy the read-cache key type.

All eleven buffered filesystem/Git HTTP readers resolve this binding before
read setup. Remote Code dispatch takes only the binding and a closed typed Code
operation, deriving its root, Machine and adapter. Original-connection checking
is atomic with enqueue and is repeated after awaiting the response. Manifest
Zed readiness and `bufferMode` selection retain that same original connection,
not a fresh lookup before each step. The complete HTTP response guard checks
both the logical Session and route after reads, cache hits, errors and `304`.
An ended observation becomes `410/no-store` without stale bytes or an ETag.

File and diff continuation keys retain this exact route too. Reconnection,
including accidental reuse of an epoch, invalidates old continuations even when
the Session, path, content and revision still match. A new HTTP request can
resolve the new route; it cannot inherit an old cursor. Session rename/status
changes and unrelated Machines preserve a continuous observation. Session cwd
ABA, deletion/recreation and foreign Hubs/registries still refuse.

Local versus remote execution is derived only from that original connection.
A named colocated Machine must remain connected; the saved database flag no
longer turns a disconnected named Session into Controller-local filesystem
access. An unavailable route does not resolve (the existing unknown-context
404), while a route lost during an admitted read produces 410. Core-owned
standalone `local` Sessions remain usable without a Machine and bind the original
core registry instead. No detached Session or worker is deleted by either case.

## Bounds and separate owners

There is no new per-Session registry, background task, timer, wire token or
durable state. Existing bounded RPC waiters and page/diff caches retain the
observation; page identity budgets include the retained connection strings.
Cancellation drops only its pending RPC observer, never replays a read or emits
cleanup. A read already enqueued may still execute.

Legacy language/resource HTTP calls keep their separate Session/connection/Unix
peer operation boundary. Original owned native buffer, synchronization and
navigation resources retain their own lifetimes; this does not rebind, retire
or restore them on reconnect. Their preparation still uses the logical Session
and its separately captured native connection, not a filesystem cursor.

## Evidence and limits

Two pre-fix regressions reproduced identical Session read identities across
same-epoch connection replacement and an old buffered body returning HTTP 200.
Thirteen new source tests cover resolution, route direction changes, pre-enqueue
refusal, parked replies, cancellation, both caches, independent scopes and
manifest readiness including the actual local Unix socket contract. The
[connected gate](plugin-code-connected-conformance.md) adds v7 check 20 for
actual authenticated core file pages and no-dispatch continuation refusal after
connection replacement/restart. Artifact acceptance and production activation
are separate from these source definitions.

The [accepted Controller-only rollout](releases/plugin-session-read-routes-2026-09-19.md)
records the expected old-artifact failure, two successful v7/20-check runs,
complete integrated gates and actual activation retaining all 16 original
workers. The canonical release skill now requires the v7 Session-route check;
historical v6 receipts cannot accept it.

No Plugin/SDK version, Machine wire protocol, native ABI, journal, SQL baseline
or private policy changes. Only the Controller requires activation. This is not
Machine-owned filesystem/inode identity, proof of native runtime continuity,
principal authority (now separately checked at finite boundaries by
[buffered read authority](plugin-code-read-authority.md)), a state reader/writer lease, atomic HTTP delivery or
independent post-effect recovery. Those broader exits remain in the
[completion ledger](plugin-refactor-completion.md).
