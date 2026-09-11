# Durable telemetry binding reader bridge

Status: twentieth spatiotemporal slice, 2026-09-11. This is the Machine journal
reader and the authenticated Service query path for P2, **not** an enabled
binding coordinator. No production path creates this namespace, writes a
binding, imports a policy epoch, or executes restoration.

## Evidence and identity

Machine protocol 14 adds only `QueryTelemetryBinding` and its distinct response.
The query names the complete original step: Service, Machine, operation ID,
Service plan digest, exact expected binding snapshot, change and deadline.
Even a missing receipt is correlated to the complete request digest. Reusing an
operation ID with another request is an identity conflict, not a cache hit.
An expired original deadline is queryable; observation never renews authority.

The snapshot separates three identities:

- Exact signed Plugin release and contract, plus its installation incarnation.
- Binding revision, advanced even when restoring an earlier selection.
- Policy epoch, which cannot decrease when restoring configuration.

Revision and epoch are distinct Rust types, encoded as canonical unsigned
64-bit decimal strings. Zero describes only the initial/unobserved baseline;
an active selection requires an observed nonzero policy epoch. An installation
revision, digest, numeric JSON value or noncanonical decimal cannot substitute
for either counter. Overflow fails instead of wrapping.

`select`, `revoke` and `restore` describe finite managed-configuration changes.
Restoration must reference a known applied forward receipt, compare against
that exact post-state, and select its prior installation (or prior absence).
Later changes, including same-release ABA, prevent that CAS. These descriptions
and receipts are data, not verified releases, execution leases or recovery grants.
They never undo HTTP emissions, reinstall a Plugin or restore a credential.

## Machine-owned persistence

The existing private `plugin-operations` journal and its exclusive process lock
own `telemetry-bindings-v1.json`. There is no second installer or telemetry
spool. The bounded file contains one Service/Machine export slot, its current
head, complete ordered operation receipts and a canonical evidence checksum.
There are at most 1,024 receipts and 4 MiB of evidence. No endpoint, token,
private-policy digest, payload, conversation or raw error is recorded.

The reader reconstructs the head from the initial state and checks the entire
receipt chain, unique operation IDs, exact predecessors, policy monotonicity,
restoration provenance and completion post-states. Rejected operations do not
advance the head. Prepared or unknown outcomes remain unresolved and cannot be
followed by ordinary mutations. They require a future separately authorized
resolution protocol; this bridge does not infer that a restart completed them.

Unknown schemas, inconsistent or rechecksummed invalid chains, oversized files,
symlinks, hardlinks, special files and non-private/wrong-owner evidence fail
closed. The checksum detects corruption, not a privileged administrator's
ability to rewrite the whole authority. The local filesystem/process ownership
assumptions of the installation journal still apply.

The binding head and its receipts are one persistence unit. A future writer must
flush its atomic replacement and parent directory before acknowledging it;
storage uncertainty must poison admission until validated recovery. This slice
only tests retained fixtures and reads them; it does not implement that writer.

## Reader-only means no legacy bypass

Absent evidence preserves today's exact private-file exporter. Opening the
Machine or querying a step never creates a managed binding or reads the private
endpoint policy. Once the managed namespace exists, this reader refuses both
legacy JSONL/Prometheus and OTLP invocation at preflight and attempt admission,
even for a completed or empty managed ledger. It cannot fall back to an old
`telemetry.json` policy and silently bypass a recorded revocation.

The query verifies the pinned Service and local Machine under the lifecycle
boundary. It returns bounded evidence, not current private-policy validation,
retained-package verification or permission to emit. The Controller additionally
checks the authenticated connection, expected reply kind and original request,
including missing-receipt identity and head/receipt coherence. Responses stay
out of ordinary Machine event history. Disconnect, cancellation and late replies
release only observation resources; no worker or Plugin is stopped.

## Rollout and remaining gates

Deploy and accept both Controller and Machine readers before admitting any new
binding state. Machine activation remains its own maintenance boundary. The
host's cold-start recovery floor must also understand this file before a writer
is enabled. Pre-bridge Machines reject the unfamiliar journal entry; deleting it
to make an old binary start would erase authority and is not rollback.

This release changes no Plugin artifact, SDK, applied SQL migration, native ABI,
authentication policy or telemetry destination. A production deployment must
verify the component receipts, protocol negotiation, worker continuity, health
and unchanged absence of managed binding state, without reading private tokens.

Hermetic tests cover exact 64-bit codecs, closed effects, stale/foreign replies,
protocol floors, actual Service-to-Machine query framing, signed legacy/OTLP
preflight fencing, corrupt/rechecksummed evidence, capacity, prepared/unknown
reopen, historical ACK queries and CAS-restoration conflicts.

Still required for P2: the Service durable selection/revocation coordinator,
fresh dual-side authorization, durable Machine policy-epoch issuance and actual
binding commits, managed export leases, lost-ACK reconciliation, separately
authorized interruption resolution/restoration, and accepted production writer
and cold-recovery floors. External OTel emission remains `NoRestore`.
