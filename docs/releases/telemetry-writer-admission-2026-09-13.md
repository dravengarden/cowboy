# Per-target writer admission implementation: 2026-09-13

Accepted Controller and Machine release of
[purpose-separated host admission](../telemetry-writer-admission.md). This is
implementation deployment, **not production writer or managed export cutover**.
The existing Victoria configuration is unchanged. No production binding
namespace or writer policy was created.

## Source and component receipts

Source `faa0451c7136db38cca15712864b9764c231334e` was clean, integrated with
fresh remote main and published before activation. Both installed component
transactions report `succeeded`, `committed`, `published=true`, and exited zero.
Activation used the clean isolated Columbus worktree at
`de1f6c6504146de3633271fc223d3aaa3c449212`; no host configuration was changed.

- Controller release:
  `/nix/store/qfyhmnvgb6rzrj18pvg06a9pzg65ardv-cowboy-controller-release`.
  Transaction `1789288127779983911-faa0451c7136`, recorded
  `2026-09-13T08:29:10.931917274Z`.
  Running PID `107478`, monotonic start `2971061420960`.
  Actual executable:
  `/nix/store/r1ywhqrq81dr28532vz6h8rx1gd7qx44-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `678743ef506c059afd27792e63ab473468ad1d0bcc602b4ae750f7cf87d6dba8`.
- Machine release:
  `/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`.
  Explicit maintenance transaction `1789288203234556320-faa0451c7136`, recorded
  `2026-09-13T08:30:12.890837454Z`.
  Running PID `110025`, monotonic start `2971136905262`.
  Actual executable:
  `/nix/store/p82fcl3674p8khklpm3k0xqdhdkjnlzx-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`;
  SHA-256 `2768c0f08e701c0ad43b9881dedf3983c69638ceca2219c29ccacbdf9c1f0c93`.

The original Controller transaction left Machine PID/start unchanged. The
separate Machine maintenance transaction left the new Controller PID/start
unchanged. All **13 worker unit PID/start pairs** were byte-identical before,
between and after the two activations. Snapshot SHA-256:
`948a31d56c21217735bae943a0043aa4c2a4752dd35b2bc2bdb7ff73b4f84ca6`.

## Deterministic and populated reader checks

`nix develop -c just check-compact` passed: 1073 Rust tests (19 separately
gated/ignored), 1426 Web tests, and 15 isolated PostgreSQL tests, plus the
lint/type/feature/dependency, component/Plugin/Provider/site gates and production
builds. Thirteen new tests use real private policies, including signed Machine
and actual HTTP paths. Nix independently built both release outputs; its
Controller check passed 831 library tests (15 ignored) and three binary tests.
Existing non-failing dependency/chunk-size warnings remain.

The canonical populated immutable reader gate passed **96/96 checks before
and after activation**, including both cold reads of all eight cases on both
Sites. Source is `faa0451c`; both schema-two receipts record `accepted=true`.

| Role | Before activation | Active / next transaction after activation |
| --- | --- | --- |
| Controller active | Candidate `qfyh…` | `qfyh…` |
| Controller rollback | Actual active `s088…` | Now-active `qfyh…` |
| Machine active | Candidate `6ic8…` | `6ic8…` |
| Machine rollback | Actual active `0x3z…` | Now-active `6ic8…` |

Historical predecessors are
`/nix/store/s0880d1bbbraamkan1l56g25q0xfkapq-cowboy-controller-release` and
`/nix/store/0x3zhdvqsa31w5rpvsjd9w7yl77w81ng-cowboy-machine-release`.
The next transaction's rollback was not inferred from a stale `previousRelease`.
Successful non-corrupt active/rollback Machine checks negotiated protocol 18;
the cold Machine negotiated 17.

The unchanged host closure is
`/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`.
Its actual absent-profile cold outputs, independently checked in its activation
script, remain:

- Controller: `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine: `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.

## Live verification and retained evidence

Loopback and public `/healthz` are `ok`. Public `/version` and root ETag are
`74fde054044d50859b3450819c615f53`; root and `sw.js` return HTTP 200 and
`cache-control: no-store`. SW ETag is `10c674e3d3f9e1d0cb5bc6a81f89c1d9`.
The independently published Web release remains
`/nix/store/r34k54spv1nbvy7ixzkc45z9zkgksw87-cowboy-web-release`, source
`0cfa636c4fa96a2e23fa384dce6998611fe78ee4`, serving
`/nix/store/h0sxjfy9bhpi0zb6ryi762flg0hwvfym-cowboy-web-0.1.0`.

Machine is connected/online with unchanged generation
`worker-92b35f0665ec33ba60f6`, workspace revision `de1f6c6`, and workspace ID
hash `109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`.
The new Controller startup reports
`admission_enabled=false managed_namespace=false`. Both Machine
`plugin-operations/telemetry-bindings-v1.json` and `telemetry-writer-policy.json`
remain absent, including no dangling symlink. No private destination policy,
credential, Provider/SDK release, native ABI or host closure was changed.

Private evidence is under `/tmp/cowboy-writer-admission.uJdwrP/`:
`check-compact-4.log` is the accepted complete gate; earlier development logs
are not acceptance receipts. Build results, activation logs, profile/process/
health snapshots, matrices and the before-reader receipt are retained there.
The before-reader receipt SHA-256 is
`559af7eef9a3a23ddcee628808ddab218389a99d98e9caac56f7f93962abc520`.
The create-only, mode-0600 final reader receipt is
`dist/telemetry-reader-conformance/20260913-writer-admission-active.json`, SHA-256
`9b0d709eb8edc4b1cce2af49a46c48659514afd8a72a5bd7cf3ac8e9553f3a3a`.

P2 still needs owned production policy cutover and cross-end write/failure/
restart acceptance. Populated journal readers do not prove compatibility with
new startup flags/configuration; candidate, recovery and cold configurations
must separately be accepted before enabling writes. First intent can fence
legacy export even when the Machine refuses. Do not open the gates alone or
delete evidence to restore legacy egress. P3/P4 and the overall Plugin refactor
are not declared complete by this release.
