# Immutable telemetry writer-policy acceptance: 2026-09-13

The actual Hawk active, next-transaction rollback and cold Controller/Machine
artifacts passed the new [writer-policy conformance gate](../telemetry-writer-conformance.md).
This closes executable finite-purpose startup/effect acceptance, **not P2,
complete production configuration, real Operator authorization or cutover**.
No production writer or managed background policy was enabled.

## Source and quality gates

The harness source is `101466e2af6418b4b3ff1bd450971177d423bdc6`. Changes are
test-only Rust helpers, the dedicated recipe, ignored generated receipts and
contract/skill documentation. Runtime source, protocols, journals, SQL baselines,
Plugin/SDK artifacts, native ABI, Provider state and worker inputs are unchanged.
No component or NixOS activation is required for this test/documentation change.

`nix develop -c just check-compact` passed: format, lint, dependencies, feature
slices, 1,078 library tests (21 explicitly ignored), binary/adapter tests, 1,426
Web tests, all 15 separately isolated PostgreSQL tests and release builds. The
new harness's ordinary fixtures/environment/cleanup tests passed as part of that
gate. The existing Vite chunk-size warning remains non-fatal and unchanged.

All immutable gates then ran from the clean committed source above, with
loopback-only namespaces, cleared child environments and disposable state:

| Gate | Required checks | Result |
| --- | --- | --- |
| Writer policy / finite effects | 294 | Accepted |
| Populated two-Site readers | 96 | Accepted |
| Managed Controller startup / local OTel recording | 78 | Accepted |

Writer acceptance includes 126 Service resolution checks, 84 Machine binding
checks and 84 Machine recovery checks. Each scenario has exactly twelve expected
mutations across all three roles and admitted purpose combinations; every other
check preserves the journal hashes. Only Service cold start two or Machine cold
start one may mutate. All successful non-rejected Machine starts use protocol 18.

The Service first creates an unsubmitted preview, then restarts. That preview
cannot authorize a write; only a separate fresh preview/confirmation may persist
the exact local abort and resolution audit. A third start reads that receipt
without reviving confirmation. Machine binding/recovery commands check exact
policy-purpose admission; their duplicates after restart return the same durable
evidence without rewriting. These fixtures use a synthetic local Operator and
signed peer, never production credentials or a connected production mutation.

## Independently captured actual roles

Read-only snapshots at `2026-09-13T20:48:16+08:00` and
`2026-09-13T20:53:33+08:00` agree on the host closure, profiles, component
receipts, effective next ordinary recovery targets and cold outputs. No
component activation was in progress. Cold paths came from the actual active
closure's absent-profile activation script, not candidate substitution. Rollback
here means the profile captured by the next transaction, not a completed
receipt's historical `previousRelease`.

The unchanged Columbus host revision is
`1da44eb88883b52a7bff266da0c07ede35193659`, with active closure:

`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`

| Site / role | Immutable release | Embedded revision |
| --- | --- | --- |
| Controller active / next rollback | `/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release` | `c1a7752f` |
| Controller cold | `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release` | `c1a7752f` |
| Machine active / next rollback | `/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release` | `faa0451c` |
| Machine cold | `/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release` | `c1a7752f` |

Receipts hash the exact manifests, launchers and actual ELF chains. The matrix
SHA-256 is `8643187cfc2e7db257612cacab3dec506c59a4bc1ba68a7eb92ad8fe60be3a8d`.
These are the same immutable roles accepted in the
[cold-floor release](telemetry-cold-start-floor-2026-09-13.md); no refresh or
Machine bootstrap activation was performed.

Controller PID/start remains `252472` / `2974253925088`; Machine remains
`110025` / `2971136905262`. All thirteen worker PID/start pairs are identical
across this run, snapshot SHA-256
`3c0d2852cf7db32792896e104ecec0c55baac7eecd33a02b92a745e7302f7d64`.
This is a fresh bounded-run comparison, not a claim that the worker set never
changes between independent user tasks. Machine health remains online on
`worker-92b35f0665ec33ba60f6`. Health/version, Web root, cache headers/ETags,
component receipts and workspace projection remain unchanged; no new failed
system or user unit appeared. No PWA reload is required.

Machine writer policy and binding journal remain absent, including no dangling
symlink. No policy, private endpoint/token, managed namespace or production
configuration was modified. The harness checks explicit synthetic policies;
its `not_checked` exclusions remain true even though actual host roles and
session continuity were independently captured alongside it.

## Evidence and remaining work

Private evidence is retained at `/tmp/cowboy-writer-conformance.YtwDXL/`:
quality/conformance logs, baseline/after host captures, the exact matrices and
an independent aggregate/continuity audit. An initial read-only capture outside
the pinned shell stopped because `jq` was unavailable; its partial directory
is retained and is not acceptance. The complete captures and all gates ran in
the pinned shell. No failed conformance receipt was produced by this run.

All final receipts are atomic create-only files, mode 0600:

| Receipt | SHA-256 |
| --- | --- |
| `dist/telemetry-writer-conformance/20260913-actual-roles.json` | `36f3c516bf5d2b8477718b04c374aef95a69cbdb1b63b88e99e394ace9569fb5` |
| `dist/telemetry-reader-conformance/20260913-writer-regression.json` | `54ab8357d973c479f4bc4ca5e249d2d2c8d4516f626c1a08d2502a37da78ade0` |
| `dist/telemetry-background-startup-conformance/20260913-writer-regression.json` | `06d857617bbc7a0417b1827ef39e6782922ea77583c3ea6acd19a3bc0c654291` |

Still separate: complete intended configuration on both hosts, real Operator
authority, connected Service/Machine mutation and network-fault/restart
acceptance, signed Plugin destination/actual external OTLP delivery, and the
owned production policy cutover. The first managed Service intent permanently
fences legacy egress, including on refusal or abort. Never open the production
gates alone, delete evidence to regain legacy export, or treat already emitted
telemetry as reversible (`NoRestore`).
