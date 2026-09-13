# Core Machine recovery confirmation surface

Status: 2026-09-13. Settings → Info exposes a separate Machine interruption
preview for the exact latest Service `NeedsAttention` operation. It uses the
ordinary Product Operator and core `ConfirmSheet`, not an admin-only page or
Plugin-rendered authority. **Production Machine recovery and all managed
binding/export writer admission remain closed.** This is not P2 exit.

## One finite purpose, two independent confirmations

The protected core routes are:

```text
POST /api/telemetry/binding/operations/{operation}/machine-recovery-plan
POST /api/telemetry/binding/operations/{operation}/recover-machine
GET  /api/telemetry/binding/operations/{operation}/machine-recoveries/{resolution}
```

Preview accepts exactly `{}`. Core reads the validated latest Service operation,
constructs the exact schema-two Machine step and queries protocol 17 on one
captured connection. Only the complete expected `Prepared` observation with no
existing audit can produce a preview. Wrong operations, old schema/protocol,
unknown, unavailable or changed evidence are refused. The actual Operator,
original Service operation, connection and budget are rechecked after the query.
No Machine mutation or Service journal write occurs.

A Prepared query is **not** proof that the Machine has reopened its journal.
Only the Machine's existing admission can verify that process-local reopen proof
and that the original attempt is no longer live. The dialog states this; it does
not promise that a preview will be admitted.

Confirmation accepts only `{plan_id, action}`, with the sole Machine action
`reject_interrupted_prepared`. It captures a fresh actual Operator credential
and atomically consumes the preview budget. The purpose-specific authority
rechecks the complete Service operation and original confirmation connection
before the existing finite coordinator sends at most one recovery command. The
Machine independently checks its closed admission, validated reopen, exact
Prepared/head CAS, connection and deadline before its atomic receipt and audit
replacement.

The Machine rejects that interrupted attempt with `AuthorizationEnded`. Its
binding head, revision, policy epoch and managed namespace remain intact. The
Service operation and fence are not modified. A separate fresh
[Service resolution](telemetry-resolution-surface.md) must query the definite
rejection again before updating Service bookkeeping. No automatic second
confirmation, original binding replay, installation, worker restart, credential
restoration or export grant is chained to this action. Already emitted OTel
remains `NoRestore`.

## Deadlines, cancellation and bounded query handles

The preview's single minute starts before authentication, storage and remote
query work. Its exact wall deadline crosses the wire unchanged; confirmation
intersects the consumed original budget with its own fresh one-minute Operator
budget, without renewing either monotonic clock or sticky expiry. Preview and
HTTP receipt queries have at most ten seconds of remote observation within their
original budgets. These are admission/async query bounds, not hard
synchronous-I/O deadlines or atomic distributed revocation.

At most 256 process-local records retain exact requests. An unsubmitted expired
preview can be evicted; a submitted record retains only query data, never the
consumed authority, for at most one hour from insertion. Repeated confirmation,
another actor/Service/operation and purpose substitution are refused. A failed
admitted task does not put its budget back. Full capacity refuses new previews,
rather than deleting durable evidence.

HTTP observer cancellation detaches an admitted task. An ambiguous transport
result permits the coordinator's one exact read, not another recovery command.
After an uncertain HTTP response, Web may make one GET for the exact plan ID.
The public result binds the full recovery request digest, Service operation
digest, both IDs, Machine, action and audit time. Missing or different evidence
never becomes success. Receipt reads use fresh Operator authorization and a
fresh read-only connection, even if the original recovery deadline has ended;
they cannot create an execution lease.

These HTTP handles are deliberately **not durable recovery history**. Controller
restart or the retention limit can make GET return not-found even when Machine
recovery committed. The Machine audit is not deleted. The UI must report
unverified, never resend or infer failure from handle absence. A new independent
Service preview can inspect the current binding evidence. Persistent HTTP audit
discovery remains a separate gap; this slice does not silently introduce a new
Service writer/schema solely to retain query handles.

Closing the Info surface, refreshing its operation scope or signing out aborts
observation and ignores late responses. A synchronous submitted-ID guard
prevents rapid clicks from repeating POST before React renders busy. Core
Mobile/Desktop confirmation remains separate from the Service dialog.

## Closed contracts and verification

Core surfaces share authentication dependencies and Web decoding/transport
primitives, but keep separate plan registries, action types, authorities and
production gates. Bodies are limited to 1 KiB, parser errors use a closed code
and responses are no-store. No Actor, raw step/observation, endpoint, policy,
credential, exception or serialized authority is exposed. Rust and Web compare
the same public JSON fixture; Web rejects unknown fields and preserves u64
revision/epoch axes as canonical decimal strings.

Hermetic HTTP tests cover exact previews, concurrent one-use confirmation,
ambiguous receipts, changed remote and Service evidence, expiry/capacity, actual
cookie logout/role loss/disabled accounts, strict bodies, purpose substitution,
history reads and simulated query-handle loss. Existing signed Victoria
interruption/reopen, wire and separate Service resolution fixtures remain part
of the full gate. They are not physical-device interaction or production
fault-injection acceptance.

No Machine protocol, durable schema, SQL migration, signed Plugin/SDK, Provider,
native ABI, private telemetry policy or worker input changes. Controller and Web
release independently; no Machine or host-system activation is needed. Actual
active/rollback/cold reader conformance remains a separate release prerequisite.

Remaining P2: ordinary select/revoke/restore confirmation, durable HTTP recovery
audit discovery, per-target writer/background-policy admission and full
production cross-end failure/restart acceptance. Unknown/schema-one Machine
evidence stays quarantined; this is not a universal repair button or executable
DAG.
