# Controller-owned buffer continuations

Status: the [original candidate](releases/plugin-controller-buffer-owners-2026-09-15.md)
is included in the separately activated Controller `0fded719`; the
[six Agent publications](releases/agent-publication-2026-09-15.md) closed its
original prerequisite. The ordinary Review buffer and language APIs remain
legacy; this is not the Web cutover or production
Machine/Zed acceptance. It consumes the independently verified
[Machine/native candidate](plugin-native-buffer-leases.md).
The additive [owned observation API](plugin-owned-buffer-reads.md) now implements
diagnostic and symbol reads; its rollout remains separately recorded.
The separate [synchronization API](plugin-service-buffer-sync.md) adds finite
original-owner preparation/confirmation, without changing ordinary Review.

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

Native `released` means local ownership removal and, when needed, successful
native close enqueue. It does not prove the unacknowledged native CloseBuffer
effect, undo file edits or constitute independently authorized recovery.

## Admission and cancellation

These routes require a currently authenticated product Operator, not a separate
admin cookie. Fresh product authentication is checked by the normal API
middleware. The continuation captures the original credential hash/identity,
without retaining its secret or consuming device proof twice, and rechecks that
credential, disabled/revoked status and current role before native dispatch.
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

Per-resource jobs serialize by admission without holding a global lock across
I/O. Other resources can progress independently. The owning task set imposes a
60-second deadline including authority checks, bounds concurrent jobs to 64,
reaps completed tasks and aborts/drains outstanding tasks at Controller shutdown.
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
Machine responses are deterministic fixtures; this is not the immutable
Controller/Machine/native connected rollout matrix or a real product login.

Controller activation can expose this additive API without changing the old
Review consumer or activating new Machine/Plugin bytes. No database schema,
Catalog format, installation, telemetry policy, Worker generation or native ABI
changes here. Machine maintenance and signed Zed publication/installation remain
independent. Diagnostic and symbol reads now have an
[additive owned interface](plugin-owned-buffer-reads.md). Before switching Review,
resolve hover/navigation content-coordinate ownership, connect the
[typed browser owner](plugin-buffer-client-owner.md) to the real identity and
Review lifetimes, present unresolved cleanup, and accept the actual Machine/Code
generation. The client implements pending/unknown observation and explicit
release with isolated StrictMode acceptance; it does not yet change live Review.
There is no heartbeat, automatic abandoned-browser cleanup, Controller-restart
restoration, continuous principal/workspace writer fence or generic DAG recovery
claim in this slice.
