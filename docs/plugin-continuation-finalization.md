# Core Code continuation finalization

This Controller-only extension closes response and time boundaries for owned
reads, synchronization and navigation. It builds on
[original product permission lifetimes](plugin-product-permission-lifetimes.md)
and [ordinary buffer outcome authority](plugin-controller-buffer-owners.md).
It is neither a Plugin capability nor a generic effect/recovery executor.

## Original authority on every completed outcome

An owned job and its HTTP observer share the same original product approval.
The observer retains that approval independently of the job's success value.
Saved observations, malformed or missing replies, failed support probes and
other completed failures must cross the same current-original-user check as
successful results. A revoked login or ended permission observation receives
`401/no-store` with no ETag or result body; it cannot fall back to another
credential. Valid authority still sees the appropriate closed failure status.
Successful live operations retain their additional original Session, connection
and purpose checks. Terminal history does not acquire a new live binding.

The actual owner records accepted native effects before checking disclosure.
Failure authorization never rewrites that record, clears an uncertain fence,
rearms Apply/Execute/Open, releases an owner or retries an operation. A fresh
original-user request may explicitly query the same retained ID. Withholding a
response is not native undo, principal continuity across restart or recovery.

## One original absolute deadline

The original 60-second monotonic request budget spans permission checks,
admission, the owned task and response delivery. Every dispatch explicitly
checks that deadline after asynchronous authority checks, before consuming a
one-use attempt. A ready future cannot start work merely because the timeout
timer was polled second. Ownership handoff does not create a fresh deadline.

Owned reads also apply that same deadline inside their task, across both the
support and read waits. HTTP cancellation does not abandon an unbounded task
or renew its budget. At expiry the Controller drops its finite read borrow and
does not change the last native owner observation. This is not proof that an
already-dispatched Machine/native read has stopped: those owners keep their
independent exclusion and transport rules. No new native cancellation, cleanup
acknowledgement or background retry is introduced.

## Acceptance

Source tests cover valid and invalid replies under original token/role loss and
unpolled role ABA, retained Unknown/ReleaseUnknown/destination evidence, expired
pre-dispatch synchronization/navigation/read jobs with no command or consumed
attempt, and HTTP cancellation across two native waits under one deadline.
The paused-clock test advances the test clock only; production timeouts are
unchanged. Existing valid-effect preservation and consumer tests remain required.

The [connected gate](plugin-code-connected-conformance.md) extends to v11/31
checks. It uses disposable product login/logout and a byte-preserving relay:
hold an actual owned read, synchronization Query or navigation Query reply,
revoke only that login, discard just the reply and wait the normal 40-second
transport timeout. Require `401/no-store/no-ETag`, no further dispatch from the
revoked login and explicit original-ID observation by the independent login.
The synchronization/navigation Queries follow the existing lost Apply/Execute
cases: their effect remains Unknown until the independent Query observes it.
No reply is fabricated, no live policy/database/credential is modified, and
earlier cancellation, lost reply, uninstall and restart checks remain intact.

This changes no wire, SDK, Plugin/native executable, durable format, public
permission API, navigation admission or Machine maintenance contract. General
DAG/state leases, Machine-owned identity, global background budgets, supported
devices and independently authorized post-effect restoration remain separate.
