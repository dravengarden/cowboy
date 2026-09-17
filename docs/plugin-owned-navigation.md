# Owned native navigation candidate

Zed `1.10.0` introduced a private adapter resource primitive, not a public Code
navigation API. A native LSP definition/reference query may open target buffers;
it is resource acquisition, not one more `content_read` variant. Ordinary owned
Review navigation remains closed until separate core authority, original-runtime
routing and a destination consumer are implemented and accepted.

The `1.11.0` candidate adds exact destination handoff and fixes native target
registration, discovered by running a nonempty stdio LSP against the actual
server. It remains private; no Service/Web navigation API is enabled.

The `1.12.0` source candidate adds an exact-pair support probe for the separate
[Machine core continuation](plugin-machine-buffer-navigation.md). That protocol-21
path owns original connection/runtime routing and destination reservations;
generic forwarding and the Service/Web consumer remain closed.

## Finite ownership contract

`prepareBufferNavigation` accepts an existing open buffer lease, complete
content identity, an exact UTF-16 point and one of the five existing navigation
kinds. It validates the original native mirror and allocates a non-recycled
`nav:` ID before any native query. Preparation is effect-free and expires after
30 seconds, including time waiting for admission. Its random instance and
disjoint ID space cannot be substituted for an ordinary buffer lease. Serialized
IDs are not grants.

`bufferNavigation` accepts only Execute, Query or Release. Execute rechecks the
original owner, native ID and captured epoch, retains a source pin, and records
Unknown before dispatching the one native query. Repeated Execute and Query
return saved state; neither reopens a path nor dispatches another LSP request.
Lost socket observers do not cancel an admitted handler. Actual task
cancellation, timeout, malformed or oversized results keep Unknown and the
original pin.

Successful results capture original target native IDs, exact content identities,
mirror epochs and UTF-16 ranges while retaining the active-map lock through
query/capture and the mirror lock through commit. Paths are bounded relative
display metadata, never subsequent lookup authority. This first primitive admits
only the source's original native worktree; external/dependency worktrees are
explicitly refused. The complete result is validated before local target pins
are installed. It never reports truncated or partially valid success.

The result may precede Zed's asynchronous target State/last Chunk. Before
capture, a bounded five-second wait observes only those original native IDs,
rechecking the source epoch. Missing shares, stream loss or timeout retain
Unknown; no fallback open or LSP retry fills the gap. Target-count limits are
checked before waiting, and incomplete/invalid mirrors cannot become success.

Native navigation shares targets but does not register them with language
servers. Execute now registers each newly retained native ID once, with one
five-second budget for the complete registration set. Existing native-ID owners
and repeated locations do not trigger another registration. All target pins and
evidence are saved before that await; a missing/invalid ACK, cancellation or late
source/target epoch change keeps Unknown. Retained requires a final epoch check
under the mirror lock. Registration is never hidden in a read. Its ACK is not a
promise that every production language server has finished initialization.

There are at most 32 live navigation groups, 256 result locations and 32
distinct target native IDs per group. Duplicate locations share a resource pin.
Only effect-free preparations expire; retained and uncertain groups cannot be
evicted to free capacity. These bounds are not a global native memory/liveness
guarantee: the native LSP can acquire resources before returning an unacceptable
result.

Unknown acquisition retains a process-wide admission fence because an unobserved
target ID could alias another buffer. New opens, navigation and synchronization
are refused. Existing safe reads and release of other known pins remain
possible. No timeout or Release silently clears uncertainty. The native LSP
protocol has no saved-query recovery operation; independent recovery remains
unimplemented.

## Reading and releasing a destination

`readBufferNavigation` selects an index in the original retained result,
supplies that exact content identity and uses the closed language/symbol/hover
query union. It validates the original target owner and epoch before and after
the read. An edit followed by undo is not fresh authority, even if the text hash
matches. Saved Query results are historical observations, not live coordinates.
A target remains owned after the source view releases its ordinary lease or
files disappear. This primitive does not read current text from disk or create a
destination view.

