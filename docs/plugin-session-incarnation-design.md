# Durable Machine Session incarnation — design (not implemented)

Status: design for review. Nothing here is built, published or activated.
It is the contract the first dependent slices must satisfy, written before any
durable-identity code per the rule in the
[October 5 status](plugin-refactor-status-2026-10-05.md).

## What exists and what is missing

- The Controller's `SessionCodeScope` incarnation is an in-memory `Arc` that
  changes on cwd change or execution-binding acceptance and does not survive a
  Controller restart. It is Controller-owned.
- Machine-owned directory incarnations ([protocol 25](plugin-session-root-observations.md))
  are random per observed root object and end on Machine restart.
- The [deletion journal](plugin-session-deletion-journal.md) makes a terminal
  Session ID a permanent fence. It does not say which *life* of a live ID an
  observation, cache entry or restored operation belongs to.
- Nothing persists "this is the same Session lineage on this Machine" across a
  Machine restart, a reset, an adoption of a surviving worker, or a rollback.

The gap is that an old observation can be revived by restart, reuse, ABA or
rollback. A durable value must refuse that, which makes it safety-critical.

## Identity

A **Session incarnation** is a 128-bit random opaque value minted by the Machine
that owns the Session slot. It is not derived from the Session ID, a path, time
or a counter, and it is never constructed by the Controller (no constructor, no
serde path on the Controller type). Equality is the only operation. It names
one lineage of one `(machine_id, service_id, session_id)` slot; it is not a
filesystem identity, grant or lease, and a serialized copy authorizes nothing.

Per slot the Machine also keeps an `epoch: u64` that increases on each rotation,
so a log or receipt can order lineages. The value, not the epoch, is the
comparison key.

## Lifecycle rules

1. **Mint** when the Machine first accepts a launch declaration for an ID it has
   no record for, before any worker is started or adopted. Adopting a surviving
   worker with no record also mints; continuity before the first Machine
   observation is not claimed and the record says `origin: adopted`.
2. **Keep** across Machine restart, worker replacement within the same lineage
   and Controller restart.
3. **Rotate** at an explicit context reset and at an accepted execution-binding
   or workspace change. The new value is committed before the reset's first
   effect (old-worker stop, cwd change). If the commit is unconfirmed, the reset
   is refused and the old lineage stays current.
4. **End** at terminal deletion. The deletion journal remains the only
   permanent fence and wins any disagreement; the incarnation record is removed
   after the journal commit. A crash between them leaves a live record whose ID
   is journal-deleted, which every reader treats as terminal.
5. **Never reuse.** A new lineage always has a new random value. A restored or
   restarted process that cannot read the dataset must not synthesize one.

## Dataset

`state_dir/session-incarnations/incarnations.json`, closed schema 1, beside a
`.lock`. Fields: owner `{machine_id, service_id}`, then entries
`{session_id, incarnation, epoch, origin}` with `origin` one of `minted`,
`adopted`, `reset`, `rebound`. Same parsing discipline as the deletion journal:
unknown fields, duplicates, another owner, another schema, nonregular files and
oversized input refuse; refusal never rewrites. Bounds: 4,096 entries and
2 MiB; exhaustion refuses new launches and never evicts. Writes are staged, file
synced, atomically renamed and directory synced under one exclusive lock with
retained handles; replacement of the directory or lock ends admission.

A write failure fences further writes in that owner and the affected command is
refused with a negative acknowledgement; surviving workers are untouched. A
rename followed by a failed sync is an unconfirmed outcome, never a rollback.

## Admission and compatibility

Unlike the [cleanup continuation](plugin-session-cleanup-continuation.md), this
dataset cannot be advisory: a Machine that ignores it stops refusing stale
observations, and a later Machine could accept an incarnation that an
intervening writer had rotated. It therefore follows the deletion journal's
reader-first discipline:

- Release metadata declares `sessionIncarnations {readerSchema, writerSchema}`.
  The installed owner decodes a release's `source.json` leniently, so an owner
  that does not know the declaration silently ignores it instead of refusing the
  release. The owner must therefore be activated on a host first; sequencing, not
  a decoding error, is the protection.
- A root-owned, per-dataset reader floor binds Machine, dataset path and exact
  reader anchor. Once it exists, an undeclared Machine refuses. A writer
  declaration requires an existing floor and a writer-capable build that also
  passes startup admission; the first release is reader-only.
- Committed state (even corrupt or dangling) requires a declared reader.
- Explicit recovery, rollback and fallback select only declared compatible
  readers of the same worker generation, exactly as for deletions.
- Portable launchers keep refusing committed state until a separate portable
  floor exists; this design does not admit them.

The owner currently binds one floor to one dataset path. Generalizing it to a
dataset key must leave the deletion dataset's bytes, floor and behaviour
unchanged, and applies to every host with an installed owner (Hawk and Falcon),
each needing its own host activation and acceptance.

## Carriage to the Controller

A new Machine protocol field (above 25) returns the incarnation with launch
acknowledgements and Session snapshots. The Controller records it as part of
`SessionCodeScope` and every read route, cache and continuation key; a Machine
that does not report one leaves the scope on its existing process-local fence
and is labelled as such, never as durably fenced. Any later response carrying a
different value for the same slot retires the observation (HTTP 410 for buffered
reads), never updates it in place. Controller restart re-learns the value from
the Machine; it cannot invent one.

## Interaction with other exits

- Cleanup continuation may later nominate the incarnation as well as the root;
  until then the nomination remains root-only.
- State leases attach here: a lease names `(slot, incarnation)`; an incarnation
  mismatch ends the lease. The lease design is separate and later.
- Local disposal, reload and read-only composition do not mint, rotate or end an
  incarnation.

## Required evidence before any step is accepted

Old/new reader and writer matrices on exact immutable releases (as the 32-group
deletion conformance): ACK/dedup, SIGKILL at staging, file sync, rename and
directory sync, reopen by old and new readers, exclusive ownership, storage and
lock failure, floor and declaration refusal vectors, reader-only fallback, and
rollback refusal once a record exists. Plus: mint-before-launch ordering,
rotation-before-effect and refused reset on unconfirmed commit, deletion winning
over a stale record, adoption of an unrecorded surviving worker, Controller
restart re-learning, and stale-observation refusal across restart, reset and
reuse. Real production evidence needs a live Session lineage observed across a
Machine activation; fixtures and an unrecorded surviving worker do not stand in
for it.

## Order of work

1. Owner dataset-key generalization, reader-only, on Hawk and Falcon. Hawk is
   active (Columbus `5af9f70b`); Falcon still needs its own host activation and
   must have it before any release declaring the dataset reaches Falcon.
2. Machine reader and declaration with the writer disabled.
3. Writer admission, mint/rotate/end rules and negative acknowledgements.
4. Protocol carriage, Controller scope and refusal vectors, then Web diagnostics.

Each step is its own release and acceptance; none belongs in an ordinary
Controller, Web or resident fix.
