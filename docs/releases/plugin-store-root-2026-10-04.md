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

## Candidate built; deployment pending

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
failure handling was requested as a separate scope extension. They were left
intact; activation was **not dispatched** and the candidate has **not shipped**.
The last successful host source remains
`ab151b1e22ee4a2181bc9af8287891805bc4f361`.

After their owning task resolves the failures, fetch fresh main and active
provenance before using the owning activation command. Rebuild if either has
advanced beyond the candidate. Do not clear failure state merely to pass the
host gate or replay an obsolete rollout.

Recursive candidate unit comparison records `mandb.service` and system-path
references in AccountsService, D-Bus and polkit drop-ins and the user D-Bus
drop-in. Cowboy/worker definitions are unchanged, and no candidate unit changes
were activated. A single observation at `2026-10-03T23:10:29.941Z` found Machine
PID `1928418`, Controller PID `486493`, 13 ACP workers and four keepers;
HTTPS health/version/SPA/SW/deployment-health returned 200. Production deletion
state remained only `.lock`. This is current health evidence, not activation
success or a before/after continuity experiment.

The [machine-readable candidate evidence](../experiments/plugin-store-root-2026-10-04.json)
retains source and immutable artifact identity, blocker states, actual current
host receipt, unit comparison and current observations. Production deletion
writing and cross-generation recovery remain closed. Portable reader admission
and independent old-tool authority remain open work.
