# Exact same-generation Machine transaction repair — October 3

Machine component recovery already rechecked the reader and worker generation
of an explicitly accepted target, but `--recover-transaction` rejected Machine
entirely. The Columbus owner now permits selection of a new compatible target
for one exact failed Machine transaction:

```sh
cowboy-release-activate --maintenance --recover-transaction <exact-id> <release>
```

The original journal must await failed rollback, retain its maintenance
authorization and have no successful receipt for that transaction. The selected
target must declare the schema-1 deletion reader and writer schema 0, match the
failed candidate's worker generation, pass the durable owner floor/dataset
checks, and belong to the same lane. It must integrate fresh main, active
provenance, the original failed candidate and any previously selected recovery
revision. Selection also requires that the active profile still belongs to that
transaction.

Under the deployment lock, the owner retains an independent GC root, archives
the prior decision and only then replaces the recovery target in the journal.
Original candidate, predecessor and generation remain unchanged. Retry of the
same durable selection is idempotent. Selection publishes no success receipt;
the existing rollback, health, pinning and receipt engine still owns recovery.
Changed or unavailable selected targets never fall back to the predecessor.
Cross-generation repair and Web failed-transaction selection remain refused.

## Source and verification

Columbus source is `e2799fe62a46f3d3f4443929327886e86ad938bf`, published to
remote main from a freshly fetched isolated task worktree. It integrates the
active Hawk source `934568f73ca97a43a3d761067b370e047cc672f3`. Pinned-shell
`just verify`, all Machine Go packages with `go test -race ./...`, and
`go vet ./...` passed.

Three new Machine test groups cover exact/stale/missing transaction IDs,
completed or nonfailed state, wrong lane, absent/incompatible reader, nonzero
writer, changed generation, absent original maintenance, malformed floor and
profile drift. Separate ancestry, GC-root and archive failures preserve the
original journal. Selection reopens after an archived intent, keeps original
identity/generation, does not promote an unchecked target and reuses the same
decision without changing history. Changed reader or generation cannot revive
the predecessor; detached arguments retain explicit maintenance and transaction
ID. Existing Controller and Web recovery tests continue to pass.

These are deterministic owner fixtures, not an intentionally failed production
Machine transaction, power-loss acceptance or cross-generation recovery proof.
No Session deletion writer or portable-device reader admission changes.

## Owner activation

The clean committed Hawk configuration was built and activated through the
owning `machines/justfile` transaction. Host activation
`1791028162804426820-e2799fe62a46` succeeded and was published at
`2026-10-03T19:49:25+08:00`, from
`/nix/store/ixabfhw01zdswm5gf9ib9ff8rlr5s0k5-nixos-system-hawk-26.05.20260731.5b4f72e`.
Both the built unit comparison and host receipt list only `mandb.service` as
changed, no changed user units and no explicit restarts. All required health
checks passed and no new failed units were recorded.

The installed activator is
`/nix/store/9lifjmfsc0m73prsavk6bik574zv29p8-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`,
SHA-256 `be95e77085e992de2363a8b473674a138f676585e75a7f65b1c1cfb3cf84aeeb`.
A valid-shaped exact Machine repair ID with the undeclared predecessor
`/nix/store/f24jdxk9c6lcr15b40ws9iixhbflpmvf-cowboy-machine-release`
was rejected before dispatch with exit 1 and
`durable Session deletion reader floor refuses legacy Machine`. Profile,
current component receipt and floor bytes were unchanged; no component journal
was created. This proves the installed Machine option path still checks the
floor, not a successful production repair.

Bounded samples at `2026-10-03T11:46:20.467Z` and
`2026-10-03T11:50:38.058Z` retain 16 of the 17 original worker/keeper PIDs.
One worker for `sess-1790913572903` changed from `1522427` to `2094923`
before the host transaction began at `2026-10-03T11:49:22.804426820Z`.
The resident broker recorded recycling an exited, already-draining old worker
at `11:48:40.788568Z`, from `worker-eed1d8105af00846771d` toward the previously
deployed `worker-6ede7a91cc8b8b3402d4`; its replacement logged successful
`session/resume` at `11:48:45.186198Z`. This is one observed prior worker
transition, not full generation-rollout or native-resume acceptance.

Machine PID `1928418`, Controller PID `486493`, Web profile, root-owned reader
floor bytes and current Machine component receipt were unchanged across the
host activation. HTTPS health/version/SPA/SW/deployment-health returned 200;
HTML/SW retained `no-store`, the separate SPA version was unchanged, and Machine
reported connected/online with `worker-6ede7a91cc8b8b3402d4`. Production deletion
state remained only `.lock`; the unchanged Machine writer remains disabled.
No record was seeded, no production repair was performed, and no Cowboy
component, portable device or iOS release was activated.

The [machine-readable receipt and observations](../experiments/plugin-machine-repair-2026-10-03.json)
retain the host receipt, exact activator hashes, before/after component samples,
pre-dispatch refusal and the independently timed worker transition. Old
activator authority, portable admission, cross-generation recovery and writer
release acceptance remain open.
