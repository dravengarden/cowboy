# Content-owned Review source consumer

The ordinary source-file Review now has a core-owned Code path. This is not a
Plugin, an alternative resource registry, a Machine upgrade or a complete Code
refactor. The [completion ledger](plugin-refactor-completion.md) still requires
owned navigation destinations, diff-coordinate integration, native rollout and
supported-device acceptance.

## Select before acquiring

The Service manifest adds the closed `bufferMode` observation: `legacy`, `owned`
or `unavailable`. Its v4 ETag includes that selection. A connected Session
Machine at protocol 20 or later selects the owned source consumer; a disconnected
Machine is unavailable, not evidence for a legacy downgrade. Local and stable
workspace contexts retain the legacy route. Native capability, current product
authority, Session incarnation and original connection are independently
checked by each resource operation; the manifest is not a grant and does not
probe, start, install or upgrade a native generation.

Web waits for selection before acquiring either source buffer. A pre-cutover
Controller with no field retains its existing route, while unknown explicit
values fail closed. Once this mounted Session view observes Owned, later
manifests cannot downgrade it. Unsupported native preparation/read/synchronization
fails visibly without legacy fallback. Protocol-19 production Machines are not
switched by this release. Their separate maintenance and the signed Zed `1.9.0`
installation remain required for the complete new native path.

Source language, Outline and hover all use the same original core owner.
Switching Session/path or source/diff presentation ends that consumer. Diff
views remain explicitly legacy, never a fallback from an owned source read.
The owned source consumer does not call path-based navigation: destination
ownership is unfinished, so cross-file symbol navigation is unavailable in
this mode. Local syntax coloring, source reading and Markdown links remain.

## Displayed content and cancellation

Review explicitly normalizes CR/CRLF to LF, renders that same text and captures
its SHA-256/byte count with the core helper. Only a complete, non-limited source
file can supply positional reads; pages, diff hunks and ETags cannot. Core
capture still rejects invalid Unicode and text over 4 MiB. Partial source
rendering remains useful without pretending to supply content equality.

The consumer retains one owner across disk observations. An eight-entry bounded
queue serializes language, hover, Outline and explicit synchronization
preparation. A cancelled waiter leaves immediately; the admitted core borrow
drains before the next queued request. Cancelled queued work does not dispatch.
Prepare/Open are one-shot, including transport failure, Pending and unsupported
responses. Closing fences new work synchronously and makes one core cleanup
pass. Unresolved originals stay in Settings; no timer, retry, LRU, JSON import,
browser-abandonment cleanup or replacement-cookie adoption is introduced.

Changing displayed text hides old annotations during render and ends its
observer at commit, before paint or another confirmation click. Late hover and
Outline results cannot attach to replacement content. File, page and manifest
success/error continuations now also check their original abort signal; a
superseded manifest cannot fetch changes using a newer request's signal. A
manifest revision change reloads disk text without reopening an owned buffer.
The open vector remains a lower bound; diagnostics remain last-observed, not
an atomic LSP snapshot.

Mismatch is explicit, never an empty success or hidden reload. Source view
shows it as a single warning row; checking, matched and incomplete states take
no space, and rendered previews omit the row because they use no positions.
**Check**
may explicitly observe the original buffer before a new conditional language
read. Initial Open uncertainty can be checked on the original ID,
without retrying Open or recreating a failed/unobserved preparation.
**Reload…** only prepares synchronization against the exact
displayed content. Settings → About owns the existing separate Apply/retirement
confirmation, reached through **Confirm**. No read, content change
or unmount applies, retires or undoes it.
Abandoning text during/after preparation closes the source owner immediately,
including before the preparation ID arrives, so an old Settings confirmation
cannot Apply. The original synchronization remains retained for explicit
observation/retirement. This reload is not filesystem writeback or an undo.

## Evidence boundaries

The deterministic consumer tests cover selection, bounded queues, captured
positions, late preparation, ambiguous Open, cancelled reads/queues, unsupported
hosts and source abandonment during synchronization. The isolated
`just review-code-browser-conformance <absolute-firefox>` runs the actual Review
hook, Outline and status surface with React development StrictMode and WebCrypto.
It checks normalized display/hash identity, shared read exclusion, late content
results, partial files, mismatch and stale Apply fencing. HTTP is synthetic;
this is not a full logged-in Review or physical iOS/WKWebView run.

The separate eleven-group connected native gate now also requires the actual
immutable Controller manifest to select Owned. Its supplied protocol-20 Machine
and exact Zed pair remain disposable fixture processes. It does not install or
activate those components on a registered Machine, establish real LSP freshness,
recover abandoned owners or accept a physical device. Preserve all existing
browser ownership, product-context, synchronization and cleanup regressions.

The [2026-09-17 Controller/Web delivery](releases/review-owned-consumer-2026-09-17.md)
records the complete gate, 44 browser cases, two actual eleven-group process
runs and production continuity. The active protocol-19 Machine and Zed 1.8
were not upgraded; conditional consumer delivery is not their native cutover.
