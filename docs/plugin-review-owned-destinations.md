# Review-owned navigation destination reader

Review's source and working-diff symbol surfaces now use explicit navigation
preparation on the same [content-owned source](plugin-review-owned-consumer.md).
They do not probe five effectful queries on hover. Prepare is effect-free;
**Acquire targets** is the separate one-use Execute. Production acquisition
admission remains default closed. A Web update grants no native capability and
changes no Service/Machine policy or installed Plugin.

## Lifetimes and evidence

The source queue serializes preparation with its existing reads. It captures the
point before queueing and binds the authentic complete LF snapshot. The diff
consumer uses only its validated new-side complete-file projection. Replacing
the source capture, changing the selected point, or leaving the symbol view
ends the observer and fences an attached or late navigation's Execute. A
preparation cancelled before admission does not dispatch. An admitted native
continuation drains; it is not retried, imported by ID or replaced by path.
Navigation immediately hides old source annotations. After group retirement,
the source status offers an explicit **Check** of the original owner and content
before restoring annotations; retirement never silently reopens or refreshes it.

The target chooser retains the original opaque target tokens and pages five
choices locally at a time without allocating resources. **Read target**
requests the [original-index handoff](plugin-browser-buffer-destinations.md),
opens that independently reserved owner once, and reads only its complete
[native text](plugin-native-text-reads.md). No file endpoint, disk ETag, legacy
navigation result or serialized hash can stand in for this ownership.

Only a verified complete capture with the advertised content identity may be
displayed. Both target endpoints are checked against the actual text, in
zero-based UTF-16 coordinates: no surrogate split, nonexistent line, column
clamp or reversed range. A zero-width/EOF target reveals a line without
highlighting a neighbouring character. CodeMirror displays exactly that LF
capture. The reader is a historical read-only navigation snapshot, not a live
disk tab or proof of present LSP freshness. It deliberately attaches no hover,
Outline, diagnostics or recursive navigation authority. Such future reads need
their own fresh conditional content checks on this same owner.

One target view is open per navigation surface. Closing it removes text and
fences new work synchronously, then performs the ordinary bounded cleanup pass.
It never releases the group. Conversely, group Release does not close a prepared
or opened target owner, and an opened target stays readable after the group is
released. Closing the surrounding symbol/document view closes its target view
too; this is not cross-tab adoption or a persisted navigation history.

## Uncertainty and recovery

- A lost handoff checks only the original navigation. A subsequently discovered
  Prepared target needs the explicit **Open target** action; status checks do
  not Open it.
- A lost/pending Open is one-use. **Check target** only observes the original
  owner. Once that original Open is observed, **Read target text** is separate.
- Mismatch, stale paging, invalid coordinates or hash disagreement show no text
  or old highlight. No partial render, path fallback, automatic reopen/reload,
  retry timer or cancellation-driven replacement is available.
- Closing during handoff retains the original capacity/late ID for cleanup but
  cannot Open it. Closing during reading discloses no late text and starts no
  further pages. A second consumer cannot take over the existing child's cleanup.
- Ending core Service/principal access hides target text and labels and fences
  every remote action. Remount or new login cannot adopt these owners.

Settings → About also contains a passive **Code navigation** recovery projection.
Mounting, expanding and paging are local, five rows at a time. It has only
original-group Query and confirmed Release: no Execute, target disclosure,
import, polling or automatic cleanup. A lost Release remains query-only.
Confirmations recheck the current core handle, not a saved row's eligibility.
Ordinary buffer cleanup is separate and still waits for group retirement.
Neither release operation promises rollback or physical native-buffer closure.

## Acceptance

Run `just review-destination-browser-conformance <absolute-firefox>` alongside
the owner, context, cleanup, synchronization, Review source/diff/document-refresh
and Settings recovery suites. The ten actual React development StrictMode,
MUI and CodeMirror cases use synthetic HTTP and real browser WebCrypto. Unit
cases additionally exercise invalid UTF-16 endpoints, forged/duplicate target
consumers, lost/pending Open, lost handoff, hash mismatch and late read cleanup.
The connected v5 native gate must pass all 18 checks on an exact immutable pair;
do not relabel unchanged binaries with the new Web revision.

These gates do not replace native pre-acquisition resource bounds, registered
Machine/exact installed-generation acceptance, supported-device checks or
independently authorized post-effect recovery. Those remain open in the
[completion ledger](plugin-refactor-completion.md).

The [2026-09-18 Web release](releases/review-destination-reader-2026-09-18.md)
records exact artifacts, browser/native gates and bounded production observations.
It does not enable production navigation acquisition.
