# Controller telemetry policy preflight release: 2026-09-14

Hawk's Controller now checks explicit managed telemetry writer/background
configuration during `serve --check-plugin-hosts`, using the same validators as
normal startup. The old early return could report success for configuration that
startup would reject. This fixes a production-cutover prerequisite, not
production managed export or P2 completion.

## Fix and configuration evidence

The [preflight contract](../telemetry-policy-preflight.md) adds a closed,
credential-free telemetry report. Explicit managed settings require the existing
exact Service identity, private-file checks, closed schemas and the same
store/mode prerequisites as startup. Identity inspection is bounded, read-only
and refuses final symlinks, non-regular files and blocking FIFOs. It never
creates a Service, opens its database, grants a writer or exports data. Legacy
selection remains explicitly `not_checked`: its durable fence must not be
bypassed by reading/adopting an obsolete configuration file during preflight.

Four new regression tests exercise the real `serve` path, including 24 invalid
policy combinations and seven invalid identity fixtures. Valid cases leave
filesystem contents and ownership/inode/time metadata unchanged. A listening
PostgreSQL fixture observes no connection. Rejections agree with the startup
constructor and do not expose fixture secrets.

The immutable candidate additionally passed six read-only checks as the actual
Service owner, with the actual host/authentication/data settings in an isolated
network namespace. The predecessor accepts a synthetic unknown writer field; the
candidate rejects it and a foreign-Service writer policy. Valid synthetic writer
configuration is checked without enabling it. Actual managed settings remain
unconfigured and the existing legacy Victoria selection stays unchanged. No
production endpoint/token, Operator credential or Machine policy was copied.

## Source and gates

Implementation commit: `e5605676e3b8446888bb60480de8381c34c90440`. Final clean
release/harness commit: `d0ff13406610cda083093509f0ad3d053e1e5278`, integrating
remote `main` `5c4ffb39`. Those incoming changes are Web-only plus its
fixed-output dependency hash; no Controller source changed. The final Nix
artifact was rebuilt for the required ancestry, and its actual Controller ELF is
byte-identical to the first accepted candidate: SHA-256
`50b67f689860242c31d9542b480c9d4fc04530ab992b6b38eeed2f0bcb66ebec`.

`nix develop -c just check-compact` passed on the integrated commit: formatting,
Clippy, dependencies, feature slices, **1,088 library tests** (22 explicitly
ignored), binary/adapter tests, **1,427 Web tests**, all **15 isolated
PostgreSQL tests**, and release builds. The existing Vite chunk-size warning is
non-fatal. The actual Catalog covers all six embedded Agent Plugin releases.

The final immutable manifest/role matrix was independently exercised again:

| Gate                                  | Checks | Result   |
| ------------------------------------- | ------ | -------- |
| Populated Service/Machine readers     | 96     | Accepted |
| Independent writer admission/effects  | 294    | Accepted |
| Managed Controller startup/recording  | 78     | Accepted |
| Authenticated connected managed flows | 45     | Accepted |

The connected gate covers all nine role pairs and 144 managed-delivery rounds,
with 324 correlated real RPC receipts/isolated HTTP requests. The aggregate
audit checks exact counters, payload/request hashes, no-resend behavior, local
recording, policy replacement, stale/active restart, and journal effects. These
are temporary authenticated identities and an isolated protocol receiver, not a
production Operator or real Victoria database.

## Activation and continuity

Final Controller release:
`/nix/store/xa07qj5xx6q72lrvnnmdnmj7yyxdw9kj-cowboy-controller-release`. Only
the machine-owned Controller component transaction was dispatched; no NixOS
closure, Machine, worker generation, Web release, Catalog or host policy was
changed by this task.

Transaction `1789343213354618294-d0ff13406610` committed successfully at
`2026-09-14T07:47:04+08:00`, with `published: true`. Controller PID/start
changed from `252472` / `2974253925088` to `1663934` / `3026147307505`; the
running process resolves to the exact accepted ELF.

Captures at `07:46:32`, `07:47:51` and `07:51:02` (+08:00) agree on Machine
PID/start `110025` / `2971136905262`, its component profile and receipt, Web
root/profile/receipt, host closure/receipt and Machine presence. Health is
successful, no new failed system/user units appear, and Web root/SW cache
headers and ETags agree. `/version` remains `ae371d1395b4ad7a2ceda868237026b5`:
it hashes the served SPA, not the Controller's Git revision.

**The strict all-worker continuity audit failed and remains failed.** The first
after-capture caught three Grok workers between stop and resume: 13 → 10 → 13.
Ten PID/start pairs remained identical. The three changed units are
`sess-1787667978570`, `sess-1788878266185` and `sess-1788878266188`; their final
PIDs are `1666049`, `1666135` and `1666052`. They exited with status 0 at
`07:47:51`, relaunched with their original native resume identities and
unchanged worker generation, and reached the broker's startup readiness gate by
`07:47:56`. Resume readiness does not prove a later real user turn or
uninterrupted execution.

