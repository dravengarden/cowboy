# Plugin recovery assessment

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
