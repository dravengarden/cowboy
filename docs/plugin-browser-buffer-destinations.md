# Browser-owned navigation destinations

This extends the finite [browser navigation continuation](plugin-browser-buffer-navigation.md)
with ordinary target-buffer ownership. Communication, capacity, identity and
cleanup remain core mechanisms, not Plugins. It adds no native dependency,
path reader, production navigation entrypoint or policy change.

## Original target, original reservation

The first valid Retained observation creates frozen, opaque `NavigationTarget`
tokens for its immutable locations. Only that group's original in-memory token
can request preparation. A number, copied object, serialized ID, path or token
from another group cannot grant ownership. A location's path is a display label;
its captured content identity accompanies the original index, not a path lookup.

`prepareDestination(target)` requires fresh idle Retained evidence, no parent
Release intent and an active source view. It first reserves an ordinary slot
against the registry's existing 64-owner limit, then consumes that index's
one-use intent, then sends exactly one POST to the original group's
`/destinations` route. Its closed body is `{ destination, content }`. A capacity
refusal precedes I/O and does not consume the target. A pre-aborted observer
also starts nothing. The same source job fence excludes parallel navigation,
synchronization, source reads and cleanup.

The ordinary owner is available immediately through `destination(target)`.
While awaiting handoff it cannot Prepare by path, Open, Query an invented ID,
read or silently release capacity. Close marks its view closed and returns
Retained. Cancellation detaches the observer, not the original request; a late
valid receipt still binds that original local owner without opening it.

## Receipt and uncertainty rules

The decoder accepts destination records only for requested indices, in strictly
ascending order, within the immutable location list. Each record has exactly
three fields. Prepared requires a valid ordinary ID distinct from the source
and all sibling IDs; Pending, Unknown and Expired require `resourceId: null`.
Before adopting any child, the full response must pass shape, parent transition,
location immutability and previous-receipt checks. Every ID is then checked
against other retained ordinary owners. One invalid sibling refuses the whole
receipt without partially adopting the others.

| Evidence | Ownership/action |
| --- | --- |
| Request pending, failed, cancelled or absent from response | Keep the original slot; explicit original-group Query, never another POST |
| Pending/Unknown, including HTTP 202 | Keep capacity; no implicit Open or retry |
| Prepared with the original ordinary ID | Bind that same slot once; explicit ordinary Open is separate |
| Confirmed inert Expired | Retire only the never-bound slot; never rearm preparation |
| Parent Released/Expired and no Prepared receipt | Abandon the inert slot; no claim of closing a native buffer |
| Core Service/principal lifetime ended | Stop remote actions, retain unresolved ownership, redact labels |

Observed Prepared must remain Prepared with the same ID; observed Expired
cannot revive; confirmed Unknown cannot disappear or regress to Pending.
Pending may disappear if Service dispatch never began, but absence alone never
frees local capacity or rearms a request. Unknown outcomes have no local expiry.

Query or Release may recover a previously lost Prepared receipt. That still
does not Open a child. If the native parent has already released, a later
explicit child Open may be refused; it cannot fall back to a path or a fresh
owner. A valid response resolving a closed view retains its ID for explicit
ordinary cleanup, not for automatic display.

## Independent child lifetime

Successful adoption supplies only the original ordinary Prepared snapshot.
Its explicit Open, read, synchronization and release use the existing typed
owner and original-ID transport. Once opened, parent/group/source release
cannot close it. Conversely, releasing a child does not release its parent.
Historical Prepared records in later group observations never reset an Open,
Unknown, closing or Released child. Neither an observation nor a remounted view
can resurrect a retired owner.

The cleanup store shows unresolved handoff as a separate status directing the
user to the original navigation. It offers no ordinary Query/Release before
the ID is known. The passive navigation store remains Query/Release-only and
cannot prepare targets or initiate Open. The later
[Review consumer](plugin-review-owned-destinations.md) supplies the explicit
reader and passive recovery panel without broadening this core API.

## Acceptance and boundaries

`contracts/code-buffer-destination.fixture.json` is shared by the actual Rust
Prepare/Execute/Destination handler chain and browser decoder. The Rust test
normalizes only generated Service lookup IDs, checks no-store, requires a
separate ordinary Open and verifies independent reads after parent release.
The fixture contains no native references. Compile-only tests reject forged
targets, IDs and any public ordinary-ID import API.

Unit tests cover the whole-receipt preflight, existing-owner ID collisions,
one-use dispatch, capacity before I/O, lost/cancelled responses, monotonic
records, view close, no implicit Open, independent native-text reading and
release, and identity loss. The isolated Firefox owner suite requires all
24 cases, including six destination cases exercising actual clicks, browser
Response streams, cancellation, WebCrypto and structured cloning. All other
Code browser suites and connected v5 checks remain required regressions.
Exact source/artifact evidence belongs in the
[candidate record](releases/browser-destination-candidate-2026-09-18.md).

This core handoff is consumed by the separate Review destination reader, which
checks complete native text, actual UTF-16 coordinates and its view lifetime.
Native pre-acquisition allocation bounds, supported-device/native acceptance,
signed runtime rollout and independently authorized post-effect recovery are
separate exits. Production navigation acquisition remains default closed.
