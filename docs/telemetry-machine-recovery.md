# Independently authorized Machine binding recovery

Status: twenty-sixth spatiotemporal slice, 2026-09-12. This stages one finite
core bookkeeping action, `RejectInterruptedPrepared`, and protocol **17**.
**Production binding, Service resolution and Machine recovery admission remain
closed.** No HTTP confirmation surface, automatic restart recovery, background
policy, Plugin release or new export grant is introduced. This is not P2
production activation or a generic executable DAG.

## What can be concluded after reopen

Machine binding head and receipt share one atomic, checksummed file under the
exclusive Plugin journal owner. An in-memory failed owner cannot know whether a
replacement reached disk; it stays poisoned. Only a new owner that successfully
validates the retained file can establish a process-local reopen proof.

If that owner reads the exact schema-two **binding step** still `Prepared`, the
original binding execution lease cannot survive that reopen. A fresh authorized
recovery may close that attempt as `Rejected(AuthorizationEnded)`. It preserves
the expected head, revision, policy epoch and managed namespace. This is **not**
a claim that no effect ever happened: the namespace and durable intent already
exist, and neither is removed. An `Unknown` receipt, historical schema-one step,
active/unreopened Prepared, missing/malformed journal, changed head or poisoned
live owner is ineligible and remains fenced. Rewriting bytes is not recovery.

## Two independent confirmations

1. A new Operator confirms Machine-only closure of the exact original Service
   `NeedsAttention` operation and exact Machine Prepared observation. Core pins
   the actual confirming credential, complete Service operation digest, both
   owners, original full step, new actor, resolution ID, action and new expiry.
   The old binding deadline may be expired; it is evidence, not renewed authority.
2. Core captures the authenticated protocol-17 Machine connection. It first
   queries this exact recovery and rechecks current Operator, original complete
   Service operation and original connection before one recovery dispatch.
3. The Machine captures a purpose-specific non-serializable, non-cloneable lease
   synchronously before detached queueing. A separately closed recovery gate,
   validated reopen proof, exact Prepared CAS and original connection/deadline
   checks admit one atomic receipt/audit replacement. No Plugin installation,
   Catalog or private routing policy is needed for this bookkeeping action.
4. The original Service operation remains **unchanged NeedsAttention**, even
   after a definite Machine recovery. A separate, freshly confirmed
   [Service `RecordRejected` resolution](telemetry-binding-resolution.md) must
   query the Machine again before updating Service bookkeeping. Neither step
   creates an export grant or reopens legacy fallback.

Each new confirmation has at most one minute from its original receipt, including
queueing. Budgets and observed revocation are not renewed by retries or repair.
The control RPC uses distinct query/commit reply kinds and the exact original
connection incarnation, not just a reused epoch string. Protocols 1–16 cannot
receive either new command; there is no fallback to ordinary binding mutation.
Uncertain acknowledgement permits one query of the exact recovery, never a
resend of recovery or of the old binding. Definite rejection does not trigger
that query. History is read-only and rechecked against current authorization.

These are admission checks, not hard synchronous-I/O deadlines or atomic
cross-store revocation. An already admitted Machine replacement can finish
after the Service times out or loses authority. Its durable audit remains
queryable; the Service stays fenced until another fresh confirmation. There is
no separate durable Service recovery-attempt log in this slice: Machine owns
this local mutation and audit; the original Service intent already persists.

## Atomic audit and reader floor

The repository-owned [populated reader conformance](telemetry-reader-conformance.md)
executes actual immutable Controller/Machine releases for this boundary. Its
supplied artifact matrix is distinct from acceptance of the host's live,
effective rollback and cold-profile configuration.

One replacement stores the old terminal binding receipt and the new complete
recovery audit together, then fsyncs the directory. Failure before rename or
after rename/before directory flush poisons the running owner and returns
uncertainty. After validated reopen, evidence is either the original Prepared
or the complete rejection plus audit. No cached partial success is reported.
Competing fresh IDs can close a given Prepared only once. Identical historical
requests are reads; a reused ID with different fields conflicts.

The Machine ledger gains a bounded `resolutions` vector. **Ledger schema one**
retains its canonical bytes and requires an empty vector. The first recovery
writes **ledger schema two** with a nonempty audit. Readers verify complete
request/receipt linkage, owner, outcome, timestamp, uniqueness and ordering
against the original receipt chain, even with a recomputed outer checksum.
The ledger remains bounded to 4 MiB and 1,024 binding receipts; each audit is
bounded to 64 KiB. New binding admission reserves recovery space before writing
Prepared. Existing near-capacity ledgers may safely refuse recovery; history
is never pruned to make room. New ordinary binding operations preserve earlier
audits and require managed-namespace CAS plus fresh ordinary execution authority.

Ordinary Machine queries, duplicate binding requests, legacy preflight and
managed export also now re-read and validate the owned file against the cached
ledger. Missing, corrupt or out-of-band replacement evidence permanently
poisons that running owner. Restoring bytes cannot silently revive cached trust.
This is corruption detection under exclusive ownership, not a substitute for
OS protection against another process with the same account authority.

Before schema-two production Machine writes, **active, rollback and cold Machine
readers** must accept populated audits. Controller readers likewise need the
existing Service schema-two floor. Neither deleting a namespace nor discarding
audits is a valid downgrade. The staged reader rollout does not enable writes,
change SQL migrations or migrate existing legacy export policy.

## Verification and remaining work

Hermetic tests cover closed codecs/purpose binding, real cookie revocation,
expired/queued/disconnected authority, exact operation changes, protocol floors,
reply correlation, same-epoch reconnect, single-query lost ACK, concurrent CAS,
rename/fsync failures, disk corruption, sticky poison, audit validation and
retained history after later binding changes. A real temporary signed Victoria
installation is interrupted by the actual Machine writer, reopened, and closed
through JSON frames and CLI admission before a separately confirmed Service
resolution. No real destination or production credential is used.

Still required for P2: accepted populated active/rollback/cold reader floors on
both Sites, user-facing finite confirmation surfaces, explicit writer and
background-policy admission, and full production cross-end failure/restart
acceptance. Unknown/schema-one evidence intentionally remains quarantined and
needs a separately justified treatment, not a universal “repair” button. Already
emitted OTel is `NoRestore`; this slice adds no external inverse or automatic
compensation. Provider, native ABI, signed Plugin/SDK and worker inputs are unchanged.

The [reader-only release receipt](releases/telemetry-machine-recovery-2026-09-12.md)
records the separately accepted Controller/Machine transactions, protocol 17,
unchanged workers/Web and still-closed production admission.
