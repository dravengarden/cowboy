# Core-owned install and upgrade attempts

Status: 2026-09-14. This repairs the existing install path; it is not the durable
P4 lifecycle coordinator or permission for a Machine/policy cutover.

Both supported Plugin/Provider HTTP routes delegate to one core coordinator.
The request must name an exact Catalog version and composite artifact digest;
omitted/null identities and unknown fields fail closed. The browser cannot
choose a URL, publisher, package payload or a moving latest release.

The Service captures the actual Operator confirmation before asynchronous
validation. A non-serializable, non-cloneable authority binds the Service,
original credential, Machine and complete resolved signed envelope. Its original
five-minute monotonic budget includes queueing and validation. Each effect
rechecks the original credential, current role, current Catalog trust, generic
and applicable Agent compatibility, and original authenticated connection.
Observed revocation is sticky: repairing a role or signing in again cannot
revive that attempt. These are effect-boundary checks, not atomic distributed
revocation of a command already admitted on the Machine.

The core attempt owns its lifecycle reservation and runs independently of the
HTTP observer. Closing the page does not drop a running reservation or cancel
an admitted install. Pre-install authentication synchronizes only when required;
the existing already-current replica exception still permits upgrading an old
package with an incompatible authentication projection. Post-install sync
rechecks current Service state. Both syncs retain the original connection and
recheck authority after encryption-key lookup, before enqueue.

| Observation | Local result |
| --- | --- |
| Validation/authority or pre-install sync fails | No install is sent; restore the prior reservation state |
| Transport proves install was not sent | Restore the prior reservation state |
| Install is acknowledged successfully | Release the installation fence; existing session generations stay pinned |
| Authentication cannot reconcile after acknowledged installation | Report installed with authentication pending, not failed/undone installation |
| Missing ACK, disconnect, generic rejected ACK, or interruption after dispatch | Retain `NeedsReconcile`; never retry, follow a replacement connection or send an inverse |

A generic rejection is not proof of zero effects: activation or authentication
may have failed after local writes. The actual Machine's installation journal
independently fences interrupted tracked transitions. The Service's existing
operations listing exposes the live `requires_reconciliation` flag. Diagnostic
responses never include raw Machine errors or credential-bearing payloads.

The admin Plugin installer now calls the same registered resource route as
ordinary Provider management, without the nonexistent `/install` suffix. It
rejects an unbound release before sending a request. Publishing still does not
install or enable telemetry.

## Acceptance and remaining boundary

Tests exercise the production observer/owner wrapper, each authorization and
authentication boundary, prior-uninstalled state, concurrent independent slots,
panic, exact request decoding, and generic transport certainty. Real correlated
Machine channels cover same-epoch connection replacement and late/missing ACKs
without retry. Credential tests include logout, role loss, disablement, queued
expiry, every envelope input and sticky refusal; the Controller-only feature
slice runs them without importing the Machine host. Web tests invoke the actual
admin API against an isolated fetch stub and check its registered route.

The Service reservation remains process-local. This change does **not** provide
durable install intent, cross-restart deduplication, install receipts or an
independently authorized install recovery API. A Controller restart is not a
supported way to reconcile uncertainty, and current inventory cannot establish
an earlier outcome. Completing that boundary requires one versioned durable
install/upgrade/uninstall protocol, Service and Machine readers, fresh recovery
authority, and accepted active/rollback/cold floors before admitting new writes.
No applied migration, Machine protocol, signed Plugin/SDK or native ABI changes
in this repair.
