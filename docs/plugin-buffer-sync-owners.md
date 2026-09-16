# Private buffer synchronization owners

Source candidate: Zed Plugin/adapter `1.8.0`, paired with the unchanged static
`cowboy-zed-server 1.0.0`. This extends the [native conditional primitive](plugin-native-buffer-sync.md),
not the ordinary Review API or core authorization. Publication, installation
and a permitted product consumer remain separate acceptance steps.

## Finite private contract

The private socket accepts `prepareBufferSync` with the original owned buffer
lease, a closed `refresh_from_disk` purpose declaration and bounded desired
content identity. It cannot accept a replacement path, native ticket or caller
version. The adapter derives the exact current CRDT clock from its passive
native mirror. Neither that declaration nor a read lease is a Service grant.
Core's generic Machine dispatcher refuses both `prepareBufferSync` and
`bufferSync` before selecting any runtime, including requests with an injected
worktree. An authorized core continuation has not yet been added.

Preparation must find one original, open Cowboy owner across **every entry
with the same native buffer ID**. Legacy owners count too. Any unresolved owned
open blocks preparation because its unobserved native buffer may alias the
target. The adapter retains the buffer's ownership and a distinct process-local,
non-recycled operation ID. Native tickets never leave this private coordinator.
The original native instance, buffer version, content identity and native-issued
ticket remain fixed; subsequent Apply/Query/Retire requests cannot change them.

Only effect-free preparations expire, after 30 seconds. At most 256 operations
are retained. A cancelled native preparation has no Apply authority and may
expire; an attempted Apply records Unknown and removes the deadline **before**
the first transport await. Cancellation, timeout, disconnection and queue errors
cannot restore its one-use budget or discard its fence. Duplicate Apply and
attempted retirement of Pending/Unknown return local evidence without native
dispatch. Query can contact only the original native ticket. A missing/retired
ticket after an attempted Apply remains unknown, not restored or released.

Applied must repeat the exact desired hash and byte length plus a validated
resulting native version. Before releasing exclusion, Applied invalidates old
coordinates and establishes that version as the passive mirror's required
floor; a reply arriving before native text events cannot expose stale reads.
Applied/Refused permits local fence removal. Terminal
evidence is retained until explicit retirement; capacity pressure never evicts
an unresolved or terminal operation. A cancelled local cleanup is completed by
a later original-ID observation without resending Apply. Old/retired IDs cannot
clear a later reservation or reopen a buffer.

## Admission and alias exclusion

The fenced buffer rejects reads and release, including legacy path-based calls.
All new native opens and legacy navigation are temporarily refused while any
fence is live: a not-yet-opened path or navigation destination may resolve to the
same native ID through overlapping worktrees or native canonicalization. This
is deliberately conservative, not a claim of per-path exclusivity. Existing
unrelated buffers may still be read and released. A refused owned open stays
Prepared rather than becoming a fabricated unknown native effect.

Ownership admission and active-buffer operations share the adapter's existing
locks; existing reads finish before preparation captures the native version.
The native server still owns its final clean/version/file/worktree/shared-peer
check and mutation in one update turn. The adapter fence does not replace that
atomic condition, fence arbitrary filesystem writers or make LSP state atomic.
No ordinary Zed state, disk content or source worktree is modified by the
coordinator; native Apply only imports the bounded observed disk text.

## Acceptance and limits

Focused tests cover legacy/owned sharing, native-ID aliases, unresolved opens,
all affected admission paths, independent reads, missing/malformed native
receipts, explicit coroutine and socket-observer loss, monotonic operation IDs,
expiry, bounded capacity and interrupted terminal cleanup. The native-process
gate also exercises two actual owners, refusal until one releases, real text
synchronization, original-owner content reads, duplicate Apply without another
native request, and cleanup after path removal.

Run the complete pinned gate, `just zed-native-sync-conformance <server>`,
`just zed-plugin-conformance <adapter> <server>` and the connected core Code
gate against the exact release pair. No production activation is implied by
the source tests. Core synchronization authority, authority-loss handling,
original-generation routing, Review integration, owned navigation destinations,
independent post-effect recovery and supported-device acceptance remain open.
The purpose enum is a closed request declaration, not that missing authority.
