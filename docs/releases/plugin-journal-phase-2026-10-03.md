# Closed Cowboy component journal phases — October 3

The owner validated durable JSON fields and transaction IDs, but accepted any
journal phase. Automatic recovery could turn a missing or unknown phase into
`rolling-back` and continue profile recovery. A `recovery-selected` journal with
its target omitted could also fall back to the predecessor instead of retaining
the explicit selection's authority.

The reader now admits only the eight phases written by the existing owner:
`prepared`, `profile-set`, `restarting`, `checking`, `committed`, `rolling-back`,
`recovery-required` and `recovery-selected`. Missing, null, empty, unknown,
uppercase and receipt-only terminal phases refuse before automatic recovery
rewrites the journal, names history files, probes health or changes a profile.
`recovery-selected` additionally requires the Controller or Machine lane and a
nonempty explicit recovery release/revision. Existing recovery-field validation
and source, ancestry, reader-floor and generation guards remain in force.

No new state schema or writer format is introduced. Existing valid commit and
rollback replay continue unchanged. The validation does not enable deletion
writing, cross-generation recovery, portable reader admission or fence old
independently invoked activators.

## Source and verification

Implementation commit is `bd17c44b`; the published, activated Columbus source is
`193565d02437b1fe2ffdb265c1fdf1c75a07f6c7`. The isolated owner task worktree
integrates fresh remote main and the active host source `e516fc8c`. A concurrent
portable qualification document landed during publication; the deployment build
gate correctly refused the stale source before building. It was merged and the
complete gate passed again before the final clean committed Hawk build.

Pinned-shell `just verify`, all Machine Go packages with `go test -race ./...`
and `go vet ./...` passed. Twenty-four malformed automatic-recovery fixtures
cover all three lanes, including selected phase without a target. They prove
refusal preserves the original journal bytes and receipt history. Twenty-three
valid phase/lane combinations reopen with their recovery identity unchanged;
the remaining Web `recovery-selected` combination refuses the unsupported
failed-transaction selection authority. Existing recovery replay tests pass.
All malformed state is confined to temporary fixtures: no production journal,
invalid phase or failed Machine transaction was seeded.

## Owner activation

The owning `machines/justfile` build and activation used
`/nix/store/lzsvki29bah3pn6nnxkjbnqls33mn3rw-nixos-system-hawk-26.05.20260731.5b4f72e`.
Transaction `1791041700033399234-193565d02437` succeeded, published, at
`2026-10-03T23:35:02+08:00`. Required health checks passed, with no new failed
units. No failed unit was cleared or retired in this release.

The installed owner is
`/nix/store/r5l93w52n82gqg9195rskmhx8nnpacp3-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`,
SHA-256 `a8e3b5649b615ceae626e9a10541338d11eb67b127da9788a4248d52846605e3`.
The host receipt lists only `mandb.service` among top-level changed unit files
and no explicit restarts. The recursive unit comparison also records system-path
references in AccountsService, D-Bus and polkit drop-ins and the user D-Bus
drop-in. The switch journal records AccountsService stopped/started, polkit
restarted, D-Bus reloaded and NixOS user activation units restarted. The receipt's
top-level unit list alone is not complete process-change evidence.

Samples at `2026-10-03T15:34:42.981Z` and `2026-10-03T15:35:28.000Z` retained
all 13 ACP worker and four execution keeper PIDs. Machine PID `1928418` and
Controller PID `486493` stayed unchanged, as did the Machine component receipt,
reader-floor bytes and deletion state containing only `.lock`. The Web target
resolved to the same immutable release and SPA version
`798bda6db1a3a8958a6102125058e8e2` remained unchanged. HTTPS health/version/SPA/SW
and deployment-health returned 200, HTML/SW kept `no-store`, and Machine remained
connected/online with generation `worker-6ede7a91cc8b8b3402d4`. No component
journal remained. These observations do not prove full generation rollout or
native resume acceptance.

The [machine-readable evidence](../experiments/plugin-journal-phase-2026-10-03.json)
retains source and closure identity, installed hash, actual host receipt,
recursive unit changes, switch journal and bounded process observations.
Production deletion writing and cross-generation recovery remain closed;
portable readers and independent old-tool authority remain open work.
