# Original-peer native close confirmation

## Original-operation recovery candidate

Zed `1.27.0` pairs adapter `1.27.0` with private server `1.12.0` and
introduces private Close protocol 2. The implementation below is a release
candidate until exact signed artifacts and serving-process acceptance are
recorded; the original protocol-1 acceptance follows separately.

Before dispatch, the adapter retains a non-recycled operation ID, server
instance, complete native ID set and original local owner bindings. The
native handler records Closed or Refused atomically with the peer-map update.
Query uses the same peer, instance, operation and complete set. It never sends
Close again or infers success from current absence. Lease Query/Release,
navigation ReleaseUnknown Query/Release and orphan-open cleanup can retrieve
the original receipt. Only a matching Closed result and unchanged original
bindings allow synchronous local pin and mirror removal. Missing, malformed,
refused or unavailable receipts retain uncertainty; navigation execution and
synchronization Unknowns are not resolved by this close protocol.

The original group's shared native IDs also reserve their independent owners
against release, including pathname aliases. Otherwise another owner could
release during uncertainty and leave recovery dropping the final local pin
for an ID excluded from the original native close batch. Safe reads of those
shared buffers remain available; independent release resumes after recovery.

The adapter retains at most 2,048 pending receipts plus queued retirements.
The server admits at most 4,096 receipts and 64 peer histories, without
eviction. After confirmed local cleanup, a separate Forget frame retires the
receipt; it cannot close buffers and allocates no reply waiter. A full
transport queue retains that retirement for a later attempt. Per-peer high
water marks prevent a retired operation from executing again, including after
the same native IDs have been shared anew. Forget cannot advance that mark.
Process loss invalidates this process-local evidence; Controller restart does
not gain authority to adopt a replacement adapter or reconstruct old owners.

The 2026-11-06 candidate passed 68 native server tests and the isolated
`zed-native-navigation-conformance` gate against these static artifacts:

| Artifact | SHA-256 |
| --- | --- |
| Adapter 1.27.0 | `b8b522c43df2cdf1e06e29d505eaf62ddc35271547c9be5643a5162205517006` |
| Server 1.12.0 | `a5c55631eaa36492f291745bcd73ec559091a83f1f85edfd8d7f298a4cb7848e` |

The fault fixture discards a real Closed reply, observes retained local
uncertainty, then recovers by the original receipt. Duplicate open of the
released lease remains refused. The same gate covers nonempty LSP navigation,
independent handoff, native input bounds and synchronization recovery. These
are isolated acceptance results, not evidence of production activation.

## Original protocol-1 acceptance

Zed Plugin/private adapter `1.15.0` selects private server `1.2.0`. Upstream Zed,
third-party pins, Code API 1, SDK 1.8.1 and Machine protocol 21 are unchanged.
This extends the [single-use Open](plugin-native-open-once.md) and
[owned navigation](plugin-owned-navigation.md) candidates. It does not publish,
install, replace a running native process or enable production navigation.

The [candidate acceptance](releases/native-close-confirmation-candidate-2026-09-18.md)
records exact immutable artifacts, native/source gates and separate lifecycle,
connected and browser evidence.

## What Closed proves

The private protobuf `CowboyCloseBuffers` route operates on the actual sender's
native peer map. An effect-free probe observes a random process instance. Close
requires that same instance and a strictly ascending, nonzero, unique set of
at most 33 native IDs (source plus 32 navigation targets). The native handler
validates every member before removing any member, in one synchronous GPUI
update. A missing member returns Refused with no prefix removal. Foreign
instances, malformed sets and unsupported pairs fail closed.

Closed echoes the exact original instance and complete ID set only after that
peer's entries and shared/LSP handles have been removed. Other peers are not
modified. This is **peer-ownership confirmation**, not proof of global buffer
deallocation, delivery of every LSP didClose, stopped background tasks,
filesystem restoration or a recovered Agent turn. There is no saved native
close journal, recovery query or independent server-side replay ticket.

## One-use local ownership

The adapter derives a non-cloneable close plan while holding the active-owner
write lock. All pathname aliases and typed owners of each native ID count;
releasing one owner never closes another owner's buffer. A purely local shared-
owner release needs no native command. Navigation submits one complete batch,
not sequential closes that could remove a prefix before a later failure.

After the effect-free probe and before the effect await, every closing native
ID is fenced and the ordinary lease becomes Unknown (navigation becomes
ReleaseUnknown). Original pins, mirrors and historical target evidence remain.
Cancellation during the probe has no close effect and leaves the original Open
or Retained state. There is no fallback to upstream unacknowledged CloseBuffer.

Only the original matching Closed response permits mirror retirement and local
pin removal, synchronously with no intervening await. Wrong/partial replies,
native refusal, disconnect, cancellation and the unchanged 30-second native
transport deadline retain uncertainty. Query, duplicate Open/Execute and Release
never resend the close or infer completion from current native absence. A late
reply to a removed waiter cannot clear the fence. The one-use dispatch is an
adapter guarantee, not a claim that arbitrary replayed private frames are
deduplicated by a native operation journal.

An unresolved close fences reads/mutations of those native IDs and new process-
wide acquisition, synchronization and destination handoff. Independently known
buffers can still be read or explicitly released; their completion cannot clear
another close's fence. Capacity remains consumed until independently authorized
resolution or process teardown. Restart/refusal/forced test teardown is not
that resolution. No automatic recovery is added here.

## Acceptance layers

- Native GPUI tests cover exact/batched removal, other-peer preservation,
  missing-member refusal, malformed sets, instance replacement and discarded
  results without treating current absence as a saved recovery record.
- Adapter tests cover actual private frames, shared owners/aliases, unsupported
  probes, cancellation on both sides of admission, malformed responses, late
  replies, a genuine 30-second deadline and unrelated-owner progress.
- The isolated native gate uses the actual immutable server for atomic refusal,
  original-peer observation and loss of a real Closed response. Only that
  source-test adapter contains the response-drop hook; shipped executables do
  not. The final static pair separately exercises confirmed ordinary/group
  release and independent handoff through its real socket and test-only LSP.
- Temporary signed lifecycle, supplied Controller/Machine connected v5 and
  browser-owner gates remain separate. Dropping a Controller/Machine reply
  after a completed native close may settle by original adapter Query; losing
  the native close reply itself cannot. Those are distinct fault boundaries.

Global retained-history/background-effect limits, independent post-effect
recovery, signed publication/installation and actual deployed native/device
acceptance remain in the [completion ledger](history/plugin-refactor-completion.md).
