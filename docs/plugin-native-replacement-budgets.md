# Private native sync/reload replacement budgets

Zed Plugin/private adapter `1.17.0` selects private server `1.4.0`, retaining
the exact upstream revision and third-party dependencies. This candidate adds
finite native replacement admission. It is not a production activation, a new
core writer grant, a general history limit or completion of the Plugin refactor.
The [candidate acceptance](releases/native-replacement-budgets-candidate-2026-09-18.md)
records the exact static pair, source gates and disposable lifecycle checks.

## Admission before work and mutation

Conditional synchronization and the existing native reload path share **four**
replacement jobs per GPUI application, across local BufferStores/worktrees.
Acquisition precedes the actual filesystem loader and diff worker. A job is a
nonserialized RAII charge: its clones retain one slot until the last holder
drops. Actual background loaders, their completed byte results, diff workers
and their completed results own the charge, independently of request observers.
An unsupported LocalFile implementation refuses rather than falling back to an
uncharged loader. This does not budget arbitrary upstream `load_bytes` callers.

Each replacement checks the actual retained text history without serializing or
cloning it. Checks run before loading/diffing and again in the mutation turn:

- at most 4 MiB current/new decoded text, retaining the independent raw-file cap;
- at most 8 MiB total base text plus inserted operation strings;
- at most 4,096 retained edit/undo operations;
- at most 16,384 aggregate edit ranges, inserted strings and undo-map entries;
- at most 256 dense vector slots for the writer replica and current/retained
  operation clocks;
- no causally deferred text operations;
- at most 1,024 diff edits, checked before allocating another result string.

The diff uses the same pinned upstream line/word algorithm and normalization.
It never substitutes a whole-file replacement. Exceeding the result cap returns
no partial edit list. Algorithm scratch/CPU is still governed only by bounded
input and concurrency, not an independently measured RSS or time limit.

A non-Clone, nonserialized Replacement binds the native entity ID and exact
base version. The consuming apply rechecks both, including edit/undo ABA, and
checks the prospective retained history before finalizing undo groups, changing
text/encoding or marking saved. Exhaustion preserves history and original
content; it does not evict old operations, rebase the buffer, reopen its path,
auto-retry or replace its generation. No-op diffs add no text operation.

## Outcomes and ownership

Private sync protocol 1 adds the closed `BUDGET = 4` refusal. Its exact paired
adapter decodes it without accepting partial content or foreign identities.
Native capacity/history refusal becomes a terminal saved result for the original
operation. Repeated Apply/Query observes that result even after capacity returns.
Pending operations and their retained owners still cannot expire or retire.

The `1.17.0` candidate's core owner codec has no budget outcome. That candidate
does **not** extend that wire union, pretend the reason was Source, or use the
private refusal as new reconciliation authority: adapter ownership remains
Unknown and fenced. Queries use only the original native instance/operation;
Apply and Retire do not resend or clear the fence. A public budget-result
projection and its independently accepted reconciliation remain separate work.
The subsequent [typed outcome continuation](plugin-sync-budget-outcomes.md)
adds the `1.18.0` adapter and core readers without changing these native limits
or adopting the preceding process's uncertain owners.

Reload preserves its original result channel and invocation-specific task
cleanup. Refusal reports failure, not a successful prefix or implicit retry.
Only test fixtures invoke legacy reload; owned reads still cannot use it as a
content-mismatch fallback.

## Limits and verification

This bounds growth through **these two writers**, not every native mutation.
Ordinary remote edits/undo, LSP workspace edits and desktop-only constructors
retain their existing interfaces. Their history is included when a later
replacement is checked, but these writers themselves are not constrained here.
General snapshot/serialization/parsing lifetimes, LSP side effects, worktree
scanning and process-wide history bytes remain unaccepted. No Close ACK or
observer cancellation proves all native resources have drained.

The later [private remote-edit candidate](plugin-native-remote-edits.md) applies
these finite retained-history limits to local-server incoming edit/undo batches
too, with original-peer admission, exact duplicate checks and atomic causal/
UTF-8 validation. It does not cover the other writer and global limits above.

Required tests cover shared job capacity, loader/result lifetimes, cross-thread
last-holder release, cancellation, source-entity and version ABA, inclusive byte
and operation caps, undo retention, aggregate parts, dense/deferred histories,
whole-diff refusal, native saved-ID deduplication and adapter Unknown fences.
The actual immutable server must refuse an oversized diff through both reload
and conditional sync, preserve original text/version/source, and allow only a
separately requested smaller replacement. Final static-pair, signed temporary
lifecycle, connected v5, browser and complete source gates remain required.

Publication, registered Machine installation, actual generation replacement and
supported-device acceptance are independent from these disposable fixtures.
See the [completion ledger](plugin-refactor-completion.md).
