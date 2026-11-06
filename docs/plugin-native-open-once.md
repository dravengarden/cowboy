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

## Recovery from the same native process (Zed 1.23.0)

Private server `1.8.0` records each peer's `OpenBufferByPath` by message ID
before the handler's first await, settling it as opened (with the original
buffer ID) or failed; `CowboyOpenAttempt` reads that record and has no effect.
The adapter remembers the latched attempt's message ID before queuing it. The
next owned or legacy acquisition first resolves the fence under the active-map
write lock:

1. An upstream `Ping` round trip proves the original request, queued earlier
   on the same connection, was received: the adapter's writer interleaves the
   upstream and private channels, so only this barrier orders them.
2. `CowboyOpenAttempt` then reports the outcome. `UNKNOWN` (never received) and
   `FAILED` clear the fence with no effect. `OPENED` with a buffer ID another
   committed owner already holds is an alias and clears it. Any other opened
   buffer is an orphan: it is released through the confirmed
   [native close](plugin-native-close-confirmation.md), and only a `Closed`
   receipt clears the fence. `PENDING`, an unreachable server or an
   unconfirmed close keep it latched.

Recovery never replays the open, never uses upstream `CloseBuffer` and never
adopts the orphan for the abandoned owner: that lease stays Unknown. It is
bound to the exact native process by the connection itself; a replaced server
starts a new adapter runtime with no latched attempt. The owner chose this
automatic, query-only recovery (2026-11-05), so no Operator step is involved.

## Reservations confined to the open target (Zed 1.24.0)

A buffer that is closing or reserved for synchronization used to refuse every
new open on the Machine, because a different path could alias its native ID.
Private server `1.9.0` answers `CowboyOpenTarget` with what an open of a path
would return (`NOT_OPEN`, `OPEN` with the existing buffer ID, or `LOADING`)
using the same lookup as `open_buffer`, without opening anything. An open is
refused only when that answer is a reserved native ID, is `LOADING`, or is
unavailable; an unopened path or another buffer is admitted. The check runs
before the lease leaves Prepared and again under the active-map write lock
before dispatch. Unresolved opens and unresolved navigation remain
process-wide fences: their native IDs are not known.

## Legacy path holdings (Zed 1.26.0)

Legacy Open retains the original resolved key before dispatch and the native
buffer ID before returning. Repeating the same request cannot retarget a
changed symlink or adopt a replacement native ID. Legacy Close uses that
holding, without resolving the filesystem again, so deletion and renaming do
not strand a known resource. Identity validation and confirmed native release
share the active-map write lock. Other owners, including interactive panes,
remain held. Missing replies retain the pin and the existing native fences.

An interrupted Open is resolved through the same-process recovery above before
Close can free its reservation. A known holding is released only by its exact
native ID; a recovered orphan is never adopted just to close it. The bounded
legacy registry serializes its own opens and closes; it is runtime-local and
does not provide Controller-restart inventory or Machine-wide resource budgets.

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
credentials are used to clear it. The subsequent
[original-peer close candidate](plugin-native-close-confirmation.md) separately
confirms native peer removal; it does not resolve an uncertain acquisition.
Independently authorized recovery and global retained
buffer/history/background-effect budgets remain separate work. These candidate
checks are not production installation or supported-device acceptance; see the
[completion ledger](history/plugin-refactor-completion.md).
