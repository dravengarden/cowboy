# Durable Session cleanup continuation — Hawk, October 5

A writer-admitted resident Machine now finishes deleted-Session artifact cleanup
across its own restart. Before this release every resident restart silently
abandoned the cleanup of Sessions deleted shortly before it: the retry plan lived
only in the process, so those Cargo targets were never reclaimed. The
[contract](../plugin-session-cleanup-continuation.md) defines the record as an
advisory nomination of the original worktree root (device, inode, creation time)
for a deletion already committed in the terminal journal. It never carries a
target list, retry cursor or marker progress. After a restart the Machine
requires the committed deletion, re-observes that exact root object and rescans
Cargo targets from scratch through the unchanged handle-anchored cleanup. A
missing, linked or replaced root retires the nomination without effects; losing
or never writing a record leaves the earlier preserve-artifacts behaviour.

Implementation commits are `c1367cd7` and the test-robustness follow-up
`f85ff9ae51ecdd0b859e97598518026e31649471`. Active source is
`9c79b9c379ec929bc74bc520d473332d1f1a1b41`, activated as
`/nix/store/gg9v8s05aw27db18l0am2whqip0v6x61-cowboy-machine-writer-host-release`
(default reader `/nix/store/h71z3rhym7wvqx16ghl7jh9ns8c0sam6-cowboy-machine-release`)
by root transaction `1791179356912329897-9c79b9c379ec`. It started at
`2026-10-05T05:49:16.912329897Z` and committed at `05:49:28.7697127Z`, reporting
succeeded, committed, maintenance and no recovery. Its receipt said
`published=false` because the commit was pushed afterwards; main
`83b32ea824d224de8b990e5c2509a17a2d547642` now contains it. The previous active
artifact was `kfkbi1i0…` (source `e51c79bc`). The release retains separately
accepted worker source `b97c2724bea23834944ded8af98e2de6729f4256` and generation
`worker-748825b42b4302fe26ca`; no worker pin, wire or SDK source changed.

Startup at `05:49:17Z` logged the deletion journal reader with writer enabled and
one recorded deletion, then `durable Session cleanup continuations ready
pending=0`. The new `session-cleanups` namespace holds only its lock; no record
exists.

## Validation

At `f85ff9ae` the source passed Rustfmt, both Clippy gates, 1905 all-features
tests (58 ignored) and 573 standalone Machine tests (18 ignored). Ten tests are
new: six store tests (reopen/retire, foreign owner and unknown/duplicate/
oversized input refusal, links and special files, exclusive ownership and
replacement, storage-failure fencing, staging never replayed and no eviction) and
four broker tests. The broker tests drive the real `StopSession` path to record
a nomination, resume it in a second resident, show completion is durable for a
third, refuse a root replaced while down, retire missing and symlinked roots,
leave an unjournaled nomination alone, and confirm deletion works with no
namespace. Disabling the identity comparison fails the replacement test.

One standalone run under heavy concurrent host load failed the unrelated
`logs::tests::concurrent_process_style_writers_keep_all_committed_events` with
"log store busy"; the unloaded rerun passed. Sibling tests that fork briefly hold
an inherited copy of a just-closed lock descriptor, which also made the new store
tests and the pre-existing journal reopen test intermittently report "already
owned". Those tests now retry exactly that refusal (test code only); content and
ownership refusals are still asserted strictly. Later merges of `origin/main`
brought web, Claude plugin, tool, test-file and Controller `src/server.rs`
changes only; no Machine-owned production source changed after these gates. An
earlier activation dispatch was refused as stale by the owner's ancestry check
and the candidate was re-integrated and rebuilt, as required.

The exact active/preceding writers, fallback reader and default reader passed 32
native production conformance groups in private mount/PID/network namespaces,
now including two new assertions: the default reader never creates
`session-cleanups`, and the admitted writer owns an empty one. These are finite
journal/IPC/admission checks, not a filesystem-race, device or power-loss proof.

## Observation window

Samples at `05:36:52Z` and `05:50:15Z` bracket the activation. Machine PID changed
`200864` to `1705677`; Controller PID `520525`, the installed owner, sudoers
digest, host source `a0419eeb…`, reader floor and the single deletion entry
(`sess-1791127100570`) were unchanged. Nineteen of 20 worker/keeper units kept
their IDs, PIDs and active state. The twentieth, an execution keeper
(`893458`), logged its exit at `05:43:45Z`, before the activation started, and is
not attributed to it. The five public health/version/SPA/service-worker/Machine
endpoints returned 200. The Web transaction changed from `5b57fb56…` to
`04a68e99…` through independent tasks; this task activated only the resident
Machine.

## Limits

No live resume was exercised: nothing was pending at startup and no deletion
occurred in the window, so the production path is accepted only as startup,
admission and fixtures. The one existing deleted Session predates the record and
is a 44 MiB worktree with no Cargo `target` directory to reclaim. A crash between the journal commit and the
nomination leaves no record, and historical deletions are not reconstructed.
Creation time is required, and a device number changed by a reboot refuses rather
than matches. This is cleanup continuity only: no continuous Session/worktree
ownership, durable Session incarnation, I/O deadline, portable writer admission,
supported-device acceptance or crash/power-loss proof. The final identity
comparison/name unlink remains non-atomic and marker unlinks are not a
transaction. See the [machine-readable evidence](../experiments/plugin-session-cleanup-continuation-2026-10-05.json).
