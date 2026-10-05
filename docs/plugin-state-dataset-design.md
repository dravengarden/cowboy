# General state-dataset compatibility — design (not implemented)

Status: design for review; nothing here is built or activated. It addresses the
P3 exit in the [completion ledger](plugin-refactor-completion.md): general
state-dataset identity, actual old/new reader and writer coexistence, exclusive
fenced ownership, principal change, crash/reopen, version change and independent
workspace/generation coexistence. It reuses what the terminal-deletion journal,
the [incarnation reader](plugin-session-incarnation-reader.md) and the
[cleanup continuation](plugin-session-cleanup-continuation.md) already proved,
and adds no second Plugin lifecycle or generic workflow executor.

## What exists

Durable state is not one thing today. Four mechanisms coexist:

| Kind | Examples | Identity and compatibility today |
| --- | --- | --- |
| Machine file namespaces with an owner floor | terminal deletion journal; incarnation namespace (reader only) | Closed schema-1 record bound to Machine and Service, exclusive lock, release declaration (`readerSchema`/`writerSchema`), root-owned per-dataset floor written by the installed owner, writer admission behind the floor |
| Machine/Controller journals tied to a protocol | Plugin installation incarnations, install attempts, native execution binding and preparation records, telemetry binding | Bound by the Machine protocol interval and the cold-recovery pin; readers must understand them to be eligible; each has its own refusal rules |
| Controller database | PostgreSQL and SQLite SQLx baselines plus later migrations | Baselines are immutable after deployment; a new migration is the only change; recovery readers must decode the schema |
| Browser datasets | product-sync dataset (`SHA-256("cowboy.product-sync.v1", Service, user)`), IndexedDB outboxes | Descriptor frozen per product root; a changed dataset permanently fences that owner |

A fifth, orthogonal mechanism decides *who may read*: the host's cold-recovery
Cowboy pin (active, next-recovery and cold readers). It is the reason resident
fixes must retain an accepted worker bundle.

One more category exists and matters: **advisory** datasets. The cleanup
continuation is liveness-only: an older Machine ignores it, and a newer one still
needs the committed permanent deletion, the exact root and a fresh scan before any
effect, so it needs no floor. Everything that refuses stale observations
(deletion, incarnation, leases) is **safety-critical** and does need one.

## Rule: classify before designing

For any new dataset ask one question: *if a Machine or Controller that does not
know it runs against this state, can it do something the dataset exists to
prevent?* If not, keep it advisory and monotone (no floor, closed parse, bounded,
failure-tolerant, authority always re-derived). If yes, it is safety-critical and
must satisfy the contract below before a writer exists.

## Contract every safety-critical dataset states

1. **Identity.** A stable dataset name, an owner (Machine, Service, user or
   principal tuple), and a bound location. Identity is storage identity, never
   authorization.
2. **Schema intervals.** Reader schema set and writer schema, declared in release
   provenance; unknown fields and duplicate keys refuse; refusal never rewrites.
3. **Floor.** A root-owned per-dataset floor anchored only on an already accepted
   reader; a writer cannot originate it; an undeclared artifact is refused once the
   floor or committed state exists, including for rollback and recovery.
4. **Exclusive ownership.** One lock, retained directory and lock handles, ended
   admission on replacement of the directory, its parent alias or the lock.
5. **Crash and unknown outcome.** Staged file, sync, atomic rename, directory sync;
   staging never replayed; an unconfirmed commit fences the writer and is never a
   rollback; commands that needed it are refused, existing workers untouched.
6. **Principal and version change.** A different owner tuple or schema refuses at
   open, not at first use; a principal switch tears down the old consumer before
   the new one mounts (as the browser root already does).
7. **Bounds.** Record count, bytes and per-record size; exhaustion refuses new
   writes and never evicts a safety decision.
8. **Coexistence.** Which worker, Plugin and Machine generations may hold it
   simultaneously, stated per workspace and per runtime generation, and what an
   older generation does (ignore, read, refuse) with committed state.
9. **Recovery.** Which declared readers may be selected for rollback or explicit
   recovery, in the exact worker generation, and what is never restored.
10. **Evidence.** An exact old/new reader and writer release matrix, SIGKILL at
    every commit boundary, floor and declaration refusal vectors, and real
    production observation across an activation. Fixtures never stand in for the
    last item.

## Duplication to remove first

The same machinery is now written three times in Rust (`deletions.rs`,
`cleanups.rs`, `incarnations.rs`: create/open without links, lock, bounded closed
parse, replacement checks, staged atomic write) and twice in the Columbus owner
(`session_deletions.go`, `session_incarnations.go`, joined by `state_datasets.go`).
A fourth copy would make the contract above drift. Proposed refactor, each step
behaviour-preserving and gated by the existing tests:

1. **Rust**: one durable-namespace module owning open, lock, handle checks and the
   atomic commit, parameterized by record type, file name, bounds and error
   wording; the three datasets become thin users. The deletion journal's error
   strings and checkpoint hooks must not change, because native conformance
   asserts them.
2. **Owner**: a descriptor table (name, declaration accessor, floor file, anchor
   file, dataset root, record file) driving one set of declaration, floor, state
   and target checks. The deletion dataset keeps its exact messages and its
   rule that a declaration is required once the floor exists; later datasets are
   optional until they have state.
3. **Conformance**: parameterize the production matrix tool by the same
   descriptors so a new dataset gets ACK/dedup, SIGKILL, lock and floor vectors
   without a new script.

None of these changes release provenance or a floor, so none needs a worker or
generation change; each is an ordinary Machine or host release. While main
carries a runtime wire change that host-only releases refuse, Machine-side steps
wait behind the pending worker-pool maintenance (see the
[pool candidate](releases/machine-pool-candidate-2026-10-05.md)).

## Mapping to the P3 exit

| P3 requirement | Design element | What still needs real evidence |
| --- | --- | --- |
| General dataset identity | Descriptor and contract items 1–2 | A live second dataset using it |
| Old/new reader and writer coexistence | Items 2–3, 8; generic matrix | Exact immutable old and new releases per dataset |
| Exclusive fenced ownership | Item 4 | Production restart observation |
| Principal change, crash/reopen, version change | Items 5–6 | Supported-device and real-account runs for browser datasets |
| Multi-workspace and generation coexistence | Item 8 | Retained and upgraded Sessions across a real generation change |
| Active, next-recovery and cold readers | Item 9 and the host pin | Immutable artifacts for all three per dataset |

The Controller database and browser datasets stay under their own rules
(immutable baselines, new migrations only; frozen descriptors). This design only
requires that each state its answers to items 2, 6, 8 and 9 in one place.

## Order

1. Rust durable-namespace extraction (no behaviour change). **Source done**
   (`src/machine_broker/namespace.rs`; deletion journal, incarnation reader and
   cleanup continuation now share it). It keeps every deletion-journal refusal
   string except the unreachable "no parent" wording. The deletion writer's native
   old/new conformance has not been re-run on a built artifact, because host-only
   Machine releases are blocked behind the worker-pool maintenance.
2. Owner descriptor table (no behaviour change; Hawk and Falcon each need their
   own host activation to adopt it).
3. Generic matrix tool.
4. Then the incarnation writer and later datasets on the shared base.
