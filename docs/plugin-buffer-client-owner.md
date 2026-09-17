# Browser owners for core Code resources

The internal client in `web/src/codeBuffers/` implements typed ownership of the
[Controller buffer API](plugin-controller-buffer-owners.md) and its
[original-owner observations](plugin-owned-buffer-reads.md). It is core Web code,
not an installable Plugin, a public SDK grant or a second resource lifecycle.

Status: source implementation and isolated browser acceptance, now connected
to the [core product identity lifetime](plugin-buffer-product-context.md).
The later [Review source consumer](plugin-review-owned-consumer.md) imports it
only after an explicit Service-owned protocol-20 selection. Protocol-19,
local/workspace consumers remain on the pre-cutover route; no owned attempt
falls back. The [working-diff consumer](plugin-review-owned-diff.md) adds a
separately validated complete-content coordinate projection; staged/history
views cannot borrow current-file positions. Native rollout and supported-device
acceptance stay separate.

The later [native text reader](plugin-native-text-reads.md) adds
`owner.readText(identity, observer)` on this same original opened owner. It
returns a complete verified `CapturedContent` or an explicit mismatch/stale
observation, never partial pages or a replacement path read. It does not add a
navigation destination-import API or connect an ordinary Review destination view.

## Ownership, not React mount lifetime

`createOwnedCodeBuffers({ context })` creates a bounded registry for one core
Service/principal lifetime. Core must retain the registry outside dismissible
views and abort that lifetime before changing authority. The context, fetch
port and timeout are captured once; mutating caller options cannot rebind them.
`productCodeBuffers.ready()` connects that signal to the actual core product
identity/dataset owner without importing the socket store. An injected signal
alone in a fixture is not a real login/logout test; the separate product-context
fixture exercises the production binding and end rendezvous, not the auth UI.

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

The subsequent [content-bound extension](plugin-content-bound-reads.md) adds
`captureContent` and `readContent` with query-specific types and a distinct
native mismatch result. It does not change the lower-bound-only semantics of
the two unbound methods below, install a Plugin or switch ordinary Review.

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

The complete `nix develop -c just check-compact` passed before integration:
1,335 all-feature Rust, 305 Machine, 26 core adapter, 35 private Zed, 1,536 Web
and 17 isolated PostgreSQL tests, plus formatting, lint, dependency, contract,
feature and release-build gates. The existing ignored acceptance tests remain
ignored; a passing unit gate does not stand in for those isolated process gates.

Source commit `b190fe69b1725a8bdb1e986071ed8216cdb4dc04` rebases this work onto
remote `c1c81e72` (retained browser-record deletion), preserving its change to the
shared browser runner. Rust/core/native sources and the new client are unchanged
by that integration. The integrated tree additionally passes Web typecheck,
lint, all 1,538 Web tests, Web build, the runner checks, and all four relevant
real-browser suites: Code buffers **6**, Settings recovery **9**, IDB owners **8**
and outboxes **16**. That publication was source-only; it activated no application
component, Machine or installed Plugin and did not restart a live session.

The subsequent [product-context integration](plugin-buffer-product-context.md)
binds the real Service/principal lifetime and keeps final local outbox drain
separate from ending remote authority. The [core Settings cleanup surface](plugin-buffer-cleanup-surface.md)
now presents unresolved owners without polling or replay. Before Review cutover,
finish positional
content/anchor semantics; accept and independently activate the exact Machine
and Code Plugin; then test the actual consumer on supported devices. The
[browser synchronization continuation](plugin-browser-buffer-sync.md) now binds
explicit refresh confirmation and retirement to the same owner, and fences
ordinary cleanup while synchronization remains unresolved. Never
mix new owner cleanup with old path-based open/read calls or silently downgrade
an unsupported host. Independent post-effect recovery and browser-abandonment
cleanup remain separate exits in the [completion ledger](plugin-refactor-completion.md).

The subsequent [browser navigation continuation](plugin-browser-buffer-navigation.md)
adds bounded, original-source Prepare/Execute/Query/Release ownership with a
separate recovery projection. It does not yet adopt destination preparations or
enable an intended Review destination view; production acquisition stays closed.
