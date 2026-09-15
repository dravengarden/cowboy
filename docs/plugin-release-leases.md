# Core verified release observations

The Catalog owns each exact signed release's **continuous accepted lifetime**.
Installation, uninstall admission and telemetry keep an original private lease;
checking matching bytes again cannot renew it. This is a finite extension of the
[composition design](plugin-spatiotemporal-design.md), not a generic executor or
completion of its graph/scope/state requirements.

## Mechanism and consumers

`resolve_verified_exact` returns `VerifiedPluginRelease`, with private fields,
no Clone/serde/public constructor, and immutable desired release bytes. Its
observation uses an in-process weak reference identity. The current Catalog
must contain that exact identity at that exact key; another Catalog instance,
a retained runtime generation or a serialized receipt cannot supply it.

A successful refresh reuses the identity only if the **complete DesiredPlugin
envelope** is unchanged, including signature, publisher key, package and bound
host bytes. Removed, replaced and re-added entries get fresh identities. Rejected
candidates never change the accepted snapshot or its observations. Unchanged
refreshes, runtime-only activation and other releases do not invalidate a lease.
Snapshot publication is protected by the existing Catalog lock; no new wire
generation counter, global invalidation or second installer is introduced.

| Finite consumer | Capture | Later checks |
| --- | --- | --- |
| Core installation | Exact Catalog lookup before awaiting the original Machine target observation | Original authority/compatibility checks, after their awaits, authentication sync and install dispatch |
| Core uninstall | Exact trusted release during confirmation validation, before Machine preflight | Each existing finite authority/effect boundary; no fresh lookup can renew it |
| Managed telemetry binding/export | Exact contract and original Machine installation observation at resolution | Original release and installation observations before dispatch; Machine installation lease also reaches its atomic enqueue check |
| Explicit legacy telemetry | Exact signed contract per batch | Release observation recheck before existing host invocation; private legacy policy is unchanged |

These observations are **not authorization**: the Operator/purpose, absolute and
monotonic budget, original Machine connection, installation incarnation/CAS,
durable namespace/fence and private policy checks remain independent. A fresh
resolution requires its own current authority; no receipt or preview grants an
inverse. No release observation is restored from a journal after restart.

## Evidence and boundaries

The binding/export regression tests both failed before the repair: after an
accepted Catalog removal/re-addition without an intervening current check, the
old binding stayed live and the old export reached the disposable HTTP receiver.
They now reject the old operation. Core tests also cover exact/forged lookup,
immutable input, another owner, retained snapshots, failed refresh, new/default
releases, runtime-only activation and signed publisher-key rotation/restoration
under an identical release tuple.

The [connected installation gate](plugin-install-connected-conformance.md)
exercises actual HTTP install/uninstall waits against the immutable candidate
and a real enrolled fixture Machine. Source tests and supplied process artifacts
do not by themselves prove actual host activation; the release record must bind
the candidate, next recovery and cold roles before publishing that result.

This detects accepted Catalog discontinuities at core checks, not unseen changes
to trust files. It does not claim linearizable Catalog revocation with remote
enqueue, cancellation of an already dispatched effect, or new uninstall-preview
lifetime semantics. Missing post-effect evidence remains unknown and fenced.
Telemetry already emitted remains `NoRestore`. There is no Plugin/component
version, Machine protocol, SQL baseline, journal format or policy change.

General verified graph resolution, Service/Workspace/Session identity, exclusive
state leases, independent post-effect/native recovery and real account/device
acceptance remain in the [completion ledger](plugin-refactor-completion.md).

Release status: implemented with source regression tests; immutable role gates,
remote publication and Controller-only activation are pending for this change.
