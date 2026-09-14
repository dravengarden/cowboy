# Durable core installation attempts

Status: 2026-09-14, [accepted and active on Hawk](releases/plugin-install-journal-2026-09-14.md).
The finite Service installer records
install and upgrade attempts in PostgreSQL/SQLite before any authentication sync
or installation dispatch. Reader revision `95c0e854` pauses new admission. The
writer-enabled descendant `6d072bab` was activated after the active,
next-transaction recovery and cold Controller readers were accepted; building or
testing it does not establish those host roles. This is not Machine maintenance
or a new Plugin lifecycle.

## Identity and authority

The install request includes a bounded `operation_id`, exact version and composite
artifact digest. Core captures the original Operator credential before awaiting
storage, then consumes that confirmation into one exact intent. The intent binds
Service, actor, Machine, Plugin kind/version/digest/fingerprint, the SHA-256 of
the complete trusted envelope, one transport request ID, and the original
deadline. It contains no package bytes, download URL, credential, error payload,
connection token or serialized grant.

The existing five-minute monotonic budget and continuous credential, Catalog,
compatibility and original-connection checks still apply. SQL waiting does not
renew them: authorization is checked after the progress commit, immediately
before dispatch. Closing the HTTP observer does not cancel the admitted task.

One request ID is used for one live Machine command. A repeated operation ID
returns historical evidence with HTTP 409 and `execution_authorized: false`;
changed actor/target/version/digest cannot observe it through that duplicate
request. Neither case dispatches again. A fresh ID cannot bypass an unfinished
installation slot. The database also serializes install and uninstall claims
across their two journals, in addition to the existing process-local guards.

## Durable progress

```text
Prepared → [SyncingAuthentication] → Installing → MachineAcknowledged
   │                 │                  │                 │
   └─────────────────┴── Aborted*        │          Completed / AuthenticationPending
                                        └── NeedsAttention
```

Any interrupted nonterminal phase can become `NeedsAttention`.
`Aborted` means no **installation** was dispatched; a preceding Service-auth
replica sync may have occurred. From `Installing`, only an in-process proven
not-sent result can take that terminal transition. A generic rejected ACK is
not proof of rollback. Lost replies, connection changes and ambiguous writes
remain fenced; no forward replay or inverse is sent.

Machine ACK observation is saved before post-install authentication sync. The
reservation is released only after `Completed`, `AuthenticationPending` or
`Aborted` commits. Failed/ambiguous progress commits cannot authorize the next
effect or release the reservation. Recording a storage failure never overwrites
a terminal outcome or repeats a command whose COMMIT response was lost.

Before dispatch/HTTP startup, the Controller validates the bounded journal and
reconstructs all unfinished slot fences. It preserves the original interruption
phase and closed problem code across repeated restarts. Corruption, unknown
schema, identity mismatch and foreign unfinished Service ownership fail closed.
Startup neither queries nor writes the Machine, synchronizes credentials,
restores sessions, clears uncertainty or treats current inventory as proof.

The additive migrations are PostgreSQL 0048 and SQLite 0022. Existing migration
bytes are unchanged. SQLite retains WAL `synchronous=FULL`; PostgreSQL commits
these transactions with `synchronous_commit=on`. All claim paths acquire their
write locks before reading. The store-copy allowlist includes installation
evidence. The journal is bounded at 4096 records, retains terminal identities
for deduplication, and rejects capacity exhaustion rather than deleting recovery
evidence automatically.

## Reloadable diagnostics

`GET /api/machines/{machine}/plugins/{plugin}/installation-operations` uses the
existing Operator lifecycle authorization. Its closed v1 projection includes
the latest 32 attempts, phases, exact release, safe problems, timestamps,
reader/admission state and reconciliation flag. It excludes actor, envelope,
request ID, deadline, credentials and raw errors. Protocol-seven operation
correlation is deliberately **not** advertised as a durable Machine receipt.

Core Machine Provider management and the admin telemetry installer expose the
same Installation history component, outside Plugin-authored surfaces. Expansion,
reload and manual refresh only GET saved evidence. The client validates bounded
closed records and rejects unknown fields/states, duplicate IDs, malformed
digests, invalid dates, bad content types and oversized streamed bodies. Failed
reads are unavailable, not an empty history or permission to retry. Historical
completion is not a claim about the presently active installation.

## Acceptance and remaining scope

The shared real SQLite/PostgreSQL contract tests cover immutable identity,
cross-kind claim exclusion, illegal progress, complete-intent CAS and restart
uncertainty. Tests cover each interrupted phase, SQLite writer contention,
corrupt/future evidence, observer loss, panic, each prior-durable effect boundary
and injected phase-write failures. Existing original-connection/late-ACK and
Operator revocation tests still exercise the live executor. Browser-side tests
check history validation, no replay, exact request identities and both UI clients.

Reader-first publication is not writer activation. A pre-journal Controller may
ignore the additive table: it is **not** an accepted post-admission recovery
target. Verify actual immutable active/recovery/cold executables and their
startup behavior before enabling the writer; an empty production table is not
reader evidence. Do not erase operation rows to boot an older reader.

The repository-owned gate is:

```bash
nix develop -c just plugin-install-reader-conformance /absolute/matrix.json /absolute/new-receipt.json
```

Its closed matrix has `schema: 1` and a `controller` object containing `active`,
`rollback` and `cold` immutable Controller release paths. From clean committed
source, it starts the actual executables with disposable SQLite storage and
isolated loopback. All 72 checks must pass: 12 absent/populated/corrupt/foreign
cases, three roles and two process opens. Duplicate POSTs only observe seeded
identities; they cannot dispatch. The private create-only v1 receipt contains
artifact provenance and closed check results, not raw journal rows or logs.
PostgreSQL process startup, actual host role selection, production credentials,
Machine receipts and activation remain separately unchecked.

Remaining P4 work includes Machine-owned install/staging/activation receipts and
installation CAS, independently authorized recovery (including proven pre-effect
abandonment), post-effect exact worker verification, and bounded evidence
archival that preserves unresolved references. This Service journal fixes the
lost-on-Controller-restart reservation; it does not finish the whole refactor.
