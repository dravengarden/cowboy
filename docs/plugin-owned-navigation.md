# Owned native navigation candidate

Zed `1.10.0` adds a private adapter resource primitive, not a public Code
navigation API. A native LSP definition/reference query may open target buffers;
it is resource acquisition, not one more `content_read` variant. Ordinary owned
Review navigation remains closed until separate core authority, original-runtime
routing and a destination consumer are implemented and accepted.

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

## Core boundary and acceptance

The Machine source explicitly refuses all three private commands before generic
runtime selection. Neither a read lease nor an optional worktree field can
bypass that check. No Machine protocol, Service endpoint, Web/native bridge or
shared component contract is added. The private adapter and consuming Zed Plugin
are versioned together; server `1.0.0`, upstream Zed revision and all dependency
pins remain unchanged. Historical component-registry entries are untouched.

Deterministic private-transport tests cover nonempty targets, duplicate
locations, source release/deletion, same-ID aliases, disconnected observers,
cancellation, source/target edit-undo, capacity/expiry,
malformed/foreign/oversized results and failed close enqueue. The existing
isolated `zed-native-sync-conformance` harness also exercises all five actual
native plaintext query kinds, one-use execution, retained source ownership,
shared-owner sync refusal and path-free local release. Plaintext returns no
destinations: this actual-process regression must not be reported as real
nonempty language-server or cross-file consumer acceptance.

Before exposing navigation, still required:

- Original-generation Machine routing with core-issued acquisition authority,
  bounded handoff/cleanup and disconnect/uninstall acceptance.
- Service/principal/Session ownership and client destination scopes, with
  complete text/position binding and no legacy fallback or automatic target
  synchronization.
- Actual nonempty LSP target acquisition/release, the connected consumer gate,
  separately accepted native rollout and supported-device tests.

This source candidate does not publish a signed Catalog release, install a
Plugin or authorize resident Machine maintenance. General DAG execution, state
leases and independent recovery remain in the
[completion ledger](plugin-refactor-completion.md).

The [candidate acceptance](releases/owned-navigation-candidate-2026-09-17.md)
records exact immutable artifacts, full source gates and connected regressions,
including the remaining native allocation and ambiguous-outcome boundary.
