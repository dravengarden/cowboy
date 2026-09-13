# Core telemetry resolution surface — 2026-09-13

Published and activated source: `5ae8279b331dc406f65411462fdb9623c946b1a4`
(implementation `72da57bc`, then a Web-only clarification of Service-recorded
heads). [Settings → Info](../telemetry-resolution-surface.md) now exposes the
core Service resolution surface without a separate admin account.

**Production confirmation admission is closed.** The UI exposes read-only
evidence/preview; its finite one-use confirmation path is exercised in hermetic
tests, not enabled on Hawk. This is not completed P2 or managed export
activation.

## Gates and exact artifacts

The complete pinned `just check-compact` passed: 1,020 Rust library tests (19
intentionally ignored), 1,381 Web tests, all 15 isolated PostgreSQL tests,
format/lint/dependency/feature/Plugin/native-shell/site gates, and production
builds. The final Web wording change additionally passed the ten type-checked
surface tests, Web typecheck, lint and formatting. Existing OTel spread and
large-chunk warnings remain unrelated; no failing check is treated as
acceptance.

Post-release integration preserved the independent favicon commits `058a51cc`
and `addba842`. The complete gate passed again at `058a51cc`, with 1,383 Web
tests. The subsequent `addba842` changes leave Controller, Cargo, Nix and
worker-generation inputs unchanged; the site gate and complete Web test,
typecheck, lint and production build passed again on that integrated revision.
These checks do not replace the clean release source or artifacts below.

Both immutable releases were built from the clean final commit. The Controller's
filtered Nix build also passed 802 library tests (15 intentionally ignored) and
its binary tests. The shared Rust/Web fixture is explicitly included in that
filtered source; ordinary Web assets are not Controller build inputs.

| Component  | Release                                                                 |
| ---------- | ----------------------------------------------------------------------- |
| Controller | `/nix/store/61i3amqivxn0xc0rdxh7gnkmv3i9823r-cowboy-controller-release` |
| Web        | `/nix/store/d7d633jk91h4y6bnh4gcx1vr7xcfbk29-cowboy-web-release`        |

Actual Controller executable:
`/nix/store/2j4pncg4zl9qa4wf21w7a292fbnx0yb0-cowboy-0.1.0/bin/cowboy`, SHA-256
`9631add15c513d9b99b0e0bc0e02f75ef800786213e6b914c34d31b4b05ad0b2`.

Actual Web root: `/nix/store/s0li9aw0bx49rxx368iczjpz9x6fdasb-cowboy-web-0.1.0`.

## Component transactions and continuity

The existing machine-owned activator in the unchanged Hawk closure was used
through the clean isolated Columbus worktree
`/home/draven/worktrees/columbus/cowboy-controller-recovery-20260912`
(`de1f6c6504146de3633271fc223d3aaa3c449212`). No aggregate lock or host
configuration was changed and no host/Machine transaction was dispatched.

Both component receipts report `succeeded`, `committed`, `published=true`:

- Controller: `1789262644293604725-5ae8279b331d`, recorded
  `2026-09-13T01:24:27.488869149Z`. Its transaction predecessor was the accepted
  `d6d48b2a` Controller at
  `/nix/store/jcavcj8p33qvxnazvvhiqr2p9bld8kl5-cowboy-controller-release`.
- Web: `1789262700120278135-5ae8279b331d`, recorded
  `2026-09-13T01:25:00.157330616Z`. Only the Web profile/root moved.

Controller PID 3282366 started at 09:24:05 CST and resolves to the executable
above. It remained unchanged through the Web switch. Machine PID 2091295 and
start counter 2908750440467 remained unchanged, as did its `d6d48b2a` profile.
Both detached activator units completed successfully without an in-progress
transaction left behind.

The 09:06 baseline had **16** workers and must not be described as wholly
unchanged: two workers were replaced at 09:17:43 and 09:21:49, and another
exited at 09:21:48. Systemd's timestamps put all three changes **before** the
Controller transaction started at 09:24:03. Their cause was not inferred from
this release check. The other baseline PID/start pairs were unchanged; the
resulting **15** workers predate the deployment and their complete
post-Controller and post-Web snapshots match. Snapshot SHA-256:
`9b64aea6f424ef693b1f4e71e19006d16802ffcbf036ee226d961281e95cf4a3`.
Worker-generation inputs and reported generation `worker-92b35f0665ec33ba60f6`
are unchanged. This is not a claim that all 16 early snapshot processes survived
the entire development turn.

## Reader floor and live checks

The actual immutable-reader conformance passed **96/96** before activation and
again afterward. The final active/next-transaction-rollback/cold matrix uses:

- Controller active and next rollback: the new `61i3amqi` release above.
- Controller cold:
  `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine active and next rollback:
  `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`.
- Machine cold:
  `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.

The active profiles and the unchanged host's actual absent-profile
initialization paths were independently inspected. The completed Controller
receipt retains `d6d48b2a` as **this transaction's** predecessor; the **next**
transaction captures the now-current `5ae8279b` profile. Earlier matrices
testing that predecessor remain evidence, not a substitute for this final role
binding.

Final private, create-only evidence:
`dist/telemetry-reader-conformance/20260913-service-resolution-surface-active.json`,
source `5ae8279b`, SHA-256
`f38a9702268239b6e3b536daf964b9b2bcbdb878238e0bebe8c6974f1f4d4701`.

At the completion of the transactions above, `/healthz` returned `ok`; Hawk was
connected/online with the unchanged workspace revision
`de1f6c6504146de3633271fc223d3aaa3c449212` and workspace-ID hash
`109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`. Both public
root and `sw.js` returned HTTP 200 with `cache-control: no-store`. Root/version
ETag is `6967ee929cb1b930135c2dbd66717bde`; Service Worker ETag is
`0ec8e6a6e92f90f562f86b15ded429cb`. The public manifest reference and Service
Worker both identify **v1666**. Mobile PWA users need the explicit Update action
to load the new bundle. An unauthenticated binding read returned 401, not
journal data. Actual authenticated production confirmation and physical-device
UI interaction were not used as release smoke tests.

During documentation finalization, a separate Web transaction activated
`addba8428b1e986029f817bfcbda0cc02a182360` at `2026-09-13T01:38:19.640386876Z`.
It retains this telemetry surface but supersedes the Web version/cache
observations above. This task did not perform that activation or overwrite it
with its earlier artifact. The final read-only check still found the `5ae8279b`
Controller receipt, unchanged Controller/Machine PIDs, online Hawk and a 401
response for an unauthenticated binding read.

Startup still reports `admission_enabled=false managed_namespace=false`. The
Machine binding namespace remains absent. No private telemetry policy or
credential was read/changed, no signed Plugin/Catalog/native/Provider state was
mutated, and no managed writer or export grant was activated. Existing legacy
export/local rotation and the durable incident ledger are unchanged.

Remaining: ordinary select/revoke/restore confirmation, separate Machine
recovery confirmation, per-target writer/background-policy admission, and
production cross-end failure/restart acceptance. Unknown/schema-one Machine
evidence stays quarantined and emitted OTel remains `NoRestore`.

Detailed build/gate/activation logs, early failed checks and process snapshots
are retained under `/tmp/cowboy-telemetry-surface.JDPZQP/`; acceptance never
overwrites those earlier diagnostics.
