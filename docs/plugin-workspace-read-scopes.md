# Continuous Workspace read scopes

Core now retains the continuous lifetime of each Workspace advertised on an
authenticated Machine connection. This extends the finite
[Code read scopes](plugin-code-read-scopes.md), not the general graph executor
or Machine-owned filesystem/state lease contract.

## Resolution and execution

Only the live Machine registry constructs a Workspace observation. It has
private immutable fields, pointer identity, no serde constructor and no public
string constructor. A clone borrows the original observation; a matching saved
inventory or serialized graph cannot recreate it.

Authenticated initial inventory and subsequent explicit Workspace inventories
update the registry under its existing connection lock. Removal, path change,
ambiguous duplicate ID, rejected observation, disconnect and same-epoch
connection replacement end the old lifetime. Re-adding exactly the old strings
creates a new identity. Changes to another Workspace, display names, ordering
and host configuration revision alone preserve unchanged roots. Older
component-only events with no Workspace list preserve the current observations;
an explicit empty list removes them. Stale connections cannot update the map.

The existing eleven filesystem/Git HTTP readers resolve against both the
non-revoked persisted enrollment and the live core-owned Service/Machine scope.
Missing, mismatched or ambiguous inventories do not choose the first entry.
Disconnected Machines cannot recover a scope from stored paths. This also ends
the old colocated-Workspace fallback after its live connection disappears;
Session compatibility routing is unchanged.

Remote Workspace reads take a closed Code operation and the opaque scope. Core
supplies the original Machine, adapter and root, then rechecks the observation
atomically with RPC registration/enqueue. The caller cannot replace those
fields. The original observation is checked again after awaiting the response,
including a reply completed immediately before an inventory change. Cancelled
observers release only their RPC waiter, never a Workspace or Session, and
create no retry or cleanup command.

The existing buffered HTTP guard covers local reads, remote reads, cache hits,
errors and conditional responses. An ended scope replaces the entire response
with 410/no-store and no stale ETag/body. File-page and diff cache keys retain
the same opaque observation, so equal paths/content after remove/re-add cannot
adopt old continuations. Independent Workspaces remain independent.

## Bounds

The additional live observation registry accepts at most 1,024 advertised roots
per Machine, 4,096 across all Machines and 8 MiB of logical identity strings.
Workspace IDs are bounded to 256 bytes and absolute advertised paths to 4 KiB;
NULs and ambiguous context separators are refused. Core does not canonicalize a
remote path on the Controller.

Limits are checked before building the new map. A rejected inventory ends that
Machine's previous observations without evicting another Machine or affecting
Plugin inventory. Duplicate IDs end only their own slots. Existing page/diff
cache and pending-RPC bounds independently govern retained consumers. These are
not a new bound for the complete Machine event history or native process memory.

## Evidence and limits

Two tests using temporary SQLite enrollment and the actual resolution function
failed against the previous implementation: observed remove/re-add and
same-epoch connection replacement both produced an identity equal to the old
one. Expanded source tests cover stable metadata and unrelated roots, foreign
Services/registries/Machines, duplicate and invalid roots, count/byte budgets,
persisted/live disagreement, revoked enrollment, pre-enqueue refusal, parked
replies, cancellation, response headers and both continuation caches.

This only detects observations accepted by the Controller. An unreported
filesystem replacement or intermediate Machine configuration change still needs
a Machine-owned continuous identity protocol. There is no inode proof, state
reader/writer lease, principal-policy grant, cancellation of a read already
enqueued, atomic HTTP delivery or effect restoration. The Machine retains its
own trusted-root checks. Detached Sessions, workers, Provider credentials and
user files acquire no new lifecycle owner.

No Plugin/SDK version, public wire protocol, native ABI, persistent journal, SQL
baseline or production policy changes. Only the Controller needs activation.
General graph linking, Machine-owned Workspace/Session/security-domain identity,
state compatibility and independent post-effect recovery remain in the
[completion ledger](plugin-refactor-completion.md).

The [2026-09-19 Controller release](releases/plugin-workspace-read-scopes-2026-09-19.md)
records final integrated source gates, exact immutable artifacts, the separate
19-check Code regression chain and actual production activation/continuity.
