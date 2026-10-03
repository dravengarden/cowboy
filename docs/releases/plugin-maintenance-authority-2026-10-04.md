# Durable Cowboy maintenance authority — October 4

New Machine transactions and explicit interrupted-transaction selection require
maintenance authorization. Automatic recovery previously read the old journal
without checking that its original authorization survived. Success-receipt
reads also did not check whether the maintenance flag agreed with the lane.

The owner now requires Machine journals and success receipts to retain
`maintenance: true`. Missing, null or false Machine authorization refuses;
Web and Controller records cannot carry Machine maintenance authority.
Automatic recovery, active-revision acceptance and explicit repair selection
refuse invalid original records before profile or recovery-selection effects.
A later invocation's flag cannot substitute for lost durable authorization.
Refusal preserves evidence instead of rewriting it as authorized.

The state schema and writer format are unchanged. The new checks do not enable
deletion writing, cross-generation repair, portable reader admission or fence
independently run old activators. Existing invocation authorization remains
required; a previous success receipt does not authorize a new maintenance job.

## Source and verification

Published and activated Columbus source is
`ab151b1e22ee4a2181bc9af8287891805bc4f361`. The isolated owner task worktree
integrates fresh remote main, including unrelated qualification documents,
and the previously active host source `193565d0`.

Pinned-shell `just verify`, final verification, all Machine Go packages with
`go test -race ./...` and `go vet ./...` passed. Ten journal/receipt fixtures
cover missing/null/false Machine authorization and true authorization on
Controller/Web. They exercise automatic journal recovery, active-revision
acceptance and supported explicit repair selection, requiring refusal before
repair ancestry, profile resolution or target rooting. Journal, receipt and
floor bytes remain unchanged. Existing valid commit, rollback and explicit
repair replay fixtures continue to pass. The Machine reopen fixture was corrected
to retain the authorization already required by its actual transaction writer.

Production receipt inspection found Machine true and Controller/Web false.
No production authorization record was corrupted, repaired or seeded to test
refusal; these are source fixtures, not a failed production Machine experiment.

## Owner activation

The clean committed Hawk build and activation used the owning
`machines/justfile` transaction and closure
`/nix/store/ny3s2x2yflxmrr0k7hbp3g96mkvjsnmd-nixos-system-hawk-26.05.20260731.5b4f72e`.
Transaction `1791043636619383248-ab151b1e22ee` succeeded, published, at
`2026-10-04T00:07:19+08:00`. Required health checks passed and no new failed units
were recorded. No unrelated failed unit was cleared or retired in this release.

Installed activator
`/nix/store/i63ay3vpcpq126n1s29b7ajgd7svn3kq-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`
has SHA-256 `06b712e4dcc205d1ffaec58c4b114c7acad66a4d235958433bd88b778de4945a`.
The host receipt's top-level changed-unit list includes only `mandb.service`
and no explicit restarts. The recursive comparison also records system-path
references in AccountsService, D-Bus and polkit drop-ins and the user D-Bus
drop-in. The actual switch stopped/started AccountsService, restarted polkit,
reloaded D-Bus and restarted NixOS user activation units. Those effects cannot
be inferred solely from the receipt's top-level unit list.

Samples at `2026-10-03T16:06:28.188Z` and `2026-10-03T16:09:11.914Z` retained
all 13 ACP worker and four execution keeper PIDs. Machine PID `1928418` and
Controller PID `486493` stayed unchanged. Machine component receipt, floor bytes
and deletion state containing only `.lock` also stayed unchanged. The resolved
Web target and SPA version `798bda6db1a3a8958a6102125058e8e2` were unchanged.
HTTPS health/version/SPA/SW and deployment-health returned 200, HTML/SW retained
`no-store`, and Machine remained online with generation
`worker-6ede7a91cc8b8b3402d4`. No component journal remained. These are bounded
process observations, not full generation rollout or native resume acceptance.

The [machine-readable evidence](../experiments/plugin-maintenance-authority-2026-10-04.json)
retains source and closure identity, installed hash, host receipt, receipt
authorization inspection, unit comparison, switch journal and process samples.
Production deletion writing and cross-generation recovery remain closed;
portable readers and independent old-tool authority remain open work.
