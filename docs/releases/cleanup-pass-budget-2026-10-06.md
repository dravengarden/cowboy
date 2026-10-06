# Cleanup pass time budget — Hawk, October 6

Resident-only Machine release, active source `933acdd1bf0ac78eaaf390b09742d06943ca740b`,
artifact `/nix/store/av60w2k3smc7kdimn8jqwsyn5krsj070-cowboy-machine-writer-host-release`.
Root transaction receipt `933acdd1bf0a`: started `2026-10-06T13:00:08.984Z`,
committed `13:00:24.486Z`, succeeded, maintenance, no recovery. The preceding artifact
was `lp23w089…` (`551474a3`). The retained worker pin and generation
`worker-d62183a8…` are unchanged, so no worker was drained.

## Change

Each deleted-Session cleanup pass has a wall-clock budget: 5 minutes for the read-only
scan and 30 seconds for the removal phase, checked only between bounded steps (a
directory, a content entry), never inside marker finalization. An expired pass is a
retryable `CleanupDeadlineExceeded`, distinct from a root or target change: the retry
plan, handles and already-removed files carry the progress and the broker retries with
its normal backoff. The scan keeps no progress, hence its generous budget. See the
[contract](../plugin-session-cleanup-targets.md#pass-time-budget). It bounds how long a
pass holds the Session's lifecycle gate and a blocking thread over a long walk; it does
not interrupt a system call that never returns. The release also carries the bounded
retry of the installer probe's `exec` on ETXTBSY (already on `main` since `0fda7320`).

## Validation

Rustfmt, both Clippy configurations, 628 standalone and 1968 all-features tests. New
tests cover a pass that expires part-way through a target (deterministically, with no
reliance on timing) keeping the plan and markers and being finished by the next pass, an
expired scan having no effect and keeping no plan, a spent removal budget removing
nothing while keeping the scan, the default budgets leaving ordinary cleanup alone, and
the broker retrying rather than retiring on an expired budget. Removing the per-entry or
the scan check, or classifying the expiry as a change in the broker, fails them. The check
before each target is a defensive duplicate: every target has at least its two markers, so
the per-entry check already stops first; no test isolates it.
Native production conformance on exact artifacts of one worker generation (the active
`551474a3` writer against this writer, with reader-only releases): **45 groups accepted**
([receipt](../experiments/cleanup-pass-budget-native-conformance-2026-10-06.json)). These
are finite journal/IPC/admission checks; they do not exercise a slow filesystem.

## Production observation

Startup at `13:00:09Z` logged the deletion journal with `deleted_sessions=7
writer_enabled=true`, `Session incarnation namespace ready incarnations=6
writer_enabled=true` (the same 6 lineages read back after the restart) and `cleanup
continuations ready pending=0`. Samples at `13:00:05Z` and `13:01:26Z` bracket the
activation: all 27 worker/keeper units kept their IDs, PIDs and states; Machine PID
`2375981` → `3262101`; Controller PID `2867826`, the deletion floor, installed owner,
sudoers digest and host source unchanged; the five public endpoints returned 200; no
launch was refused for an unconfirmed lineage. The receipt's `published=false` reflects
that the commit was pushed afterwards.

## Limits

No real cleanup has hit its budget in production, so expiry is exercised only by fixtures.
A hung system call is not bounded, and a non-Linux pathname fallback checks only the scan
and the gaps between targets. A pass that keeps expiring counts toward the bounded
in-process retries (eight failures), after which a nominated Session relies on its durable
continuation at the next restart.
