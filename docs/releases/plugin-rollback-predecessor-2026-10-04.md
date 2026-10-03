# Cowboy rollback predecessor admission — October 4

Ordinary rollback previously returned the journal's predecessor path without
opening its release manifest or checking its lane. Controller and Web restoration
could therefore change the profile before health checks discovered an unavailable,
incomplete or wrong-lane predecessor. Machine's reader guard covered availability
but did not independently bind this ordinary rollback target to the recorded lane.

Published Columbus source `8ad136700572f5356c946c6494e34e9dfb57484f`
validates every nonempty ordinary predecessor with `ValidateSource` and requires
its lane to match the journal before returning a rollback target. Both ordinary
failure rollback and interrupted recovery use this target check before profile
restoration or service restart. Interrupted recovery may still persist its
`rolling-back` intent before rejecting an invalid predecessor; this change does
not promise byte-preserving refusal for a structurally valid journal.

Machine bootstrap remains a valid ordinary predecessor. Explicit recovery still
uses `ValidateCandidateSource`, rejects bootstrap and retains the selected
release/revision, reader and same-generation checks without falling back to the
implicit predecessor. Empty predecessors retain existing bootstrap removal
semantics; Machine reader-floor admission remains a separate gate. Independent
Web assets retain their existing restoration behavior after a valid target has
been selected, including restoration when the profile operation fails.

Pinned-shell `just verify`, all Machine Go packages with `go test -race ./...`
and `go vet ./...` passed. Fixtures cover all nine transaction/predecessor lane
pairs, missing and malformed manifests, incomplete releases on each lane,
accepted Machine bootstrap and empty predecessors. Existing explicit selected
recovery, interrupted recovery, reader-floor and independent Web asset tests
pass. Read-only inspection found same-lane source manifests for the current
production predecessor on each lane. No invalid production journal was seeded.

The isolated, clean committed owner worktree integrated fresh remote main and
active source `f5096b38`. Its owning Hawk build produced
`/nix/store/5kmpi3ix5j8a4imz8699mrfbwzjbi783-nixos-system-hawk-26.05.20260731.5b4f72e`.
Activation succeeded and published as transaction
`1791070593243991365-8ad136700572` at `2026-10-04T07:36:36+08:00`.
Required host health checks passed with no new failed units. No unrelated failed
unit was cleared during this release.

The installed activator is
`/nix/store/yipvlh2cvi4j9a6dnp71lkj1w61lfvk5-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`,
SHA-256 `82173004ae2280a6bc4a5b6acd450515ead66d5a309381a440cb3006ab504087`.
Recursive unit comparison found `mandb.service` and system-path changes in
AccountsService, D-Bus and polkit drop-ins, including user D-Bus. Cowboy/worker
unit definitions were unchanged. The switch journal records AccountsService
stopped/started, polkit restarted, D-Bus reloaded and NixOS user activation units
restarted. The host receipt's top-level changed-unit list alone does not capture
these process changes.

Samples at `2026-10-03T23:36:17.176Z` and `2026-10-03T23:36:56.434Z` retained
all 13 ACP worker and four keeper PIDs, Machine PID `1928418` and Controller PID
`486493`. The Machine component receipt, reader-floor bytes, resolved Web target
and SPA version `798bda6db1a3a8958a6102125058e8e2` were unchanged. HTTPS
health/version/SPA/SW/deployment-health returned 200; HTML/SW kept `no-store`.
Machine stayed connected with generation `worker-6ede7a91cc8b8b3402d4`.
Deletion state remained only `.lock`; no Machine component journal remained.
These bounded samples are not full generation rollout or native resume acceptance.

The [machine-readable evidence](../experiments/plugin-rollback-predecessor-2026-10-04.json)
records immutable artifact identity, production predecessor inspection, host
receipt, recursive unit differences, switch journal and continuity samples.
Production deletion writing and cross-generation recovery remain closed.
Independent older activation authority and portable compatible-reader admission
remain open work.
