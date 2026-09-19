# Private native remote-edit admission

Zed Plugin/private adapter `1.20.0` selects private server `1.6.0` with the same
exact upstream and third-party dependencies. Its
[acceptance and signed publication](releases/native-remote-edit-budgets-2026-09-19.md)
are complete; live Hawk installation remains separate. This is not a new public
Code writer. It extends the
[sync/reload history bounds](plugin-native-replacement-budgets.md) to the private
local server's incoming `UpdateBuffer` edit/undo route.

## Target, input and history

A local native server requires an already-open buffer, the exact sharing peer
and the remote-server project. Unknown or expired IDs cannot allocate an
`OpenBuffer::Operations` waiting entry. The separate upstream remote-replica
bootstrap route is unchanged; its waiting queues are not covered here.

Before upstream decoding can narrow replica IDs or allocate dense clocks, the
whole request is checked: at most 128 edit/undo operations, 4 MiB aggregate
inserted text, 16,384 aggregate parts, and 256 replica slots. Versions are
canonical and ordered, ranges are ordered and bounded, inserted text is LF,
and undo target IDs are unique with bounded counts. Other update variants
refuse explicitly; the private Cowboy adapter does not send them. This is a
bound after protobuf transport decoding, not a bound on that preceding decoder.

The native buffer must be writable and have complete retained history. Existing
and prospective operations share the replacement limits: 8 MiB base/inserted
history, 4,096 operations, 16,384 parts and 4 MiB visible text. Exact duplicate
operations are observations and consume no additional history; changed content
under an existing operation identity rejects the entire batch. Missing causal
predecessors, omitted author history and unknown undo targets cannot enter a
deferred queue.

## Validate the whole batch before publishing

Preflight uses the exact native text engine on one detached branch, in the same
GPUI mutation turn. It retains the original acquisition lineage and has no
filesystem, language-server, subscription or asynchronous effect. Native
FullOffsets include deleted text; a bounded ordered traversal checks UTF-8
boundaries in both live and deleted ropes against each operation's causal
version, without reconstructing a second full-text string.

Only after every operation succeeds does the original language buffer publish
the new operations together. Refusal preserves original content, version, saved
state, undo history and pending ownership. It cannot publish a valid prefix,
clear an uncertain owner, acknowledge a partial edit, retry or reopen a path.
The ordinary request error is not a new cross-site no-effect/recovery grant.

## Evidence and limits

Native GPUI tests cover exact duplicates, conflicting identities, batch/clock/
shape limits, missing history, Unicode tombstones and concurrent edits,
inclusive retained-byte/operation limits, undo, visible-text and read-only
refusal, and unknown-ID queue admission. The accepted final static-pair gate also
exercises real RPC failures, exact duplicates, tombstone offsets and editing
after peer closure, with unchanged disk bytes. Since the native server does not
echo a sender's own remote edits, independent exact-version plaintext queries
verify the native result and stale-vector refusal; the direct GPUI tests verify
complete content and retained history. An unchanged local mirror alone is not
native no-mutation evidence. Complete source, temporary signed installation
lifecycle, 57 native tests, two 19-check connected v6 runs and 24 browser checks
pass; the dated receipt retains exact supplied-role boundaries and an unrelated
production continuity failure rather than claiming a live installation.

This bounds one more writer, not every upstream mutation or total RSS. LSP
workspace edits, desktop-only writers, remote-replica bootstrap, snapshot counts,
global retained bytes, general background tasks and independently authorized
post-effect recovery remain separate. No production private navigation policy,
Machine generation, existing native owner or supported-device claim changes.
