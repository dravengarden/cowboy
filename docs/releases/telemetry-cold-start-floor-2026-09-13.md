# Managed telemetry cold-start floor: 2026-09-13

Hawk's actual cold recovery artifacts now understand managed-background startup
and private writer policy. This closes the Hawk cold-configuration gap recorded
in the
[startup availability release](telemetry-background-startup-2026-09-13.md).
**Production telemetry writers and managed export remain closed. This is not P2
completion, fleet-wide acceptance, or production policy cutover.**

## Owned host release

The isolated Columbus task published and activated
`1da44eb88883b52a7bff266da0c07ede35193659`. Its four-file change updates only
Hawk's Cowboy input, the corresponding aggregate lock node, recovery assertions
and the existing deployment runbook. The exact published Cowboy pin is
`c1a7752fe1845079bf99a135c4a111e28a03910e`, already accepted as the live
Controller. Nixpkgs, Falcon, Provider dependencies and service policy are
unchanged.

The root `nix develop -c just verify` passed, including machine configuration
checks and the exact bootstrap revision/CLI assertions. Nix independently passed
831 Controller library tests (15 ignored) and three binary tests. The Machine
bootstrap built successfully; its actual reader was then exercised by the
separate populated conformance gate. The prior full Cowboy application gate
remains applicable: no application source changed in this host release.

After committing, the owned `sys-build` produced:

`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`

The owned `sys-activate ./result` transaction `1789292750373176550-1da44eb88883`
succeeded at `2026-09-13T17:45:56+08:00`, with `published=true` and matching
candidate/active closure. Its predecessor remains
`/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`.
The independent root activation unit exited successfully.

## Actual roles and immutable acceptance

Both gates ran from clean Cowboy source
`842a8d848ed410e488d47c044caf5c48d9c1a2b2`, first against the built candidate
and then against the activated host. Cold paths were extracted from each exact
closure's absent-profile activation script, not substituted from standalone
builds or inferred from matching Git versions. Existing component profiles and
receipts were independently captured at both boundaries; no incomplete component
transaction existed. The effective next ordinary rollback is the then-current
profile, not a completed transaction's historical `previousRelease`.

| Site / role                       | Immutable release                                                              |
| --------------------------------- | ------------------------------------------------------------------------------ |
| Controller active / next rollback | `/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release`        |
| Controller cold                   | `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`        |
| Machine active / next rollback    | `/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`           |
| Machine cold                      | `/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release` |

Both new cold readers embed `c1a7752f`. Their actual ELF SHA-256 values are:

- Controller:
  `7309e8abd3897ee6bb49bd39be5fc33dead55327ab43da09c6b895868cc9fed0`.
- Machine: `cdd5d25200844d1237c1665c591d79a769e3aa7e153d86554f02d25b5ebf18f7`.
  Its packaged launcher separately hashes to
  `091c936ca8fbc36f311e2801b64eec515231ff27dedee55344cd99f604092db8`.

The [populated two-Site reader gate](../telemetry-reader-conformance.md) passed
**96/96 before and after activation**. All successful Machine observations use
protocol 18, including recovery-audit discovery on the new cold reader. The
[configured Controller startup gate](../telemetry-background-startup-conformance.md)
passed **78/78 before and after activation**. The old cold role's 24 configured
failures are therefore closed for these actual Hawk roles. Both cold reads of
every case remain mandatory; all four final receipts report `accepted=true`.

The same pin also supplies absent-profile Web bootstrap
`/nix/store/9dp62pr4msjbk91yrmdrqd5f8ppc7m2l-cowboy-web-release`. It was built,
not activated as an application release. No existing Cowboy profile was
repointed, and the bootstrap Machine remains forbidden as an activation
candidate.

## Continuity and unchanged authority

Before, immediately before activation, and after activation snapshots agree on
Controller PID/start (`252472`, `2974253925088`), Machine PID/start (`110025`,
`2971136905262`), and **all 13 worker PID/start pairs**. The worker snapshot
hash is `948a31d56c21217735bae943a0043aa4c2a4752dd35b2bc2bdb7ff73b4f84ca6`. All
three component profiles and receipts, Web root and served version remain
identical. Controller/Machine unit bytes retain both lifecycle markers and are
unchanged. The host receipt's changed-unit `retainedProcesses` lists are empty;
the independent PID/start snapshots provide the continuity evidence here.

The normal system switch reloaded D-Bus, restarted Polkit/accounts-daemon and
ran NixOS user activation. It did not restart Cowboy. No system or user unit was
failed afterward. An initial over-strict comparison of failed-unit lists stopped
because a pre-existing GTK portal failure disappeared; the relevant
no-new-failure check passed. This is not a claim to have repaired that portal.

Machine remains online on `worker-92b35f0665ec33ba60f6`. The owned workspace
refresh reports Columbus revision `1da44eb8`, with unchanged workspace ID hash
`109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`.
Loopback/public health is `ok`; public root and `sw.js` return 200 and
`no-store`. Web version/root ETag remains `74fde054044d50859b3450819c615f53`,
and SW ETag is `10c674e3d3f9e1d0cb5bc6a81f89c1d9`. No PWA reload is needed.

The same live Controller still reports
`admission_enabled=false managed_namespace=false`. Machine binding journal and
writer policy remain absent, including no dangling symlink. No Victoria policy,
credential, Plugin installation, managed journal, migration or worker input was
changed. Full intended startup configuration on both hosts, fresh
Operator-confirmed cross-end writes, policy cutover and failure/restart
acceptance remain separate work. First durable managed intent fences legacy
egress; never delete evidence to regain it. These synthetic gates do not prove
external OTLP delivery or undo already emitted telemetry (`NoRestore`).

## Retained evidence

Private evidence root: `/tmp/cowboy-telemetry-cold-floor.vXp3rD/`. It retains
`verify.log`, the Nix build logs, `sys-build.log`, closure/unit diffs,
activation log/journal, three process/profile snapshots, public headers, both
host-role matrices and all four conformance logs. The old negative receipts
remain intact.

Final receipts are private, create-only, mode 0600:

| Receipt                                                                            | Result | SHA-256                                                            |
| ---------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------ |
| `dist/telemetry-reader-conformance/20260913-cold-floor-candidate.json`             | 96/96  | `2f534322bb92c9b29e2bfb99ad518ebb7efd7830b4dbe8aa470410c0929e9e3c` |
| `dist/telemetry-reader-conformance/20260913-cold-floor-active.json`                | 96/96  | `6ac06e28acfffbc205a8ddfd1d7bebf5ce8478cc9b4b97188e956f2da6a8a64e` |
| `dist/telemetry-background-startup-conformance/20260913-cold-floor-candidate.json` | 78/78  | `ac896a09906c58ac3044a3ce801899a7df5b872871e6b660a2e82b9919abf9ef` |
| `dist/telemetry-background-startup-conformance/20260913-cold-floor-active.json`    | 78/78  | `2ae189306bc13bd52787e775885985efae793ebcf10b3616afeb5058d639855c` |
