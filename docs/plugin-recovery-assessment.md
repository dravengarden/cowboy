# Plugin recovery assessment

## Offline completion of a failed, already-deactivated uninstall

The 2026-10-01 maintenance path handles an uninstall whose active link was
removed before read-only runtime caches caused cleanup to fail. This is an
explicit host-maintenance action; the observation protocol below remains
read-only and startup never retries it.

Inspect the original Service operation and its affected-session count:

```sh
cowboy operator uninstall-operations --machine hawk --plugin claude-code
```

Only a zero-session operation interrupted in `uninstalling` can subsequently
be completed by the Controller. This path cannot restore workers, finish an
active installation or recover a different installation incarnation. Follow
the host's Machine maintenance boundary: verify no target Plugin workers,
arrange an independent restart, then stop only the resident Machine. Keep
unrelated detached worker services running.

Run the immutable Machine release as the state-directory owner. Preview:

```sh
cowboy-machine --state-dir /path/to/machine-state \
  --service-id SERVICE --machine-id MACHINE \
  --complete-absent-uninstall ORIGINAL_OPERATION_ID
```

Repeat with `--confirm-uninstall-digest` set to the exact reported request
digest to apply. The command acquires the resident Machine's exclusive journal
owner lock. It verifies the failed effect receipt, exact uninstall predecessor
and request digest, absent active link, private state/auth directories, and absence
of an unresolved installation attempt. A matching stable removal tombstone also
permits finishing an interrupted maintenance pass. Empty inventory alone is
never authority. A package directory may be 0755, matching the ordinary installer,
but it must be owned by the invoking UID and not writable by other users.

Before effects it durably archives the original failed receipt and invoking UID
under `plugin-maintenance/`. Every explicit invocation has a 60-second budget.
It deletes only `materialized` and `runtime`; directory permissions are repaired
through opened descriptors, symlinks are unlinked without following them, and
file permissions stay unchanged. Filesystem boundaries, excessive nesting and
expired budgets fail closed. Verified absence and directory fsync precede the
native journal's completion of the matching tombstone and step receipt. Original
failure evidence remains in the maintenance audit. There are no database edits,
copied credentials or installation-pointer writes.

Restart the resident Machine, then complete the original Service transaction:

```sh
cowboy operator reconcile-uninstall --machine MACHINE --plugin PLUGIN \
  --operation-id ORIGINAL_OPERATION_ID
```

This captures fresh host Operator authority, holds the Service slot fence,
queries the exact Machine receipt and current tombstone on one authenticated
connection, and requires `MatchingRemoval`. Its bounded local transaction
compares the entire original operation and atomically records the confirming
actor, Machine evidence and `completed`. It performs no Machine mutation,
session deletion, worker restoration or replay. Unknown receipts, changed
incarnations, affected sessions, revoked grants and racing Service edits remain
fenced. The original Service failure stays alongside the resolution.

Ship the Web decoder and Controller before writing the new
`complete_verified_removal` audit action; an older Web decoder rejects that
history. Machine step/installation formats and the SQL schema do not change.
Older Machine readers still read the ordinary applied step and stable
tombstone; the maintenance archive is outside their journal. Older Controllers
can read the terminal operation but cannot decode the new resolution audit;
use this release to inspect that audit after rollback.

Signed retained packages, sealed replicas and audit/history retention keep their
existing lifecycle. This completes the normal uninstall contract; it is not a
global Provider credential revocation or source/worktree deletion.

## Read-only observation protocol

Status: seventh spatiotemporal slice, 2026-09-10. Protocol 12 adds a **read-only**
recovery observation to the existing core uninstall lifecycle. It does not add
a restore command, grant, automatic retry, new journal format or SQL migration.
Installation CAS remains enabled; durable compensation remains disabled.

## Historical outcome and current installation are different facts

The older `machine-receipt` query reports historical step evidence. Even an
`applied` receipt cannot prove that its removal tombstone is still current:
the same release may have been reinstalled and removed again. Recovery needs
both the original receipt and the current installation authority.

