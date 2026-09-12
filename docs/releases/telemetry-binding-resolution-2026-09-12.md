# Service binding resolution reader-only release — 2026-09-12

Code: `e3e4dacef70460071298e71de5eff0a59b54db67`, published to Cowboy main after
integrating the independent Web accent release `10752158`. This adds the staged
[independently authorized resolution](../telemetry-binding-resolution.md), not
production writer admission, background activation or completed P2.

## Verification and immutable artifacts

The full pinned `nix develop -c just check-compact` gate passed both before and
after integrating remote main. The final run passed 985 Rust library tests
(17 intentionally ignored), 1,371 Web tests and all 15 isolated PostgreSQL tests,
plus strict lint, dependencies, independent feature builds and release builds.
This slice adds 16 tests, including the separately run PostgreSQL contract.
The signed-Machine fixture exercises fresh JSON observation without another
dispatch, private-policy adoption or export. Tests also cover original budgets,
real credential revocation, CAS races, lost COMMIT acknowledgement, audit
corruption, restart, same-epoch reconnect and exact restoration provenance.

Built from the exact clean committed source:

- Controller: `/nix/store/fwfjs16ikqshhw4kissacmhvgis93rh0-cowboy-controller-release`.
- Executable: `/nix/store/7s3rxpzhafx7jzxgwii4k8001qhjyymz-cowboy-0.1.0/bin/cowboy`.
- Source boundary: `/nix/store/6kr9mvak3h9pc4qkb3mwf2ffsk5wy1b3-cowboy-source-boundary`.

No Machine, Web, Plugin/SDK, native ABI or aggregate NixOS release was activated
or published by this slice. Routine gate builds are not component deployments
or signed Plugin publication.

## Actual activation receipt

The clean isolated Columbus transaction source was
`42b3845233ebba35874fdc712c19a5400eb05876` at
`/home/draven/worktrees/columbus/cowboy-controller-recovery-20260912`.
The owned `cowboy-controller-activate <release> '' candidate` recipe dispatched
transaction `1789217851647506348-e3e4dacef704`.

The detached root unit finished successfully. Its receipt at
`/var/lib/hawk-component-deployments/cowboy-controller/current.json` records
`succeeded`, `committed`, `published=true`, `maintenance=false`, at
`2026-09-12T12:57:46.092700652Z`. The retained predecessor is
`/nix/store/l5jskhg4xbsm46l70ksapjrmc8w8q51g-cowboy-controller-release`
(code `51f799725bc8aef24deaaaa830bd0ef02716635a`).
Actual Controller PID 1876740 started at `2026-09-12 20:57:31 CST` and its
`/proc` executable matches the immutable candidate above.

## Continuity and deliberately closed admission

- Machine PID **1760647**, start `2026-09-12 19:52:14 CST`, and executable under
  `/nix/store/vmxzi8w33zpxvic8h10h4sj6f19fjj3h-cowboy-machine-0.1.0` are unchanged.
  It remains on the previous protocol-16 release; no maintenance was dispatched.
- All **15** pre-activation worker PID/start records are identical afterward.
  No session restart or generation drain was forced. Reported active generation
  remains `worker-92b35f0665ec33ba60f6`; this does not claim every old busy worker
  has adopted that generation.
- `/healthz` returned `ok`; Hawk reports connected/online. Workspace revision
  `f0d1093cec96b7f54144345bb1df5fb45acaf0e1` and workspace-ID hash
  `109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd` are unchanged.
- Web remains at `/nix/store/qcwawc3alvmdz3svbx5f4kkiaz1l944i-cowboy-web-0.1.0`.
  Public version/root ETag `b85d8b88a592838909a87e3c5c9326f9` and service-worker
  ETag `fa8019e41386eaaf3c506379f9e63f8a` are unchanged across this activation.
  Both returned HTTP 200 and `cache-control: no-store`. The independent Web
  accent release was preserved, not replaced by this Controller deployment.
- Controller startup reported `admission_enabled=false managed_namespace=false`.
  The Machine binding namespace remains absent as both file and symlink.
  Binding and resolution writer gates remain closed; no new audit row or export
  authority is created on startup. Existing explicit legacy export and local
  rotating telemetry were not migrated to the staged path.
- No production destination/token policy, Provider/native state, credentials or
  stored migration checksum was read as release evidence or changed.

This is **not** acceptance of schema-two production writes. The rollback and
cold Controller readers must accept populated schema-two audit state before
such writes are enabled. Machine unresolved-record handling, explicit production
writer/background-policy admission, confirmation surfaces and complete cross-end
failure/restart acceptance remain P2 work. Emitted OTel remains `NoRestore`.

Detailed evidence: `/tmp/cowboy-binding-resolution.w7FI5G/acceptance.md`, final
`check-integrated.log`, `build.json`, `build.log`, `activate.log`, worker snapshots
and public response headers. Earlier lint failures are retained separately and
are not acceptance evidence.
