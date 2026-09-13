# Durable Machine recovery audit discovery

Status: 2026-09-13. Core Settings → Info can explicitly inspect recorded Machine
recovery without a retained confirmation preview. Machine is the sole audit
owner; Service keeps only its existing binding operations and resolution
records. **All production binding, recovery, resolution and managed-export
writer admission remains closed.** This is not P2 exit.

The [Hawk release receipt](releases/telemetry-recovery-audit-discovery-2026-09-13.md)
records the accepted Controller/Machine/Web transactions, reader floors and
worker continuity.

## Separate read purpose

Protocol 18 adds `QueryTelemetryRecoveryAudit` and its distinct correlated
reply. Its closed schema-one query contains only the complete original
schema-two `BindingStep`. A separate digest binds every query field. There is
no new Actor, deadline, resolution ID, recovery request or execution lease to
reconstruct. An expired original step is historical evidence, not authority.

The Machine takes its bounded lifecycle lock, checks the Plugin journal fence,
rereads the exact retained binding file and applies sticky poisoning on evidence
loss/change. Its validated ledger proves the unique recovery audit and links it
to the complete original binding receipt. The query returns that historical
audit separately from the current binding head, including after later bindings.
Neither read, reconnect nor reopen creates a namespace, changes policy, clears
a fence, installs a Plugin or dispatches a recovery/export command.

Only a verified observation can contain `receipt: null`. Unsupported protocol,
wrong owner/step, corrupt/missing retained evidence, timeout and a replaced
connection are unavailable, never interpreted as absence or success. Even a
verified absence describes one read, not proof that a prior action failed or
permission to retry it.

## Core HTTP and Web

```text
GET /api/telemetry/binding/operations/{operation}/machine-recovery-audit
```

Core derives the complete step from its validated Service ledger; the caller
cannot supply a digest, Actor or raw Machine query. The operation can be
historical, not just the latest head. Actual Product/admin Operator access is
required before and after remote observation. One ten-second budget bounds the
async read, including authentication and database access. The original captured
Machine connection is rechecked before returning. Synchronous filesystem I/O
is not a hard-real-time deadline or atomic distributed revocation.

The closed public projection contains the current retained Service operation
and either `recovery: null` or `{before, receipt}`. If Service has since resolved
the operation, its resolution's **original `NeedsAttention` before** must match
the Machine audit's complete `service_operation_digest`; the new terminal
operation digest cannot stand in for it. A Service rejection must also retain
the exact Machine binding receipt. Conflicting conclusions fail closed. No
Actor, raw step/observation, destination, credential or serialized authority is
exposed; responses are no-store and Web decoding remains bounded to 64 KiB.

The existing exact receipt GET (`machine-recoveries/{resolution}`) retains its
protocol-17 path while a matching submitted handle exists. When that handle is
gone, it uses protocol-18 discovery and additionally requires the exact saved
resolution ID. This is a fresh Operator history read, not revival of the old
actor's confirmation. A missing audit does not verify an ambiguous submission.
Old Machines remain supported for their existing commands, but cannot satisfy
handle-free discovery; that route explicitly reports unavailable.

Web offers **Inspect recorded Machine recovery** for the displayed operation,
including its terminal Service state. This sends one GET, never an automatic
mutation, polling loop or POST retry. The original digest and current operation
are decoded independently and linked; if the parent operation changed, refresh
is required. Logout/unmount cancels observation and ignores late replies. The
separate interruption confirmation is available only for `NeedsAttention` and
continues to use the core `ConfirmSheet` and its original one-use budget.
Historical audit never selects a backend or acts as current export authority.

## Verification and rollout boundary

Hermetic tests cover strict codec/purpose separation, old protocol refusal,
wrong reply kind/digest, same-epoch connection replacement, observer timeout,
validated Machine reopen, sticky corruption, later binding heads, actual cookie
logout/role loss/account disable on both sides of a read, and a fresh Controller
HTTP owner with empty preview registries. The latter reads a reopened real
Machine ledger after separate Service resolution and a later Service operation;
SQL and Machine binding bytes remain unchanged. Rust/Web share a public JSON
fixture and validate finite ambiguous-result inspection without replay.

The [immutable populated reader gate](telemetry-reader-conformance.md) still
requires all actual active/next-rollback/cold readers and both cold reads. A
protocol-18 Machine must also answer audit discovery exactly; protocol-17
recovery floors continue to exercise their existing two queries. The private
schema-two conformance receipt records each successful Machine negotiation.
This does not replace signed Victoria/OTLP runtime tests or actual production
write/failure/restart acceptance.

No durable journal format, SQL migration, signed Plugin/SDK, native ABI,
Provider state or private telemetry policy changes. Controller, Machine and Web
have independent immutable releases. Protocol 18 requires the explicit Machine
maintenance lane; Controller/Web activation cannot recycle Machine or session
workers. Verify actual generation convergence, component receipts, cold floors
and running workers; do not assume application health proves all of these.

Remaining P2: explicit per-target production writer and background-export
policy admission, plus cross-end production write/failure/restart acceptance.
Unknown/schema-one recovery stays quarantined. There is no generic executable
DAG, automatic compensation or universal repair capability.
