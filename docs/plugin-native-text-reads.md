# Complete text from an original native buffer

Zed Plugin/private adapter `1.13.0` adds a finite read prerequisite for owned
navigation destinations. Controller, Machine and core Web retain the existing
ordinary buffer owner. This adds no open, reload, synchronization, navigation
acquisition, installation or second resource lifecycle. Private server `1.0.0`
and every upstream dependency pin are unchanged.

The [candidate acceptance](releases/native-text-reads-candidate-2026-09-18.md)
records the clean source gate, exact native and Controller/Machine artifacts,
two successful v5 connected runs and the separate browser-owner acceptance.
It does not publish, install or activate those candidates.

## Closed original-owner protocol

`POST /api/code/buffers/{resourceId}/read` additionally accepts:

```json
{
  "kind": "text",
  "content": { "sha256": "<complete lowercase SHA-256>", "utf8Bytes": 65543 },
  "page": { "kind": "start" }
}
```

The digest is the requested complete native LF text, not the displayed path,
disk bytes, ETag, page or open vector. It may come from a retained navigation
location, but that JSON is not a grant: the original resource must first have
successfully completed its explicit ordinary Open.

The existing no-store API-1 envelope carries original `resourceId` and
`openedVersion`, plus `result: { kind: "text", content, result }`. The innermost
closed result is one of:

- `{ kind: "mismatch" }`: complete content differs. No replacement digest or
  text is disclosed; missing/invalid native history is an error, not mismatch.
- `{ kind: "stale" }`: a continuation's original owner/revision observation no
  longer matches. This cannot be an initial-page result.
- `{ kind: "page", snapshot, offset, text, nextOffset }`: one exact UTF-8 page;
  `nextOffset` is explicitly null only at complete EOF.

To continue, send the same content and original owner with
`page: { kind: "continue", snapshot, offset: nextOffset }`. The opaque lowercase
64-hex snapshot hashes a domain-separated tuple of original adapter instance,
ordinary owner, native ID, monotonic mirror revision and complete content. It
does not expose those native references, allocate a cursor registry, extend an
owner or authorize any other read/effect. It cannot be used without the original
owner. Edited-then-undone text, mirror replacement, another same-native-ID owner
and process replacement cannot revive an old continuation.

The adapter holds the actual owner and passive-mirror locks through identity,
revision and page capture, with no asynchronous gap and no native command.
Only the bounded rope segment is copied; there is no complete-text allocation
per page. Every request still checks the original Service user/credential/role,
Session incarnation and exact Machine connection before/after I/O and before
disclosure. The new pathless core-only `bufferLeaseTextSupport` API-1 probe does
not start a Plugin or infer support from protocol 21 or native health. An older
Machine or private adapter refuses, with no unbound or path fallback.

## Bounds and client lifetime

Complete content remains at most 4 MiB, with the existing independently bounded
mirror history. A page copies at most 64 KiB, clipping up to three trailing
UTF-8 bytes; split starts, zero-progress and undersized nonfinal pages are
refused. Core validates request/reply identity, exact offset, stable snapshot,
length, LF and EOF; complete single-page text is independently hashed. This is
not a new bound on native LSP acquisition or total process RSS.

`OwnedCodeBuffer.readText(identity, observer)` captures its input before I/O,
retains one busy owner across at most 65 pages and returns only `complete`,
`mismatch` or `stale`. No partial text is returned to a consumer. Complete text
must pass actual WebCrypto SHA-256/UTF-8 length verification before becoming a
genuine process-local `CapturedContent`; CR, unpaired surrogates and changed
snapshot/owner/offset/shape are errors. BOMs, NULs and final newlines are retained.
Each page response is limited to 512 KiB including JSON escaping and vector
metadata, below core's independent 2 MiB reply cap.

New pages and final disclosure stop after a 60-second overall observation
budget. An already admitted HTTP request still drains within its existing
65-second transport bound: the budget is not cancellation of a Service borrow.
View cancellation or close also drains only the current page and stops new page
admission. Authority end fences transport and cleanup rather than adopting new
cookies. Original-ID cleanup and retained uncertainty are unchanged; partial,
stale or mismatched text never triggers retry, release/reopen or synchronization.

A completed read is an observation, not a continuously live revision, native
anchor grant or renewed navigation position. A future destination consumer
must display this exact complete capture, validate its target range against it,
end positional observers when replacing the view and obtain fresh conditional
reads. It cannot treat a historical location as current native authority.

## Acceptance and remaining exits

`plugins/zed/adapter/fixtures/text.json` is shared by independent native, core
handler/codec and browser tests. Deterministic tests cover UTF-8 page boundaries,
empty/maximal content, owner/revision ABA, incomplete history, malformed/stalled
pages, final digest mismatch, cancellation/cleanup and authority loss. The
existing browser-owner gate adds three actual browser/WebCrypto/stream cases.
Those eleven results are now part of the eighteen-case owner suite, extended by
the separate [browser navigation continuation](plugin-browser-buffer-navigation.md).

The static native-pair gate reads a two-page Unicode target after parent release
and source/target deletion. The signed temporary lifecycle gate also reads
original target text after uninstall. The connected gate advances to v5 with
eighteen checks and actual authenticated/enrolled two-page destination reads
after parent release/path removal. Historical v4 evidence does not accept this
new read. These are disposable fixtures, not a production installation or an
actual Review destination view.

Service navigation acquisition remains default closed. The intended Web
destination handoff/view, native pre-acquisition resource budget, signed rollout,
supported-device acceptance and independent recovery remain separate exits.
No running Controller, Machine, native process or installed Plugin is replaced
merely by adding this reader.
