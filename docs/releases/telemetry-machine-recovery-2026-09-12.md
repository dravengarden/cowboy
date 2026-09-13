# Machine binding recovery reader-only release — 2026-09-12

Code: `d6d48b2abc902086ec929c81d6d46c78c63f14ea`, published to Cowboy main after
rebasing onto the independent website artwork change `aad59f76`. This delivers
the staged [Machine recovery contract](../telemetry-machine-recovery.md), not
production writer admission, automatic repair or completed P2.

## Verification and immutable artifacts

The final integrated source passed the complete pinned
`nix develop -c just check-compact` gate: **1,006 Rust library tests passed**
(17 intentionally ignored), **1,371 Web tests passed**, and all **15 isolated
PostgreSQL tests passed**, with strict Rust lint, dependency checks, independent
Machine/adapter feature checks, Plugin isolation and release builds. This slice
adds 21 tests. A separate recovery-focused run passed 42 tests, including prior
recovery contracts and the new real signed-Machine interruption/JSON/CLI path.

The initial gate found a default-feature test-helper cfg issue; strict lint also
caught closed-match/import and formatting issues. These were corrected before
the final integrated gate. Existing non-failing Web spread/chunk warnings and
the pinned transitive `spin 0.9.8` yanked warning remain; no dependency or worker
input was changed to bypass checks. Local unsigned fixture builds are not
Plugin publication.

Built from the exact clean committed source:

- Controller release: `/nix/store/jcavcj8p33qvxnazvvhiqr2p9bld8kl5-cowboy-controller-release`.
- Controller executable: `/nix/store/liw68fkdq7ir85wra5hrs3ipq8k5wpbq-cowboy-0.1.0/bin/cowboy`.
- Machine release: `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`.
- Machine executable: `/nix/store/l2d3mi9ixwrgwfzvzqgcvlgbz6yn2kh6-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.
- Source-boundary check: `/nix/store/5973k0zn4zlw8kcibnsj8kq5jhxygina-cowboy-source-boundary`.

Both manifests embed the exact clean revision. The Machine manifest preserves
`worker-92b35f0665ec33ba60f6`. No Web, signed Plugin/SDK, native ABI, Provider
runtime or aggregate NixOS release was activated by this slice.

## Separate accepted transactions

Both transactions used the clean isolated Columbus activator source
`42b3845233ebba35874fdc712c19a5400eb05876` at
`/home/draven/worktrees/columbus/cowboy-controller-recovery-20260912`, refreshed
against its remote main. They ran in independent root systemd units.

Controller first, through `cowboy-controller-activate <release> '' candidate`:

- Transaction `1789225747139054676-d6d48b2abc90`.
- Receipt: `succeeded`, `committed`, `published=true`, `maintenance=false` at
  `2026-09-12T15:09:21.068439082Z`.
- Retained predecessor: `/nix/store/fwfjs16ikqshhw4kissacmhvgis93rh0-cowboy-controller-release`
  (code `e3e4dacef70460071298e71de5eff0a59b54db67`).
- Actual PID **2088827**, start **2026-09-12 23:09:07 CST**, executable matches
  the candidate. The original Machine PID/start, all workers and Web were
  unchanged before the separately dispatched Machine maintenance.

Machine second, through explicit `cowboy-machine-activate <release> candidate`:

- Transaction `1789225818958306833-d6d48b2abc90`.
- Receipt: `succeeded`, `committed`, `published=true`, `maintenance=true` at
  `2026-09-12T15:10:28.795498689Z`.
- Retained predecessor: `/nix/store/js8w2yjwsbgwzx9cjh285bzg5nvk5xdr-cowboy-machine-release`
  (code `51f799725bc8aef24deaaaa830bd0ef02716635a`).
- Actual PID **2091295**, start **2026-09-12 23:10:19 CST**, executable matches
  the candidate. Its own journal confirms authenticated **protocol 17** at
  `2026-09-12T15:10:28.764932Z`. Controller PID/start stayed unchanged.

Receipts reside under
`/var/lib/hawk-component-deployments/cowboy-{controller,machine}/current.json`.
Dispatch output alone was not used as proof of activation.

## Continuity and deliberately closed admission

- All **15** pre-release worker PID/start records compare byte-for-byte equal
  after both transactions. No detached session was restarted or force-drained.
  Active generation remains `worker-92b35f0665ec33ba60f6`; this does not claim
  every older busy worker adopted that generation.
- `/healthz` returned `ok`; Hawk reports connected/online, exact expected active
  ACP generation, workspace revision `f0d1093cec96b7f54144345bb1df5fb45acaf0e1`
  and workspace-ID hash
  `109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`, unchanged.
- Web remains `/nix/store/qcwawc3alvmdz3svbx5f4kkiaz1l944i-cowboy-web-0.1.0`.
  Public version/root ETag `b85d8b88a592838909a87e3c5c9326f9` and service-worker
  ETag `fa8019e41386eaaf3c506379f9e63f8a` are unchanged. Root and SW both return
  HTTP 200 with `cache-control: no-store`.
- Controller startup reports `admission_enabled=false managed_namespace=false`.
  The Machine binding namespace remains absent as file and symlink. Ordinary
  binding, Service resolution, Machine recovery and managed-export production
  admission remain closed. No audit or new export grant is created on startup.
  Existing explicit legacy routing and local rotating telemetry were not
  migrated to the staged path.
- No private endpoint/token policy, production credential, Provider/native
  state or stored migration checksum was read as acceptance evidence or changed.

This is not acceptance of populated schema-two production writes. Active,
rollback and cold readers must all accept retained audit data before writers
open. Finite confirmation surfaces, explicit production writer/background-policy
admission and complete production cross-end failure/restart acceptance remain
P2 work. Unknown/schema-one evidence stays quarantined; emitted OTel remains
`NoRestore`. The managed namespace is not deleted to enable fallback or downgrade.

Detailed evidence is under `/tmp/cowboy-machine-binding-recovery.p0ydGv/`:
`check-integrated.log`, `recovery-tests.log`, `build-release.json`,
`build-release.log`, both activation logs, and the `pre-controller`,
`post-controller`, `post-machine` receipts, process snapshots and public headers.
Earlier failed attempts remain separately identifiable and are not acceptance
evidence.
