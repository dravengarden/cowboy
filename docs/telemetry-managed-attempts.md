# Managed telemetry: one bounded OTLP attempt

Protocol 16 adds a **single-attempt** managed export path. It does not enable
binding writes, add an HTTP mutation endpoint, activate a background exporter,
or restore authorization on startup. Production continues using its explicitly
configured legacy export path while both managed binding admission gates stay
closed. Local rotating files and durable incident persistence remain independent.

## Separate authority and evidence

`ExportAttempt` is a closed schema: exact Service/Machine owners, attempt identity,
complete binding revision/policy epoch/installation incarnation, one standard
OTLP signal and bounded protobuf batch, and an expiry. Its canonical digest
binds every field. An installation receipt, `Completed` binding operation, or
serialized request cannot construct `TelemetryExportAuthority` or either Site's
non-serializable execution scope.

This slice requires a **fresh independent Operator confirmation for one attempt**.
It is deliberately not a production authorization model for continuously sending
background telemetry: scoped background policy admission and explicit restart
activation still need design/acceptance. A binding-mutation confirmation cannot
be converted to export authority. Neither exporter startup nor recovery creates
that confirmation from durable history.

Service admission checks the original credential/current Operator, its original
15-second maximum monotonic budget, the current validated SQL binding with no
unresolved operation, the accepted signed Catalog release, lifecycle fences, and
the original authenticated Machine connection. Exact active installation and
protocol 16 are checked again atomically with RPC enqueue. A same-epoch-string
replacement connection is not the original connection. Older peers receive no
legacy fallback. The scope is consumed even on failure; no automatic retry or
read-and-resend path exists.

The Machine captures its owner, immutable request and at-most-15-second budget
before validation/scheduling/lock acquisition. Before preparing the export and
again immediately before HTTP admission, under its lifecycle lock it checks:

- Exact durable binding owner, revision, policy epoch and installed incarnation;
  no unresolved Machine operation or Plugin lifecycle fence.
- Private, owned, bounded, checksummed journal bytes still match the owner's
  current ledger. Missing/tampered evidence poisons this reader; replacing its
  bytes cannot revive an already rejected attempt.
- Exact active signed package/contract and activation observation, plus the
  original opened private policy's inode, metadata and digest. An attempt cannot
  adopt a replacement endpoint or bearer token.
- Its original connection and budget still admit the effect.

The lifecycle lock is released at attempt admission, before network I/O. A
Service check is admission of **one bounded attempt**, not an instantaneous
cross-network revocation barrier. Already-admitted HTTP may finish after a
disconnect, revocation or uninstall; it cannot be reverted. Any later attempt
needs new authorization. The Machine's ongoing checks do not claim to observe
Service-side credential revocation synchronously across the network.

## Transport and outcomes

`ExportBoundTelemetry`/`TelemetryExported` is distinct from legacy host execution,
binding mutation/query replies and generic command acknowledgements. Exact digest
and original connection correlation are required. Late replies never enter the
ordinary Machine event history. Request Debug output contains signal and encoded
length, not protobuf bodies; receipts contain no endpoint, token, response body,
or exception text.

One request carries one OTLP logs, metrics or traces batch. The existing signed
schema-two routes, protobuf validation, no-proxy/no-redirect policy, three-second
HTTP timeout, and bounded partial-success parser are reused. **This path makes
one HTTP attempt, including on transport errors, 429 and 5xx.** It never retries
an entire partially accepted batch or downgrades its encoding. Existing explicitly
configured legacy export keeps its two-attempt behavior unchanged.

Outcomes are closed: `Delivered`, `Partial` (bounded rejected-item count),
`Disabled` (private lane off), `NotAdmitted` (no HTTP admitted), or `Unknown`
(admitted but delivery not established). Missing/mismatched RPC responses and
timeouts cannot prove no emission. Attempt IDs/digests correlate evidence, not
external exactly-once delivery or durable replay deduplication. No telemetry
payload, per-batch replay instruction or new grant is persisted in either ledger.

## Acceptance and remaining P2 work

Hermetic tests use a real temporary signed Victoria installation, accepted
Catalog, real Operator checks, SQL binding coordinator, JSON protocol frames,
the actual CLI dispatcher, and loopback HTTP. They cover official client protobuf
fixtures for all three signals, partial success, no retries/redirects, missing
and changed binding state, credential/fence/installation/connection changes,
expired or disconnected queued calls, old protocol rejection, exact receipts,
and independent persistence with unchanged binding evidence.

Still required before production managed activation: accepted live/rollback/cold
reader floors on both Sites, explicit writer and background-export admission,
Machine-side unresolved-record handling and cross-end restart/recovery
acceptance. This finite path is not P2 completion or a generic executable Plugin
DAG. Signed Plugin/SDK bytes, native ABI, worker generation inputs and Provider
authentication/installations are unchanged by this slice.

The subsequent [Service resolution slice](telemetry-binding-resolution.md) stages
freshly authorized local abort or acceptance of definite Machine observations.
It does not enable the writers, repair unresolved Machine evidence, or create
export authority from a resolution audit.

The later [Machine recovery slice](telemetry-machine-recovery.md) adds a finite
closure for validated reopened Prepared evidence, retaining both the namespace
and Service fence. Unknown/corrupt evidence stays isolated; the production
reader-floor, confirmation-surface and policy-admission requirements remain.
