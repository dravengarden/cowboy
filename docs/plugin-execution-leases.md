# Plugin execution leases

Status: eighth spatiotemporal slice, 2026-09-10. Machine protocol 13 advertises
connection-bound execution leases for the existing durable uninstall step.
This hardens a live effect boundary needed by future compensation; it does
**not** enable restoration, introduce a generic executor, or replace current
Operator, Catalog, Service-auth or Machine-admission policy.

## A queued command is not permanent authority

Previously the Machine spawned an uninstall task with the configured Service
identity and a wall-clock deadline. That task could wait for the lifecycle lock
after its originating connection ended; receipt correlation protected the
Controller's observer, but not the Machine's delayed effect. Wall-clock-only
expiry also did not independently bound time spent queued or verifying bytes.

Each authenticated Machine control connection now owns one core-only
`PluginExecutionScope`. Before spawning a journaled uninstall task, it creates
an `UninstallExecutionLease` bound to the complete validated request digest,
configured Service, actual Machine and that connection's unique process-local
owner. The scope is not cloneable; detached tasks keep only revocable leases.
Returning, failing or dropping the connection future invalidates its leases.
A new connection with the same Service/Machine names cannot revive an old one.

Execution access is a closed `Observe | Execute(&lease)` type. Queries need no
lease and cannot become writes through a boolean flag. Leases are neither
serializable nor deserializable; reopening a journal can recover evidence, not
permission to replay it. Retained receipts still bind the complete original
request, and identical completed/unknown steps never execute again.

## Time and effect checkpoints

The lease begins on command receipt, before asynchronous scheduling or lock
acquisition. Its process-monotonic budget is the smaller of 60 seconds and the
remaining original absolute deadline. It is checked before and after retained
artifact verification, and again after slot-intent persistence immediately
before the first Plugin mutation. Waiting or checking cannot renew it.

The original wall-clock expiry is also checked. Observed wall-clock rollback,
expiry or invalid time permanently closes that lease; a later clock adjustment
cannot reopen it. This is a process-monotonic admission budget, **not** a hard
timeout for a blocking filesystem syscall, a cross-restart/offline grant, or a
claim of portable suspend-inclusive elapsed time. A resumed/restarted process
cannot deserialize this authority, and a new recovery action still needs fresh
policy/auth checks and its own verified time rules.

| Window | Result |
| --- | --- |
| Wrong request/owner, or disconnected before a new intent | Unavailable; no new receipt or Plugin effect |
| Deadline ends before effect admission | Durable `rejected/expired`; no Plugin mutation |
| Connection ends after parent intent but before effect | Parent remains unknown/fenced; no guessed completion |
| Lease ends after installation intent, before active-link removal | Unknown step and pending installation remain fenced; active link unchanged |
| Connection/deadline ends after the effect already started | Finish that admitted effect's durability/result recording; do not invent an undo |

Connection loss is observed locally, not a global instantaneous revocation
guarantee. Once an effect crosses its final admission checkpoint, its remaining
cleanup and receipt writes are completion of that same attempt. It may finish
with applied or unknown evidence even if its response is lost. This is not
cancellation of an Agent turn, worker lifetime, code-runtime lease or HTTP
observer. Existing detached workers remain independently owned.

## Protocol, compatibility and rollout

The Controller selects a closed uninstall transport on the current connection.
Protocol 13+ selects leased durable execution. Protocol 10–12 is rejected before
Service intent admission or worker stop; it cannot downgrade to the legacy
mutation path. A changed/missing connection is also an error, not evidence of a
legacy peer. The existing protocol 5–9 path retains its documented weaker
legacy behavior for schema-one, untracked requests only; an installation-CAS
intent cannot take that path. It does not acquire this guarantee.

The existing `UninstallStep`, receipt, installation journal and Service SQL bytes
do not change. Historical schema-one/two receipt queries retain protocol 10/11
floors, and recovery assessment retains protocol 12. Older Controllers talking
to the new Machine receive the same closed result vocabulary; the Machine still
enforces its local leases. A no-longer-owning connection is `wrong_owner`;
deadline rejection uses the existing `expired` receipt. No serialized grant or
new durable enum is written.

Build and activate the Machine and Controller as independent component
releases, Machine first. No Plugin package/Catalog, SDK, Web/native binary,
database migration or host-policy update is required. Existing rollback readers
can read every stored result; older binaries do not implement the new lease
hardening. This is not acceptance of older binaries as lease-enforcing executors.
Do not exercise uninstall/reinstall or fault injection on production Plugins as
a release smoke test.

## Verification and remaining work

Tests cover complete request/owner identity, scope replacement, monotonic and
absolute deadlines, sticky clock rollback, queued execution, expiry/revocation
during verification, pending installation retention and completed-result replay.
Signed Agent, Zed and telemetry installation fixtures verify that the final
checkpoint follows inventory validation and refused mutations preserve active
links and installation records. Invalid receipt-time clocks remain rejected even
if the first execution check sees a repaired clock. Existing durability/read-only recovery
fixtures exercise the same leased production path. Protocol tests preserve
historical reads while refusing weaker writes and stale connections.

Still missing: separately authorized durable restoration steps, current
policy/auth epochs at recovery, recovery-specific bounded/offline leases,
verified exact worker/native-session restoration, operator recovery actions,
evidence archival, Victoria binding lifecycle and the generic finite executor.
The read-only recovery assessment remains observational with all four
`not_verified` requirements; it does not mint a lease or report restoration.
