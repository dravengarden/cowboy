# Private native text-snapshot lifetimes

Zed Plugin/private adapter `1.19.0` selects private server `1.5.0`, retaining
the exact upstream revision, third-party dependencies and public contracts.
This extends the [acquisition lifetime budget](plugin-native-acquisition-budgets.md).
The [acceptance and publication](releases/native-snapshot-lifetimes-2026-09-19.md)
records 48 native tests, the final pair, temporary signed lifecycle, 19 connected
v6 checks, 24 browser checks and the actual complete-Catalog reader floor.
The signed release is available; Hawk installation remains separately pending.

## One charge, all original holders

The existing pool admits at most **64 acquisition lineages** per native GPUI
application. Previously, the resulting language Buffer retained the charge,
but a detached text or language snapshot could outlive that entity and keep
its text and CRDT trees after capacity had been returned. Repeatedly closing
and reopening a file could therefore admit new acquisitions while snapshots
of old acquisitions remained alive.

A closed, nonserialized native `Permit` now lives below the language layer.
The application still owns the pool. The original language Buffer attaches
clones of its charge to its native text Buffer and text BufferSnapshot before
it becomes observable. Ordinary snapshot cloning, text branches, edited
previews and background preview results retain the same charge. No path,
buffer ID, request ID or client message constructs or relinquishes a permit.

Clones do not consume additional slots. The last actual holder returns the
original slot, including when it is dropped on a background thread. The
permit is the last field of both native text owners, so their retained text,
trees and history are dropped before that owner's permit. A completed but
unobserved background result is still a holder; losing its observer is not
proof that the work or its result has already drained.

BufferStore indexes still disappear when their matching language entity
dies. Index cleanup and acquisition release are intentionally different
lifetimes. A later open at the same path obtains an independent charge; it
does not adopt an earlier snapshot's owner. At capacity, a new acquisition
fails before filesystem access. Dropping just one clone cannot make room.

## Explicit limits

This fixes premature release of the existing count budget, not a global
memory budget. Arbitrarily many snapshots or historical versions derived
from one acquisition still share one slot. Detached Rope/string copies,
syntax-only data, serialization, unrelated constructors, workspace scans,
all edit writers, CPU and total process RSS are not covered by this charge.
The separate input and sync/reload replacement limits remain unchanged.
There is no history pruning, owner eviction, automatic replay or migration
of an existing native generation.

## Required evidence

Five additional native GPUI test groups cover raw/language snapshots and
clones, last-clone cross-thread release, saturation with detached snapshots
across stores, completed/unobserved and cancelled preview tasks, and native
text branches/edited previews. Existing acquisition, replacement, filesystem
and LSP groups must continue passing against the exact patched upstream.

Acceptance also requires the complete source gate, the final immutable static
pair, a temporary signed Plugin lifecycle, all 19 connected v6 checks and
24 browser regressions. Publication requires exact signed artifacts and the
complete Catalog to pass the actual active, next-recovery and cold-reader
floor. Record concrete receipts separately. Installing the new Plugin is a
distinct operation; old native owners, general post-effect recovery and
supported-device acceptance cannot be inferred from fixture teardown.
