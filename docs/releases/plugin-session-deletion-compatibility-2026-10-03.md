# Machine Session deletion reader compatibility — Hawk, October 3

The reader-first release now declares its finite dataset capability in immutable
Machine provenance. The Columbus component activator refuses a Machine without a
declared schema-1 reader whenever a committed deletion entry exists. This closes
the installed component lane's legacy-reader admission gap; it does not admit
production writers or complete general state compatibility/recovery.

## Source and verification

- Cowboy declaration: `dd05752740bbeee8056d1da1edb86dea86b2a30b`.
- Integrated Cowboy release: `ae00cdc884de0cad92f303731ce52541a837efc5`.
- Columbus owner: `1e6672372cc00b6a037b4e92efa9a9a381904115`.
- Columbus pinned-shell `just verify`, all Machine activation Go packages with
  `go test -race ./...`, and `go vet ./...` passed. Five new fixture tests cover
  closed declarations, writer/foreign-lane refusal, empty versus committed
  namespaces, replaced roots and rollback refusal before profile mutation.
- The clean committed Hawk system was built through `machines/justfile`.
- The integrated immutable Cowboy Machine and source-boundary check built. The
  default-feature Cowboy package check passed 1,302 unit tests with 24 ignored,
  plus its enabled integration target. Machine Rust source was unchanged by this
  declaration slice; its production writer remains hard-coded off. This is not a
  new all-feature Rust gate or an executable crash matrix.

The first Columbus Nix gate omitted newly created, untracked Go files; staging
them made the full gate pass. The initially inherited sparse PATH also lacked
`hostname` and selected the non-setuid sudo binary. Re-entering the pinned shell
with the normal system and wrapper paths resolved those environment failures. No
failed build or dispatch was activated.

## Admission boundary

`sessionDeletionJournal` declares exactly `readerSchema: 1`, `writerSchema: 0`.
The owner rejects missing/unknown/duplicate declaration fields, unsupported
readers, foreign lanes and every nonzero writer declaration. Missing capability
metadata denotes an undeclared legacy reader, including the prior production
reader envelope that predated this declaration.

On the configured Hawk/Falcon dataset path, only absence of `deletions.json`
permits legacy compatibility. A corrupt file, directory or dangling committed
symlink still requires a reader. The owner does not parse, repair, delete or
replay Machine records. Candidate dispatch and locked candidate/fallback
admission check the requirement. Profile restoration checks it before mutation;
interrupted recovery checks both Git-pinned candidates and already-healthy
predecessors rather than letting the healthy shortcut bypass admission.

The installed component owner is required for these checks. Older host
activators, portable signed updater recovery and independently accepted Machine
recovery remain outside this slice. A durable owner reader floor must still be
admitted before the first write. No declaration or constructor toggle supplies
that authority. Existing source fixtures are not independently supplied old/new
executable, OS-process crash or power-loss acceptance.

## Production receipts

The host owner activated successfully at `2026-10-03T16:01:32+08:00`, published,
from
`/nix/store/xmc1jyy5vsgsx9wf523vyb07lqbj6yck-nixos-system-hawk-26.05.20260731.5b4f72e`.
The installed activator is
`/nix/store/xq5gkazafjqwk40nzcxyn7vr4srx228s-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`.
Cowboy Controller and network units compared unchanged before dispatch. The host
receipt lists only `mandb.service` as changed and no failed health checks. One
pre-existing, finished transient Stormbird observer had blocked preflight before
any switch. Its `MainPID` was zero; only its failed marker was reset, retaining
its journal, before the successful retry.

Machine transaction `1791014541293399326-ae00cdc884de` committed successfully at
`2026-10-03T08:02:34.640476318Z`, published, from
`/nix/store/vb8kp467y6lmwsa4h6ga5fv0ygf2rkpc-cowboy-machine-release`. It
replaced `/nix/store/f24jdxk9c6lcr15b40ws9iixhbflpmvf-cowboy-machine-release`.
Integrating concurrent Provider changes from main changed the desired generation
to `worker-44e379c40a139cd05cc7`; the Controller's live deployment-health
endpoint reported exactly that generation, connected and online. This generation
report does not assert that all old busy workers were replaced or that native
resume was exercised.

The resident Machine PID changed from `3409731` to `3759060`. Controller PID
`1998362` remained unchanged. All 14 original ACP worker and two execution
keeper units retained their PIDs and active/running state in the bounded samples
across both host and Machine activation. Health/version, SPA index and service
worker returned 200; index/SW retained `no-store`. The separately released SPA
version was `1027d6ec677b82888789522db411aa0b` in these samples.

At startup the production reader reported zero terminal IDs and
`writer_enabled=false`. Its namespace still contained only `.lock`. No terminal
records were seeded, migrated or written. Newly deleted production Sessions
therefore remain process-local. No Controller, Web, native-device or iOS release
was made by this slice.

The
[captured receipts and continuity samples](../experiments/plugin-session-deletion-compatibility-2026-10-03.json)
retain the exact observations. The
[dataset contract](../plugin-session-deletion-journal.md) records the remaining
writer-admission and identity boundaries.
