# Canonical Cowboy durable release roots — October 4

Explicit recovery identities already required clean direct children of
`/nix/store`, but journal candidate/predecessor and receipt release/active paths
used prefix checks. Receipt predecessors were not validated. A prefix accepts
subdirectories, traversal and redundant separators, weakening the identity
used by recovery and active-revision acceptance.

Published Columbus source `f5096b38d24d2dcca37c8a96cd194bfdbe0173d3` shares
the canonical store-root predicate between explicit recovery fields, journal
candidate/predecessor and receipt release/active/predecessor paths. Paths must
be clean direct children of `/nix/store`. Empty optional predecessors retain
bootstrap semantics. Refusal never normalizes or rewrites path evidence.
Existing source, lane, reader and generation checks still decide whether an
immutable root is an accepted release; a lexical path check does not replace
manifest validation. Web asset paths use their separate existing contract.

Pinned-shell `just verify`, all Machine Go packages with `go test -race ./...`
and `go vet ./...` passed. Thirty-two fixtures cover journal release/predecessor
and receipt release pairs/predecessor with store subdirectories, traversal,
redundant separators and trailing dot/slash forms. Receipt release/active pairs
remain equal, isolating the newly rejected prefix-only cases. Canonical baseline
records first reopen successfully; malformed records then refuse direct reads,
automatic journal recovery and active-revision acceptance without rewriting
evidence. Existing valid reopen, recovery and receipt tests pass. Production
receipt roots on all three lanes were inspected and are already canonical.
No malformed production path or failed production Machine transaction was seeded.

## Candidate build and initial deployment hold

The isolated owner task worktree integrates fresh remote main and active source
`ab151b1e`. Its clean committed owning Hawk build produced
`/nix/store/nscaasxka172z73ga4z3vxvam7yaw4jl-nixos-system-hawk-26.05.20260731.5b4f72e`.
Candidate activator
`/nix/store/4z6c52b17g2dd6n9zkm5bdpa8jspxzvc-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`
has SHA-256 `2356982aa7582d607c60126f24451740ad5eda0c4513c7ee4a9d9c89b45e5f88`.

Read-only pre-deployment inspection found two unrelated failed Stormbird tasks:

- `stormbird-hawk-jms-family-eb8c9ba-20261004.service`: exit-code result, status 1,
  exited at `2026-10-04T01:42:11+08:00`.
- `stormbird-jms-family-host-stream-r1.service`: timeout result, status 15,
  stopped at its runtime limit at `2026-10-04T05:02:33+08:00`.

The host contract refuses unchanged failed units. These are new rollout/data
stream tasks, not the previously authorized obsolete probe retirement. Their
failure handling was requested as a separate scope extension. They were initially
left intact; activation was **not dispatched at that point**. The previous
successful host source was
`ab151b1e22ee4a2181bc9af8287891805bc4f361`.

The later authorized retirement and successful activation are recorded below.
The ordinary host gate's failed-unit policy remains unchanged.

Recursive candidate unit comparison records `mandb.service` and system-path
references in AccountsService, D-Bus and polkit drop-ins and the user D-Bus
drop-in. Cowboy/worker definitions are unchanged. A single observation before
activation at `2026-10-03T23:10:29.941Z` found Machine
PID `1928418`, Controller PID `486493`, 13 ACP workers and four keepers;
HTTPS health/version/SPA/SW/deployment-health returned 200. Production deletion
state remained only `.lock`. This is current health evidence, not activation
success or a before/after continuity experiment.

## Authorized retirement and successful activation

The user instructed this candidate to go live after the question naming the
two deployment blockers. Read-only inspection confirmed both transient tasks
had stopped with MainPID 0. The wave's terminal event was
`paused_healthy_candidate`, reason `recovered_gate_failure_operational_retention`,
at revision 410 with connected state. Its `paused-retained` marker existed and
no JMS-family rollback timer was pending. Current Stormbird revision 411 was
connected with current control-plane state. The timed-out host stream's task
directory was already absent. Neither task was restarted or replayed.

Their unit definitions, journals and explicit retirement receipt were archived
under `/var/lib/columbus/retired-tasks/cowboy-store-root-20261004`. They were
stopped and their failed states reset as the authorized retirement of terminated
tasks. The wave directory and paused marker bytes were preserved. No network
configuration, daemon profile or Stormbird release acceptance was changed or
claimed; a paused operational candidate is not a passed rollout gate. This was
specific user-authorized cleanup, not a new automatic host-gate exception.

Fresh remote and active provenance still matched the same clean committed
candidate. The owning `machines/justfile` activation succeeded, published, as
transaction `1791069502031885650-f5096b38d24d` at
`2026-10-04T07:18:24+08:00`. The active closure is the candidate above; installed
activator path and SHA-256 match exactly. Required health checks passed and no
new failed units were recorded.

The host receipt lists only `mandb.service` among top-level changed unit files
and no explicit restarts. The actual switch journal also records AccountsService
stopped/started, polkit restarted, D-Bus reloaded and NixOS user activation units
restarted, consistent with the recursive drop-in comparison. The top-level
receipt list is not complete process-change evidence.

Samples at `2026-10-03T23:17:39.781Z` and `2026-10-03T23:19:37.954Z` retained
all 13 ACP worker and four keeper PIDs. Machine PID `1928418` and Controller PID
`486493`, Machine component receipt and reader-floor bytes stayed unchanged.
The resolved Web target and SPA version `798bda6db1a3a8958a6102125058e8e2` were
unchanged. HTTPS health/version/SPA/SW/deployment-health returned 200, HTML/SW
kept `no-store`, and Machine remained online with generation
`worker-6ede7a91cc8b8b3402d4`. Deletion state remained only `.lock`; no component
journal remained. These are bounded process observations, not full generation
rollout or native resume acceptance.

The [machine-readable release evidence](../experiments/plugin-store-root-2026-10-04.json)
retains the original hold, authorized task retirement, successful host receipt,
immutable artifact identity, recursive unit comparison, switch journal and
process samples. Production deletion writing and cross-generation recovery
remain closed. Portable reader admission and independent old-tool authority
remain open work.
