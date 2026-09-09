# Plugin uninstall: Service operation journal

Status: fourth spatiotemporal slice, 2026-09-09. This is a finite Controller
coordinator for the existing uninstall workflow, not a generic composition
executor or a Machine-side durable receipt protocol.

## Authority and admission

An unconfirmed preview remains a bounded, expiring in-memory object. It is bound
to its authenticated product user or admin account, exact Machine,
Plugin/version/digest/fingerprint, session set, active-turn set and retention
deadline. Confirmation by another actor or against another target does not
consume it. At most 256 previews and 1024 affected sessions per operation are
accepted. The current Catalog must still trust the selected exact release.

On confirmation, the Service owns execution independently of the HTTP observer.
The existing core database records a schema-one intent before any worker stop or
Plugin mutation. The preview ID becomes the operation identity; duplicate IDs
and concurrent open operations on the same installation slot are rejected. This
is not a retry API: replaying an ID never executes its command again.

The intent contains the Service, actor, exact target and approved impact, but no
credentials, endpoint policy, command payloads, exception messages or serialized
grants. Its canonical serialized bytes have a checked SHA-256 digest. Stored
evidence is not renewed authorization; no startup path deserializes it into a
Machine command. Unknown schema, corrupt evidence or a different owning Service
fails closed before dispatch starts.

SQLite uses WAL with `synchronous=FULL` on every connection. PostgreSQL journal
transactions explicitly set `synchronous_commit=on`. Diagnostic OTel or `/tmp`
files are not involved. These durability settings assume the underlying database
and storage honor their durability contract; they do not protect against manual
database replacement or storage that lies about flush completion.

## Progress and uncertainty

```text
Prepared → StoppingSessions → Uninstalling → MachineUninstalled → Completed
                                  │                 │
                                  └── RestoringMachine → RestoringSessions
                                                           │
                                             Compensated / NeedsAttention
```

Every effect boundary has a prior committed phase. A crash, uncertain response
or failed recovery retains `NeedsAttention`, the phase where it stopped, and
closed failure codes. A compensation failure preserves the primary cause too.
Expiry before any effect can become `Aborted`; expiry after effects cannot.

The existing Machine protocol distinguishes three Controller observations:

| Observation                                     | Service action                                                      |
| ----------------------------------------------- | ------------------------------------------------------------------- |
| Command was not enqueued                        | Retain the fence if worker-stop intent already began                |
| Correlated rejection on the original connection | Attempt the approved, exact retained-generation compensation        |
| Disconnect, replacement, timeout, lost ACK      | Record unknown outcome; do not resend, reactivate or switch Machine |

Forward and compensating commands are bound to the same authenticated connection
incarnation. Even an equal epoch string cannot move recovery to a replacement
connection. This is a short-lived connection fence, **not** a durable Machine
installation incarnation or an idempotency receipt.
Compensation also rejects a missing/ambiguous inventory or an observed different
active release before enqueue. Until the Machine protocol gains an installation
CAS, this is not a guarantee against out-of-band Machine-local changes racing
after enqueue; this recovery is `Compensate`, never `RestoreIfUnchanged`.

Once Machine uninstall is acknowledged, the Service's exact session soft-delete
and journal `Completed` transition share one database transaction. A changed
session identity, missing row or changed deletion state rolls back the whole
transaction. A conditional progress update serializes compensation with a
possibly ambiguous earlier COMMIT: a commit that actually succeeded cannot be
followed by reactivation from that path.

Compensation means the retained Plugin generation was reactivated; it cannot
restore canceled turns, emitted telemetry or arbitrary Agent effects. A worker
reload return value only acknowledges request enqueue. Where workers need
restoration, the operation stays `NeedsAttention` with
`worker_recovery_unverified` until an explicit recovery path can verify their
exact readiness. It is not reported as `Restored` or a successful uninstall.

## Restart, isolation and retention

Before scheduling or serving HTTP, the Controller loads unfinished records and
reinstates `(Machine, Plugin)` lifecycle fences. It does not send remote
commands, kill workers, refresh credentials or drop session data during this
scan. Existing unrelated Machine sessions continue normally.

Install, uninstall, new-session creation, runtime reload, prompt dispatch and
WebSocket worker-start/reset/delete entrypoints respect the fence. Viewing the
existing transcript does not authorize revival of a fenced worker. Pending
workspace preparation exits rather than waiting forever on `NeedsAttention`. An
interrupted admitted task retains its fence even if its HTTP client closes.

Session GC conservatively retains deleted rows on any installation slot with an
unfinished operation. Their events and attachment references stay retained too.
This does not delete source projects or worktrees and does not lower the
Provider auth generation. The journal has no Plugin/session cascade or automatic
expiry. It currently admits at most 4096 records in total and rejects new
operations at capacity; a future explicit evidence-retention policy must not
delete unresolved recovery references to make space.

`GET /api/machines/{machine}/plugins/{plugin}/operations` is protected by the
existing Plugin-lifecycle authorization. It returns the latest 32 bounded
receipts, failure codes, exact release, session count, retention date and
`requires_reconciliation`, without actors, session IDs, private policy or error
payloads. `admission_enabled` identifies a reader-only bridge. Current recovery
is deliberately fail-closed: there is no retry/clear-fence endpoint, no
automatic inference from "currently active" inventory, and no supported direct
SQL surgery to turn an unknown effect into success.

## Reader-floor rollout

The additive migrations are PostgreSQL 0044 and SQLite 0018. Prior migrations'
exact bytes remain unchanged. The PostgreSQL-to-SQLite copy allowlist includes
the journal and still requires full table/column coverage.

The first Controller release reads/reconstructs the journal and applies
retention fences but pauses new uninstall admission. After that immutable bridge
is active, a descendant enables the coordinator. Its automatic rollback
predecessor then understands every new record and keeps uninstall paused safely.
Controller and Web publication are independent; no Plugin artifact, Machine
protocol, Machine runtime, native binary or live worker is upgraded by this
rollout.

Once operations have been admitted, a pre-journal Controller is not a compatible
rollback target. Never erase recovery records or authority markers to start one.

The reader bridge `00e2b69b1cc2e8bc873988c1b7948e3de6b9260f` was activated on
Hawk at `2026-09-09T11:09:42Z` with a successful committed Controller receipt,
admission disabled and zero unfinished slots. The following enabling release
uses that bridge as its automatic rollback predecessor.

## Verification and remaining scope

Tests exercise the real SQLite/PostgreSQL transaction implementations, duplicate
and concurrent slot claims, exact intent binding, rollback of partially applied
session updates, compensation-versus-commit CAS, reopen of every interrupted
phase, bad evidence, connection replacement, lost replies, forced task
interruption and detached observers. Side-effect fixtures assert persisted phase
before each mocked remote operation and never use production credentials.

Still missing: Machine-owned durable operation/step IDs, receipts and
installation incarnations; fresh-policy recovery authorization; verified worker
restoration; operator recovery UI; bounded evidence archival; Victoria binding
activation; and the generic finite executor. A current inventory match is not
proof that an old remote effect completed or was undone. This slice does not
claim the P2/P4 exit gates or the whole Plugin redesign are complete.
