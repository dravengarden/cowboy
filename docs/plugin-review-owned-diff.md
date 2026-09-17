# Content-owned working diff reads

Ordinary Review's `unstaged` and `combined` working-file diffs now use the same
core owner as the [source consumer](plugin-review-owned-consumer.md), after the
existing Service `bufferMode: owned` selection. This is a finite consumer of
the existing content-bound API, not a new Plugin, native operation, writer,
public schema, resource registry or general navigation graph.

An owned-mode diff never opens/reads/closes through the old path-based Code
routes. Staged/index and historical commit views remain useful render-only
observations: current native file positions cannot describe their old text.
Local/workspace and pre-cutover legacy selection retain their existing route;
failure on an owned route cannot select it as a fallback.

## Two observations, one checked coordinate mapping

Only a complete, non-limited patch can request a current-file projection. The
file read waits for the existing core Service/principal discovery and captures
its original lifetime. It uses the ordinary read-only file endpoint, with at
most 32 sequential pages, 4 MiB of source bytes and a 65-second overall deadline.
Every page must preserve path, file revision and size, use a new bounded cursor,
and terminate with exactly the declared bytes. Changed, missing, limited,
malformed and incomplete observations refuse without retry/cursor restart.
An ended view or core context cannot dispatch another page or consume a late
reply with replacement authority. No credential is captured or persisted.

Normalize the complete assembled file to LF, including CRLF split across two
pages. Normalize the displayed patch by the same rule. The projection then
validates every new-side context/addition line against the complete file,
ordered non-overlapping hunk ranges/counts and EOF newline markers. It rejects
unsupported conflict/multi-file formats, invalid Unicode, oversized text and
more than 100,000 split lines. The patch budget is 8 MiB; larger Review previews
can still render without Code intelligence. There is no whitespace approximation:
an ignore-whitespace diff may refuse if its context does not exactly match.

The immutable process-local projection is backed by private WeakMaps. Cloned
JSON and forged structural values cannot map positions. Zero-based display
UTF-16 positions map only to admitted new-side rows; columns exclude the diff
marker and cannot exceed a line or split a surrogate pair. A tap on a deleted
or header row is rejected before nearby-symbol ranking, so it cannot borrow a
different new-side symbol merely because one is close on screen.

The native request hashes the **complete current-file text**, not the patch or
just its visible lines, and reads on the original core owner. Native equality
and epoch checks remain independent on every read. Metadata revision is only
paging evidence, not a content certificate. The two filesystem observations do
not claim atomicity, old-side identity, a Git base snapshot, causal history or
fresh atomic LSP diagnostics. Hidden file contents stay part of the exact native
comparison even though Review renders only the diff.

## Lifetimes and explicit checks

Changing the displayed patch/projection ends its observer before paint, even
when the resulting complete file text is equal. The previous borrowed request
drains before the next conditional read. Old hover results cannot paint on the
new patch. Within one mounted document this keeps the original buffer rather
than reopening it. The product's actual Session/path/source-or-diff/scope key
still ends a consumer when switching documents; unresolved cleanup remains in
the existing core Settings registry.

Patch/file disagreement and file/native disagreement are distinct refusals.
**Check file** explicitly re-observes the bounded complete file and
validates the still-displayed patch. It never replaces the displayed patch or
reloads the native buffer. **Check** observes/re-reads only the original
owner. Neither action starts a timer, replays Open or switches to a legacy API.

Diff chrome offers no native refresh/preparation/Apply. A native mismatch directs
the user to full source for separately reviewed synchronization. No diff read,
revalidation, projection replacement, unmount or cancellation writes to disk,
prepares synchronization or retires an effect. Owned cross-file navigation
remains unavailable until destination ownership exists.

Routine states occupy no status row. Refusals use the same single flat,
paint-only attention row as source Review; details remain in the row's title.
The existing in-place document refresh, explicit reading-time prompt and scroll
anchor are preserved. Accepting a changed patch invalidates its old projection.

## Acceptance boundary

`ownedDiffProjection.test.ts` and `ownedDiffSource.test.ts` cover Unicode/EOF,
all-line equality, bounds, malformed hunks, original pagination, cancellation
and core identity loss. `just review-diff-browser-conformance <absolute-firefox>`
mounts the actual hooks, status controls and CodeMirror under development
StrictMode, with WebCrypto and deferred fixture file/native HTTP. It checks
staged/partial refusal, full-file hashing, actual deleted/new-line clicks,
equal-source patch replacement, explicit mismatch checks and late context loss.
The original Review/source, owner/content, context, cleanup, synchronization and
Settings regressions remain required, together with the connected native gate.

The browser runner retains one served/hashed module, including CodeMirror's
language-loader dependencies; it does not expose a directory server. Module
initialization failures now report through the same bounded fixture result.
Synthetic HTTP is not a logged-in product or physical-device test. Actual
Machine/native rollout, independent restoration, abandoned-browser/restart
recovery and general graph/state exits remain in the
[completion ledger](plugin-refactor-completion.md).
