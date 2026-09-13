# Core telemetry binding confirmation

Status: 2026-09-13. Settings → Info now stages ordinary selection, revocation
and exact restoration alongside the separate Service resolution and Machine
recovery surfaces. Core owns this UI and HTTP boundary. **Production managed
binding, recovery and background-export admission remain closed.** This slice
does not complete P2 or publish a new Plugin/SDK.

The [release receipt](releases/telemetry-binding-surface-2026-09-13.md) records
the actual Controller/Web activations, populated reader checks and session
continuity boundary.

## Closed target discovery and preview

```text
GET  /api/telemetry/binding/choices
POST /api/telemetry/binding/plan
POST /api/telemetry/binding/confirm
GET  /api/telemetry/binding/operations/{operation}/receipt
```

All routes require the actual Product/admin Operator through the existing core
authentication, freshness and origin middleware. There is no separate admin UI
or Plugin-rendered confirmation authority. Bodies are limited to 1 KiB, errors
are closed categories, and responses are bounded projections with `no-store`.
No Actor, credential, endpoint, policy bytes, raw observation or serialized
authority is exposed. A malformed body is refused without echoing its fields.

Choices contain at most 64 exact installed telemetry targets. Core intersects
the accepted signed Catalog with the currently connected Machine inventory,
installation incarnation, release/contract and lifecycle fence. Older peers
cannot supply namespace-aware targets. A retained Service slot cannot move to
another Machine. Unresolved Service evidence offers no ordinary mutation.
Choices are discovery data, never permission or proof of private policy.

Preview accepts one closed request:

- `select`: an exact `{machine_id, installation}` target from discovery;
- `revoke`: no additional fields; requires an existing managed selection;
- `restore`: the exact completed forward `operation_id` whose post-state still
  equals the current Service head.

Core derives the Service, actual actor, expected namespace/head, fresh operation
ID, next policy epoch and complete step digest. It validates slot ownership,
capacity and restoration provenance in memory before querying the Machine. One
query on the captured connection must show the exact expected namespace/head,
no unresolved attempt and no receipt for the new operation. Full Service
evidence, current Operator, target and budget are rechecked before publication.
No Service row, Machine namespace or export is created by preview.

Restoration chooses only the recorded prior installation or absence and advances
both counters. Removed/reinstalled targets and binding ABA are refused; it does
not reactivate a package, recover credentials or rewrite private policy.
Revocation/restoration to absence do not depend on an obsolete installation.
Only the Machine can validate its current private destination policy.

## One-use authority and durable receipt lookup

At most 256 process-local previews retain the exact request, a digest of the
complete Service before-evidence, original connection-bound transport and a
non-serializable time budget. A single minute starts before authentication,
storage and the preview query; the query is bounded to ten seconds inside that
minute. Expired previews may be evicted. Restart discards unsubmitted previews;
capacity does not delete durable journals.

Confirmation accepts only `{plan_id, action}` and captures a NEW actual Operator
credential. Atomic consumption checks the preview actor, Service, purpose and
original expiry. Its budget is intersected with the fresh confirmation budget;
neither the wall deadline nor monotonic lifetime can be renewed. The retained
transport cannot adopt a replacement connection. A failed consumed plan is never
put back.

The existing finite [Service coordinator](telemetry-service-coordination.md)
revalidates evidence and authority, closes the **same** running legacy-export
fence, persists Prepared/Dispatching and makes at most one Machine mutation.
Ambiguous acknowledgment permits one read of the original complete step, not
replay, a new ID, installation or inverse command. Machine-local admission,
private policy and its own durable CAS remain independently required. Admission
checks do not claim atomic distributed revocation or hard synchronous-I/O bounds.

An HTTP observer can disconnect without canceling the admitted coordinator.
After an uncertain response, Web may make one GET of the exact operation. This
route reads the existing validated Service journal, not a process-local plan
handle; it works independently of preview eviction and across Controller reopen.
It does not query or mutate the Machine. The public receipt binds the complete
request digest, operation, Machine, expected head, change and recorded phase.
It is historical evidence, not a statement that a later current head is unchanged.
Prepared/Dispatching/NeedsAttention remain unverified; absent evidence does not
prove failure. No automatic retry or chained recovery/resolution occurs.

## UI and verification

Core `ConfirmSheet` serves Desktop and Mobile. Opening choices and previews is
explicit, no target is silently selected, and no actionable confirmation appears
while admission is closed. Parent evidence scopes, logout/unmount cancellation,
late-response guards and synchronous submitted-ID ownership prevent UI replay.
Revision/epoch values remain decimal u64 strings, including above JS precision.
Web rejects unknown fields, wrong-purpose or foreign receipts, counter reuse,
incorrect post-states and responses above 64 KiB.

The dialog states that the first durable intent/namespace closes legacy export
admission even if the requested selection later aborts or is rejected. Neither
restoration nor resolution automatically removes that fence. Local rotating
files remain independent. A completed binding grants no background export;
already emitted OTel is `NoRestore`.

Tests drive real HTTP authentication/handlers, the existing coordinator,
temporary SQLite, signed Victoria Catalog/installation, JSON Machine frames and
the actual finite Machine writer. They cover both restoration directions,
concurrent one-use confirmation, exact targets, expiry, Service CAS, connection
replacement, bounded registries, strict bodies, cookie logout/role loss/disable,
and an HTTP observer lost before a real dropped Machine ACK. Durable GET is
verified after later head changes and preview-registry loss without remote calls.
Rust/Web share a checked fixture. These are not physical-device or production
fault-injection/restart acceptance.

No Machine protocol, journal schema, SQL migration, signed Plugin/SDK, Provider,
native ABI, worker generation or private destination policy changes. Release
Controller and Web independently through the component activator, preserving
Machine/workers and checking actual populated active/rollback/cold readers.

Remaining P2: durable HTTP **Machine recovery audit discovery** (distinct from
these durable ordinary operation receipts), per-target production writer and
background-export policy admission, and cross-end production failure/restart
acceptance. Unknown and schema-one Machine recovery evidence remain quarantined.
