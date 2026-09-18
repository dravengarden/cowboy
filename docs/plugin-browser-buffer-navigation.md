# Browser-owned navigation continuation

This finite core candidate extends the [browser buffer owner](plugin-buffer-client-owner.md)
with the [Service navigation protocol](plugin-service-buffer-navigation.md).
Communication, identity, ownership and cleanup stay core-owned; this is not an
installable navigation Plugin, generic DAG executor or new native dependency.
Production acquisition remains default closed. No Review entrypoint is enabled.

## One original source, one continuation

Only a buffer whose owner submitted the original Open may prepare navigation.
It requires fresh, idle Open evidence, an authentic complete LF content capture,
a checked UTF-16 position and one of five closed navigation kinds. A remote
observation claiming Open cannot create that original intent. Paths, JSON hashes,
native references and serialized IDs cannot manufacture a source or continuation.

Prepare performs no acquisition. Execute is a separate, explicit, one-use call;
its intent is consumed before I/O. An observer's abort only detaches that caller:
the original owner drains the response and retains the same group. A preparation
that finishes after view close remains available through that source's
`navigation()` handle, but cannot subsequently Execute. New views cannot adopt
it by path, navigation ID or copied JSON.

The browser permits one navigation or synchronization per source, and at most
32 navigation groups per core registry, including pending preparations. The
existing registry still bounds ordinary sources to 64. Source reads, observation,
synchronization and release are excluded until the navigation is explicitly
Released or proven inert Expired. This intentionally retains a stronger local
source fence than the Service's short Execute borrow. View close drains a busy
continuation but does not secretly Query, Execute or Release its group. Unknown
effects do not expire locally or make space through eviction.

## Evidence and action boundaries

| Evidence | Permitted next action |
| --- | --- |
| Fresh Prepared, no Execute/Release attempt, source view active | Explicit Execute, Query or Release |
| Fresh Prepared after source close, never executed | Query or Release; no Execute |
| Execute response lost, or fresh Unknown | Original-group Query only |
| Fresh Retained, Release not attempted | Query or explicit Release |
| Release response lost or ReleaseUnknown | Original-group Query only; never resend DELETE |
| Released or inert Expired | End the group; ordinary source cleanup needs fresh source evidence |
| Core Service/principal lifetime ended | No remote action; retain/redact local recovery status |

Execute is not rearmed by HTTP 202, refusal or later Prepared evidence. A failed
Execute that never reached admission can end through a Service Expired receipt;
once Unknown or Retained has actually been observed, Expired cannot erase it.
Retained locations cannot change, regress or disappear during group release.
An unexecuted preparation cannot suddenly acquire targets in a cleanup response.

A valid DELETE/202 response with the exact pre-release state acknowledges that
**this** release was not admitted; it is not queued or successful. Only a fresh
explicit Query may make a separately requested Release available. An admitted
ReleaseUnknown response, including HTTP 202, never rearms Release. A lost DELETE
also remains query-only even if a later Query still reports Retained.

Ending a group only releases its local native ownership; it is neither physical
buffer-close proof nor rollback of language-server effects. It never releases a
separately opened destination. Automatic retry, alternate routes, path reopen,
account adoption, persisted-handle import and cross-restart recovery are absent.

## Closed data and local recovery

`NavigationId`, ordinary `ResourceId`, synchronization IDs and page-local recovery
handles have disjoint types. The response decoder binds the exact source, group,
content, query and point. It rejects unknown fields, foreign IDs, native handles,
inconsistent HTTP/pending states, malformed paths/ranges and oversized evidence.
Bounds match Service: 2 MiB transport body, 256 locations, 32 distinct target
paths, 4 MiB complete content identity and 4,096 UTF-8 bytes per display path.
Repeated paths must have the same content. Location coordinates are only bounded
historical evidence; a future destination display must validate actual complete
text and UTF-16 ranges before use.

This client has not requested any destination: nonempty `destinations` are
therefore refused instead of silently importing ordinary owner IDs. Destination
preparation/adoption needs its own typed continuation and capacity reservation.

The passive `navigations` recovery store, also exposed by `productCodeBuffers`
without calling `ready()` or Service discovery, accepts only original in-memory
handles for Query and group Release. It has no Execute, path lookup, durable import,
polling or implicit cleanup. It projects status and the original source label,
not private target locations, raw errors, hashes or native references. Ending
the core identity synchronously redacts labels and fences actions. Ordinary
cleanup displays a distinct navigation fence and disables its unrelated controls.
No recovery panel or navigation consumer is activated by this candidate.

## Acceptance and remaining work

`contracts/code-buffer-navigation.fixture.json` is shared by the actual Rust
Prepare/Execute/Query/Release handlers and TypeScript decoder. Rust compares the
entire serialized response, normalizing only randomly generated lookup IDs, and
checks no-store headers. Compile-only tests reject exchanged domains, fabricated
captures, generic operations and destination import. Unit tests exercise
cancellation, unknown outcomes, one-use admission, terminal/target monotonicity,
capacity before dispatch, context loss and stale/foreign recovery handles.

The isolated Firefox owner suite now requires 18 cases: the previous 11 plus
seven navigation cases, using browser WebCrypto, Response streams, real clicks
and cancellation. Other product-context, cleanup, synchronization, source,
working-diff and document-refresh suites remain mandatory regressions. Exact
results belong in the [candidate record](releases/browser-navigation-candidate-2026-09-18.md).

This is not complete destination handoff/view integration, native acquisition
pre-allocation bounds, intended-consumer or supported-device acceptance, a
production rollout or independently authorized post-effect recovery. Those
remain explicit exits in the [completion ledger](plugin-refactor-completion.md).
