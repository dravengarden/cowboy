# Finite cross-site telemetry binding protocol

Status: twenty-third spatiotemporal slice, 2026-09-12. Protocol 15 connects the
finite Service coordinator to the real Machine command handler and local
transaction. Both production writer gates remain closed. This is a compatible
reader/transport release, **not enabled managed telemetry or completed P2**.

## Exact namespace CAS

Schema-one steps collapsed absent namespace and an existing initial head into
the same Machine snapshot. Service preflight distinguished them, but a namespace
could appear between that query and the Machine commit without changing either
counter. Schema two binds an explicit `expected_namespace` to the complete
request digest; the Machine compares it under the same journal/lifecycle locks
as the head, before creating Prepared. A prior rejected operation can therefore
leave revision zero without being mistaken for unmanaged absence.

Schema-one steps retain their exact canonical bytes and digests and remain
readable. They cannot be sent as mutations. Schema two requires the namespace;
schema one cannot carry it. Unmanaged requires the initial empty snapshot.
Service intent schema two derives this expectation from its original optional
head. Nested intents/steps are versioned independently from the unchanged outer
journal document and SQL table. Old readers are **not** adequate for new nested
schema-two evidence merely because the outer schema number remains one.

| Operation | Minimum Machine protocol | Step schema |
| --- | --- | --- |
| Historical observation | 14 | 1 |
| Namespace-aware observation | 15 | 2 |
| Finite commit | 15 | 2 only |

`CommitTelemetryBinding` has its own closed `TelemetryBindingCommitted` result.
An ordinary command ACK or read-only query reply cannot complete its waiter.
Receipts are correlated against the full request and original connection, not
only an RPC ID or a reused epoch string. Storage failure is uncertain: a failed
directory flush may follow a visible replacement. It is not proof of no effect.
No new receipt is retained in the ordinary transient Machine event history.

## Original authority, finite dispatch

The Service transport captures the original connection and full schema-two
step. It rechecks the exact trusted Catalog release, contract, installation
incarnation, active inventory and Service lifecycle fences. Target selection is
also checked atomically with connection identity and command enqueue. Revocation
to absence does not require a removed Plugin, old policy or old Catalog entry.
Observed authorization failure is sticky; reconnecting or repairing inventory
cannot retarget or revive that transport.

This transport does not grant Operator authority. The coordinator separately
requires the non-serializable core confirmation, its original credential and
one-minute monotonic budget. Owner, capacity, current Service slot and restore
provenance are validated before even querying the Machine or closing legacy
admission, then checked again inside the real database transaction.

The Machine captures its execution lease synchronously on command receipt,
before spawning or queuing behind a lifecycle lock. It binds the original pinned
Service, Machine, request and local monotonic deadline. Disconnect ends queued
authority. The existing finite transaction independently rechecks its exact
signed installation and original private policy around durable intent and
commit. Neither protocol negotiation nor an Operator field opens the local gate.

One dispatch attempt has a 45-second RPC timeout. Failure leads to one 15-second
read-only query of the original full step, never resend, installation, new ID,
inverse command or connection replacement. Those timers cannot renew the
Operator budget. A valid applied receipt arriving after authority ends is
evidence only and cannot adopt a Service head. Recovery never revives a grant.
No endpoint, token, private policy bytes, raw errors or OTel payload enters a
binding step, receipt or Service ledger.

## Acceptance boundaries

Hermetic tests now use a real signed Victoria package and Machine installation,
an accepted signed Service Catalog, actual core Operator confirmation, SQLite,
the real Service transport, JSON `MachineFrame` roundtrips and the same command
admission function used by the socket dispatcher. They exercise finite select,
revoke, exact restoration, receipt reads and historical duplicates without replay.
Dropping the actual committed reply exercises timeout then original-step query
with exactly one mutation, using the real 45-second timeout and original
monotonic authority budget. Separate fixtures exercise authority expiry.

Additional tests cover protocol floors, closed codecs, unchanged schema-one
bytes, namespace ABA after rejection, reader reopen, reply-kind substitution,
storage uncertainty, late connection replies, removed-installation revocation,
installation/contract/lifecycle changes and loss of a queued execution scope.
Production Service capture creates no intent or command; Machine reader-only
admission creates no namespace. These are real cross-component fixtures, not
an authenticated network deployment or cross-process fsync-failure acceptance.

Deploy Controller and Machine as separate owned components. Preserve detached
workers, Web, Provider installations and private destination policy. Verify
Machine generation continuity and continued absence of the managed namespace.
Before any future writer is enabled, both live readers and accepted rollback and
cold recovery artifacts must understand schema-two evidence. Existing legacy
export remains valid only while each binding namespace is absent. Never remove
authority records or alter applied SQL migrations to satisfy an older reader.

Still required: managed per-attempt export leases, explicit production admission,
accepted rollback/cold floors on both Sites, independently authorized interrupted
operation resolution and live cross-end failure/recovery acceptance. Binding
restoration never retracts already emitted OTel data (`NoRestore`).
