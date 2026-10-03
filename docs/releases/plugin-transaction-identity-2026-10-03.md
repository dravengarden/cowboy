# Cowboy durable transaction identity — October 3

The explicit repair command already required an exact generated transaction ID,
but the owner's journal and success-receipt readers did not. Automatic recovery
uses the journal ID to name history files; repair selection also compares durable
IDs to decide whether a transaction already succeeded. Missing or malformed IDs
must refuse before those decisions or filesystem effects.

Published Columbus source `e516fc8c33ee88fe87b596b4776ce86845687cd9` shares one
validator between command admission, journal reads and success-receipt reads.
It requires a decimal timestamp beginning with a nonzero digit, followed by a
hyphen and 12 lowercase hexadecimal characters. Missing, null, empty, path,
uppercase and newline IDs refuse. This preserves the existing schema and
generated format. A compatible recovery retains the original transaction ID;
its recovered revision may differ from the ID suffix.

Pinned-shell `just verify`, all Machine Go packages with `go test -race ./...`
and `go vet ./...` passed. Sixteen malformed journal/receipt fixtures verify
refusal before profile resolution, ancestry or recovery-root effects. Invalid
journals also refuse automatic recovery before journal/history mutation.
Existing Controller, Web and Machine reopen and recovery receipt tests pass
with generated-format IDs. Production component receipt IDs were inspected and
already satisfy the rule. No malformed production record was seeded.

## Candidate build and initial activation refusal

The clean committed source integrates fresh Columbus main and the active host
revision. The owning Hawk build produced
`/nix/store/p7x3pqycziwpfmy5y4q401xgndy44a3x-nixos-system-hawk-26.05.20260731.5b4f72e`.
Candidate activator
`/nix/store/3rim03xsprcxwqsyl35b0jr82k2gmgwq-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`
has SHA-256 `7cd321be5284d0466a5c95193f5245bcf069013904ca24112ea9458596f564bf`.
Its public pre-dispatch command refused `../../escaped` with exit 1, no dispatch,
no component journal and unchanged Machine profile, receipt and floor.
This exercised the built candidate, not an installed owner update.

`machines/justfile activate hawk ./result` dispatched at
`2026-10-03T22:14:27+08:00`, but `hawk-activate.service` exited 42 before switching:

- `ovh-falcon-current403-observe-20261003.service` was already failed.
- `ovh-hawk-current409-observe-20261003.service` was already failed.

Both were unrelated transient observation units. The host contract refuses
unchanged failed units and prohibits clearing failure state to pass preflight.
They were left intact. The previous successful host receipt and installed owner
remained unchanged at source `3520f6821760ad97461290e720d0b517c145dc05`; this
change had **not shipped at that point**. The later repair and successful
activation are recorded below.

## Observation repair follow-up

After explicit authorization to handle the two observation failures, their
original samples showed that all connectivity, Cowboy inventory and HTTPS
checks had passed. They failed because Hawk/Falcon's expected revisions
409/403 were superseded by 410/404 while the observation was running; neither
daemon restarted. The original failures were retained rather than rewritten
as successful acceptance of the older revisions.

The same transient units ran the full 21-sample, 300-second observation against
current revisions 410/404, with fresh SSH connections avoiding the original
multiplexed connection's broken pipe. Both completed with 21/21 passing samples
and exit 0. Their temporary command overrides were then removed. No daemon,
overlay configuration, authentication material or Cowboy component was changed.

A further unrelated failed unit then blocked activation:
`hawk-jms-family-probe-20261003.service`. Its original probe build omitted
`with_utls` and could not create the isolated reality engine. The original task
had already rebuilt the binary with the tag; rerunning that repaired binary
then failed because the original experiment's token file had been removed.
No token or provider credentials were copied or recreated. This does not
establish a successful JMS network probe. Retirement was requested separately.
After explicit user confirmation, the unit definition, failure journal and
retirement receipt were archived under
`/var/lib/columbus/retired-probes/cowboy-transaction-identity-20261003`.
The task was stopped and its failed state reset as an authorized retirement of
an obsolete diagnostic unit. No replacement token was created and no JMS network
acceptance is claimed. The ordinary host transaction's refusal policy was not
changed. Evidence retains the original failures, retirement authorization and
all 42 successful successor observation samples.

The recursive built-unit comparison records `mandb.service` and system-path
references in AccountsService, D-Bus and polkit drop-ins, plus the user D-Bus
drop-in. Cowboy and worker unit definitions are unchanged. The initial preflight
refusal activated none of these changes.

Samples at `2026-10-03T14:13:20.117Z` and `2026-10-03T14:16:46.438Z` retained
all 13 ACP worker and four execution keeper PIDs. Machine PID `1928418` and
Controller PID `486493`, component receipt, floor bytes and Web profile were
unchanged. HTTPS health/version/SPA/SW/deployment-health returned 200; HTML/SW
kept `no-store`, SPA version stayed unchanged, and Machine remained online with
generation `worker-6ede7a91cc8b8b3402d4`. Deletion state remained only `.lock`.
These observations establish refusal continuity, not successful release or
full generation/native-resume acceptance.

## Successful owner activation

After a fresh remote/provenance check, the same clean committed closure activated
through `machines/justfile`. Transaction `1791039734507287815-e516fc8c33ee`
recorded success at `2026-10-03T23:02:17+08:00`, with published source
`e516fc8c33ee88fe87b596b4776ce86845687cd9`. All required health checks passed and
no new failed units were recorded. The installed activator matches the candidate
path and SHA-256 above. Its public command rejects an invalid repair ID before
dispatch, leaving the Machine profile, receipt and floor unchanged and creating
no component journal.

The host receipt lists only `mandb.service` among top-level changed unit files
and no explicit restart requests. The recursive comparison and switch journal
also record system-path drop-in effects: AccountsService stopped/started,
polkit restarted, D-Bus reloaded and the NixOS user activation units restarted.
The top-level receipt list is not a complete account of process changes.

Final samples at `2026-10-03T15:02:02.210Z` and
`2026-10-03T15:02:41.553Z` retained 16 of 17 original worker/keeper PIDs.
Machine PID `1928418` and Controller PID `486493` stayed unchanged. Worker
`sess-1789954176239` changed from PID `3899663` to `2984204`: at
`15:02:21.012147Z` the broker logged an exited, stale
`worker-2801f50d44994e96b2b4` worker before session revive, then started the
existing current generation `worker-6ede7a91cc8b8b3402d4`. This records the worker
transition; it does not prove native resume acceptance or unchanged worker PIDs.

Machine component receipt and floor bytes stayed unchanged; deletion state
remained only `.lock`. Host switching rewrote `/run/cowboy-web` from a direct
store target to the component-profile target, but both resolve to the same
`qia373a79gqz8mrq50fzv4mk95s4hw33` Web release. HTTPS health/version/SPA/SW and
deployment-health returned 200, HTML/SW kept `no-store`, SPA version
`798bda6db1a3a8958a6102125058e8e2` stayed unchanged, and Machine remained online
with the existing current generation.

The [machine-readable evidence](../experiments/plugin-transaction-identity-2026-10-03.json)
retains both activation attempts, probe retirement, successful host receipt,
recursive unit comparison, switch journal and actual process changes.
Cross-generation recovery and production deletion writing remain closed;
old independent tool authority and portable reader admission remain open work.
