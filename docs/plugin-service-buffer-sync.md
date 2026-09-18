# Service-owned buffer synchronization

This additive core API continues the [protocol-20 Machine owner](plugin-machine-buffer-sync.md).
It does not change ordinary Review, publish/install Zed `1.9.0`, activate a
Machine, or implement independently authorized restoration. Communication,
product authorization and finite resource ownership remain core mechanisms,
not installable Plugins or a generic DAG executor.

## Explicit preparation and confirmation

The browser supplies only its existing core buffer ID and exact desired disk
content identity. Preparation requires that this Service actually admitted the
buffer's open, the original Session incarnation is still current, and the
original product user can mutate it. A query merely reporting Open is not such
an admission. Local legacy sessions and protocol-19 Machines cannot use this
path; there is no fallback to an adapter command, path lookup or new runtime.

| HTTP request | Closed input | Effect |
| --- | --- | --- |
| `POST /api/code/buffers/{resource}/synchronizations` | `{ "purpose": "refresh_from_disk", "content": { "sha256": "<64 lowercase hex>", "utf8Bytes": 123 } }` | Prepare and reserve the original owner; do not synchronize text |
| `PUT /api/code/buffer-synchronizations/{operation}` | `{}` | Fresh confirmation of this exact prepared operation; at most one Apply |
| `GET /api/code/buffer-synchronizations/{operation}` | No body | Query original evidence only |
| `DELETE /api/code/buffer-synchronizations/{operation}` | `{}` | Explicit retirement; cannot discard Pending/Unknown after Apply |

An operation ID is a process-local lookup key, not a bearer grant. Its separate
`sync-` namespace cannot be used as a buffer ID. Preparation captures the
original buffer, Session, Machine connection, purpose and content; follow-ups
cannot replace any of them. Neither native buffer nor Machine/private operation
references leave the Service. The same closed content codec is shared with
Machine and owned reads, bounded to 4 MiB UTF-8.

All methods require a currently authenticated product Operator. Separate admin
cookies and current automation scopes do not grant this surface. Each admitted
task captures the original credential identity without retaining its secret or
reusing a device proof. It rechecks that credential and current role before
dispatch and before responding, without switching to replacement credentials.
Apply additionally requires the original Session incarnation and permission.
Query/retire remain available to the original user after Session deletion;
neither can write in a replacement Session. Every remote command uses the
original connection, including cleanup. A reconnect cannot adopt it.

The Machine separately validates Site/protocol and constructs its own bounded
connection invocation. Its actual private adapter must report the distinct
ownership-support contract. A serialized purpose, content hash, signed Plugin
or Service snapshot does not supply Machine or native write authority.

## Evidence and lifetime

Responses have exactly `apiVersion: 1`, `operationId`, `resourceId`, `purpose`,
`content`, `state` and `pending`. States are tagged objects with `kind`:
`prepared`, `pending`, `unknown`, `applied`, `refused`, `retired` or `expired`.
Applied evidence repeats exact content and a bounded canonical native version;
refusal has only the closed `changed`, `source`, `shared` or `budget` reason.
The [Budget continuation](plugin-sync-budget-outcomes.md) requires updated
readers and the exact producing adapter; old process owners are not migrated.
That native
version is observed evidence, not a subsequent write grant.

HTTP 202 / `pending: true` means an already-admitted Service job is running; it
does not enqueue the repeated request. A completed observation uses HTTP 200,
even when its native state is still Pending/Unknown. Successful replies,
handler errors and JSON/body rejections use `Cache-Control: no-store`.

- Service admission is bounded to 256 operations, including pending prepares,
  with 64 simultaneous synchronization jobs. Each job's 60-second monotonic
  budget includes authority waits and dispatch; the transport keeps its normal
  40-second timeout. No caller can supply or renew either deadline.
- Only inert preparations expire, after 30 seconds. Ordinary buffer operations
  also trigger expiry, on both Service and Machine; a forgotten preparation
  cannot leave their read/release fence permanently closed. Service `expired`
  means local effect-free abandonment, not a remote retirement receipt.
- Apply consumes its one-use budget and records Unknown before transport I/O.
  Its independently owned task outlives the HTTP observer. Timeout, cancellation,
  malformed/wrong-owner replies and disconnect cannot rearm that budget.
- Preparing and unresolved synchronization fence buffer reads/releases and
  another synchronization. Only inert abandonment or exact terminal evidence
  clears this local fence. Machine and native ownership checks remain separate;
  shared/dirty/changed buffers cannot be overwritten by this API.
- Query observes only the original operation. Prepared/Retired after an
  uncertain Apply cannot establish that no effect occurred. Terminal evidence
  cannot regress or change. A lost retirement response is resolved by Query,
  never another retirement dispatch.
- Explicit retirement drops Session/connection/fence/capacity ownership.
  At most 256 terminal local snapshots and small anti-reuse identities remain.
  Exhaustion never evicts active, terminal-but-unretired or uncertain operations.

Revocation before dispatch prevents a new operation. It cannot undo an already
admitted native mutation. Exact terminal evidence is retained even if the
response is withheld because its original credential expired. Shutdown drains
the owned task set without interpreting possible effects as cleanup. Controller
or Machine restart loses these process-local continuations; missing evidence
does not grant a replay, native adoption or restoration.

## Verification and remaining integration

Source tests cover closed bodies, route ownership, protocol floor, current
credentials/roles, Session ABA, replacement connections, cancelled observers,
wrong-content/owner replies, one-use mutation, uncertain cleanup, bounded jobs
and retained capacity, inert expiry and original-user/process isolation.

The [connected Code gate](plugin-code-connected-conformance.md) additionally
requires real product login, shared-owner refusal, preparation before HTTP
uninstall, Apply afterward on the retained native process, deliberate loss of
an actual reply through the normal timeout, original-ID observation, exact
content reads and cancellation-safe retirement. It also refuses old operations
after connection/Controller replacement. Fixture signatures, accounts and
teardown are not production authorization or recovery evidence.

The [accepted Controller-only rollout](releases/service-buffer-sync-2026-09-17.md)
records two successful 11-check runs against exact immutable inputs, the full
gate and bounded production process continuity. The resident protocol-19
Machine and installed Zed were not upgraded, so this is not production
end-to-end synchronization or a browser/Review cutover.

The [typed browser owner and confirmation/unknown surface](plugin-browser-buffer-sync.md)
now implement the client continuation without switching ordinary Review.
Still separate: ordinary Review content/position lifetimes, owned navigation
destinations, signed Plugin installation, explicit Machine maintenance and
supported-device acceptance. No browser call site, SDK grant, state-journal
schema, host policy or public Plugin release changes in this Service slice. See the
[completion ledger](plugin-refactor-completion.md).
