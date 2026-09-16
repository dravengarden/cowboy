# Browser owners for core Code resources

The internal client in `web/src/codeBuffers/` implements typed ownership of the
[Controller buffer API](plugin-controller-buffer-owners.md) and its
[original-owner observations](plugin-owned-buffer-reads.md). It is core Web code,
not an installable Plugin, a public SDK grant or a second resource lifecycle.

Status: source implementation and isolated browser acceptance. Ordinary Review
does **not** import this client yet. Its existing buffer/language calls, Web
bundle, Machine generation and installed Code Plugin are unchanged. Publishing
this prerequisite does not warrant a Controller restart or claim Web cutover.

## Ownership, not React mount lifetime

`createOwnedCodeBuffers({ context })` creates a bounded registry for one core
Service/principal lifetime. Core must retain the registry outside dismissible
views and abort that lifetime before changing authority. The context, fetch
port and timeout are captured once; mutating caller options cannot rebind them.
The future product consumer must connect this signal to its real identity
lifecycle. An injected signal in a fixture is not a real login/logout test.

`reserve({ sessionId, path })` captures immutable input without network I/O.
An effect may then call `prepare()`, followed by at most one `open()` attempt.
Only preparation sends the path and Session ID. Query, read and cleanup use
the original server-issued ID. There is no restore-from-JSON, caller-supplied
resource import, persistent browser grant, path reopen or old-host fallback.
The branded `ResourceId` describes validated syntax, not execution authority;
the Controller still owns authentication, admission and native references.

The optional observer `AbortSignal` detaches one caller. It does not cancel the
owned operation or free its capacity. `close()` synchronously prevents new
open/read calls, waits for the current continuation, and performs one bounded
cleanup pass. Late read results after close or observer cancellation are
discarded. A React consumer must also guard its own state updates after unmount.
StrictMode replay must allocate a new owner in the new committed effect; it must
not reuse an owner already closing or create network effects during render.

| Evidence / event | Client action | Not inferred |
| --- | --- | --- |
| Prepare response arrives after view closes | Release that prepared ID; do not open | A late response belongs to the next mount |
| Open response lost | Retain ID, observe it; never resend open | `prepared` rearms the attempt |
| Fresh open and idle | Permit the requested closed read kind | The open vector certifies current positions |
| Read still running when view closes | Drain it, discard late result, reconcile and clean up | Aborting the view cancels the server borrow |
| Valid `202` DELETE | Retain; a later explicit close first observes, then may release | The server queued cleanup |
| Ambiguous DELETE | Observe original ID only; never rearm DELETE | An `open` observation proves no prior effect |
| `404`, unknown, timeout or malformed reply | Retain unresolved ownership | Successful release |
| Authority lifetime ends | Fence all new I/O, including cleanup | New cookies can adopt the old owner |
| Valid terminal release | Remove the registry entry; repeat close is local | Controller restart restoration |

`close()` returns a discriminated `unopened`, `released` or `retained` result.
`unopened` means no native open was attempted; an unobserved effect-free prepare
may still expire at the Controller. `retained` is not success, an automatic
poller or an instruction to replace the owner. The registry keeps it strongly
referenced so the same core context can explicitly inspect/reconcile it. A
failed request makes the last observation stale; it cannot authorize new reads.

There are at most 64 retained owners, with no LRU eviction of active or unknown
resources. Only terminal release or a no-open local retirement frees a slot.
Session IDs and paths are bounded to 128 and 4,096 UTF-8 bytes. HTTP has a maximum
65-second deadline including response-body reads, above the Controller's
60-second admitted-job limit. Each close pass can use a query and a release,
in addition to draining the one in-flight operation; it is not a 65-second
overall close deadline. No polling, heartbeat, `keepalive` or background retry
is installed. Page termination may strand a live resource; it does not prove
release or implement abandoned-browser/restart recovery.

## Closed reads and bounded transport

`read("language")` and `read("symbols")` return distinct readonly result types.
The codec checks exact fields, operation, API version and original resource ID;
recursively freezes results; and enforces the Controller's numeric, byte, count,
range, version-vector and symbol-depth limits. Nullable `source` and `kind`
fields match Rust serialization, rather than the old Review's optional fields.
`unobserved` diagnostics cannot contain entries. Invalid results are errors,
never a fabricated successful empty observation.

Requests are same-origin, credentialed, no-store and redirect-refusing. Snapshot
bodies are capped at 1 KiB and observation bodies at 2 MiB, with a separate
65,536-chunk bound. UTF-8 decoding is strict. Deadlines include body reads and
do not wait indefinitely for an uncooperative stream's cancellation promise.
Late transport replies cannot revive a timed-out owner operation. Error objects
expose only a closed failure kind and optional HTTP status, not private bodies,
paths, diagnostic contents or underlying transport exceptions.

`openedVersion` remains the native open's lower bound. Neither this client nor
the codec supplies edited-text coordinates, cross-LSP atomic snapshots,
hover/navigation anchors or destination ownership. Do not feed these results
into legacy positional APIs by pretending they share an owned version.

## Acceptance and remaining cutover

`contracts/code-buffer-client.fixture.json` is a public, nonempty wire fixture
consumed by actual Rust lifecycle/read handler tests and the browser codec. It
is not a durable schema, a credential or a generated authorization object.

Focused source acceptance includes 26 Web tests and two Rust handler tests.
The isolated Firefox suite additionally covers six cases: StrictMode replay,
same-stack duplicate reads, read/unmount/remount, pending cleanup, lost open and
authority-lifetime abort. It uses real React development effects, DOM clicks,
Response streams and AbortSignals, but deferred fixture HTTP only. No product
endpoint, normal browser profile or account is accessed.

From the repository's pinned shell, run `just code-buffer-browser-conformance`
with the absolute `/bin/firefox` from `.#cowboy-idb-test-browser`. The accepted
2026-09-16 run used Firefox `151.0.1`, with fixture SHA-256
`aa5f95c0617c04289a26a54f668374a7ba5cc6325e61059ce604a20f11cb82ae`.
This evidence is not actual Review integration, a production identity switch,
native generation coexistence, nonempty real LSP results or supported-device
acceptance.

Before Review cutover, connect the registry to the real Service/principal
lifetime and explicit unresolved-resource presentation; finish positional
content/anchor semantics; accept and independently activate the exact Machine
and Code Plugin; then test the actual consumer on supported devices. Never
mix new owner cleanup with old path-based open/read calls or silently downgrade
an unsupported host. Independent post-effect recovery and browser-abandonment
cleanup remain separate exits in the [completion ledger](plugin-refactor-completion.md).
