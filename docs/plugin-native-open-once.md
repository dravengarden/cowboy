# Single-use native buffer acquisition

Zed Plugin/private adapter `1.14.1` retains private server `1.1.0`, the exact
upstream revision and all third-party pins. This candidate fixes ordinary native
Open ownership; it does not enable production navigation or introduce a new
Plugin lifecycle, recovery grant or Machine maintenance operation.

The [candidate acceptance](releases/native-open-once-candidate-2026-09-18.md)
identifies the exact static pair, clean source and complete gate evidence.

## One acquisition, no implicit replacement

One admitted native acquisition dispatches `OpenBufferByPath` once, observes its
original nonzero buffer ID and complete initial State/last Chunk, and registers
that original buffer once. The event subscription precedes dispatch because
native sharing can arrive before the Open response. The existing five-second
initial-share observation deadline is unchanged.

Previously, missing initial sharing triggered `CloseBuffer` followed by another
Open. An already shared native buffer may return its original ID without sending
another initial share. The fallback could therefore close another owner's
original peer and silently substitute a reopened buffer. Timeout or stream loss
now returns uncertainty without Close, retry or registration. Invalid Open or
registration replies likewise cannot become successful ownership.

## Admission outlives its observer

Before native I/O, the active-map lock protects admission of a non-cloneable,
non-serializable, one-use attempt. Dropping that attempt does **not** clear its
process-local fence. Only committing the original native ID and owner, with no
intervening await, consumes the attempt and ends the fence. Cancellation before
the Open response, during initial sharing or during registration cannot authorize
a replacement. A late reply cannot revive the abandoned attempt.

An unresolved Open fences new legacy/owned opens, navigation, synchronization
preparation and both stages of destination handoff, even when no native ID or
owned lease exists yet. Effect-free buffer preparation is still allowed; refused
replacement opens retain Prepared. The original attempted owned lease remains
Unknown on Query, repeated Open and Release. Editing the source file or releasing
an unrelated owner cannot clear uncertainty.

Existing safe content-bound reads and explicit release of other known owners
remain available. Adding an explicit owner to an already committed key requires
no native I/O in the healthy case, but does not bypass an unresolved Open fence.
This conservative availability tradeoff is intentional: pathname inequality
cannot prove that an unobserved native ID is independent.

## Evidence and limits

Source tests cover cancellation at all three native awaits, malformed replies,
zero IDs, broadcast loss, real deadline expiry, early shares, one-use commit,
legacy admission and independent known reads/releases. The actual native gate
reopens an already shared buffer, observes the normal timeout, and then checks
the original native peer through the private source-ownership handler. The final
static adapter/server socket gate additionally refuses an actual oversized file,
retains Unknown without replay after the file changes, denies replacement opens
and verifies independent original text read/release.

The fence is not durable recovery, a saved native acquisition record or proof
that background work stopped. No restart, timeout, automatic cleanup or new
credentials are used to clear it. Verified native close acknowledgements,
independently authorized uncertain-acquisition recovery and global retained
buffer/history/background-effect budgets remain separate work. These candidate
checks are not production installation or supported-device acceptance; see the
[completion ledger](plugin-refactor-completion.md).