Release uses captured keys/native IDs and removes only the group's source/target
pins. It never canonicalizes a pathname, adopts a replacement or writes source
files. Native-ID aliases are counted across active pathname entries: removing
one entry cannot close another owner's native buffer. Last-owner local close
enqueues run synchronously under the active-map lock. Transport failure retains
ReleaseUnknown without replay. Native CloseBuffer has no acknowledgement, so
Released means local pin removal and successful enqueue, **not verified native
cleanup, filesystem restoration or post-effect recovery**.

## Exact destination handoff

`prepareNavigationBuffer` selects an original retained destination index and
exact content identity. It reserves an ordinary buffer lease, without a native
open, registration, file read or ownership change. That reservation shares the
existing 1,024-slot capacity, 30-second inert expiry and non-recycled ID space.

`openBufferLease` on this origin rechecks the original navigation group, target
ID, content, epoch, owner and admission after waiting for locks. Final epoch
validation, pin insertion and Open commit have no intervening await. A cancelled
waiter remains effect-free; a lost socket reply can observe the same prepared ID
as Open, without allocating a replacement. Parent release before Open refuses;
a same-path/same-content later group cannot substitute for it. No pathname is
re-resolved, even if source or target files disappeared.

After Open, the ordinary owner is independent: the parent navigation group can
release source and other targets, while that exact destination continues through
the existing content-bound reads, fresh navigation preparation and explicit
release. Handoff does not renew old navigation positions, supply target display
text or grant synchronization. Source-path and navigation origins form a closed
Rust enum, so a failed handoff cannot fall through to path-based open.

## Core boundary and acceptance

The Machine source explicitly refuses all four private commands before generic
runtime selection. Neither a read lease nor an optional worktree field can
bypass that check. The separate protocol-21 Machine continuation does not relax
this check or enable a Service endpoint, Web/native bridge or shared component
contract. The private adapter and consuming Zed Plugin
are versioned together; server `1.0.0`, upstream Zed revision and all dependency
pins remain unchanged. Historical component-registry entries are untouched.

Deterministic private-transport tests cover nonempty targets, duplicate
locations, source release/deletion, same-ID aliases, disconnected observers,
cancellation, source/target edit-undo, capacity/expiry,
malformed/foreign/oversized results and failed close enqueue. The existing
isolated `zed-native-sync-conformance` harness exercises all five actual native
plaintext query kinds and nonempty cross-file queries through a separately
launched deterministic stdio LSP. It checks complete target content, non-BMP
UTF-16 ranges, one-use execution, retained source ownership, shared-owner sync
refusal, independent handoff with no native I/O and path-free read/release.

`just zed-native-navigation-conformance <immutable-adapter> <immutable-server>`
additionally drives the final static pair through its real private Unix socket.
It discards an actual handoff reply, observes that original lease without
resending Open, releases the parent after deleting source/target paths, reads
the retained destination and checks each fixture document's one open/close.
Both gates use disposable homes, closed environment, isolated network/PIDs and
an explicit test-only LSP executable; no ambient language tools or downloads
complete the fixture. Neither establishes production language semantics, a
public consumer, universal close acknowledgements, native allocation bounds or
independent recovery. Plaintext alone still proves no nonempty destinations.

Before exposing navigation, still required:

- Accept the Machine continuation through an enrolled original-connection
  consumer, including its authority, bounded handoff/cleanup and native budget.
- Service/principal/Session ownership and client destination scopes, with
  complete text/position binding and no legacy fallback or automatic target
  synchronization.
- The connected consumer gate beyond the private nonempty-LSP fixture,
  separately accepted native rollout and supported-device tests.

This source candidate does not publish a signed Catalog release, install a
Plugin or authorize resident Machine maintenance. General DAG execution, state
leases and independent recovery remain in the
[completion ledger](plugin-refactor-completion.md).

The [original candidate acceptance](releases/owned-navigation-candidate-2026-09-17.md)
records the `1.10.0` plaintext and deterministic-fixture milestone. The
[handoff acceptance](releases/owned-navigation-handoff-2026-09-17.md) records the
exact `1.11.0` artifacts, four actual nonempty-LSP static-pair runs, full source
gates and eleven connected regressions, including the remaining native allocation
and ambiguous-outcome boundary.
