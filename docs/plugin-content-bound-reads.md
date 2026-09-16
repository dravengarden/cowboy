# Reads conditional on displayed content

Zed Plugin/private adapter `1.6.0` adds content-bound language, symbol and hover
observations over the original core buffer owner. The server and private
`proto`/`clock`/`text` pins remain Zed `1.13.0` at
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`. No new dependency, native writer,
durable format, public Plugin capability or generic graph executor is added.
Source support does not install that Plugin or switch ordinary Review.

## One conditional read, not a transferable certificate

`POST /api/code/buffers/{id}/read` additionally accepts:

```json
{
  "kind": "content",
  "content": {
    "sha256": "1e0c92b4782cedba7c6b7149c796cc5f31a128c1b55d54999018fbefeebfc95e",
    "utf8Bytes": 7
  },
  "query": { "kind": "hover", "position": { "row": 0, "column": 3 } }
}
```

This example names the complete text `a🙂z\n`. Positions are zero-based UTF-16;
column 3 is immediately after the emoji, not a UTF-8 byte offset. The other
closed queries are `{ "kind": "language" }` and `{ "kind": "symbols" }`.
There is no caller-selected path, Machine, native ID, reload, mutation or
navigation destination in this body. The HTTP body limit is now 512 bytes;
the lifecycle PUT/DELETE limit remains 128 bytes. The old unbound `language`
and `symbols` requests retain their exact shape and meaning.

The digest is lowercase SHA-256 over exact complete UTF-8 native/editor text,
with a separate byte count at most 4 MiB. It does not hash an ETag, file mtime,
JSON encoding, a page, a diff hunk or an open version vector. The browser helper
requires LF text and rejects CR and unpaired UTF-16 surrogates rather than
silently normalizing them. It preserves BOMs, NULs, final newlines and Unicode
normalization differences. A consumer that normalizes a disk file must render
and hash that same resulting text, not hash different source bytes.

The original native owner's passive mirror compares this identity before any
LSP dispatch. A mismatch returns HTTP 200 with an outer `result` containing
`kind: "content"`, the requested `content`, and `result: { kind: "mismatch" }`.
It echoes only the requested identity; it does not expose another native text or
digest. Mismatch is not empty hover, released ownership, a reload request or a
reason to automatically close/reopen. Missing/incomplete/invalid native history
is an error, not a fabricated mismatch observation.

On equality, hover returns `{ "kind": "hover", "contents": [...] }` in the
inner result. Language/symbol queries return `{ "kind": "observed",
"observation": <the existing closed language or symbols result> }`.
The outer HTTP envelope still contains API 1, original resource ID and original
`openedVersion`; that vector remains only the original open's lower bound.
Core independently checks content equality, reply kind, original owner, nested
fields and all existing observation bounds. Hover adds at most 32 blocks and
64 KiB per text/language string; oversized native hover is rejected, not silently
truncated into success. Nested content queries are not representable.

## Spatial and temporal ownership

The adapter captures the mirror epoch with the content comparison, uses native
anchors/current vectors, and checks the original epoch again after the entire
query. This covers the gap before individual native-query admission and
edit/undo ABA while awaiting a reply. The digest is cached on the mirror and
invalidated by actual text operations; it is not cached across buffers or read
from disk. Initial share, reload floors, buffer-state replacement and bounded
history refusal preserve the existing native-coordinate rules.

Equal text before a new query may legitimately have different native history.
The identity is text equality evidence, not a causal version, exclusive writer
fence, filesystem snapshot or serialized authorization. Diagnostic results are
still explicitly **last-observed**; content matching does not prove all LSPs
finished a refresh against one atomic version. A later native/browser edit can
invalidate a result after delivery. Consumers must end the displayed snapshot's
observer and discard its results when changing what they display.

Controller checks and borrows remain those of the original-owner read API:
original user/credential/role, Session incarnation and authenticated Machine
connection at both remote boundaries and response delivery. The distinct core
`bufferLeaseContentSupport` API-1 probe does not start/select a Plugin. Old
Machines refuse before dispatch; old private adapters refuse the closed new
request. Neither can fall back to an unbound positional read. Machine/native
owner locks survive the read, including after uninstall or path removal.
Read failures and cancellation cannot replay an open/release or retire a route.

`captureContent(text)` supplies an immutable, process-local browser value. Its
WeakMap-owned digest cannot be imported from JSON or mutated by a caller.
`owner.readContent(snapshot, query, observer)` captures the validated query
before I/O and returns a query-specific readonly type. It uses the same bounded
transport and original owner job, not a second lifecycle or background poller.
An ended content observer detaches the view but drains the in-flight borrow;
ended product authority also forbids cleanup with replacement credentials.

## Acceptance boundaries

The shared nonempty wire fixture lives at
`plugins/zed/adapter/fixtures/content.json` so the independent private crate can
test without importing the core workspace. Core and browser tests consume the
same data. Nix explicitly includes this data-only fixture in cropped Controller
and Machine checks, never the GPL adapter implementation. The Machine source
closure also now includes its previously omitted `code_buffer_read` module.

Source tests cover exact Unicode hashing/positions, closed fields and output
limits, mismatch without native dispatch, edit/undo ABA, original-owner HTTP
projection, unsupported hosts, cancellation/cleanup and authority loss.
Compile-only tests require query-specific return types and reject arbitrary
operations or JSON snapshots. The isolated Firefox/StrictMode gate adds actual
WebCrypto and displayed-content replacement; its HTTP remains fixture-owned.
The static adapter/server gate checks disk/native divergence at still-valid
positions, then all three content-bound reads after signed temporary uninstall,
source-file deletion and worktree rename. Plaintext fixtures do not claim
nonempty real language-server or physical-device acceptance.

Remaining: explicit disk/native synchronization with its own effect authority
and shared/dirty-buffer rules; owned navigation destinations; Review's consumer
and unresolved-cleanup UI; independently accepted/activated Machine and signed
Code Plugin; supported devices; abandoned-browser/restart and independent
post-effect recovery. A conditional read deliberately does not hide any of
these as a reload side effect or report the entire refactor complete.
