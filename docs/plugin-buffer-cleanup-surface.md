# Core Code cleanup in Settings

The internal buffer registry now supplies the shared Settings → Info surface
with a typed local projection of retained owners. This is core Web resource
management, not a Plugin capability, another lifecycle, native synchronization
or independently authorized post-effect recovery. Ordinary Review remains on its
existing API; no Machine, Plugin or worker is installed by this change.
The [verified Web-only release](releases/plugin-buffer-cleanup-surface-2026-09-16.md)
is active, with all fourteen observed workers retained and no process restart.

## Original ownership, visible uncertainty

`productCodeBuffers.cleanup` implements the core component library's
`ReadableStore` contract. Reading/subscribing does not discover a dataset, open
IndexedDB, import the socket store or send HTTP. The projection has stable
frozen snapshots for `useStore`, coalesced notifications and independent
subscription leases. Subscribers cannot re-enter a partially admitted job or
make an admitted operation fail. Context observation exists only while a
subscriber or retained owner needs it; unsubscribing never releases a native
resource.

Active views are counted, not offered for cleanup. An owner becomes actionable
only after its consumer has already called `close()`. An ended context also
shows otherwise-active unresolved owners, but without private paths/Session
labels or actions. The model never includes file contents, raw transport errors,
credentials, native references or even the Controller resource ID.

Rows carry an opaque process-local handle to the exact original owner. The
display ordinal is never recycled and is not an authority identifier. Foreign,
serialized and retired handles are refused before I/O, even if a new buffer has
the same path. Reading an earlier snapshot does not extend its authority.

| Observation                                              | Available action                                                           |
| -------------------------------------------------------- | -------------------------------------------------------------------------- |
| Active consumer                                          | None; only included in the active count                                    |
| Owned operation or cleanup pass still running            | Wait; no extra request or background poll                                  |
| Valid pending/unknown/unavailable evidence after a close | Explicit status query or separately confirmed bounded cleanup pass         |
| Release sent without terminal acknowledgement            | Status query only, even if later evidence says `open`                      |
| Original context ended                                   | Redacted evidence only; no new or replacement credentials                  |
| Original owner acknowledges terminal release             | Retire that row; do not infer undo or physical CloseBuffer acknowledgement |

Status queries are original-ID GETs. Continuing cleanup uses the existing
one-pass `close()` implementation: drain an admitted borrow, observe if needed,
and release only when original-owner evidence permits. `202` never queues the
request; an unknown, failed or malformed result remains retained. Ambiguous
DELETE never rearms. Validation and claims happen synchronously before awaits,
including when two views act in the same event-loop turn. UI confirmation is not
an independent grant; the actual Controller still validates its original
user/credential and current Operator permission.

## Bounded presentation and view lifetime

The section is absent when this page retains no owners, otherwise collapsed
initially. It renders at most five cleanup rows with local pagination, including
long-path wrapping and safe page clamping when acknowledged rows disappear. The
core registry still has its existing 64-owner ceiling and no LRU eviction. No
mount, disclosure, page change or remount starts an operation.

`Check status` is distinct from `Continue cleanup…`, which uses the existing
core `ConfirmSheet`. Cancelling does nothing. A changed/removed owner
invalidates the confirmation without selecting a replacement. Ending the
original context fences even a still-rendered confirmation before React updates,
then removes private details and actions. A consumer replacing the source must
explicitly remount; an existing view does not adopt that registry.

Unmount ends this view's status observer and subscription, not an admitted core
query or cleanup. A remount sees the original in-flight or unresolved owner; it
never retries work. No local operation result is persisted or imported after a
browser reload, and absence in a new page is explicitly not evidence that an
older page's native resources were released.

## Acceptance and remaining work

Unit/compile checks cover stable immutable projections, subscriber isolation and
lifetime, pending/ambiguous release, foreign/retired/serialized handles, late
reads, authority loss and pre-auth local-only observation. The isolated
`just code-buffer-cleanup-browser-conformance <absolute-firefox>` gate runs
actual React/MUI/StrictMode with the original core owner and deferred HTTP.
Seven cases cover passive mounts, confirmation/cancellation/double clicks,
release across unmount/remount, lost acknowledgement/404, same-stack authority
loss, twelve long-path rows at 360px and stale confirmation after replacement.
The existing owner/content, product-context and Settings recovery browser gates
remain separate regression checks. No account, normal browser profile or native
Plugin is accessed by these fixtures.

Still required: ordinary Review integration, explicit disk/native content
synchronization and its own effect authority, navigation destination ownership,
independent Machine/Code rollout, abandoned-browser/restart recovery and
supported device acceptance. A Settings cleanup pass is not generic Plugin
compensation, restoration of file edits, durable native recovery or completion
of the refactor.
