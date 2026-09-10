# Plugin Service continuation authority

Status: ninth spatiotemporal slice, 2026-09-10. Confirmed uninstall now retains
a core-only reference to its actual confirming credential and rechecks current
Operator authority at each Service effect boundary. This hardens the existing
finite coordinator; it does not grant independent recovery or enable durable
compensation.

## An actor is evidence, not an authenticated continuation

Previously confirmation checked the actor once and detached execution from the
HTTP observer. The durable intent retained the actor's identity, but not a way
to recheck the confirming login after preflight, database waits or remote
commands. Logout, device/token revocation or downgrade could leave delayed
operations using an earlier role snapshot.

`OperatorApproval` is captured only from the existing authenticated request
path. It preserves middleware precedence: admin cookie first, otherwise the
verified Product cookie, personal token or sender-constrained device identity.
Explicit auth-off mode keeps its existing local identity; it is never a fallback
for a revoked authenticated continuation. Automation scopes do not authorize
Plugin mutations.

The detached task retains only the credential hash and minimal lookup identity,
never bearer secrets, whole headers, OIDC token material or DPoP proofs. A device
continuation rechecks the same access-token registry entry without consuming its
already-verified nonce again. Hash-based helpers are crate-private continuation
checks, not public authentication inputs.

After validation, approval binds to the complete canonical uninstall request
digest, including Service, actor, target, installation revision, impact and
deadlines. Neither approval nor bound authority implements Clone, Debug,
Serialize or Deserialize. Loading an Actor or receipt cannot construct either.
Observed authorization failure permanently closes that instance; another login
or later role promotion cannot revive it.

## Effect checkpoints and time

The coordinator rechecks the original credential's validity, user/account
status, Operator role and applicable Product-session freshness policy, plus
exact Catalog trust and the original Machine connection/transport. No parallel
user, session, credential or policy system is introduced.

Checks precede durable admission, the first worker-stop phase and each worker
stop. They follow persisted remote intent before sending uninstall, and the
Machine result before session deletion. Existing legacy compensation checks
again after its durable phase and before reactivation or each worker reload.
It cannot use expired/revoked forward approval. The leased durable path still
refuses that legacy inverse.

Service and Machine share a core `OperationBudget`, not an installable Plugin.
Service time starts at confirmation credential capture, before scheduling,
validation or DB waits, and is capped by both the original preview deadline and
five process-monotonic minutes. Machine admission retains its one-minute maximum.
Binding and checking never renew time. Observed clock rollback, invalid initial
time and expiry remain sticky; the budget is checked again after validation.
There is no portable suspend-inclusive, offline, cross-restart or hard
blocking-syscall timeout claim.

| Authority loss window | Result |
| --- | --- |
| Before Service intent | No durable operation or worker/Plugin effect |
| Prepared, before effects | Existing `aborted/preconditions_changed` result |
| After stopping began, before another effect | `needs_attention`; retain partial progress and slot fence |
| After Machine uninstall, before session deletion | Retain Machine evidence; no session deletion or invented inverse |
| During legacy compensation | Preserve primary cause and `compensation_failed`; no claimed worker recovery |

Closing the HTTP observer is not logout and still does not cancel an admitted
operation. Recording actual results or uncertainty remains allowed after
authority loss. These are local point-in-time checks across existing security
stores, not an atomic distributed policy epoch or instantaneous remote
revocation. An already-admitted local transaction may finish, and a command
already dispatched may finish under its Machine lease. Database-lock waits are
not an atomic credential/transaction fence. Existing detached Agent sessions and
Provider-auth generations are unchanged.

## Verification and rollout

Hermetic tests use actual SQLite-backed users, cookies, personal tokens and
freshness rules, plus real temporary device proof keys. They cover logout,
downgrade, disabled users, expiry, credential precedence, nonce replay, complete
intent/Service binding and delayed admission. Coordinator tests deny authority
at each phase and between worker stops, preserving partial progress and primary
and compensation failures. Existing clock/Machine tests exercise the shared
budget implementation.

There is no new wire version, SQL migration, journal field/enum, Plugin SDK,
Catalog package, Web/native release or host policy. Existing readers understand
all results. Independently build and activate the affected Machine and Controller
components; preserve worker generation and installation records. Production
logout, revocation, uninstall or clock fault injection is not release smoke.

Still missing: independent post-restart recovery approval, Provider-auth/policy
epoch checks for recovery, journaled restoration with tombstone CAS, verified
worker/native-session recovery, operator recovery actions, evidence archival,
Victoria binding lifecycle and the generic finite executor. Recovery assessment
still reports four `not_verified` requirements and grants no restoration.
