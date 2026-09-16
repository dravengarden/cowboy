# Observations borrowed from original buffer owners

The additive Controller API now connects **diagnostics/inlays/semantic tokens
and document symbols** to an already-open
[original buffer owner](plugin-controller-buffer-owners.md). The private Zed
candidate is `1.4.0`; its pinned upstream server remains `1.13.0`. This does not
switch Review, install a Plugin, activate a Machine generation or complete the
Plugin refactor. The [Controller release is active](releases/plugin-owned-buffer-reads-2026-09-16.md);
that receipt separately records the uninstalled native candidate and remaining
end-to-end acceptance.

## Closed protocol and coordinates

`POST /api/code/buffers/{id}/read` accepts exactly one of:

```json
{ "kind": "language" }
```

```json
{ "kind": "symbols" }
```

There is no path, Session ID, Machine ID, native reference, arbitrary operation
or position in this body. The 128-byte input bound and closed tagged Rust union
apply before resource lookup. Success is HTTP 200 with `Cache-Control: no-store`:
`apiVersion: 1`, `resourceId`, `openedVersion` and `result`. Result is a closed
`language` or `symbols` union matching the requested operation. Handler errors
are generic and no-store; native paths, references and error details are not
returned as browser execution authority.

`openedVersion` is the vector captured when the native buffer was opened, used
as a **lower bound** by Zed's language queries. It is not a current content
snapshot, an edit revision, a writer fence or a position certificate. Diagnostic
ranges and symbol ranges are observations from the retained native buffer; this
slice does not claim that several language servers observed one atomic version.

Zed `1.13.0` acknowledges the diagnostic refresh trigger but sends diagnostics
as buffer operations, not `LspQueryResponse`. The adapter now consumes those
original-buffer events instead of timing out waiting for a nonexistent reply.
`result.diagnosticsState` is `unobserved` until a diagnostic update is actually
received, or `observed` after one (including an explicitly empty update). This
is the last observation, not proof that an asynchronous refresh completed.
Per-server Lamport stamps reject older diagnostic updates. Buffer update
requests are acknowledged after local observation/invalidation so Zed can send
subsequent chunks; this is not an acknowledgement of a complete diagnostic pull.

Anchor conversion uses only a bounded native base-text snapshot. An observed
edit, undo or reload announcement invalidates it; language reads then fail closed
instead of converting against current disk text. Foreign, unsupported-revision
and split-UTF-8 anchors also fail. Symbol queries use native UTF-16 results and
do not require this base-anchor conversion. Full edited-buffer coordinate support is still
unfinished; no automatic close/reopen or lease replacement is performed.

Hover/navigation are deliberately rejected by this new protocol. The old
implementation converts UTF-16 coordinates using current filesystem text and
then builds an anchor in Zed's original base insertion. Simply attaching a
buffer handle or comparing the old vector would not make those coordinates
correct after edits. A later positional reader needs actual native content /
anchor ownership and version semantics, including navigation destinations.
The legacy Review request/response shapes remain unchanged. Their language
implementation gains the same native diagnostic cache, conservative anchor
checks and transport-error propagation; only the new owned response exposes
the explicit `diagnosticsState` observation label.

## Three retained lifetimes

- **Controller:** admit a bounded read borrow only for the original user's
  confirmed-open, idle resource, with no release attempt. Recheck the captured
  credential and role, original Session incarnation/owner and exact Machine
  connection before the support probe, before dispatch and at the HTTP response
  boundary. Deletion/recreation, cwd ABA, reconnect, revocation or role loss
  discards results. Authentication remains core-owned.
- **Machine:** answer the separate pathless `bufferLeaseReadSupport` API-1 probe
  in core, without starting/selecting a Plugin. Retain the original native
  process and worktree route through I/O; never consult the current installed
  slot or filesystem path. An old host or adapter fails closed, without health
  fallback. Read results cannot retire a route or change effect evidence.
- **Native:** require the original reference to be `open` and the exact typed
  owned ID to remain in the active buffer's owner set. Retain the registry and
  buffer read locks through the native query, including against legacy close.
  Do not canonicalize, reopen, select a new worktree or read source file text.
  A legacy string with the same spelling cannot substitute for an owned ID.

The Controller task survives cancellation of its HTTP observer, holds one of
the existing 64 job permits and has a 60-second deadline including authorization
and reply delivery checks. Query/release while busy only observes pending state;
a pending DELETE is **not queued**. The caller must explicitly request release
after the borrow drains. Error, cancellation and deadline drop the borrow without
changing the last native effect observation or replaying an open/close. Read
failure is not proof of release. Explicit original-user cleanup remains usable
after Session deletion under the existing resource API.

## Validation and bounds

Controller and Machine share one closed core codec. The independently built
Plugin owns its private codec, checked through the actual adapter/server
conformance gate. Core requires exact reply kind, API, owner and operation;
unknown nested fields are rejected. Limits are 2 MiB serialized reply, 256 sorted
unique version entries, 1,000 diagnostics, 2,000 inlays, 50,000 semantic-token
words in complete groups of five, 2,000 total symbols with depth at most 16,
and 64 KiB per text value. Ranges and inlay offsets are validated. Limits reject
bad observations; they never claim a successful empty result or successful
cleanup. The existing transport/frame bounds remain independent.
Native snapshots are capped at 1,024 buffers, 4 MiB text per buffer and 32 MiB
total text; diagnostic strings at 1 MiB per buffer and 8 MiB total, with at most
32 servers per buffer. Unused diagnostic protocol fields are not retained.
Native close removes the snapshot; limits or invalid coordinates never evict
or release the underlying buffer owner.

Source tests cover nonempty codec vectors, wrong owners/kinds, unknown fields,
size/depth bounds, old-host refusal, Session ABA, reconnect, cancellation, busy
release, retained runtime generations, file disappearance and native transport
failure. Temporary SQLite token/role tests exercise revocation after both remote
boundaries. The real signed-install/uninstall Zed conformance test additionally
reads both kinds after uninstall, file deletion and worktree rename, then drains
the retained native generation. Fixture acceptance is not a production login.

Before Web cutover, still accept the actual Machine/Code generation, implement
client pending/unknown/release handling, resolve positional read semantics and
verify supported-device behavior. Abandoned-browser cleanup, Controller restart
restoration and independently authorized post-effect recovery remain separate.
No new durable state, migration, public SDK capability or generic DAG executor
is introduced here.
