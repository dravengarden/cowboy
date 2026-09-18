# Service-owned navigation continuation

This finite candidate joins the [Machine navigation owner](plugin-machine-buffer-navigation.md)
to the existing [Service buffer owner](plugin-controller-buffer-owners.md).
It is not a generic DAG executor, automatic rollback or a Web navigation cutover.

## Admission and public declarations

The Controller's private `--code-navigation-admission` /
`COWBOY_CODE_NAVIGATION_ADMISSION` policy accepts only `closed` (default) or
`candidate`. Protocol 21 is necessary but never enables acquisition on its own.
Keep production admission closed pending native allocation bounds, intended
consumer and device acceptance. Query and cleanup do not require acquisition
policy; they still require the original resource user, fresh product Operator
permission. Live continuations require the original connection; bounded terminal
receipts are historical observations without one. Admin/automation are not grants.

| HTTP operation | Closed input | Meaning |
| --- | --- | --- |
| `POST /api/code/buffers/{id}/navigations` | `content`, UTF-16 `position`, `query` | Effect-free preparation from an existing Service-admitted Open |
| `PUT /api/code/navigations/{id}` | `{}` | One-use acquisition on the original source/runtime |
| `GET /api/code/navigations/{id}` | No body | Original-ID observation; never another language query |
| `DELETE /api/code/navigations/{id}` | `{}` | Explicit original-owner release; no rollback or background-drain claim |
| `POST /api/code/navigations/{id}/destinations` | Result `destination` index and exact `content` | Effect-free ordinary buffer preparation, never implicit Open |

The five closed query kinds are definition, declaration, typeDefinition,
implementation and references. Requests accept no path, native reference,
Machine/runtime selector, deadline, grant or authorization flag. Unknown fields
are refused. All route responses, including extractor rejections, are no-store.
Machine/native references never enter browser snapshots. Separate `nav-` lookup
IDs bind original user, immutable Session scope, original connection and source
resource. Complete content identity and points remain part of that record.

Close evidence depends on the exact retained native pair. Historical candidates
confirmed only local enqueue; the Zed `1.15.0` / private server `1.2.0`
[close candidate](plugin-native-close-confirmation.md) additionally requires
original-peer removal confirmation. Losing that native reply retains
ReleaseUnknown and cannot be repaired by observing current absence. Losing only
an upper-layer reply may still settle by querying the completed original adapter
record. Neither case permits replay, generation replacement or a recovery claim.

Preparation, Execute and destination preparation recheck a live owned Session.
Execute borrows the original Service-admitted Open through native dispatch,
excluding ordinary release and synchronization; a remote Query claiming Open
cannot turn an inert reservation into an admitted source. Query/release remain
possible after Session deletion, under that resource user's fresh permission.
Neither a replacement connection with an identical epoch nor another Session
can adopt the group. Credential/role checks precede dispatch and repeat before
disclosure; an effect receipt is saved even when disclosure is refused.

## Bounded lifetime and unknown outcomes

There are at most 32 groups, including preparations, 64 command jobs and 32
bounded terminal snapshots. Preparations expire after 30 seconds; full command
budgets are 60 seconds including permission waits, not a renewed transport
deadline. The Machine retains its separate 15-second admission budget. Owned
jobs survive HTTP observer loss and drain independently; shutdown does not
pretend that ambiguous effects were undone.

Execute records Unknown before any transport await and removes inert expiry.
Duplicate Execute returns saved evidence, not a second command. Acquisition
Unknown cannot be released; only Query of the original group may repair a lost
receipt. Release similarly records ReleaseUnknown before I/O and is never
resent. Unknown effects are never expired or evicted for capacity. Local inert
expiry is explicitly Expired, not Released. Terminal snapshots retain no Session,
connection, process or capacity permit.

Responses are bounded to 2 MiB and validate all locations, complete identities,
UTF-16 bounds, relative display paths, destination indices, ordering and reference
uniqueness. Retained locations cannot change. Unrequested targets and replacement
references are refused before adoption. Saved coordinates and preparation states
are historical evidence, not current text, native liveness or authority.

## Ordinary destination handoff

Before the first destination dispatch, Service records the index, content and
ordinary buffer capacity reservation in the group, not solely in an HTTP future.
A received target is synchronously inserted into the ordinary owner and linked
to the group, with no await between. The response contains an ordinary
`resourceId`, never a native lease.

Ambiguous dispatch leaves saved Unknown. Only Query may discover the same
Machine reservation. The original 30-second Service preparation deadline is
never extended. Late recovery marks the local target Expired, releases capacity
and creates no replacement. A queued request cancelled before dispatch frees
its inert reservation. Ordinary buffer admission also reaps expired navigation
reservations without reversing registry lock order. No target is opened by its
display path to repair uncertainty.

The caller must explicitly Open the ordinary resource. Existing Session/user/
connection checks still apply. Parent release or target epoch changes before
Open remain native refusals; successful Open owns an independent lifetime and
survives parent release. Group release never releases an opened ordinary target.
Preparation snapshots do not replace the ordinary owner's current evidence.

## Acceptance and remaining boundary

Source tests cover one-use effects, strict codecs, stored product-token and role
revocation before/after dispatch, Session deletion, connection replacement,
cancellation, target adoption and bounded expiry. The
[connected gate](plugin-code-connected-conformance.md) requires schema v4 and
17 checks: real enrolled protocol-21 Controller/Machine and signed Zed processes,
plus an explicit test-only stdio LSP with nonempty Unicode answers. It discards
actual acquisition/release replies and waits the original transport timeout;
it does not manufacture control/native observations.

The [candidate acceptance](releases/service-navigation-candidate-2026-09-17.md)
records the exact immutable Controller/Machine/native inputs, complete source
gate and two successful 17-check connected runs. It also records the failed
fixture attempts and the corrected synchronization/handoff ordering.

This does not implement full native destination text/view consumption, Web or
native-shell navigation, global pre-allocation bounds, independently authorized
post-effect recovery or cross-restart restoration. Native/OS isolation limits,
production signed publication/installation and the separate resident Machine
maintenance boundary remain. Candidate acceptance never enables production.
