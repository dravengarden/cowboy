# Finite Machine telemetry binding transactions

Status: twenty-first spatiotemporal slice, 2026-09-12. The Machine-local
`select`, `revoke` and `restore` transaction is implemented and exercised with
real private files and signed Plugin fixtures. **Production admission remains
closed.** There is no new wire command, HTTP mutation endpoint, startup adoption
or flag that enables this writer. Only hermetic tests enable their own store.
This is not the completed Service/Machine P2 coordinator.

## Exact admission and restoration

The existing authenticated connection owner constructs a non-cloneable,
non-deserializable lease for one complete step before scheduling. The store
consumes that lease. Its original wall deadline and at-most-one-minute monotonic
budget bound lifecycle-lock admission; disconnect, expiration and clock
regression cannot be repaired by a new connection or a saved receipt. The
shared Machine lifecycle lock serializes installation and binding checks.
The budget governs admission, not a hard timeout on synchronous filesystem
calls. An admitted atomic replacement may finish after connection or policy
change; its configuration receipt grants no subsequent export authority.

A new operation compares the exact current binding snapshot. The Machine
issues exactly the next policy epoch, independently of the next binding
revision; requests cannot skip, reuse, decrease or overflow that counter.
Restoration also advances both counters. The reader retains its wider
non-decreasing schema-one compatibility rule for historical evidence.

Selecting an installation, including restoring a prior selection, requires
its currently active installation incarnation, exact release and contract,
an unfenced lifecycle slot, retained signed bytes, no Provider auth generation,
and fresh valid Machine-private endpoint policy. Each checkpoint re-verifies
that target and the original policy file observation; replacing a file with
identical JSON still ends that attempt. Revoking or restoring absence needs
no obsolete Plugin installation or endpoint file.

Restoration references an applied forward request, compares against that exact
post-state and selects only its recorded prior installation or absence. A
later mutation, including same-release reinstallation or binding ABA, prevents
the CAS. The executor does not install/reactivate a Plugin, rewrite private
policy, restore credentials, or emit HTTP to make restoration succeed.

These are Machine-local checks. The connection lease is not fresh Service
Operator authorization, and a recorded policy epoch is not a policy credential
or managed export lease. The future Service coordinator must independently
authorize and journal its plan before issuing this finite step. Restart cannot
construct export authority from this ledger.

## Persistence and interruption

The existing exclusively owned Plugin journal owns the unchanged schema-one
`telemetry-bindings-v1.json` format and its 1,024-receipt / 4 MiB limits. New
admission re-reads and validates the retained file against the cached head;
corruption, removal or an out-of-band replacement cannot be overwritten from
an old in-memory snapshot. Capacity never prunes prior evidence.

Before changing the head, the writer reserves bounded encodings for the intent
and every finite completion, rechecks admission, atomically replaces the file
with `Prepared`, and flushes the parent directory. It then rechecks the original
lease, installation and policy. The final head and receipt are one atomic
replacement, acknowledged only after file and parent-directory flushes.

Initial preflight rejection creates no managed namespace and leaves legacy
export admission unchanged. **Creating the first Prepared namespace already
fences legacy export**: it is a managed admission change, not a claim of zero
effects. If authorization or policy ends after that flush, the final receipt
records rejection without advancing the head or epoch, but the namespace is
retained. Recording that rejection is completion bookkeeping, not renewed
authority to apply the requested binding.

Any uncertain persistence result poisons binding reads, writes and legacy
export admission in the running process. In particular, failure after rename
but before directory flush must not return a cached `Prepared` or claim that
the old selection still exists. Validated reopen reports the actual retained
file: absent, unresolved Prepared, or the complete final head and receipt.
Unknown evidence remains fenced. Neither restart nor repeating an operation
automatically resumes it.

Identical operation IDs return historical evidence only, even after subsequent
commits. Changed requests are identity conflicts. A lost ACK therefore has a
read-only reconciliation path through the existing protocol-14 query, but this
slice does not yet implement the Service's durable reconciliation workflow.
Prepared/Unknown interruption resolution still requires its own separately
authorized, auditable protocol; ordinary restoration cannot clear that fence.

## Acceptance and remaining work

Hermetic tests cover the production finite executor, signed legacy and OTLP
installations, private policy rejection/replacement, retained-byte tampering,
same-release installation ABA, binding CAS races, exact epochs and exhaustion,
owner/request/connection mismatch, queued and post-intent expiry, capacity,
duplicate/lost-ACK observations, and failures before/after each atomic rename.
They verify private permissions, retained evidence, unchanged policy bytes and
continued refusal of legacy export after managed namespace creation.

This release changes no public Plugin/SDK, applied SQL migration, native ABI,
authentication policy, configured telemetry destination or wire schema. The
Machine reader can be activated independently while the writer stays closed;
Controller and detached Agent sessions need not restart for this change.
Actual live component receipts, not the Git commit alone, establish that
reader's activation. The cold-start reader floor remains a separate acceptance
requirement before any production namespace is written.

Still required for P2: Service durable selection/revocation intent and fresh
Operator authorization, a versioned command with explicit writer admission,
accepted live/rollback/cold reader floors, managed export leases, durable
lost-ACK coordination, separately authorized interruption resolution and
end-to-end recovery acceptance. External OTel emission remains `NoRestore`.
