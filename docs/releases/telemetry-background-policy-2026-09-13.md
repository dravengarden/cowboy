# Managed background policy implementation: 2026-09-13

Accepted Controller-only release of
[explicit managed background policy](../telemetry-background-policy.md).
This receipt does **not** claim a production managed binding or export-policy
cutover. Existing explicit legacy Victoria configuration remains unchanged;
production binding/recovery/resolution write admission remains closed.

## Source and immutable component

- Source: `890028b05310e8098f71caf2dc22ab4c7a9aec52`, clean, integrated and
  published to `origin/main` before activation.
- Controller release:
  `/nix/store/s0880d1bbbraamkan1l56g25q0xfkapq-cowboy-controller-release`.
- Actual ELF:
  `/nix/store/r0m4kjzpc5ba1mvidq61hq6wr3sj2qw6-cowboy-0.1.0/bin/cowboy`.
- ELF SHA-256:
  `c16836ccf26d3c846aaa350a16aacfeb4535e9188833fa942935ea17932cda02`.
- Component transaction: `1789284210831450653-890028b05310`;
  recorded `2026-09-13T07:23:54.994141819Z`, `succeeded`, `committed`,
  `published=true`. The independent root activation unit exited successfully.
- Historical predecessor:
  `/nix/store/lsv34w1xcpz5f4z473w5kbf8s30x71ki-cowboy-controller-release`
  (`74a71b102de976659d457a6898a81608f7291f78`).
- Running PID `4134706`, monotonic start `2967144868959`; `/proc/4134706/exe`
  resolves to the candidate ELF. Startup reports
  `admission_enabled=false managed_namespace=false`.

Activation used the installed Cowboy component activator from the clean isolated
Columbus task worktree at `de1f6c6504146de3633271fc223d3aaa3c449212`. There was no
Columbus configuration edit, full host activation, Machine maintenance, signed
Plugin/SDK publication, native change or credential/policy change.

## Deterministic and immutable reader gates

`nix develop -c just check-compact` accepted the complete source change:
1058 Rust tests passed (19 separately gated/ignored), 1418 Web tests passed,
15 isolated PostgreSQL tests passed, with lint/type/feature/dependency,
component/Plugin/Provider/site gates and production builds. Ten new tests cover
the policy and actual signed-Machine background path. The immutable Nix
Controller build separately passed 824 library tests (15 ignored) and three
binary tests. Existing non-failing lint/dependency/chunk-size warnings remain.

The repository-owned populated reader gate passed **96/96 checks before and
after deployment**, each including two cold reads of all eight cases on both
Sites. Both schema-two receipts bind source `890028b0` and record
`accepted=true`. No production policy or journal is used as a fixture.

| Role | Before activation | After activation / next transaction |
| --- | --- | --- |
| Controller active | Candidate `s088…` | `s088…` |
| Controller rollback | Actual active `lsv3…` | Now-active `s088…` |
| Machine active and rollback | `0x3z…` | Unchanged `0x3z…` |

The actual Machine active/next-rollback root is
`/nix/store/0x3zhdvqsa31w5rpvsjd9w7yl77w81ng-cowboy-machine-release`.
Its successful non-corrupt checks negotiated protocol 18; the cold Machine
negotiated 17. Neither protocol number alone nor an empty production ledger is
treated as reader acceptance.

The current host closure remains
`/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`
at `de1f6c6504146de3633271fc223d3aaa3c449212`. Its actual activation script's
absent-profile cold outputs remain:

- Controller: `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine: `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.

Profiles/host were independently recaptured immediately before and after this
transaction. The old receipt's `previousRelease` was not substituted for the
next transaction's default rollback. The bootstrap artifact was only a cold
reader, never an activation candidate.

## Live continuity and evidence

- `/healthz` is `ok`; loopback and public `/version` remain
  `e14b8094c12863f7e2225d7b635211b4`.
- Public root and `sw.js`: HTTP 200, `cache-control: no-store`. Root ETag matches
  the version; SW ETag remains `d27bb4e7342762844e70b4e856699976`.
- The independently deployed Web release is preserved:
  `/nix/store/kkm5jp11av82vd1i4l7rhw6jzgyzdyzl-cowboy-web-release`, source
  `4d1226c6968a352fea741caea35b2aa4775b5736`. Served root remains
  `/nix/store/vb6461hjvddq5dwclvn6v08zflzxwbjd-cowboy-web-0.1.0`.
- Machine PID `3817934`, monotonic start `2964008082119` are unchanged.
  Deployment health reports online/connected, generation
  `worker-92b35f0665ec33ba60f6`, workspace revision `de1f6c6`, and workspace ID
  hash `109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`.
- All **14** worker unit PID/start pairs are byte-identical before/after this
  activation. Snapshot hash:
  `2aec37ad4e8f755a30eaa8d1fcee975ab094db27094dd70edfac0b122e11201c`.
- The Machine managed binding file remains absent and not a symlink. No binding
  namespace, recovery action, background policy, session replay or new endpoint
  was created by deployment. No physical-device or production fault injection
  acceptance is claimed.

Private local evidence is under `/tmp/cowboy-background-export.HKXEqb/`.
The create-only final reader receipt is
`dist/telemetry-reader-conformance/20260913-background-policy-active.json`, mode
0600, SHA-256 `bab91c25ab4cedf5440b709ae95e467e4f248436eeb5fd725e2fcf4e5afc7349`.
The before-deploy receipt SHA-256 is
`a0ea163c4b1a7fd4a5c6748212f138ec14db0496641aace27bda278f7b6dba7f`.
Earlier failed development test logs remain diagnostic, not acceptance receipts.

Remaining P2 work: explicit per-target production writer admission, the owned
configuration cutover to managed export, and cross-end production write/failure/
restart acceptance. Do not open the first binding write in isolation: it fences
legacy export even if the operation is later aborted. Already emitted OTel is
`NoRestore`; this Controller implementation release is not the complete Plugin
DAG/refactor.
