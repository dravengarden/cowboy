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

## Candidate built; activation blocked

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

Both are unrelated transient observation units. The host contract refuses
unchanged failed units and prohibits clearing failure state to pass preflight.
They were left intact. The previous successful host receipt and installed owner
remain unchanged at source `3520f6821760ad97461290e720d0b517c145dc05`; this change
has **not shipped**. After the observation task resolves its failures, rerun
the owning activation with fresh remote/provenance checks. Rebuild if main has
advanced beyond the candidate.

The recursive built-unit comparison records `mandb.service` and system-path
references in AccountsService, D-Bus and polkit drop-ins, plus the user D-Bus
drop-in. Cowboy and worker unit definitions are unchanged. Since preflight
refused, none of these candidate changes were activated.

Samples at `2026-10-03T14:13:20.117Z` and `2026-10-03T14:16:46.438Z` retained
all 13 ACP worker and four execution keeper PIDs. Machine PID `1928418` and
Controller PID `486493`, component receipt, floor bytes and Web profile were
unchanged. HTTPS health/version/SPA/SW/deployment-health returned 200; HTML/SW
kept `no-store`, SPA version stayed unchanged, and Machine remained online with
generation `worker-6ede7a91cc8b8b3402d4`. Deletion state remained only `.lock`.
These observations establish refusal continuity, not successful release or
full generation/native-resume acceptance.

The [machine-readable evidence](../experiments/plugin-transaction-identity-2026-10-03.json)
retains candidate hashes, blocker states, unchanged host receipt, recursive unit
comparison and process samples. Cross-generation recovery and production
deletion writing remain closed; old independent tool authority and portable
reader admission remain open work.
