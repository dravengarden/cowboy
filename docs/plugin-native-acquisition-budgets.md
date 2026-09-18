# Private native acquisition lifetime budgets

Zed Plugin/private adapter `1.16.0` selects private server `1.3.0`, retaining
the same upstream revision, dependencies and public contracts. This is a source
candidate, not a published release, Machine activation or production navigation
cutover. It extends the single-input and whole-query bounds with a shared native
acquisition limit. It does not complete the global history/background budget.

## Admission and lifetime

One native GPUI application admits at most **64** combined in-flight and live
file/untitled buffer acquisitions. The headless server has one such application;
all its local BufferStores and worktrees share the same pool. An already-open
buffer or same-path pending load reuses its original acquisition at capacity.
A new local path reserves before filesystem access, allocation of its loading
task, or insertion into `loading_buffers`. Untitled creation reserves before
spawning its task. Exhaustion returns a native `CapacityExceeded` error, without
queueing, evicting existing buffers, repeating an earlier attempt or pruning
Unknown owners.

The charge is a nonserialized RAII value, not a path, RPC ID, timeout or close
receipt. Clones share one charge. The actual worktree loader and its completed
text result retain it; the background CRDT constructor and its completed result
also retain it. The resulting language Buffer owns it through destruction.
Failure and cancellation return capacity only when the last relevant holder
actually drops. A dropped result observer, removed worktree/store, completed
Close ACK or another peer's release cannot relinquish a still-live buffer's
charge. No client can mint or release these values through the protocol.

Buffer destruction now removes the matching weak opened-buffer entry, path
mapping, local entry-ID mappings and non-searchable marker. Matching includes
the GPUI entity identity, so an old entity's delayed release cannot delete a
replacement's indexes. This closes normal open/release metadata accumulation;
it does not garbage-collect unresolved operations or an effect journal.

The public/adapter ownership rules are unchanged. A native request error after
dispatch still leaves the original adapter Unknown fence intact. This internal
admission error is not a new wire-level proof that every part of the enclosing
operation had no effect. Reads cannot repair it by loading a path, and no
automatic retry or independently authorized recovery is added.

## Scope and verification

The cap covers the actual local file and untitled acquisition routes used by
the private headless server, including navigation targets. It is **not** a cap
on every ephemeral language/text Buffer constructor, desktop-only in-memory
construction, remote replicas, edit/reload history, shared text snapshots,
parsing/LSP jobs, worktree scanning, process RSS or total bytes. Existing raw
and decoded input limits remain 4 MiB per file. A count cap cannot by itself
bound retained CRDT history or the lifetime of snapshots held after an entity
is destroyed. These need separate pre-effect/resource-lifetime designs.

Required native tests cover shared cross-store capacity, reuse at saturation,
pending-path deduplication and lost observers, untitled cancellation, failed
loads, loaded-result retention, store teardown, cross-thread shared charges,
repeated index cleanup, replacement ABA and Close ACK with other holders.
The immutable process fixture independently saturates two real worktrees,
requires actual overflow errors rather than timeouts, verifies preserved
original mirrors and a separate acquisition after the fixture releases its
native handles. That last fixture observation is not general background drain.

Acceptance additionally requires the exact final static pair, signed temporary
installation lifecycle, all 18 connected v5 checks, 24 browser regressions and
the complete source/Nix gates. Record actual identities and results separately;
building or fixture teardown alone grants no production acceptance or recovery.
