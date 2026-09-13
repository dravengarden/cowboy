# Per-target telemetry writer admission

Status: 2026-09-13. Core now implements explicit, independent host admission for
one exact Service/Machine pair. The configuration is not enabled in production
by this source change. P2 still requires an owned configuration cutover and
cross-end production failure/restart acceptance.

## Independent host policy

The Controller accepts `--telemetry-writer-policy /absolute/private/writer.json`
or `COWBOY_TELEMETRY_WRITER_POLICY`. It requires the exact Service identity and
durable Service store before starting. The Machine independently reads optional
`<state-dir>/telemetry-writer-policy.json` when opening its Plugin journal. The
file is outside `plugin-operations`; it is configuration, not recovery evidence.
An absent setting/file leaves that host reader-only. A present malformed or
unsafe file is a startup error, not permission to ignore the setting.

Both hosts use this closed schema (identities below are illustrative):

```json
{
  "schema": 1,
  "service_id": "service-example",
  "machine_id": "machine-example",
  "purposes": {
    "binding": true,
    "machine_recovery": false,
    "service_resolution": false
  },
  "legacy_fence": "retain_managed_namespace"
}
```

Every field and boolean is required; unknown fields are rejected. The fence
acknowledgment must be that exact JSON string, not an object or null. All three
booleans may be false. Each file must be an owned, private, single-linked regular
file, at most 64 KiB; symlinks and group/other permissions are refused. Policies
contain no endpoint, credential, actor, export grant or retry instruction.

| Purpose | Service admission permits | Machine admission permits |
| --- | --- | --- |
| `binding` | Confirmed finite select/revoke/restore coordination | The matching finite binding CAS |
| `machine_recovery` | Separately confirmed recovery command | Reject only an exact validated reopened Prepared attempt |
| `service_resolution` | Local abort-before-dispatch or adoption of definite Machine evidence | Nothing; Service bookkeeping is never delegated to Machine |

Rust scopes have sealed nominal purpose types. They cannot be serialized,
cloned or converted between binding, Machine recovery, Service resolution and
background-export authority. Transport entrypoints recheck the policy's exact
Service/Machine pair. The existing fresh Operator confirmation, original
connection, original deadline, installation checks and exact durable evidence
remain independently necessary. Admission is not a general DAG executor or a
Plugin-provided security mechanism.

Loading policy, discovering targets, previewing, querying or restarting creates
no binding namespace and sends no mutation or export. With a configured policy,
ordinary choices are restricted to its Machine; retained slot ownership cannot
move. Each plan checks its actual target. `confirmation_available` reports
**Service** admission, not Machine readiness or distributed atomic permission.
The Machine rechecks its own policy when executing the actual command.

## Revocation and effect boundaries

Each host activation retains the opened policy's device/inode, ctime and digest.
Original scopes re-read and validate it at effect boundaries. Removal, invalid
permissions, corruption, in-place edits or replacement (even equal JSON) stop
the activation. Repair does not revive outstanding scopes. Explicit process
restart/reload is a new host activation and never restores an Operator grant,
connection lease or process-local preview.

Service ordinary writes recheck admission before intent, before Dispatching and
at dispatch, including checks inside the SQL transaction. Resolution rechecks
inside its transaction, including after a storage lock wait. Machine binding
rechecks before and after durable Prepared; if admission ends after Prepared,
it may record Rejected bookkeeping without applying the requested head. Machine
recovery checks before the atomic receipt/audit replacement. These checkpoints
do not promise atomic distributed revocation or cancellation of synchronous
filesystem I/O already admitted.

Closing admission does not close authenticated read surfaces, erase journals,
clear legacy-export fences, reopen an old policy or recover credentials.
Historical duplicate commands remain reads, not fresh writes. Machine recovery
does not settle the Service: that requires a different purpose and a fresh
confirmation. Unknown/schema-one evidence remains quarantined.

**The first Service intent fences legacy export even if the Machine refuses.**
An abort or rejection cannot restore unmanaged absence. Reverting a binding is
an explicit CAS with new counters; already emitted OTel remains `NoRestore`.
Binding admission neither activates a background queue nor proves delivery.
[Background export policy](telemetry-background-policy.md) separately requires
the completed exact binding and its own private Machine destination policy.
Local rotating files remain independent.

## Verification and production cutover

Tests use real private policy files and production constructors, independent
from fixture writer booleans. Coverage includes all purpose combinations,
foreign owners, closed schema, replacement/revocation, private file failures,
no namespace on open/preview, post-Prepared rejection, durable reads after
revocation, and policy loss while a real SQLite resolution waits for its lock.
Signed Victoria installation, JSON Machine frames and real HTTP confirmations
exercise normal writes, each host's independent refusal, and recovery followed
by a separately admitted Service resolution. These are hermetic tests, not a
claim of production fault acceptance.

No protocol, journal schema, applied SQL migration, signed Plugin/SDK, native
ABI, Provider state or worker-generation input changes. Release Controller and
Machine independently, preserving detached workers, and run populated immutable
[reader conformance](telemetry-reader-conformance.md) for actual active,
next-transaction rollback and cold roles before any production write.

Reader compatibility is not startup-configuration compatibility. Older
Controllers may not recognize the new CLI setting; older Machines ignore this
new file and remain reader-only. Before a production policy cutover, separately
accept the complete candidate/recovery/cold startup configuration and explicit
export behavior. Verify both host policies and account for the interval between
the first intent fencing legacy export and explicit managed background
activation. Do not open the production gates alone, silently migrate private
configuration, delete evidence, or recycle Agent sessions to complete that task.
