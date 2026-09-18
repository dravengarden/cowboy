# Private native reload input bounds

Zed Plugin/private adapter `1.15.1` selects private server `1.2.1`, with unchanged
upstream source and third-party dependencies. This candidate extends the
[single-input limits](plugin-native-input-bounds.md) to an existing native
reload path. It adds no operation, automatic reload, retry, recovery authority
or production navigation admission.

The [accepted candidate](releases/native-reload-bounds-candidate-2026-09-18.md)
records exact source/artifacts, the full source gate, 24 native tests, final pair,
temporary signed lifecycle, 18 connected v5 checks and 24 browser regressions.

## Gap and boundary

Initial open and navigation-target acquisition already use a bounded descriptor
read. Upstream `language::Buffer::reload_impl` instead calls
`worktree::File::load_bytes`, which used an unbounded filesystem read. A file
that was small when opened could become arbitrarily large before reload. Even
bounded raw input can expand beyond the text budget when decoded.

The actual private `LocalFile::load_bytes` now uses the same 4 MiB descriptor
limit plus one overflow sentinel, on the background pool. Linux `openat2`
refuses every symlink component, nonregular files and unsupported kernels
without a weaker fallback. Reload independently validates the raw size before
decoding, then the decoded UTF-8 size before diff computation or CRDT mutation.
Both limits are inclusive. Upstream automatic encoding detection, explicit
forced encodings and binary rejection are otherwise unchanged.

Reload completion now clears its local task on every exit, including failed
reads, binary/size rejection and a missing local file. Previously these early
returns could leave `reload_task` present after completion and permanently
refuse conditional synchronization. A replacement cancels its predecessor's
owned task; cleanup synchronously compares the original, nonserialized invocation
identity so it cannot remove a replacement task.
An abandoned result observer is not cancellation of that native task.

Rejected input leaves original text, native version and source bytes unchanged;
it does not produce a successful reload transaction or a partial result.
Clearing a completed task neither retries nor clears adapter/core Unknown,
releases an owner, changes authorization, or restores a lost process.

In this pinned headless server, file-watch changes emit `ReloadNeeded` but do
not automatically invoke reload. The fix protects the existing reload route,
including other callers, without introducing a watcher-to-mutation bridge.
Owned reads still cannot call legacy reload to repair a content mismatch.

## Verification

The native build includes five GPUI regression groups using the actual
Worktree/LocalFile and Buffer reload code: post-open raw growth, UTF-16 and
forced single-byte expansion, binary input, exact 4 MiB acceptance, absent
files, untitled buffers, replacement, reentrant observers and lost observers.
Failure preserves the original version/text and permits a separately initiated
later operation.

The isolated immutable-native gate sends actual reload RPCs for oversized,
expanded, binary and symlink-replaced sources, observes actual refusal and
unchanged native mirrors/vectors/files, and tests completed-task cleanup through
the native conditional-sync preparation check without sending Apply. Only the
test fixture explicitly invokes legacy reload; no product read gains fallback.
Final pair, signed temporary lifecycle, connected v5 and browser regression
gates passed for the accepted candidate; their evidence is separate from
production activation.

This is a per-input bound, not a limit on total retained buffer/history,
background concurrency, decoding scratch, process RSS or worktree scanning.
No independently authorized recovery, actual deployed generation replacement
or supported-device acceptance follows. See the
[completion ledger](plugin-refactor-completion.md).

The subsequent [acquisition lifetime candidate](plugin-native-acquisition-budgets.md)
bounds the number of initial loads and live acquired buffers across worktrees,
without claiming to bound reload history or all background work.
