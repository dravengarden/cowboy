# Controller-owned buffer continuations

Status: the [original candidate](releases/plugin-controller-buffer-owners-2026-09-15.md)
is included in the separately activated Controller `0fded719`; the
[six Agent publications](releases/agent-publication-2026-09-15.md) closed its
original prerequisite. The subsequent [owned Review consumer](plugin-review-owned-consumer.md)
and [working-diff consumer](plugin-review-owned-diff.md) use this ownership path;
their Web rollout and actual Machine/Zed and device acceptance remain separately
recorded. It consumes the independently verified
[Machine/native candidate](plugin-native-buffer-leases.md).
The additive [owned observation API](plugin-owned-buffer-reads.md) now implements
diagnostic and symbol reads; its rollout remains separately recorded.
The separate [synchronization API](plugin-service-buffer-sync.md) adds finite
original-owner preparation/confirmation, independently of ordinary buffer reads.
The [outcome-authority rollout](releases/plugin-buffer-outcome-authority-2026-09-19.md)
adds fresh original-login checks without discarding recorded effects; it is
accepted against two supplied Machine artifacts and activated on Controller.

## Fixed core ownership

The Controller, not a Plugin or browser, retains one original product user,
process-local Session incarnation, authenticated Machine connection and native
buffer reference. A new HTTP request cannot substitute a path, native handle,
Machine ID, installation or current Session for those captured owners. Core
communication, authentication and resource lifetime remain outside the Plugin
lifecycle; the signed Code Plugin still owns its private engine.

Preparation first checks the exact core-only `bufferLeaseSupport` response on
the original Machine connection. A native health field is insufficient. It then
asks that same connection to prepare a buffer in an already-ready worktree and
accepts only a closed API-1 `prepared` reply. It does not ensure/open a worktree,
open a buffer, select a newer Catalog release or fall back to a legacy socket.
Local legacy sessions are unsupported. Missing preparation observations remain
effect-free buffer reservations and expire at their respective owners.

The browser receives a core resource ID, not the private native reference. IDs
contain a random Controller-instance component and a monotonically increasing
counter; neither is restored or recycled. An ID is a lookup key, not a bearer
grant. A different user, Service process or restarted Controller cannot adopt it.
All successful and handler-error responses use `Cache-Control: no-store`.

| HTTP request | Closed input | Meaning |
| --- | --- | --- |
| `POST /api/code/buffers` | `{ "sessionId": "...", "path": "src/main.rs" }` | Prepare only; return the original core resource ID |
| `PUT /api/code/buffers/{id}` | `{}` | Admit at most one open attempt |
| `GET /api/code/buffers/{id}` | No body | Observe the original owner; never reopen |
| `DELETE /api/code/buffers/{id}` | `{}` | Release the observed original owner; never resolve its old path |

Responses contain exactly `apiVersion: 1`, `resourceId`, `state` and `pending`.
State is `prepared`, `open`, `released` or `unknown`. HTTP 202 with `pending: true`
is only an observation of an already running job; it does **not** enqueue the
new request, including a concurrent DELETE. The caller must observe completion
and explicitly request release afterward. Successful saved observations are
HTTP 200. A duplicate open/release never resends its native mutation. A query
that discovers `prepared` after an ambiguous open does not rearm that attempt.
An evicted terminal ID is unavailable (404), never a new admission opportunity.

Native `released` follows the original runtime's close contract. The original
candidate acknowledged local removal/close enqueue only; the later
[close confirmation](plugin-native-close-confirmation.md) requires observation
of the original native ownership removal. Neither undoes file edits nor
constitutes independently authorized recovery.

## Admission and cancellation

These routes require a currently authenticated product Operator, not a separate
admin cookie. Fresh product authentication is checked by the normal API
middleware. The continuation captures the original credential hash/identity,
without retaining its secret or consuming device proof twice, and rechecks that
credential, disabled/revoked status and current role before native dispatch and
before disclosing its outcome. Saved observations also require fresh original
authority; a retained snapshot is not a new Session or native-use grant.
Current automation scopes do not authorize this new effect surface.

Prepare and open additionally require permission to mutate the original Session
owner and the same current Session incarnation. Delete/recreate, cwd ABA or
retarget therefore cannot authorize a new open. Query and release instead
require the original resource's user and fresh Operator permission: they remain
possible after Session deletion, file removal, rename or ownership changes,
without granting new work in the replacement Session. An Owner cannot use
another user's resource ID as an administrative cleanup shortcut.

Once admitted, the Controller owns the bounded continuation independently of
the HTTP observer. Possible open/release effects are marked before transport
awaits. Observer cancellation, a missing or malformed reply, a wrong reference
or a changed Machine connection never causes automatic retry, inverse or
replacement-runtime selection. The retained original reference can be queried;
an unknown open must first be observed before requesting release. A lost release
is observed without resending it. Independent resolution of unresolved effects
remains unfinished.

The owned task records a valid native outcome before the HTTP observer rechecks
authority. Logout, role loss or observer cancellation must not erase an actual
Open or Release. The revoked login receives no result, while an independently
authenticated original user can observe the same ID and explicitly request
original-owner cleanup. This separation neither compensates a native effect nor
authorizes new work in a replaced Session. Failed or missing native observations
retain their existing uncertainty and never rearm a consumed attempt.

Per-resource jobs serialize by admission without holding a global lock across
I/O. Other resources can progress independently. The owning task set imposes a
60-second absolute request deadline including authority checks; handing work to
the owned task cannot renew it, and expiry cannot begin an effect. It bounds
concurrent jobs to 64, reaps completed tasks and aborts/drains outstanding tasks
at Controller shutdown.
Shutdown is not native cleanup or restoration: possible effects keep their
process-local unknown evidence until this Controller goes away.

## Bounds and rollout

The Controller permits 1,024 resources, including pending preparations. Only
effect-free preparations expire, after 30 seconds, checked on subsequent
resource operations. Active/unknown references never expire or undergo LRU
eviction. Exhaustion rejects before native open. Terminal observation receipts
are capped at 1,024 and retain only core ID and original user, not Session,
connection, credentials or native runtime references. Paths are capped at 4,096
bytes, a retained Session scope at 16 KiB and the requesting user ID at 256
bytes; HTTP bodies are bounded independently. These bounds are not a complete
native-memory, filesystem scheduling or worktree-cache reclamation guarantee.

Source tests exercise actual Hub observations, the Machine connection registry,
closed HTTP routes, independent task ownership, late/wrong replies, cancellation,
expiry, saturation, user/process isolation and current product credentials.
Source tests use deterministic Machine replies; they do not replace the separate
[immutable connected gate](plugin-code-connected-conformance.md). Its v10 checks
26–28 exercise actual product logout during real native Open/Query/Release, with
saved original-ID outcomes and exact no-replay command counts.

Controller outcome-authority changes activate independently of Machine/Plugin
bytes. No database schema, Catalog format, installation, telemetry policy,
Worker generation or native ABI changes here. Machine maintenance and signed
Zed publication/installation remain independent. Diagnostic and symbol reads
have an [owned interface](plugin-owned-buffer-reads.md); the
[typed browser owner](plugin-buffer-client-owner.md), product-context lifetime
and explicit unresolved-cleanup projection now support the separate Review
consumers. Intended navigation destination views, actual supported-device and
retained native-generation acceptance remain separately required.
There is no heartbeat, automatic abandoned-browser cleanup, Controller-restart
restoration, continuous principal/workspace writer fence or generic DAG recovery
claim in this slice.
