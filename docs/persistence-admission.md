# Process-local persistence admission

This is Controller core infrastructure, not a Plugin or a new durable journal.
The database schema, reducer format, Plugin installation/telemetry policies and
Machine protocol are unchanged. The limits and lifecycle are specified in
[Storage](architecture/05-storage.md#write-behind).

## Incident and cause

On 2026-09-16 at 13:45:36 UTC, the active Controller rejected two append intents
estimated at 437 and 132 bytes while approximately 15.6 MB was pending. Its old
8 MiB rule allowed one oversized event only when empty, then charged all later
events against that same exhausted budget. The queue drained and database batch
failures stayed zero, but sticky degradation correctly kept health at HTTP 503.
See the [bounded original observation](releases/zed-sync-owners-2026-09-16.md#later-persistence-degradation--unresolved).

The former load/check/increment admission also raced concurrent producers.
When the count limit was full, metadata writes spawned detached waiting tasks;
those were not bounded by the channel and could enqueue after a later intent.
Closing the receiver could discard already-reported accepted metadata. Pending
message attachments and several metadata variants were substantially undercounted.

## Repair and invariants

`src/core/persistence_queue.rs` owns atomic admission and FIFO order with one
receiver. A retained entry records its exact estimated charge; dequeue refunds
that charge once. Ordinary events and one oversized event have independent byte
budgets. Lifecycle and metadata have small finite reservations, without changing
FIFO execution order. New control writes cannot bypass an earlier queued clear,
setting or title. Capacity reservation never spawns work or waits on the DB.

The final sender closes admission. Receiver close stops new writes but retains
accepted entries; cancellation of an empty receive does not lose wakeups.
Receiver destruction accounts for each undrained intent as a drop. The writer
handles an initially set shutdown flag and a lost shutdown sender as drain
requests, without a permanently ready watch channel starving receive. Its
batch gathering also has a byte threshold instead of only an intent count.

The two historical rejected events cannot be identified or reconstructed from
the old counters/log message. No replay, DB edit or counter reset is a repair for
that loss. A new Controller's healthy epoch does not restore them. Hard crash,
writer timeout, repeated DB failure and genuine bounded-capacity overload remain
explicit failure cases; this change is not durable spooling or synchronous
commit acknowledgement.

## Regression evidence

Queue tests reproduce a 16 MiB event followed by small and independent-session
lifecycle events; concurrent normal/oversized admission, finite reservations,
FIFO, refunds, receiver cancellation, close/drop, and attachment/settings sizing
are covered. Real SQLite and isolated PostgreSQL tests drive the actual writer,
reopen through `Store`, and verify exact event content/sequence and latest
title/settings. Additional tests cover clear/new-event order and shutdown drain.
PostgreSQL fixtures run only under the owned `just test-postgres` temporary
cluster, never against the production database.
