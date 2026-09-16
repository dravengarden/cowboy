# Native buffer synchronization: decision required

**Investigation and proposed boundary, not an implemented effect.** The owned
Code APIs refuse a content mismatch; they must not silently repair it by calling
the legacy reload API. Ordinary Review still uses its legacy API. The connected
acceptance gate now covers actual Code installation, but does not close this
gap.

## Observed upstream limitation

The private adapter pins Zed revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`. At that revision:

- [`ReloadBuffers`](https://github.com/zed-industries/zed/blob/aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45/crates/proto/proto/buffer.proto#L76-L83)
  carries project and buffer IDs, not an expected version, content identity or
  dirty-state condition.
- `crates/project/src/buffer_store.rs::handle_reload_buffers` resolves those
  buffers and starts reloading them without an atomic caller-supplied condition.
- `crates/language/src/buffer.rs::reload_impl` awaits the file load before
  computing a diff against the then-current buffer. Its later base-version check
  does not reject an edit made between the caller's observation and that diff.
- `crates/project/src/lsp_store.rs` accepts language-server workspace edits. A
  read-only Cowboy editor and one adapter connection do not prove there are no
  other native writers.

The current upstream `main` protocol was also inspected on 2026-09-16; it still
has the ID-only reload request. That observation is not a pin or a guarantee
about a future upstream release. The actual pinned static headless-server test
also shows disk/native mismatch without automatic reload. Neither source
inspection nor that test accepts a destructive reload on a production buffer.

An adapter mutex, `is_dirty` preflight or content check before an ordinary
`ReloadBuffers` request cannot close the native race. Reopening a path can adopt
the same shared native buffer and is not a synchronization or ownership proof.
Ending a read observer does not authorize either operation.

## Proposed finite effect

Communication, authorization and installation stay core-owned. A Code Plugin may
provide the native conditional primitive through its existing exact signed
runtime; this does not introduce a generic DAG executor or an installable core.

The native primitive needs all of these properties before Review can use it:

1. Operate on the original retained native owner and qualified buffer version,
   never resolve a replacement runtime or silently reopen a path. The open
   vector is only a lower bound, not a synchronization precondition.
2. Require separate synchronization authority and explicit shared-buffer rules.
   A read lease, a matching hash or sole Web consumer is not a writer grant.
   Until that authority exists, a dirty or non-exclusively governed native
   buffer is refused without changing another consumer's resource.
3. Capture the expected version before asynchronous source loading. In the
   native buffer's mutation context, recheck that exact version, clean state,
   owner and authority immediately before applying; no await may separate the
   final condition from mutation. Edit/undo ABA and authority loss must fail.
4. Bound the candidate text and preserve exact UTF-8/UTF-16 semantics. Report
   the actual applied content identity and resulting qualified version. A disk
   read is an observation, not a transaction spanning arbitrary filesystem
   writers; never claim the file stayed unchanged after that observation.
5. Use one admitted operation identity. Cancellation detaches the observer; an
   ambiguous native response becomes retained/unknown, not permission to reload,
   retry with a fresh identity or invoke the legacy path. Observation and
   independently authorized recovery remain distinct operations.
6. Invalidate borrowed observations after a successful change. Other owners
   retain their own release authority; text equality cannot revive stale
   position/navigation claims or certify an atomic LSP refresh.

Native change requires a new private protocol capability, adapter/server pair,
Plugin version and immutable runtime bindings. Unsupported installed pairs must
refuse the capability before any mutation. This cannot be advertised by updating
only the browser or Controller probe.

## Delivery choice

Cowboy currently packages the pinned upstream prebuilt server. A private native
patch means owning its reproducible build, static release matrix, patch rebases
and exact server/adapter compatibility; it must not modify a user's ordinary Zed
installation or mutable state. The alternative is to wait for an accepted
upstream conditional primitive, keeping synchronization unavailable and content
mismatch explicit. The implementation decision and separate Hawk Machine/Code
maintenance are pending; neither is implied by the Controller cleanup release.

Acceptance must cover real edits during loading and before commit, dirty and
shared buffers, edit/undo ABA, independent readers, loss of authority, dropped
HTTP/native replies, crash/unknown outcomes and exact original-ID observation.
Actual Review must additionally reject stale file/outline results and positions,
own navigation destinations, and run its real consumer/browser/device checks.
The current eight-group connected gate is a prerequisite, not that acceptance.

See the [completion ledger](plugin-refactor-completion.md). Conditional reload,
independent post-effect restoration and verified native generation recovery
remain separate from local resource disposal and structural graph validation.
