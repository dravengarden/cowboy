# Session incarnation writer (schema 1)

Active on Hawk since 2026-10-06 ([release](releases/incarnation-writer-2026-10-06.md)).

Third step of the [durable incarnation design](plugin-session-incarnation-design.md),
on top of the [reader](plugin-session-incarnation-reader.md). No Controller
consumes the value yet, so nothing observable depends on it.

## Behaviour

The Machine owns one lineage per Session slot: 32 lowercase hex digits from 128
random bits, an epoch and an origin. It is never derived from the ID, a path or
time, never constructed by the Controller, and authorizes nothing.

- **Mint.** When a declaration (`EnsureSession`) reaches the Machine for an ID
  with no record, the lineage is committed *before* the declaration is registered,
  a worker is launched or an existing worker is adopted. Origin is `adopted` if a
  worker is already attached for that ID (continuity from before the first Machine
  observation is not claimed), otherwise `minted`. An existing lineage is returned
  unchanged and **nothing is written**: reconnect replays, adopt-only declarations,
  ordinary revival and waking a hibernated session all keep the same lineage.
- **Rotate.** An explicit context reset commits a new random value with a higher
  epoch (`reset`) after the deletion-journal check and before the reset's first
  effect (cache revocation, fencing, stopping the old worker). If the commit cannot
  be confirmed the reset is refused with a negative acknowledgement and the old
  lineage stays current, nothing stopped or fenced.
- **End.** After the terminal deletion is committed in the deletion journal the
  record is removed. The journal is the only permanent fence: a record that cannot
  be removed is logged and left, since every reader treats a journal-deleted ID as
  terminal. A deleted ID is refused before any lineage can be created.
- **Not written.** `rebound` (workspace or execution-binding change) is valid on
  disk but nothing emits it yet; hibernation neither rotates nor ends a lineage.

A write that cannot be confirmed (staged file, sync, atomic rename, directory
sync, with ownership rechecked either side) is an unconfirmed outcome, never a
rollback: the writer is fenced, the launch or reset that needed it is refused with
`durable Session incarnation was not confirmed`, and existing workers are
untouched. A launch refused for this reason is a deliberate availability cost of a
safety dataset; builds without an admitted writer never refuse for it.

## Admission

Default and reader-only builds never write and never refuse a launch for the
dataset. The dedicated writer build compiles
`COWBOY_SESSION_INCARNATION_WRITER_BUILD` (runtime environment cannot set it) and
declares `sessionIncarnations {readerSchema: 1, writerSchema: 1}`. Its startup
admission requires, in this order and **before any namespace is opened**: the
deletion writer admitted; the fixed root profile selecting this exact native
executable; a schema-1 release source declaring both writers; and a root-owned
`session-incarnation-reader-floor.json` binding the Machine and the exact
`session-incarnations` path, with trusted ownership and permissions. Any failure
refuses startup. The installed Columbus owner independently refuses a writer
declaration without that floor and, once a record exists, any artifact without a
declared reader, including rollback and recovery.

## Evidence

Store tests cover mint-once, higher-epoch rotation, ending, reader-only refusal
with existing lineages still readable, commit-stage ordering (nothing published
before the file is synced), storage-failure fencing, budget exhaustion without
eviction, and a SIGKILL matrix over mint, rotate and end at all four commit stages
in real child processes (unpublished changes never replayed, published ones
valid). Broker tests drive the real declaration, reset and delete paths: mint
before declaring, replays keep the lineage, reader-only builds neither write nor
refuse, an unconfirmed lineage refuses a launch or reset with no effect, a
confirmed rotation changes the lineage, hibernation keeps it, and deletion ends it
only after the journal commit. Changing the commit order or removing any of the
three wiring points fails these tests.

The native production conformance additionally runs the writer against exact
immutable releases (mint on a real declaration, replay, reopen by new, reader-only
and previous writers, delete ending the lineage, storage-failure refusal, and six
incarnation-floor refusal vectors that leave no namespace behind). See the
[release record](releases/incarnation-writer-2026-10-06.md).

## Limits

No Controller reads the lineage: no stale-observation refusal, cache or
continuation key, 410 or diagnostics exists yet, so this does not by itself fence
anything. Lineages are not state leases, not filesystem identities and do not
survive loss of the dataset. Historical Sessions get a lineage only when first
declared after activation, as `adopted` or `minted`. Creation is not atomic with
the worker launch that follows it, and the deletion commit and lineage removal are
two commits. Not a power-loss proof; Falcon's owner has not learned the dataset.
