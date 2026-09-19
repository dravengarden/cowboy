# Plugin cold-reader recovery floor — 2026-09-19

Status: the separately owned **Hawk cold-start recovery refresh is activated**.
The current complete Catalog and five reader gates pass against the actual
active, next-transaction recovery and new cold artifacts. No running Cowboy
component or installed Plugin was upgraded by this maintenance. This closes
the [previously recorded cold Controller defect](zed-budget-rollout-2026-09-19.md#controller-reader-boundary-and-cold-failure),
not whole-refactor, native-generation or independent post-effect recovery
acceptance.

## Cause and exact source

The old cold Controller, Cowboy `869c269f`, lacks the closed
`RuntimeBinding::CredentialDirectory` variant. A supported outer release schema
does not imply support for every newer Agent runtime payload. The earlier
negative receipt rejects Claude Code `3.1.27` before Zed publication; this
maintenance reproduces the same `RuntimeValue` rejection on `3.1.28`, which is
encountered first in the current Catalog snapshot. Both versions remain present.
No history, authority marker or migration was deleted; no decoder was relaxed
and no Agent was downgraded.

An isolated Columbus worktree started from freshly fetched `origin/main`
`d016409b`, then integrated the already active, unpublished `f834acb2` floor.
This preserves its three existing PostgreSQL/Suger/GNOME commits; those are not
new changes introduced by this maintenance. Commit `0295a14b` fixes five
pre-existing Markdown heading-level failures without changing their prose.
The complete host change is **`2189c9e81ae76c363511eb887a7680f2bd264929`**:
only Hawk's Cowboy input, its one aggregate lock node and the exact three-role
source assertion advance to **`94382b94e8275c6ad21cfdb9da1907bf1dab8d7a`**.
Falcon and all other dependency pins remain unchanged.

Clean final host closure:
`/nix/store/g612grcjk53982zsl0sl3yp2v84filgx-nixos-system-hawk-26.05.20260731.5b4f72e`.
Its actual absent-profile bootstrap paths are:

| Role | Immutable cold output |
| --- | --- |
| Controller | `/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release` |
| Machine | `/nix/store/xqs19nm0vzwc3whmzm08svy71d2da8k6-cowboy-machine-bootstrap-release` |
| Web | `/nix/store/d3ca615v2z0slwqr40rrfpkgb8xg0bn1-cowboy-web-release` |

These are built with Hawk's followed inputs, not substituted standalone Cowboy
outputs. The Machine artifact is bootstrap-only, not a resident Machine update.
All three source manifests are clean `94382b94`. The Web subtree is identical
to test source `d3e95624`: `165fa08f089d6ea07a0c169c4fad965fde42de8f`.

For both pre- and post-activation matrices, actual active and next-transaction
recovery use the same current successful component profile:

- Controller: `/nix/store/ygyk7dd8r0c92fxmk6a2zb47rrw52ndh-cowboy-controller-release`,
  source `17444b80`.
- Machine: `/nix/store/03li1x7fh85ga3ycqiwwhz47l89af442-cowboy-machine-release`,
  source `3f82f19c`, generation `worker-795a7ae472286bc7993b`.
  This independently advanced after the preceding Zed rollout; this task retains
  it rather than restoring that rollout's older resident generation.

Successful receipts, profiles and absence of an incomplete component transaction
were checked. A completed receipt's historical `previousRelease` is not the
next transaction's recovery target.

## Verification

Columbus's complete `just verify` passed, including both host output sweeps and
the exact cold-source/reader-flag assertion. The final clean system was built
through `just --justfile machines/justfile build hawk`. Independent read-only
review found no correctness, safety or compatibility finding in the two-commit
change. These checks alone are not application-data compatibility evidence.

From clean Cowboy **`d3e9562404264d745b53ee1918aaf188e5c673d5`**, the following
gates ran in the pinned shell, with disposable state and isolated loopback:

| Actual immutable reader gate | Before activation | After activation |
| --- | ---: | ---: |
| Dataset-bound Controller, including lifecycle history | 6 | 6 |
| Service installation journal | 168 | 168 |
| Machine installation attempts | 72 | 72 |
| Telemetry readers | 96 | 96 |
| Telemetry background startup | 78 | 78 |
| Total | **420** | **420** |

Every receipt and individual check is accepted. These are two runs of the same
420 checks, not 840 distinct vectors. Every dataset role is `bound` with
`lifecycle_history_checked=true`. Three real Firefox 151.0.1 suites also pass:
IndexedDB **8**, outbox **16**, lifecycle/React StrictMode **6**. Fresh profiles,
synthetic records and private networking do not establish physical-device or
real-account acceptance. No new full Cowboy source or private native-pair gate
is claimed for this host-only change.

Before activation, after activation and at the settled observation, each of the
three actual Controller roles independently reads the **complete 87-release
Catalog twice** and passes actual-Service host configuration preflight. The
Catalog includes Claude `3.1.27`, `3.1.28` and Zed `1.18.0`. Both Controller
telemetry policies remain `unconfigured`, legacy selection `not_checked`; each
actual Machine role's configuration-only writer preflight is also unconfigured.
No private destination inspection, export admission or installation occurs.

The initial private helper expected the old reader to encounter `3.1.27` first;
the actual failure named `3.1.28`. Its incomplete attempts are retained and are
not success receipts. The corrected negative records the actual package and
still requires the same closed runtime rejection. The original historical
negative receipt is untouched. Auxiliary PATH/type/receipt-field setup errors
are not product gate passes.

## Activation and continuity

The owning `machines/justfile activate hawk` command dispatched one independent
`hawk-activate.service` transaction:
**`1789784684986564566-2189c9e81ae7`**, successful at
**`2026-09-19T10:24:51+08:00`**. The immutable receipt under
`/var/lib/hawk-deployments/` records the new closure, Git-pinned revision, zero
new failed units and no runtime-override reconciliation. `published=false`
records dispatch-time state; subsequent Git publication does not rewrite it.

All 342 top-level system and 124 user unit files compare equal. The complete
inspection also includes drop-ins: the changed system path contains only
`nixos-version` executable/manual link changes, but its derived references refresh
D-Bus and desktop helper configuration. NixOS reloads system/user D-Bus and
restarts AccountsService/polkit and its normal user activation unit; `mandb` and
the already configured Carrack allowance observer also start. The receipt's
empty `changedUnits`/`explicitRestarts` fields are **not** a zero-system-restart
claim. D-Bus PID/start observations remain unchanged; AccountsService/polkit do
change as shown in the switch journal.

Immediate and settled audits retain **all 20 original Cowboy process identities**:
Controller PID **1051040**, resident Machine PID **1229854**, and **18 ACP workers**,
including original PID/start/executable/unit identities. All three component
profiles and successful receipt contents remain unchanged. Victoria's
three PID/start observations remain unchanged. This is bounded continuity, not
drain completion or native-owner resume.

Machine remains online on `worker-795a7ae472286bc7993b`; workspace revision advances
to `2189c9e8` while the workspace-ID fingerprint remains
`a8373cda2b6005dabc69c3aab9bedd2e17ee2f27d8b664d0695c48f96d20db81`.
Local/public health succeeds. `/version` remains
`4bb51ded1747cbc6ffef8a7940736881`; both audits pass six local/public HTML, service
worker and entry-JS byte/ETag/cache checks. No new SPA was activated.

## Retained evidence and limits

Private evidence lives at `/tmp/cowboy-cold-reader.p4waLt`. It includes role-bound
matrices, public Catalog snapshots, complete logs and bounded observations, not
copied Service or Provider credentials.

| Evidence | SHA-256 |
| --- | --- |
| `candidate-3-old-reader-negative.json` | `34f60551a22d46808a9952248e67359e934b9caec04e822e3cf397fb7c97892b` |
| `candidate-3-host-readers.json` | `d45b7ac14561667778eb702fb1a1b22d7a9d0ba2cfca6d2a8fafe9bbe1ef8481` |
| `after-host-readers.json` | `98792752d654a146ef09cc563945b6fb3431c518133da2dd7115668fc5429b67` |
| `after-sync-receipt.json` | `5d525306019438243993d3372974ad7f22ce836482a6c7feff5ba499dba74475` |
| `after-service-receipt.json` | `bfd9271ea44896d02f152e0e1856f3048a4e8d64006401725c8e5f38cb9d00a6` |
| `after-machine-receipt.json` | `24617549bf81f5b10f367b603fe05701f9697ec966a3474bd6ca5537f58eca18` |
| `after-telemetry-receipt.json` | `325a2c7c0134f8fa3d920ee89a6fd4283d78307a991d6051ee9c4b3e5140eaef` |
| `after-startup-receipt.json` | `57871418fe04bb862f5ad53a2c9742825e376d13894facce9694fb13a15518d6` |
| `audit-settled.json` | `9e1301c4dae240ac9887cc10a106564a4e27ca9b3ffe139152725df32d1c0d8f` |
| `settled-host-readers.json` | `6e09b61753bb3afde8c2b4edbf0035d5372537cf299014efcc3e9fdfcfe12ff8` |
| Successful host receipt | `74277c19907fb3e99ad843e553770d6fc44da0fbaedd35d3dbbf71a584f8e7d3` |

The [completion ledger](../plugin-refactor-completion.md) still requires general
typed graph/site/state leases, remaining global native resource bounds,
capability/device acceptance and independently authorized post-effect recovery.
Production security, managed telemetry and private navigation policies remain
unchanged. Compatibility of this Catalog is not a promise about a future
publication; recheck the actual cold floor before admitting new payloads.
