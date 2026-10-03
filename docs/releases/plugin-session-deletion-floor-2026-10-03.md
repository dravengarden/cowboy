# Durable Machine deletion reader floor — Hawk, October 3

The installed component owner now retains a schema-1 reader floor independently
of the user-owned deletion dataset. An empty or missing dataset can no longer
admit an undeclared reader. Explicit Machine recovery is available within the
candidate's exact worker generation. Production Session deletion writing remains
disabled; this is not completion of general state leases or recovery admission.

## Source and checks

- Columbus owner: `934568f73ca97a43a3d761067b370e047cc672f3`.
- Cowboy contract: `392575e715f612fd734615023afd306842af635f`.
- Integrated Cowboy release: `38453c13456bddb4261572ab270354e47b2ffc0e`.
- Pinned-shell Columbus `just verify`, all Machine activation Go packages with
  `go test -race ./...`, and `go vet ./...` passed.
- Four new test groups cover floor reopen with missing/empty Machine state,
  pre-mutation legacy rollback refusal, corrupt/owner-mismatched/bounded closed
  floor records, independent recovery ancestry, changed reader/generation
  refusal, journal reopen and actual recovery receipt identity.
- The clean committed Hawk host was built and activated through the owning
  `machines/justfile` transaction.
- The integrated immutable Machine release and source-boundary check built. Its
  Cowboy default-feature package checks passed 1,302 unit tests, 24 ignored, and
  the enabled three-test integration target. Machine Rust source and the
  production writer constructor were unchanged by this slice.

Concurrent main changes included Provider revisions and Web fixes. Integrating
them changed the release's desired worker generation to
`worker-e0ccbb90546656dbb013`; the Web changes were not deployed by this task.

## Owner floor and recovery contract

The root-owned Machine component receipt directory contains
`session-deletion-reader-floor.json`. The bounded, closed record binds Machine,
dataset path, reader schema and exact accepted reader anchor release/revision.
An activation first verifies that its active compatible fallback matches the
successful receipt, retains that release as a Nix GC root, then atomically
writes and syncs the floor before changing the profile. Initial reader-only
adoption from a legacy artifact is still possible; the following activation
anchors the accepted reader. No historical Session deletions are reconstructed.

Once admitted, the floor is not automatically removed or downgraded. Namespace
absence does not clear it. Invalid, mismatched, oversized, symlinked, duplicate,
unknown or trailing owner state refuses admission. Profile restoration and
interrupted recovery continue to check the floor before taking effect.

An explicit Machine `--recovery-release` must declare the compatible reader and
match the candidate generation and lane. It must pass fresh-main and active
provenance floors and be an ancestor of the candidate. The exact target is
retained and recorded; reopen and rollback revalidate its identity, reader and
generation. Failure never selects the legacy predecessor. The recovery receipt
records the actual recovered release/revision and the validated generation.
These are source fixtures; no intentionally failed production recovery
transaction was run.

Cross-generation recovery, Machine `--recover-transaction` repair selection,
older host activators and the portable signed updater remain outside this
admission. An independently supplied old/new executable matrix, OS-process
crash/reopen/failure and power-loss acceptance are still open. No writer may be
enabled merely by changing metadata or a constructor boolean.

## Live receipts and negative acceptance

Hawk host activation succeeded, published, at `2026-10-03T16:37:56+08:00` from
`/nix/store/h56zhm1aq225j30752ha8v03mmfa8amv-nixos-system-hawk-26.05.20260731.5b4f72e`.
The installed activator is
`/nix/store/99klkmkxs5514sdwa6cac4qll1fh8ydh-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`.
Controller and network units compared unchanged; the host receipt lists only
`mandb.service` as changed and no failed health checks.

Machine transaction `1791016928619981597-38453c13456b` committed successfully,
published, at `2026-10-03T08:42:17.146912285Z` from
`/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release`. It
replaced `/nix/store/vb8kp467y6lmwsa4h6ga5fv0ygf2rkpc-cowboy-machine-release`.
The Controller reported connected, online and the exact desired generation.

The 251-byte floor is owned by root with mode `0644`; its schema is 1 and its
anchor revision is `ae00cdc884de0cad92f303731ce52541a837efc5`. The anchor
resolves to the prior accepted reader release and was confirmed as a Nix GC
root. Invoking the undeclared predecessor
`/nix/store/f24jdxk9c6lcr15b40ws9iixhbflpmvf-cowboy-machine-release` through the
installed activator returned exit 1 and
`durable Session deletion reader floor refuses legacy Machine` before dispatch.
The profile and success receipt were byte-for-byte unchanged, and no incomplete
component journal was created. This is old-artifact admission evidence; its
executable was not started.

Machine PID changed from `3759060` to `70549`. Controller PID `1998362` and all
14 original ACP worker plus two execution keeper PIDs were unchanged in the
bounded samples across host and Machine activation. No full worker replacement
or native-resume claim follows from those samples or the generation report.
Health/version/index/service-worker requests returned 200; index/SW retained
`no-store`. The separately released SPA version was
`4aae6c684d7dd1914dd7520ccfa82cb4` in these samples.

The Machine reader reported zero deleted IDs and `writer_enabled=false` at
`2026-10-03T08:42:08.703306Z`. Its namespace still contained only `.lock`. No
deletion records were seeded or written; newly deleted production Sessions
remain process-local. No Controller, Web, native-device or iOS release was made
by this slice.

The
[receipt, floor and negative/continuity evidence](../experiments/plugin-session-deletion-floor-2026-10-03.json)
retains exact observations. The
[dataset contract](../plugin-session-deletion-journal.md) records the remaining
writer and identity boundaries.
