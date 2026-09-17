# Machine-owned navigation continuation

Protocol 21 adds a finite core continuation for the
[private navigation primitive](plugin-owned-navigation.md). Zed `1.12.0` adds
a distinct, effect-free `bufferNavigationSupport` probe of the actual pair.
The server and all third-party dependency pins are unchanged. This is a
Machine/source candidate, not a Service Operator grant, public navigation API,
native rollout or complete refactor.

## Admission and exact routing

`CodeBufferNavigation` is a closed control command, separate from generic
`AdapterRequest`. The common Controller outgoing boundary checks the declared
Service/Machine Site and protocol floor, even through generic send entrypoints.
The enrolled Machine connection captures a non-cloneable, non-serializable
invocation before scheduling. It owns the exact request, actual connection and
15-second monotonic command budget. Waiting for a route does not renew authority;
disconnection revokes queued admission. A matching Service name on a later
connection cannot adopt an existing group.

Preparation accepts an original open buffer reference, complete content identity,
exact UTF-16 point and one of five closed navigation kinds. It accepts no path,
runtime, native ID, deadline or caller authorization flag. Core captures the
retained original runtime and worktree route, rechecks the source after waiting,
and requires that adapter's navigation/handoff support contract. Health or an
older synchronization/native probe cannot substitute for it.
Direct navigation also retires expired inert synchronization reservations before
checking the source; it does not require an unrelated buffer request to clear
that exclusion. Pending or unknown synchronization effects keep their fence.

Core issues a separate random-instance, monotonic `navigation:` reference.
Native `nav:` references stay Machine-private. The group pins its original
worktree route as well as its exact runtime: an ordinary source or legacy
worktree close cannot forget a still-owned group. Uninstall, path removal and
runtime death cannot cause continuation against another installation. Runtime
death is unavailable execution, not release or recovery.

## Finite lifetime

- At most 32 groups, including in-flight preparations, and 64 executing/queued
  commands. Capacity never evicts unknown effects. Only inert preparations
  expire after 30 seconds; retirement takes the original route before dropping
  its pin. A busy route is retained for a later pass.
- Execute rechecks that the source is still open and not reserved for
  synchronization. Before transport I/O it records Unknown and removes expiry.
  Duplicate Execute returns saved evidence. Timeout, cancellation, invalid
  observations and lost replies never renew the one-use acquisition.
- Query of Unknown observes only the original adapter group, without another
  LSP query. Prepared or missing evidence after attempted Execute cannot prove
  no effect. Queries of known retained/terminal states are saved historical
  observations, not fresh coordinates or runtime-liveness guarantees.
- Results are validated in full: closed fields, at most 256 locations and
  32 distinct bounded relative display paths, consistent complete text identity
  for repeated paths and bounded UTF-16 ranges. Native still validates the
  actual text/epochs. No partial/truncated success is published.
- Release of a retained group records ReleaseUnknown before I/O. After a lost
  reply only original-ID Query may establish Released; release is never resent.
  Acquisition Unknown cannot be retired through this operation. Uncertain release
  retains the original location evidence and runtime pin.
- Released groups retain at most 32 bounded tombstones and no process/capacity
  permit. Inert local retirement is not a native effect. Adapter Released means
  local owner removal/close enqueue, not a native acknowledgement or rollback.

## Destination handoff uses ordinary owners

`PrepareDestination` chooses an index and exact content identity from that
original retained result. There is at most one saved ordinary reservation per
index. It shares the existing 1,024-buffer capacity and inert expiry; it cannot
select a path, installation or new process. Different indices may intentionally
create independent owners, including duplicate LSP locations.

The original runtime route and ordinary buffer reference are committed before
publishing the group's saved destination lookup, with no await between the two.
An admitted handler survives loss of its transport observer; Query discovers
the same reservation and a repeated destination request never allocates another.
If actual task cancellation interrupts the effect-free native preparation before
its reply, only an inert adapter reservation may remain, bounded by its TTL.

This is preparation, not Open. The existing ordinary buffer continuation
explicitly opens that reference, then performs the existing content-bound
reads/release. Native Open still checks the original parent, target and epoch;
parent release or edit/undo before Open cannot adopt a replacement. Once Open
succeeds the ordinary owner is independent of the parent navigation group.
Uncertainty cannot be repaired by opening its display path. The future Service
consumer must own this handoff before returning a browser resource; a raw
Machine reference is not principal/Session authority.

## Verification and remaining boundary

The [candidate acceptance](releases/machine-navigation-candidate-2026-09-17.md)
records exact immutable inputs, complete source gates and all three process
gates. It distinguishes the synthetic navigation authority from the older
Controller's eleven connected buffer regressions and production acceptance.

Focused core tests exercise closed codecs and Site/protocol checks, actual
connection loss, queued command/prepare expiry, cancellation, duplicate/unknown
outcomes, original-runtime death, bounded capacity, source release/synchronization
exclusion and independent destination routing. Private adapter tests retain the
native content/epoch/ABA and cancellation checks.

`zed-plugin-conformance` now builds the explicit test-only stdio LSP and runs
inside isolated network/PID namespaces with read-only cgroups. Its temporary
signed lifecycle gate exercises all five nonempty kinds through Machine core,
complete Unicode target identities and UTF-16 positions, one-use acquisition,
destination preparation before uninstall, Open on that retained process after
uninstall, then parent release and exact-content hover/release after path removal.
It uses synthetic core authority and synthetic language answers, not an enrolled
protocol-21 Controller/Service/browser consumer. The separate static-pair and
eleven-check connected-buffer regressions remain required.

Generic forwarding still denies all four private navigation commands. No
Web consumer or native bridge capability is enabled. The subsequent
[Service candidate](plugin-service-buffer-navigation.md) supplies principal/Session
ownership and destination handoff through a separate default-closed admission
policy and protocol-21 connected gate. The native query may allocate resources
before result limits reject it; global native allocation bounds, OS filesystem
isolation and independent saved-query recovery remain unproved. Those gaps and
consumer/device acceptance must be addressed before public cutover. General
graph/state leases, independent restoration, signed publication/installation
and resident Machine maintenance remain separate.
