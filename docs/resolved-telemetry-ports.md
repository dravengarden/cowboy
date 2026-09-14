# Finite live telemetry resolution

The core composition layer now resolves the **existing managed telemetry**
binding and export inputs into distinct, private `ResolvedBinding` and
`ResolvedExport` types. The finite Service coordinators dispatch those exact
results. This is a first live-resolution slice, not an executable graph DSL or
completion of P0.

The [Controller release record](releases/resolved-telemetry-ports-2026-09-14.md)
records the accepted immutable role matrix, production activation and limits.

## What is resolved

- The exact release tuple and contract fingerprint must resolve through the
  signature-validated, accepted Catalog. Matching Machine inventory alone,
  embedded source, a structural report or a deserialized receipt is insufficient.
- OTLP export additionally requires the selected signal's signed schema-two
  protobuf route. A retained legacy telemetry contract can be selected but
  cannot satisfy a managed OTLP port.
- Core captures the original authenticated Machine connection and a unique,
  active telemetry installation with the exact durable installation incarnation,
  release and fingerprint, and no Provider authentication generation.
- A private, reference-identity observation lease records that installation
  slot's continuity. Binding and export keep their complete immutable original
  request; dispatch accepts no replacement request. The same lease reaches the
  transport's final check under its inventory/connection/enqueue lock.

These types have no public fields, serialization, deserialization or cloning.
Their constructors query live core objects. They contain no destination policy,
credentials, filesystem paths or synthetic generation-number conversion. The
read-only composition schema and its 86 cross-runtime vectors are unchanged;
neither its proposal nor its checked report is accepted by these constructors.

## Observation continuity

Previously, managed telemetry repeatedly compared a currently matching tuple.
If inventory changed away and back between those comparisons, an old operation
could still proceed. Regression tests reproduced both a revived binding and an
export reaching the disposable HTTP receiver.

The connection owner now retains per-slot observation identities. Removal,
ambiguity, installation/release/contract/state/auth-generation change, or
connection replacement invalidates the old lease, even if the original tuple
returns before the next check. This is checked again at atomic enqueue. Fresh
resolution is possible, but does not inherit the failed operation's authority.

Unrelated Plugin changes, inventory ordering, session-lease counts, diagnostic
details, rollback hints and replica/materialization status do not invalidate an
unchanged telemetry slot. The older generic host binding keeps its existing
whole-inventory conservative fence; this does not silently change other domains.

A failed current check is terminal for the resolved port. Catalog refresh keeps
its existing contract: a failed candidate preserves the last accepted snapshot;
a successful removal prevents resolution/current checks. This is not immediate
trust-file revocation, nor detection of a Catalog removal/re-addition between
checks. There is no remote-installation observation guarantee before the Machine
reports it. The Machine remains responsible for its actual installation CAS,
original execution lease and private policy at the effect boundary.

## Authority and effects stay separate

Resolution is **not authorization**. The existing coordinators still require
their original finite-purpose Operator/standing-policy authority, writer scope,
absolute and monotonic budget, lifecycle fences, and durable Service binding
state. The Machine independently checks its writer/private export policy and
namespace CAS. No preview, reconnect, saved receipt or resolver can renew those
grants. No new writer is enabled and legacy production export is unchanged.

Revocation, including restoration to no selection, does not require the removed
Plugin's Catalog entry or installation lease. Read-only observation of the exact
original operation remains possible on its original connection after a port
ends; it neither dispatches nor revives a grant. Missing/lost post-enqueue
receipts remain uncertain, not permission to retry or claim an inverse effect.
Already emitted telemetry remains `NoRestore`.

## Verification and remaining work

Tests use temporary signed packages, disposable Machine/store/catalog state,
JSON protocol frames and a loopback HTTP receiver. They cover the observed
away-and-back regression for nine installation discontinuities, per-slot
isolation, forged release/fingerprint claims despite matching inventory,
duplicate slots, Catalog refresh/removal/restoration, exact input and purpose
binding, legacy/OTLP separation, all three signals, and the final enqueue fence.
Existing finite select/revoke/restore, admission, lost-ACK, recovery and delivery
tests remain required, as does immutable connected-process conformance before
Controller activation.

No Machine protocol, signed Plugin/component version, SQL migration, durable
journal encoding or production policy changes. This is a Controller-only runtime
change. Publication, activation and the actual artifact-role acceptance must be
recorded separately; source tests alone do not establish production acceptance.

General graph-to-verified-contract resolution, Service/Workspace/Session scope
identity and general state-dataset compatibility/leases remain in the
[completion ledger](plugin-refactor-completion.md). This port lease is only an
in-process observation fence, not a durable state lease or exclusive state writer.
Independent post-effect Plugin/native recovery and real account/device/managed
policy cutover remain separate exits.
