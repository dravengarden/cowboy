# Core telemetry resolution confirmation

Status: 2026-09-13. Settings → Info now exposes Service binding evidence and a
finite resolution preview through the ordinary Product Operator account. It does
not require a separate admin account. The same core `ConfirmSheet` serves
Desktop and Mobile. **Production Service resolution admission remains closed**;
the deployed surface permits inspection, not confirmation. This is not P2 exit.

The [release receipt](releases/telemetry-resolution-surface-2026-09-13.md)
records the separate Controller/Web transactions, actual reader checks and
session continuity boundaries.

## Exact preview, fresh confirmation

The core, not a Plugin, owns these Product/admin Operator routes:

```text
GET  /api/telemetry/binding
POST /api/telemetry/binding/operations/{operation}/resolution-plan
POST /api/telemetry/binding/operations/{operation}/resolve
GET  /api/telemetry/binding/operations/{operation}/resolution
```

Preview accepts exactly `{}`. It reads the validated last Service operation. A
Prepared operation may preview `abort_before_dispatch` without a Machine. For
Dispatching/NeedsAttention, core queries the exact original Machine step on one
captured authenticated connection and chooses `accept_applied` or
`record_rejected` only from definite, current, correlated evidence. Old Applied
history, a later head, unresolved/unknown/unavailable evidence and another
operation remain fenced. Preview is not a resolution or an export grant.

At most 256 process-local, one-use previews survive for two minutes, including
authentication/storage/query time. The Machine preview query is capped at ten
seconds and the original preview budget. The stored intent binds the actual
previewing Operator, complete original operation digest, both owners, closed
action, exact observation digest, new resolution ID and original expiry. Another
actor/target/action cannot consume that preview. Restart discards previews.

Confirmation accepts only `{ "plan_id": "…", "action": "…" }`, not an Actor,
replacement operation, Machine command, force flag or serialized authority. Core
captures the NEW actual credential and its one-minute budget. That budget is
intersected with the original preview's monotonic deadline, wall high-water mark
and sticky expiry; confirmation cannot mint a new preview lifetime. Consumption
is atomic and failures after admission cannot put the plan back.

The existing [Service resolution coordinator](telemetry-binding-resolution.md)
then independently repeats full-operation CAS, current credential checks and,
when needed, one fresh read-only Machine query on the confirmation's original
connection. It changes only atomic Service bookkeeping and audit. It never
repeats the old binding, repairs the Machine, installs a Plugin, restarts
workers, restores credentials, reopens legacy fallback or grants export. Already
emitted OTel remains `NoRestore`. Admission checkpoints are not hard
synchronous-I/O timeouts or atomic cross-store revocation barriers.

## Cancellation, evidence and presentation

HTTP observation cancellation does not cancel an admitted resolution task. After
an uncertain response the Web client may make **one GET of the receipt**, never
another POST. Only the exact new resolution ID, original complete digest,
Machine, operation, action and terminal phase can conclude that attempt. Missing
or different receipts remain unverified. Closing the Info surface or signing out
cancels both initial requests and result inspection; stale responses are
ignored. The UI claims submission synchronously so rapid clicks cannot repeat
the POST.

All successful and application-error responses are bounded closed projections
with `no-store`. Request bodies are capped at 1 KiB; JSON/path extractor
failures are normalized without echoing submitted fields. No Actor, credential,
endpoint, private policy, raw observation, exception or authority is exposed.
Rust and Web share a checked public fixture; Web additionally rejects unknown
fields, malformed digests, incompatible phases and noncanonical counters.
Revision/epoch axes stay decimal u64 strings even above JavaScript's
safe-integer range. Response bodies are bounded to 64 KiB, with no automatic
request retries.

Absent Service evidence does **not** mean remote export is disabled: explicit
legacy configuration may still be active. Any retained journal continues to
fence legacy fallback; neither a terminal audit nor a read establishes an export
grant. UI copy states these distinctions and shows production admission closed
without an actionable confirmation button.

## Verification and remaining work

Temporary SQLite/HTTP tests exercise the real handlers, credential resolver,
Operator continuation, coordinator, receipt projections and connection-bound
Machine query. They cover local offline abort, one-use confirmation, changed
operation/remote evidence, logout/disable/role loss, strict bodies, preview
bounds and the closed production gate. Shared Rust/Web fixtures and Web failure
tests cover receipt correlation, lost-response inspection, nonrenewing clocks,
bounded responses and no POST replay. Existing signed-Machine, PostgreSQL and
recovery tests remain required by the complete repository gate. These are not
physical iPhone interaction or production write/fault-injection acceptance.

Unchanged: signed Plugin/SDK payloads, Machine protocol, durable
schema/migrations, native ABI, private destinations and Provider state. Release
Controller and Web independently; no Machine/worker or host-system activation is
needed. The accepted
[Hawk populated reader floor](releases/telemetry-reader-floors-2026-09-13.md) is
a separate prerequisite, not authority to open writes.

Still required: ordinary binding select/revoke/restore confirmation, the
distinct Machine recovery confirmation, per-target production writer and
background-export policy admission, and production cross-end failure/restart
acceptance. Unknown and schema-one Machine evidence remain quarantined. This
surface is not a generic executable DAG, automatic compensation or a universal
repair interface.
