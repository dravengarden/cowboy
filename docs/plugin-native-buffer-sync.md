# Native buffer synchronization

**Private native primitive published and installed on Hawk; not an enabled
Service/Review effect.** The [accepted rollout](releases/zed-native-sync-2026-09-16.md)
includes separate Machine maintenance and exact signed Zed installation.
The owned Code APIs still refuse a content mismatch; they must not silently
repair it by calling the legacy reload API. Ordinary Review still uses its
legacy API. Installation acceptance is a prerequisite, not consumer cutover.

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

## Finite effect contract

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

## Private implementation

The approved implementation is a source-pinned, static Linux x86_64 private
server (`cowboy-zed-server 1.0.0`) paired with Zed Plugin/adapter `1.7.0`. Upstream's
`version` command still identifies `1.13.0`; the signed runtime dependency
version/digest identifies the patched distribution. The additive protobuf
extension uses envelope tags 1000/1001 and protocol 1; it does not alter or
silently replace upstream `ReloadBuffers`.

- Probe is effect-free. Prepare retains the original buffer entity, file
  object and exact native vector. Native-issued IDs are monotonic within a
  random 128-bit process instance; neither expiration nor retirement recycles
  one. Serialized IDs are not authorization.
- Apply admits once. The Store owns its task before responding Pending, so
  dropping an observer cannot cancel admission or make it replayable. Query
  uses only the original instance/ID. Retire cannot remove a Pending record.
  Only effect-free Prepared records expire (30 seconds); at most 256 records
  and one pending source load are retained.
- The final version/clean/read-write/file/worktree/shared-peer checks and
  mutation run in one native update turn. Read-only, dirty, edit/undo ABA,
  changed file objects, removed worktrees and additional native peers refuse.
- Source reads are limited to 4 MiB. UTF-8 BOM, CR/CRLF and invalid UTF-8 refuse
  instead of being silently normalized. Linux `openat2` refuses symlinks in
  every path component; nonblocking descriptor validation rejects FIFOs,
  directories and oversized sources, and a bounded read rejects growth.
  Kernels without this primitive fail closed; there is no weaker fallback.
- Applied reports the candidate content identity and resulting native vector.
  Native operation/reload events invalidate previous adapter observations.
  This is not a guarantee of an atomic LSP refresh or a filesystem transaction.
  Losing the process loses the journal; an unknown outcome is not restoration.

`plugins/zed/runtime/server.nix` owns the source revision, source/dependency
hashes and overlays. The bounded-open helper uses the already upstream-locked
`rustix 1.1.2`; the small manifest/lock patch does not upgrade its bytes. This is
a static dependency, not an ambient executable or shared library requirement.
The recipe includes the GPL source overlays and upstream license, builds the
headless server separately from desktop feature unification, runs native tests,
and rejects ELF interpreter/shared-library dependencies. It does not modify a
user's ordinary Zed installation, Cargo checkout or mutable configuration.

Validation has two distinct layers:

1. The Nix build runs native GPUI buffer-store tests with barriers before source
   loading and immediately before commit. They exercise edit/undo, clean-state
   and sharing/owner changes at both boundaries, observer loss, retained Pending
   state, expiration, capacity and no replay. Test barriers do not exist in the
   shipped binary. Native filesystem tests cover bounded growth, nonregular
   files and final/parent symlinks.
2. `just zed-native-sync-conformance <immutable-server>` uses the real server
   and private wire decoder in disposable state, with no network, host PID
   visibility, writable cgroup hierarchy, ordinary Zed settings or ambient
   language tools. It verifies actual mutation/content events, edit/undo/close
   refusal, invalid source bytes, a deliberately unobserved Apply reply,
   original-ID queries, duplicate/retired/unissued IDs and process restart.

The adapter additionally tests closed request/reply correlation, cancellation
cleanup, bounded waiters, mixed upstream/private replies, every split point and
bytewise framing. These tests are not proof of core authorization or an actual
Review consumer.

## Remaining delivery boundary

The `1.8.0` source candidate adds [private adapter ownership exclusion](plugin-buffer-sync-owners.md)
and a separate synchronization operation identity. It refuses shared native IDs,
retains exclusion through Pending/Unknown, and never resends Apply. Its private
socket purpose declaration is **not** a core grant. The Machine explicitly
rejects these commands on its generic Code route; no Controller/Web effect or
Review consumer is enabled. Core purpose/authority and original-generation
routing remain required. Read leases and content hashes remain insufficient,
and no fallback to legacy reload is permitted.

The exact static adapter/server pair passed the eight-group connected Code gate
and was signed, published and installed on Hawk through the normal Plugin
installer. Separately authorized Machine maintenance retained immediate worker
processes; one later native identity-preserving roll is recorded in the
[rollout evidence](releases/zed-native-sync-2026-09-16.md). Building or publishing
alone still does not install a Plugin or enable its effects. Actual Review must
pass independent-reader/authority-loss/HTTP-cancellation tests, reject stale
file/outline results and positions, own navigation destinations, and run its
real consumer/browser/device checks. None of these follows from the native
primitive's gate or its installation.

See the [completion ledger](plugin-refactor-completion.md). Conditional reload,
independent post-effect restoration and verified native generation recovery
remain separate from local resource disposal and structural graph validation.