`QueryPluginUninstallRecovery` resolves both under the Machine store's existing
lifecycle lock. It checks the pinned Service, actual Machine, complete original
request digest, journal health and every outstanding step fence on the slot.
The current installation is a closed union: untracked, installed, removed,
pending or unavailable. A removed record includes its new revision, predecessor
revision and complete uninstall request digest. No reader infers authority from
the active link or creates missing installation records.

The query independently checks the active link against the recorded state.
Regular files, malformed or escaping targets, dangling generation directories,
and a link contradicting a tombstone are unavailable evidence, not absence.
This is a link/authority consistency check, **not** retained artifact or runtime
readiness verification. Queries never run probes, materialize credentials,
install, reactivate, stop/reload workers, clear fences or complete a receipt.

The Controller validates the complete response identity even when no receipt
exists, then derives the assessment itself:

| Evidence | Assessment |
| --- | --- |
| Applied original step, same predecessor and request digest, stable current tombstone, no outstanding fence | Matching removal; tombstone revision available for a future CAS |
| Same release installed again, or a later uninstall's tombstone | Installation changed |
| Original step rejected | Historical forward rejection; no claimed worker restoration |
| Missing or uncertain forward receipt | Unknown, even if a matching tombstone exists |
| Pending installation or inconsistent active link | Unknown |
| Another unresolved step on the slot | Slot fenced |
| Original uninstall lacked an installation revision | Legacy/untracked; no restoration CAS basis |

Matching removal is deliberately **not** named ready, authorized, restorable or
restored. It supplies evidence for the next recovery primitive, not permission
to execute one. Recovery must recheck all preconditions at its own effect
boundary; an observation can become stale immediately after the lock is released.

## Operator API

```text
GET /api/machines/{machine}/plugins/{plugin}/operations/{operation}/recovery-assessment
```

The existing Product/admin Operator middleware protects the route. The Service
resolves the original operation from its own journal and checks all three path
identities plus owning Service; clients cannot supply a substitute intent.
The query requires protocol 12 and the same authenticated connection throughout
RPC correlation. Older/offline peers fail without sending an older mutation.
Wrong reply kinds, changed request identity and old-connection replies do not
complete the observer. Replies are not stored in Machine event history.

The response combines the Service phase, primary/secondary failure codes,
affected-session count, Machine evidence and derived basis. It never exposes
actor names, session IDs, credentials, Provider policy, executable paths or raw
exceptions. `recovery_execution_available` and `reconciliation_performed` are
always false. `not_verified` explicitly names fresh policy/auth authority,
retained artifacts/probes, bounded execution leases and exact session/worker
restoration. A Service journal change while awaiting the remote observation
invalidates the assessment. These are separate transaction domains, not one
cross-site atomic snapshot.

## Verification and rollout

The subsequent [execution-lease slice](plugin-execution-leases.md) hardens
journaled forward effects. This read-only assessment still creates no lease;
its recovery prerequisites and false execution/reconciliation flags are unchanged.

Hermetic signed fixtures exercise tombstone provenance, same-release ABA,
reader-only reopen, legacy/untracked state, missing/uncertain receipts, pending
slots, other outstanding steps, poisoned storage, wrong owner and malformed
active links. Journal bytes and file membership remain unchanged after reads.
Protocol tests reject malformed evidence and wrong replies; observer cancellation
and connection replacement release only the waiter. Service tests detect a
racing phase change and preserve the operation instead of committing recovery.

This slice changes only read-only wire behavior, not durable readers/writers or
Plugin package contracts. It needs independent Controller and Machine component
releases, not a host-policy change, new cold-bootstrap pin, Catalog publication,
Web or native release. The existing incarnation-compatible recovery floor can
still read all durable state; it simply cannot answer protocol-12 queries.
Production uninstall/reinstall or fault injection is not a release smoke test.

Remaining: a separately authorized, journaled restoration step with fresh
policy/auth checks and bounded leases; verified worker/session restoration;
operator recovery actions; evidence archival; Victoria binding lifecycle; and
the generic finite executor. This assessment does not meet the P2/P4 exits or
claim reversible external Agent effects.

The later [pre-effect resolution](plugin-no-effect-resolution.md) is a distinct
local action for a Service interruption proven to precede all effects. Its
independent confirmation does not consume this Machine observation or change
any of the assessment's four restoration requirements.