Controller lifecycle logs show an automatic Machine-refreshed Grok credential
promotion from generation 43 to 44 at `07:47:16`, followed by stale replica
observations. The unchanged Machine path sends `RollProvider` after an applied
auth generation advances; its broker drains idle matching workers and resumes
them. This is consistent with the observed same-generation cutovers, not an
explicit Machine activation. The task did not initiate login, credential
refresh, Provider rollout or worker commands. A complete credential-convergence
and trigger-correlation acceptance was not performed; subsequent stale/timeout
warnings are not marked resolved. Do not infer universal worker continuity from
a Controller-only component transaction.

A separate, explicitly narrower component/telemetry audit passed after the
settled capture. It checks the accepted immutable matrix, exact process,
receipts, host/Machine/Web boundaries and health, but **does not replace** the
failed worker audit. The bounded observation records
`worker_continuity_accepted: false`; no capture or strict assertion was
rewritten to report a pass.

The candidate/next-transaction rollback roles use the new Controller artifact.
The current transaction's automatic recovery target was the previous active
`/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release`. Its
retained 2026-09-13 45/294/96/78-check evidence was independently hash-checked
and matched to the actual pre-deployment matrix. Historical `previousRelease` is
not substituted for the next transaction's recovery target.

Cold Controller and active/rollback/cold Machine roles remain exactly those
recorded in the
[previous acceptance](telemetry-managed-delivery-conformance-2026-09-13.md).
Columbus remains `1da44eb88883b52a7bff266da0c07ede35193659`, with host closure
`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`.
The after and settled role captures exactly match the accepted final matrix;
continuity is measured immediately around activation, separately from earlier
independent Web task activity.

## Retained evidence and remaining work

Private directory: `/tmp/cowboy-policy-preflight.YycnWL/`. The final acceptance
uses the `*-integrated-2.log` gate logs, integrated quality/build logs,
`probe-integrated/receipt.json`, `preactivate/`, `after/`, `settled/`, and the
independent artifact and scoped component audits. All gate/probe receipts are
private mode-0600 create-only files. The initial candidate's passing receipts
remain separate and were not rewritten.

| Final receipt under `dist/`                                                            | SHA-256                                                            |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `telemetry-connected-conformance/20260914-policy-preflight-integrated-2.json`          | `003c1193381f991e543375ec35bd8adee8f3b3e113fadb5de603c0a7ed977095` |
| `telemetry-writer-conformance/20260914-policy-preflight-integrated-2.json`             | `9d96b11be555c40eaab9b8201f9a95329a78aad0263c2d837a6e3a660b9b6dde` |
| `telemetry-reader-conformance/20260914-policy-preflight-integrated-2.json`             | `07a795739d810e054e6d25f171589f3bfe7bfec57fe1a436abff858114025a7d` |
| `telemetry-background-startup-conformance/20260914-policy-preflight-integrated-2.json` | `8d15369951e46d0db2171599eafc1d43b154f6541504cf9837177e6fadab7d10` |

Matrix SHA-256:
`00842400d86b5c314da2dff54a9a225e72bf9f28f4baa62f0a3901908950b9a5`. Host
preflight receipt:
`335e615e6933b9d2e349e026586e3a12ce3e77614b24f5e2c0fc6127250d2331`. Worker
observation (not uninterrupted acceptance):
`5f0579f299e0c2efeaef7349322853d2e1c0270c54700bd64641a3e795314ddc`. The failed
`audit-after.log`, separate `audit-component-observation.log`,
`worker-observation.json`, all three worker snapshots, and bounded
activation/Provider-refresh/worker lifecycle logs remain retained.

Local invocation corrections are retained: the initial host probe used an
unsupported public-origin CLI option and then omitted the explicit boolean
value; the integrated gate invocation initially used relative receipt paths, and
the Catalog coverage command initially named the wrong directory. Each was
rejected or failed its check; corrected runs use the declared interfaces and
fresh logs/receipts. The initial Clippy warning was fixed without an allow. No
runtime oracle, timeout or receipt was weakened to obtain acceptance.

The automatic Provider-auth rollover/worker continuity observation needs its own
acceptance before claiming session invariance under Controller reconnect. Do not
inspect private credentials or alter auth state to manufacture it.

Next: the complete intended production Service/Machine policies, fresh actual
Operator confirmation, real Victoria ingestion/query verification and the owned
managed-policy cutover, including production failure/restart acceptance.
Existing legacy Victoria export is not disabled by this release. The first
managed Service intent permanently fences that legacy path even if refused or
aborted; do not open writers alone or erase evidence to regain it. Emitted
telemetry is `NoRestore`. P2 and the later generic Plugin DAG/P3/P4 migration
remain unfinished.
