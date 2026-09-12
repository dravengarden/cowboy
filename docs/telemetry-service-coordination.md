# Service telemetry binding coordination and reader floor

Status: twenty-second spatiotemporal slice, 2026-09-12. The Service now has a
closed binding intent, bounded durable journal, original Operator continuation
and staged finite coordinator. Its startup/export reader is live code. **There
is no production mutation route, dispatch adapter or writer switch.** Machine
protocol 14 remains read-only, and the Machine-local writer remains closed. This
is not P2 end-to-end activation or recovery acceptance.

## Evidence and authority

The intent binds schema, operation, Service, actual confirming actor, Machine,
expected namespace/head, finite select/revoke/restore change and deadline. Its
canonical digest becomes the Machine step's plan digest. Unmanaged absence is
distinct from a managed initial head; installation incarnation, release and
contract remain exact. Every new plan requests exactly the next policy epoch.
Restoration references a completed forward operation, compares its exact
post-state and selects only its recorded prior installation or absence. Neither
restoration nor revocation lowers binding revision or policy epoch.

`OperatorApproval` moved from the uninstall module into a shared core module;
existing uninstall and no-effect resolution behavior is unchanged. Binding
confirmation consumes it into a separate, non-cloneable/non-serializable
authority. It checks the original credential, actor, role, freshness, Service,
complete step digest and original at-most-one-minute budget. Another login,
repaired connection, role promotion or future deadline cannot revive an observed
failure. The coordinator explicitly requires this authority; a transport's
boolean check or a deserialized Actor is insufficient.

The future live transport must independently check the exact trusted Catalog
installation and original connection at each boundary. It has deliberately not
been implemented as a protocol-14 mutation or a private-config write shortcut.
No endpoint, token, policy bytes/hash, telemetry payload, raw error or grant is
serialized into Service evidence.

## One durable Service export slot

Additive PostgreSQL migration 0047 and SQLite migration 0021 create an empty
`telemetry_binding_journal` table. Its fixed slot has a checksum-protected,
closed schema-one document containing owner, current head and the complete
ordered operations. Limits are 1,024 operations and 4 MiB; admission reserves
completion space before inserting an intent. Capacity never prunes evidence. The
explicit non-null slot and table check prevent a second export authority.
Store-copy coverage and published migration checksums include the new table. No
deployed SQL baseline or prior checksum changes.

The reader validates the entire chain, unique IDs, exact owners/predecessors,
restoration provenance, bounded correlated observations and head/outcome
coherence. A historical Applied receipt with a later head cannot commit the
Service selection. Prepared, Dispatching and NeedsAttention must be last and
block new operations. Rejected evidence may establish a managed initial Machine
namespace without pretending that namespace is still absent.

Head and completion evidence share one DB transaction. PostgreSQL forces
synchronous commit and serializes journal writers; SQLite reserves its FULL-
synchronous WAL writer before reading, including an empty table. The original
admission budget is checked after lock acquisition and before commit. This is
not an atomic distributed credential/policy transaction or a hard DB timeout.
Corrupt evidence is never overwritten from a cached head. Moving the slot to
another Machine is not supported by this finite protocol.

## Finite execution and uncertainty

The staged coordinator uses these boundaries:

1. Return identical saved operations as history only. A changed request with the
   same ID is a conflict. Never resume a saved grant.
2. Check original authority and observe the exact expected Machine state.
3. Close new legacy Service export admission and persist Prepared.
4. Recheck authority; persist Dispatching before one remote attempt, then check
   again before issuing it.
5. On an ambiguous response, query the same full step once. Never resend,
   generate another operation ID or issue an inverse.
6. Validate current correlated evidence and recheck authority before the atomic
   Service head/completion commit. If its COMMIT response is uncertain, re-read
   the local atomic record, not the Machine mutation.

Authorization ending before dispatch may record an aborted Prepared operation.
After Dispatching, missing/unknown/mismatched results remain fenced. An Applied
Machine observation arriving after authority ends is retained as NeedsAttention
without adopting the Service head. Evidence bookkeeping remains allowed after
authority loss; it grants no new effect or export permission.

Startup reads evidence before background writers, sweepers, Plugin hosts or
exporters start. It restores only the fence, never commands or managed export
authority. Any retained row, including Aborted or Completed, disables legacy
fallback. A running fence is sticky even if the first INSERT fails ambiguously.
On a validated reopen, actual absence is still distinguishable from a namespace.

The reader checks the Service fence at batch admission and just before invoking
the legacy Machine port. It does not retract already-admitted commands or HTTP
emissions. Local rotated telemetry and incident persistence remain independent.
Managed per-attempt export authority is a separate missing P2 mechanism.

## Verification and rollout

Tests exercise actual core Operator capture and continuation, the finite
coordinator with a deterministic transport fixture, and real SQLite/PostgreSQL
transactions. They cover lost ACK, every pre-dispatch authorization boundary,
late authority loss, duplicate/foreign/stale/unknown evidence, budget expiry
inside a transaction, concurrent slot CAS, rollback atomicity, restart,
corruption, capacity, restoration and refusal of all legacy signal lanes. The
existing protocol-14 correlation tests and Machine-local finite writer tests
remain separate; these fixtures do not claim cross-wire write acceptance or real
network/fsync failure coverage for the entire distributed operation.

Deploy only the Controller reader for this slice. No Machine, detached worker,
Web, native ABI, public Plugin/SDK or configured telemetry destination changes.
The predecessor migrators tolerate new additive migration versions, so rollback
with this table **empty** remains valid. Pre-bridge Controllers do not
understand its admission fence; once a row exists, merely tolerating the SQL
table is not a safe rollback. Live, rollback and cold reader floors on both
sides must be accepted before enabling any production writer. Never delete
authority to make an older artifact start.

Still required: a versioned finite mutation transport and explicit admission,
managed export leases, accepted rollback/cold recovery floors, independently
authorized interruption resolution and cross-end failure/recovery acceptance.
Already emitted OTel data remains `NoRestore`.
