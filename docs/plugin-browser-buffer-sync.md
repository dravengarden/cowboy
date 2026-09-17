# Browser-owned Code synchronization

Core Web now owns typed continuations for the [Service synchronization API](plugin-service-buffer-sync.md).
Its [verified Web-only delivery](releases/browser-buffer-sync-2026-09-17.md)
is published and active with all sixteen observed workers retained.
This is not an installable Plugin, a generic DAG executor or an additional
native grant. The later [Review source consumer](plugin-review-owned-consumer.md)
can prepare this continuation after protocol-20 selection; it cannot Apply as a
read side effect. The resident
Machine and installed Code Plugin still require their independent acceptance
and maintenance/installation boundaries.

## One original buffer, content and confirmation

`OwnedCodeBuffer.prepareSynchronization(capturedContent)` requires an originally
attempted Open and fresh Open evidence on that exact owner. The complete LF
text must come from `captureContent`; JSON, caller-supplied hashes and cloned
objects cannot forge a capture. Synchronization additionally refuses leading
BOMs, matching the native primitive; it never silently normalizes source text.
Preparation submits only the fixed `refresh_from_disk` purpose and the captured
SHA-256/UTF-8 byte count. It never sends Apply or uploads file contents.

The prepared continuation remains attached to that buffer, independently of
the observing React view. At most one synchronization exists per retained
buffer, inside the existing 64-owner bound. No LRU eviction, durable browser
grant, ID importer, path reopening, fallback to old Code APIs or background
polling is added. An unsupported host fails closed.

`preview("apply" | "retire")` is local. It returns an opaque process-local token
bound to this operation, action and observation revision. `confirm(token)`
rechecks the original context and admission state synchronously, consumes the
token and starts one bounded request. Foreign, cloned, consumed and stale
tokens cannot execute. An intervening Query invalidates a preview even if it
reports the same Prepared value. Closing the source consumer disables Apply
immediately; the consumer must close the owner when abandoning its text/view.
UI confirmation is not Service authorization: the Service still checks its
original credential, user, Session and Machine connection at effect boundaries.

Apply is consumed before I/O and never rearmed by cancellation, failure, 202,
Prepared evidence or a new view. Mutation replies cannot change the operation,
resource, purpose, content or terminal native version. Pending/Unknown is not
completion. HTTP 202 describes the Service job separately from the native
state; HTTP 200 can still contain Pending or Unknown.

## Synchronization and buffer cleanup share ownership

The buffer and its synchronization share one synchronous admission fence and
the existing bounded, same-origin/no-store transport. Response bodies are
strict JSON/UTF-8, capped at 16 KiB; Applied versions have at most 256 strictly
ordered replica entries. The HTTP deadline is at most 65 seconds, including
body reads. An observer's AbortSignal detaches only that observer; core drains
and records the admitted continuation.

| Event | Permitted continuation |
| --- | --- |
| Prepared, live source consumer | Separately preview/confirm Apply or retirement |
| Source view closes while preparing/applying | Drain the original job, retain synchronization; never auto-Apply or auto-retire |
| Apply outcome unknown or pending | Original-operation Query only; no buffer read/release or inverse |
| Applied/refused, fresh and idle | Explicit retirement; no repeated Apply |
| Retirement reply lost | Original-operation Query only; no repeated DELETE |
| Valid 202 retirement | Query first; only an exact no-admission acknowledgement permits a new, separately confirmed retirement |
| Service reports Retired or effect-free Expired | Remove only this synchronization; require a fresh buffer observation before reads or later explicit cleanup |
| Context ends, 404, malformed reply or transport failure | Retain unresolved evidence; never adopt new authority or fabricate release |

The client deliberately holds buffer reads/releases until synchronization is
retired, including known terminal effects. `close()` is a bounded pass and
returns Retained while this continuation exists. It does not automatically
query, retire or compensate synchronization. The Settings buffer-cleanup row
explains this fence and disables ordinary cleanup actions until it is resolved.

An unreceived preparation response supplies no operation ID and cannot grant
Apply. Only inert Service/native preparations may expire; the browser does not
invent a local expiry timer or claim a remote retirement. The buffer remains
owned with stale evidence. Any subsequent explicit original-buffer operation
still faces the Service's admission/expiry checks. A reload or process loss
does not recover these in-memory owners.

## Bounded core presentation

`productCodeBuffers.synchronizations` is a passive `ReadableStore` projection
sharing the original product identity lifetime and notification leases. Core
Settings → Info renders it only while operations exist, initially collapsed,
with at most five rows per page. It exposes captured path/Session and content
identity, not file contents, transport errors, native references or HTTP IDs.
Context end removes those private labels and fences stale confirmations before
React renders the redaction. Construction, mount, expansion, pagination and
remount do not discover a dataset, access storage or issue HTTP.

Check synchronization is distinct from both Review refresh and Retire operation.
Both mutations use the shared core `ConfirmSheet`, name the exact captured
target and explain their limits. A native refresh does not write the source
file and has no automatic undo. Retirement is not buffer release, Session
closure or post-effect restoration. Unknown operations cannot be hidden through
a dismiss/forget action. Replacing the projection requires an explicit remount;
an old row cannot target a new operation even on the same path and buffer.

## Acceptance and remaining exits

The shared `contracts/code-buffer-sync.fixture.json` is compared with actual
Rust Service serialization and the browser codec, including a nonempty native
version vector and the genuine SHA-256 of captured UTF-8 text. Unit/compile
checks cover closed decoding, ownership, shared exclusion, one-use tokens,
terminal monotonicity, cancellation, authority loss, unsupported hosts and
original-handle retirement. The isolated
`just code-buffer-sync-browser-conformance <absolute-firefox>` gate uses real
React/MUI/StrictMode and deferred fixture HTTP to exercise confirmation,
unknown results, view replacement, identity end and bounded 360px presentation.
It never opens a real account, native Plugin or production endpoint.

These gates do not prove ordinary Review integration, production end-to-end
synchronization, owned navigation destinations, abandoned-browser/restart
recovery, independently authorized restoration or physical iOS/WebKit/native
acceptance. Those exits remain in the [completion ledger](plugin-refactor-completion.md).
