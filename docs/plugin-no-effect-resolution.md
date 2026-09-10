# Plugin pre-effect interruption resolution

Status: tenth spatiotemporal slice, 2026-09-10. An Operator can now explicitly
resolve an interrupted uninstall that provably never reached its first effect.
This is the first independently confirmed recovery action, not a restoration
grant, Machine compensation, general retry, or unrestricted clear-fence API.

## Why this action can be local

The existing executor persists `StoppingSessions` before the first worker stop
and `Uninstalling` before dispatching Plugin removal. A recorded
`needs_attention` originating from `prepared`, with `interrupted` or
`storage_failure` and no compensation cause, is therefore eligible for
`abort_before_effects`. Even when the approved impact included live sessions,
this operation has not stopped them. Mere absence of an active link, an empty
inventory, or an applied historical Machine receipt cannot establish this case.
Here "before effects" means before worker/Plugin lifecycle mutations; it does
not erase prior preview reads, temporary request blocking or audit observations.

Every later phase, unknown origin, compensation cause, already terminal record,
or still-live coordinator is refused. Ordinary phase advancement still cannot
leave `NeedsAttention`; only this closed recovery transaction has that right.
It does not restore a canceled turn, session, native ID, installation or account.

The action leaves the original immutable uninstall intent, actor, impact,
deadline and failure evidence intact. Its only durable mutation is an atomic
pair: a new resolution receipt and the parent operation's `Aborted` phase.
There is no session UPDATE, Machine command, artifact activation, Provider login,
credential materialization, Catalog change or worktree access. This local
resolution remains possible when the Machine or Catalog is unavailable.

## New confirmation and bounded authority

The two-minute, bounded, one-use preview binds the current Operator, owning
Service, exact Machine/Plugin/operation, closed action and SHA-256 of the complete
operation snapshot, including its original intent and progress timestamps. A
wrong actor, target or action does not consume another Operator's preview.
At most 256 previews are retained; process-local budgets prevent an observed
clock rollback or repaired clock from reviving expired previews.

Confirmation captures the actual NEW credential through the existing core
authentication path. Another currently authorized Operator may confirm the
resolution; the original actor's stored identity is not reused as a grant.
Cookie/admin/PAT/device precedence, user disable, current role, revocation and
applicable login freshness checks reuse Service continuation authentication.
Device proof nonces are not consumed twice. Original uninstall expiry does not
prohibit this new local action and is never extended to permit uninstall replay.

The confirmation produces a non-Clone, non-Debug, non-serializable core permit
with a one-minute monotonic budget starting at credential capture, capped by
the preview deadline. Failed authentication consumes that attempt. The Store
checks the budget before admission, after obtaining its write lock and immediately
before COMMIT. It repeats the complete snapshot CAS inside that transaction.
Unknown fields and action tags fail closed; no boolean `force` or supplied
replacement operation is accepted.

Credential checks are point-in-time before local transaction admission, not an
atomic transaction spanning every security store. Later revocation may race an
already admitted transaction. Deadline checks bound admission/commit checkpoints,
not blocking syscalls, suspend-inclusive time or offline/cross-restart authority.
Restart discards previews and permits; a new action always needs new confirmation.

## Concurrency, crash windows and receipts

The memory guard may acquire only an existing `NeedsReconcile` slot. It cannot
steal a live uninstall, install, reload or another resolution. The old coordinator
sets its guard's final state under the same lock before allowing resolution.
While resolution executes, the usual runtime/lifecycle fences remain in place.

SQLite takes its write lock before reading the source record; PostgreSQL uses
the existing journal table lock. Competing resolutions cannot both commit.
Receipt insertion, parent phase change and the complete evidence check share
the same transaction and existing durable DB settings. An injected failure after
receipt insertion rolls back both writes. Original failure codes remain visible.

HTTP observer cancellation does not cancel the admitted transaction. If COMMIT's
response is lost, the coordinator queries its exact saved resolution and checks
the parent result; it never retries mutation or infers success from `Aborted`
alone. Verified completion releases only this Service slot's memory fence.
Unknown results or task interruption retain the fence. If even result observation
is unavailable after a possible commit, restart reconstructs the fence from the
actual durable phase, without executing a recovery action. A read-only receipt
request itself never clears a fence.

## Operator API

All three routes use existing Product/admin Operator authorization:

```text
POST /api/machines/{machine}/plugins/{plugin}/operations/{operation}/resolution-plan
POST /api/machines/{machine}/plugins/{plugin}/operations/{operation}/resolve
GET  /api/machines/{machine}/plugins/{plugin}/operations/{operation}/resolution
```

The confirmation body is exactly `{ "plan_id": "…", "action":
"abort_before_effects" }`. The operations list exposes `resolution_candidates`
as a hint, not authorization; preview and commit recheck their own preconditions.
Receipt/preview responses omit actors, session IDs, credentials, policy and raw
exceptions. Receipts explicitly report no Plugin/session mutation and no worker
restoration. This slice adds the Operator API, not a recovery UI button.

The existing Machine recovery assessment remains read-only, with all four
restoration requirements `not_verified` and execution unavailable. This separate
local action does not claim its restoration checks passed.

## Storage, compatibility and release

Additive migrations PostgreSQL 0045 / SQLite 0019 introduce
`plugin_uninstall_resolutions`, with one checked record per original operation,
an at-most-4-KiB intent, unique resolution identity and a non-cascading parent reference.
The existing 4096-operation admission cap also bounds this table. There is no
automatic evidence expiry or archival. Stored intent/hash pairs are evidence,
not credentials or restorable permits. Resolution reads reject corrupt,
oversized, unknown-schema or mismatched evidence.

All applied migrations remain byte-for-byte unchanged. The PostgreSQL-to-SQLite
copy allowlist includes the new table after its parent. The existing journal and
Machine codecs, enums and bytes do not change. Prior journal-aware Controllers
already read `Aborted` and ignore newer additive SQLx migrations while checking
every known checksum. They retain the new table but cannot issue this action;
there is no new unfinished recovery phase for them to misinterpret. Tests reopen
the new databases with the previous exact migration set and reader policy.

Build and activate only the Controller component from clean committed source.
No Machine maintenance, cold-bootstrap pin, host-policy cutover, Web/native,
SDK or Plugin/Catalog release is needed. Production resolution, uninstall,
credential revocation and fault injection are not release smoke tests.

## Verification and remaining work

Hermetic SQLite/PostgreSQL fixtures cover atomic failure after receipt insertion,
competing confirmations, complete snapshot conflicts, late forward CAS, unchanged
session data, reopen, old migration readers and preservation of later-phase
fences. Additional tests exercise expired queued permits, new Operator authority,
revocation/downgrade, one-use preview bounds, observer cancellation, exact receipt
replay and public response redaction.

Restoration after a worker stop or Machine effect still needs independently
authorized, journaled tombstone CAS, current Provider-auth/policy checks and
verified exact worker/native-session restoration. Operator recovery UI, evidence
archival, Victoria binding lifecycle and the generic finite executor also remain
unfinished. This limited local action is not strict reversibility or P2/P4 exit.
