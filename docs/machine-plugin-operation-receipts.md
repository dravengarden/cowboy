# Machine-owned Plugin uninstall receipts

Status: fifth spatiotemporal slice, 2026-09-09. Protocol 10 adds a finite
durable uninstall step and a read-only query, integrated with the Service
coordinator. The Machine defaults to **reader-only**. Building/publishing a new
Controller does not upgrade a resident Machine, enable admission, or restart
workers.

## Identity, authority and observation

`UninstallPluginStep` and `QueryPluginUninstallStep` use a closed schema-one
`UninstallStep`. Its identity is `(service_id, operation_id, uninstall)`, with
the exact Machine, Plugin/version/digest/fingerprint, expiry, and a digest of
the entire persisted Service intent (including actor and approved impact). The
Machine records and validates a separate complete request digest. Reusing the
identity with any changed argument returns `identity_conflict`; it never returns
the earlier successful result for a different request.

RPC correlation remains a fresh connection-owned ID, distinct from the durable
step. The Controller checks both reply kind and complete receipt identity on the
authenticated connection. An old connection's late reply cannot complete a
replacement connection's waiter. Step replies do not enter ordinary Machine
event history.

The configured `--service-id` and resolved runtime Machine ID must match the
step. An absent Service pin does not acquire authority from the request. The
Service checks current Operator authorization, exact Catalog trust and approved
impact before admission. The Machine independently re-verifies retained signed
generation bytes and the active exact release before removal. Stored hashes are
integrity/equality evidence, not new authorization, actor credentials or a
cryptographic signature from the product user.

Protocol-ten preflight is a query before Service admission or worker stop. A
reader-only, unavailable or conflicting result rejects that new operation
without stopping workers. A protocol 5–9 Machine retains the existing explicit
legacy workflow and its documented weaker recovery guarantees. The Controller
never sends a protocol-ten command to it or treats its ordinary ACK as durable.

## Local durability and failure semantics

The existing `MachinePluginStore` owns `STATE_DIR/plugin-operations/`. This is
private core state, not a Plugin payload, a second lifecycle, Provider home,
OTel exporter or `/tmp` diagnostic spool. Records are at most 8 KiB; total
retention is bounded at 4096 records. Capacity rejects new identities while
preserving old queries and unresolved recovery evidence. There is no automatic
archive, expiry or cleanup yet.

A process-held file lock prevents two new journal-aware Machine stores from
mutating this namespace concurrently. The existing lifecycle lock serializes
Plugin effects. File content is flushed and closed before atomic rename; the
journal directory is flushed after rename, and its parent after creation.
Uninstall flushes the Plugin and credential-projection parent directories before
writing an `applied` receipt. This relies on local Unix filesystem durability;
it does not protect against administrators replacing state or storage that lies
about flush completion. An old pre-journal binary does not know the lock.

Before removal, the Machine commits an uncertain intent. Only a successful
effect and durable final receipt can return `applied`. A failed precondition or
expiry returns a durable `rejected` with a closed reason and no Plugin effect. A
partial removal/error returns `unknown/effect_failure`. Interruption at an
intent/effect/receipt boundary retains `unknown/interrupted`; startup never
replays it or guesses success from an absent active link. A write/flush failure
poisons local admission/observation until a clean reopen validates the evidence.
Unknown schema, oversized/corrupt evidence or a conflicting record makes journal
startup fail closed: the Machine refuses that state. Record files are not
followed through symlinks.

Repeated identical commands return only saved evidence, even if the Plugin was
subsequently reinstalled. Therefore `applied` is a historical receipt, not proof
of the installation's current state. An unresolved step fences install, new
launch-context resolution and new host bindings for its Plugin; unrelated slots
remain available. Existing detached workers and code-runtime leases are not
killed by a journal scan. After any journaled step on a slot, legacy unjournaled
uninstall/reactivation is refused even by a reader-only successor/rollback.
Service auth revocation remains independently authoritative.

## Service integration and query

The existing Service intent/SQL schema is unchanged. Successful live durable
uninstall feeds the existing atomic session-soft-delete/completion transaction.
Missing, mismatched or non-applied evidence retains the Service recovery fence.
If the later Service commit needs compensation, the durable path does **not**
fall back to unjournaled `ReactivatePlugin`: a verified, separately authorized
Machine recovery step with installation CAS is still required.

An Operator may make the read-only request:

```text
GET /api/machines/{machine}/plugins/{plugin}/operations/{operation}/machine-receipt
```

The Controller resolves the operation from its own journal, checks Service,
Machine and Plugin ownership, derives the exact original step, and queries a
fresh authenticated connection. The response contains closed Machine evidence
and `reconciliation_performed: false`; it does not include actor names, session
IDs, endpoint/token policy, credentials or exception text. A protocol-nine or
offline Machine returns unavailable without changing the Service record.

`not_found` cannot prove that an older unjournaled command never ran. No query
automatically resends an effect, commits session deletion, reinstalls a Plugin
or clears a fence. Expired requests can still query their original receipt.

## Machine reader-floor rollout

1. Build the immutable Machine release from the verified clean Cowboy commit.
2. In a separately scoped Machine maintenance operation, activate it with the
   default reader-only mode. Verify its component receipt, protocol inventory,
   broker/worker adoption and existing sessions. Do not infer that acceptance
   from a Controller release or from compiling the artifact.
3. Establish that journal-aware Machine as the accepted rollback floor before
   enabling `--plugin-operation-admission` with an explicit `--service-id` in
   the owned Machine service configuration. This policy change is not a Plugin
   installation or Catalog publication.
4. Verify the intended reader-only rollback against retained receipt fixtures.
   Once a step exists, a pre-journal Machine is not a compatible rollback. Never
   remove evidence to start it. Removing the admission flag pauses new durable
   effects, but keeps queries, receipts and legacy-bypass fences.

No Machine maintenance or admission enablement is implied by this source change.
The existing Controller journal-reader floor remains valid because there are no
new Service phases, problems or database migrations.

## Verification and remaining scope

Hermetic tests cover signature-verified install/uninstall/reinstall, duplicate
and changed-ID requests, lost observers/old-connection replies, reopened intent
and completed receipts, partial effects, pre/post-effect write failure, wrong
Service/Machine, reader-only rejection, exact reply validation, bounded
retention, single writer, corruption and symlink rejection. They do not
uninstall a live Plugin or use production credentials.

Still missing: durable installation incarnation/CAS (including same-release
ABA), fresh recovery policy/auth epochs and monotonic cross-restart lease rules,
verified worker restoration, journaled compensation, evidence archival, operator
recovery UI, Victoria binding activation/revocation and the generic finite
executor. This is at-most-once **attempt with possible Unknown**, not
exactly-once effects or strict reversibility. It does not meet all P2/P4 exits.
