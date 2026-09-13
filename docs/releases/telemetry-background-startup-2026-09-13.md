# Managed telemetry startup availability: 2026-09-13

Released the [startup availability correction](../telemetry-background-policy.md):
a valid but stale/unresolved managed policy stops optional export, not core
startup. Corrupt journals and invalid explicit configuration still fail closed.
Configured-but-stopped mode creates no remote queue, selects no legacy fallback,
and cannot adopt later history without a new explicit host activation.

This is a Controller implementation release, **not production managed-policy
cutover or completion of P2/the Plugin refactor**. Victoria configuration and
both hosts' closed writer admission remain unchanged.

## Release and live acceptance

Clean source `c1a7752fe1845079bf99a135c4a111e28a03910e` includes the runtime fix
`2edac52d` and the conformance sample-identity correction. It was published to
remote main before the owned component activation.

- Controller release:
  `/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release`.
- Actual running ELF:
  `/nix/store/60zxyq81m94hr4fxral8ax2sy06qpiqw-cowboy-0.1.0/bin/cowboy`,
  SHA-256 `ea81dcf403e672ae6de7771f4ca2e50418fffb1ff7974479a3158d44c0895483`.
- Successful transaction `1789291319780645661-c1a7752fe184`, recorded
  `2026-09-13T09:22:24.927454244Z`, reports `succeeded`, `committed`,
  `published=true`; the independent activation unit exited successfully.
- Controller PID `252472`, monotonic start `2974253925088`.
  Historical predecessor is
  `/nix/store/qfyhmnvgb6rzrj18pvg06a9pzg65ardv-cowboy-controller-release`.
  The **next** transaction's default recovery target is now `gz5v…`, not that
  historical predecessor.

Activation used the isolated Columbus owner at
`de1f6c6504146de3633271fc223d3aaa3c449212`. The first dispatch failed while
refreshing GitHub due to a TLS EOF, before any profile/PID switch or in-progress
component transaction. The same owned activation command then succeeded; no
verification was bypassed. Routine profile retention removed old profile
generation 178; the predecessor's immutable release remains available.

All **13 worker PID/start pairs**, Machine PID/start (`110025`,
`2971136905262`), Machine/Web profiles, Web root/version and host closure were
unchanged before and after activation. Worker snapshot SHA-256:
`948a31d56c21217735bae943a0043aa4c2a4752dd35b2bc2bdb7ff73b4f84ca6`.
Machine remains connected/online on `worker-92b35f0665ec33ba60f6`, workspace
revision `de1f6c6`, workspace ID hash
`109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`.

Loopback and public `/healthz` return `ok`. Public Web version/root ETag remains
`74fde054044d50859b3450819c615f53`; root and `sw.js` return 200 with `no-store`.
SW ETag remains `10c674e3d3f9e1d0cb5bc6a81f89c1d9`. The live Controller startup
reports `admission_enabled=false managed_namespace=false`, with no managed
background activation. Machine binding journal and writer policy remain absent,
including no dangling symlink. No destination policy, credential, Provider/SDK,
native ABI, Machine protocol, SQL/file journal schema or worker input changed.

## Quality and immutable gates

`nix develop -c just check-compact` passed after the final test correction:
**1075 Rust tests**, 20 separately gated/ignored tests, **1426 Web tests** and
**15 isolated PostgreSQL tests**, plus formatting, lint, dependency, feature,
component/Plugin/Provider/site and production-build gates. Nix independently
passed 831 Controller library tests (15 ignored) and three binary tests.

The new [background startup gate](../telemetry-background-startup-conformance.md)
passed **78/78** with the candidate in all three supplied roles. All successful
starts admitted actual client OTLP samples and appended private local records;
stopped modes had zero remote failures, and active/offline queues consumed each
sample exactly once as not-admitted. This is candidate-only evidence, not a
claim that the host's cold role uses those bytes.

The populated two-Site reader gate passed **96/96 before and after activation**.
Before activation it tested candidate Controller, actual then-active `qfyh…`
recovery, current Machine in both active/recovery roles, and actual cold floors.
After activation Controller active/next recovery both use `gz5v…`. Machine
active/next recovery remain
`/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`, protocol 18.
Both reads of every populated/corrupt case and all supplied roles were required.

The unchanged host closure is
`/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`.
Its actual absent-profile outputs remain:

- Controller: `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine: `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`,
  protocol 17.

## Explicit cold-configuration gap

This historical gap was subsequently closed for Hawk by the separately owned
[cold-start floor refresh](telemetry-cold-start-floor-2026-09-13.md), with both
gates repeated against the actual activated roles. The original negative
evidence below remains unchanged; production policy cutover is still pending.

The separate startup matrix using the actual post-release roles is deliberately
**not accepted**: active and next-recovery Controller each passed 26/26; cold
passed only the two unconfigured reads and failed 24 configured checks. The
actual cold executable's help also lacks the managed-export/writer settings.
Before release, the corrected diagnostic additionally reproduced the old active
Controller's startup exit for absent, prepared, unknown, revoked, restored and
advanced bindings. Normal unconfigured local recording passed.

Thus populated-journal compatibility is established, but startup-configuration
compatibility is not. Production cutover still needs separately owned cold-floor
maintenance, acceptance of both hosts' complete intended startup configuration,
explicit policy setup, and fresh Operator-confirmed cross-end write/failure/
restart acceptance. First intent permanently fences legacy egress; do not open
writer gates alone or delete evidence to restore it. No host refresh or policy
cutover was performed by this release.

## Retained evidence

Private evidence root: `/tmp/cowboy-background-startup.2Xa6RE/`.
`check-compact-2.log` is the final complete gate; `build-final.*`,
`nix-controller-final.log`, `activate-controller-2.log`, the before/before-deploy/
after-controller snapshots, public headers and matrices are retained there.
The initial startup diagnostic had duplicate metrics sample IDs and is not
acceptance. The corrected existing-floor diagnostic is
`startup-existing-floors-2.receipt.json` (`accepted=false`), SHA-256
`186a6e2d27114644de90b5d2e424132af03ab41a31463700c5365d76c6916804`.
The first post-activation runs stopped at the clean-source prerequisite because
the new receipt output directory was not yet ignored; they wrote no receipts.
Metadata-only commit `ca04d7c7` adds that narrow ignore rule. Subsequent gates ran
from that clean descendant; runtime release bytes remain `c1a7752f`.

Private, create-only final receipts (mode 0600):

| Receipt | Result | SHA-256 |
| --- | --- | --- |
| `dist/telemetry-background-startup-conformance/20260913-candidate-c1a7752f.json` | 78/78 candidate-only; source `c1a7752f` | `cd09128d37a1ede5bb833791eb1ba1a6d89df0d7ea107d0a02a0c13d21e89d97` |
| `/tmp/cowboy-background-startup.2Xa6RE/reader-before.receipt.json` | 96/96 before; source `c1a7752f` | `10b0c163bb61d4033b1b9168ff1fefcb4f9b4fcac7d909a5fabd121112963081` |
| `dist/telemetry-reader-conformance/20260913-background-startup-active.json` | 96/96 active; source `ca04d7c7` | `638f370ced0627d0fee068e6dfd09bd64780368776dd5aa68ce0594e81fdd15c` |
| `dist/telemetry-background-startup-conformance/20260913-active-cold-gap.json` | 54/78; **not accepted**, cold gap; source `ca04d7c7` | `a4aaf853cb0f24cc2158df1d71cc8f1178ec2e21f295bb50bb98bdf7b0d5f03d` |
