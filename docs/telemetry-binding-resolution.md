# Independently authorized Service binding resolution

Status: twenty-fifth spatiotemporal slice, 2026-09-12. A staged, finite core
coordinator can resolve a pending Service binding using a new Operator
confirmation. **Production resolution and binding writes remain closed.** No
HTTP route, automatic restart action, Machine mutation or background-export
policy is introduced. This is not P2 production activation.

## Closed actions and evidence

| Action | Eligible Service state | Required evidence | Local result |
| --- | --- | --- | --- |
| `AbortBeforeDispatch` | Exactly `Prepared` | Full original-operation CAS; no Machine query | `Aborted`, unchanged head |
| `AcceptApplied` | `Dispatching` or `NeedsAttention` | Fresh correlated Applied receipt, no unresolved Machine operation, exact current post-state | `Completed`, observed head |
| `RecordRejected` | `Dispatching` or `NeedsAttention` | Fresh correlated Rejected receipt, no unresolved Machine operation, exact expected head | `Rejected`, observed head |

An unchanged head without a receipt does not prove no effect after Dispatching.
Prepared/Unknown/unavailable receipts, later Machine heads and missing provenance
stay fenced. `NeedsAttention` cannot be aborted merely because its observation is
absent. Generic advancement cannot resolve `NeedsAttention`. The original
coordinator may still abort its own Prepared operation before dispatch when its
authority ends; independent recovery uses the new confirmation and audit path.

The new intent binds a unique resolution ID, exact original operation ID and
canonical full-operation digest, both owners, the NEW confirming actor, closed
action, expected observation digest and expiry. It is a separate authorization
purpose from binding mutation, uninstall recovery and OTLP export. Another
current Operator may resolve an expired historical operation, but its old
confirmation is never renewed or replayed.

Core checks the original confirming credential and current Operator before
reading/observing and again before creating the local permit. Its one-minute
maximum budget starts at that new request's original receipt, covers queueing,
query and storage waits, and transfers unchanged into the non-serializable,
non-cloneable permit. Logout, role loss, owner/action changes and observed expiry
are sticky failures, not opportunities to select another credential.

## One query, one local commit

Remote resolution uses the originally captured authenticated Machine connection
for the exact full binding step. It cannot adopt a reconnect, including one with
the same epoch string. That transport only exposes observation, not dispatch.
Query timeout uses the remaining confirmation budget. Connection and budget are
checked again after the query and at the storage admission checkpoints.

Observation may record an already-applied fact without an available Catalog
installation or private export policy. It does not select/install that Plugin,
restore an endpoint or token, or grant an export. A future export independently
checks current signed installation, lifecycle fences, private policy and fresh
export authority. This is not an atomic distributed revocation barrier: a
Machine may change after an observation, and already-emitted OTel is `NoRestore`.

Local Prepared abort requires no live Machine. Its complete-operation CAS is
atomic with the terminal result; a concurrent original coordinator cannot then
advance that operation to Dispatching. If dispatch advancement wins, abort loses.

Both databases atomically store the terminal operation, Service head and complete
resolution audit. They revalidate the whole ledger under the writer lock and
check the permit's own original budget after locking and immediately before
COMMIT, independently of the caller's check. Budget checks are admission checks,
not hard database syscall timeouts or an atomic cross-store credential lease.
An ambiguous COMMIT allows one local read of the exact saved resolution only;
there is no write retry, second Machine query, resend or inverse command.
An identical historical resolution is returned as evidence without a query;
changed IDs/intents conflict. Any retained row keeps legacy fallback closed.

## Reader-first audit format

The existing checksummed SQL document gains a closed `resolutions` vector. Schema
one retains its canonical bytes and requires an empty vector. The first accepted
resolution writes schema two with a nonempty vector. Each audit records the new
intent, complete before-operation, terminal after-progress and resolution time.
The reader checks their exact linkage, owners, eligibility, correlated outcome,
timestamps, unique operation/resolution IDs and bounds. Audit evidence cannot
construct a permit. No applied SQL migration or stored checksum is rewritten.

The ledger remains bounded to 4 MiB and 1,024 operations, at most one audit per
operation and 96 KiB per resolution. New Begin admission reserves additional
resolution headroom. Older near-capacity ledgers can refuse resolution safely;
capacity exhaustion never prunes or rewrites history. Malformed audit remains an
error even with a recomputed outer checksum.

Only the Controller changes; Machine protocol remains 16 and its journal format
is unchanged. Before a production schema-two row may be written, **active,
rollback and cold Controller readers must all accept it**. A schema-one reader is
not a safe rollback after that write. An empty production table makes this
reader-only rollout compatible with its predecessor; deleting a row/audit to
enable downgrade is forbidden.

## Verification and remaining work

Tests cover offline abort, full-operation races, SQLite/PostgreSQL atomicity and
budget rollback, checksum/structural corruption, restart fences, exact terminal
observations, lost local commit acknowledgement, credential revocation, original
deadline and connection loss. A temporary signed Victoria installation, actual
Machine CLI admission, JSON query frames and real SQL exercise fresh Applied
resolution without another dispatch, policy adoption or export.

Still required for P2: accepted active/rollback/cold reader floors on both Sites,
explicit production writer and background-policy admission, user-facing finite
confirmation surfaces, Machine-side unresolved-file repair/recovery and full
cross-end restart/failure acceptance. Service resolution deliberately cannot
declare a Machine's unresolved Prepared/Unknown record repaired. Generic DAG
execution, durable compensation and strict external reversibility are not
claimed by this slice.

The [reader-only Controller release receipt](releases/telemetry-binding-resolution-2026-09-12.md)
records the accepted artifact, gates, unchanged Machine/workers/Web and closed
production admission.

The subsequent [Machine recovery slice](telemetry-machine-recovery.md) stages
protocol 17 and atomic Machine audit schema two. It can close only a validated
reopened schema-two Prepared step, without changing its head or the Service
operation. A separate Service confirmation remains necessary afterwards;
Unknown/corrupt evidence and all production writer gates remain fenced.
