# Durable core installation attempts

Status: 2026-09-14. The historical schema-one writer milestone is
[recorded separately](releases/plugin-install-journal-2026-09-14.md).
The schema-two target/receipt bridge's actual Service/Machine reader floor is now
[accepted and active on Hawk](releases/plugin-install-receipt-readers-2026-09-14.md),
with **all fresh Service installation admission paused** and Machine attempt
writing disabled. Connected installation execution and writer cutover are not
yet accepted; the reader release is not that evidence.
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

Schema two additionally binds a closed Machine target: genuinely vacant,
installed incarnation plus digest, or removed incarnation. Core observes that
target on the original protocol-19 connection before binding the confirmation;
it requires Machine admission readiness, never synthesizes vacancy from inventory
absence, and preserves the confirmation's original deadline through that wait.
The complete intent, including actor and target, is hashed into the Machine step.
Schema-one intent bytes remain unchanged and readable; they cannot enter the new
coordinator or gain a target retrospectively. Fresh execution has no legacy RPC
fallback. See [Machine attempts](plugin-machine-install-attempts.md).

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
For schema one, `Aborted` means no **installation** was dispatched. Schema two
also permits an exact durable Machine `Rejected` receipt, which proves rejection
before Staging, not that no command was sent. Both may follow a Service-auth
replica sync. From `Installing`, only proven not-sent or this matching typed
rejection can release the slot. Generic ACKs, unavailable or mismatched evidence,
lost replies, connection changes and ambiguous writes remain fenced; no forward
replay or inverse is sent.

Schema two atomically saves the full bounded, checksummed Machine receipt and
Service phase in the same transaction. Only exact `Applied` can enter
`MachineAcknowledged`, `Completed` or `AuthenticationPending`; Pending/Unknown
becomes `NeedsAttention`. Receipt CAS requires the identical original intent,
Installing phase and no existing receipt. Duplicate, uncertain and terminal rows
cannot be overwritten by this forward writer, even with a later successful
observation. Recording evidence does not require renewed effect authority.

Machine evidence is saved before post-install authentication sync. The
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

The original migrations are PostgreSQL 0048 and SQLite 0022. Additive 0049/0023
introduce paired nullable receipt/checksum columns, retaining existing intent
bytes, phases and indexes; no applied migration is edited or table rebuilt.
SQLite retains WAL `synchronous=FULL`; PostgreSQL commits
these transactions with `synchronous_commit=on`. All claim paths acquire their
write locks before reading. The store-copy allowlist includes installation
evidence. The journal is bounded at 4096 records, retains terminal identities
for deduplication, and rejects capacity exhaustion rather than deleting recovery
evidence automatically.

## Reloadable diagnostics

`GET /api/machines/{machine}/plugins/{plugin}/installation-operations` uses the
existing Operator lifecycle authorization. Its closed v2 projection includes
the latest 32 attempts, phases, exact release, safe problems, timestamps,
reader/admission state and reconciliation flag. Every row carries its original
`evidence_schema` and a nullable closed Machine **outcome only**, never the full
receipt/step. It excludes actor, envelope, plan, target, request ID, deadline,
credentials and raw errors. Legacy protocol-seven evidence is explicitly marked
without a durable Machine receipt. The client accepts the old v1 projection
during Web/Controller rollout and normalizes it to schema-one rows; it never
promotes an old ACK into Machine evidence.

Core Machine Provider management and the admin telemetry installer expose the
same Installation history component, outside Plugin-authored surfaces. Expansion,
reload and manual refresh only GET saved evidence. The client validates bounded
closed records and rejects unknown fields/states, duplicate IDs, malformed
digests, invalid dates, inconsistent receipt/phase combinations, bad content
types and oversized streamed bodies. Failed
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
isolated loopback. All 168 checks must pass: 28 absent/schema-one/schema-two/
corrupt/foreign cases, three roles and two process opens. Exact stored receipts,
all three target states, rejected/pending/unknown outcomes and missing receipt
or target rejection are included. Duplicate POSTs only observe seeded identities
or return reader-only unavailability; they cannot dispatch. The private
create-only v2 receipt contains
artifact provenance and closed check results, not raw journal rows or logs.
PostgreSQL process startup, actual host role selection, production credentials,
actual Machine execution and activation remain separately unchecked.

Before writer admission, accept both actual Controller/Machine reader floors and
connected immutable execution/lost-response/restart gates. A recovery Controller
must pause legacy admission too: the first Machine attempt permanently fences
that path. Remaining P4 work also includes independently authorized installation
recovery, post-effect exact worker verification, and bounded evidence archival
that preserves unresolved references. This bridge does not finish the refactor.
