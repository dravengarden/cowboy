# Compatibility exits

Every path that exists only for older clients, Controllers, Machines, workers,
releases or stored data, with the condition that would let it go and the check
that proves the condition. Nothing is removed merely for being old: a row
leaves this table only in the change that removes its code after the proof
holds, or when it is reclassified as permanent.

Inventory taken 2026-11-05 for slice 7.1 of the
[structural refactor plan](structural-refactor-plan-2026-11.md).

## The missing floor

`MIN_PROTOCOL_VERSION` (`components/wire/src/runtime_wire.rs`) and
`MIN_MACHINE_PROTOCOL_VERSION` (`components/wire/src/machine_protocol.rs`) are
both **1**, while the current versions are runtime 5 and Machine 41. No branch
for an older Machine or worker can therefore be proven dead: every wire row
below shares one prerequisite, raising a floor.

Raising a floor edits `components/wire`, which the retained-worker interface
pin in `flake.nix` hash-compares, so it is a Machine/worker-generation
maintenance boundary. Its proof is that every Machine registry entry reports a
protocol at or above the new floor and the retained worker generation
(`worker-generation.txt`) is at or above the release that introduced it.

**Controller-side floors (2026-11-05).** Without touching the pinned wire
constants, the Controller now refuses older peers itself:
`MACHINE_PROTOCOL_FLOOR = 37` in `server/machine_handshake.rs` and
`BROKER_PROTOCOL_FLOOR = 5` in `src/remote_runtime.rs`. Measured that day:
Hawk, Falcon and OVH run Machine releases speaking 41 and the MacBook Air's
bootstrap build (2026-11-01) at least 40; every retained worker pin since
2026-10-19 speaks runtime 5, and every live worker on the four hosts started
in November. A host rolled back to a Machine release older than 2026-10-22 is
refused. Controller code that branches on a Machine protocol at or below 37
now only sees "connected" or "not connected"; each such branch is removed
individually, because `machine_supports` is also false for a disconnected
Machine. Machine-side tolerance of older Controllers is unchanged.

Boundary legend: **M** Machine/worker-generation maintenance, **C** ordinary
Controller release, **W** ordinary Web release.

## Wire protocol

| Path | Site | Keeps working | Exit condition | Proof | Boundary |
|---|---|---|---|---|---|
| Legacy artifact URL | `server/routes.rs` `/provider-artifacts/{digest}/{name}`; `http_policy.rs` | Published pre-Catalog URLs | Stated: "after those immutable URLs age out" | No `release.json` of an installed Machine generation names `/provider-artifacts/`. Not met on 2026-11-05: Falcon's installed `claude-code` generations still do. The Controller does not log request paths, so absence of callers cannot come from logs | C |
| Handshake proof v2 | `machine_protocol.rs` (signs v2 when an older Controller asks) | Older Controllers | Every Controller requests v3 | Controller rollback window past the v3 release | M |
| `SetDesiredGeneration.worker_command` optional | `runtime_wire.rs` | Older cores in a rolling update | Runtime floor | Raised `MIN_PROTOCOL_VERSION` | M |
| Launch without `execution_binding` | `runtime_wire.rs` | Unbound workers | Runtime floor ≥ 2 and no unbound session | Floor plus a store query for sessions without a binding | M |
| Optional fields older Machines omit | `runtime_wire.rs`, `machine_protocol.rs` (absent inventory means a legacy Machine), `src/worker_execution.rs`, `src/machine_control/workspace.rs`, `server/workspace_preparation.rs`, `execution/sessions.rs` | Older Machines and workers | The field's `*_PROTOCOL_VERSION` ≤ floor | Floor | M |
| `legacy_provider_usage_dimension` | `machine_protocol.rs` | Usage rows without dimensions | `NATIVE_USAGE_PROTOCOL_VERSION` (37) ≤ floor | Floor | M |
| Legacy Machine session reads | `src/machine_control/session_reads.rs` | Older Machines | Machine floor | Floor | M (floor), C (branch) |
| Path-based code resource and language reads | `server.rs`, `code_reads/language.rs`, `code_buffers.rs` | The mobile Review path queries | Review uses only the buffer API and `CODE_BUFFER_SYNC_PROTOCOL_VERSION` (20) ≤ floor | `rg` over `web/src/mobile/review`; floor | C+W |
| Reconciliation grace for legacy hosts | `server.rs` | Hosts that heartbeat only after reconciliation | Machine floor | Floor | C |
| Auth-generation retire refused by old Machines | `background_jobs.rs` | Machines without the adapter | Every Machine has it | Floor or capability inventory | C |
| Older-worker refusals and diagnostics | `components/machine/src/machine_broker.rs` | Retained worker generations | Retained worker ≥ the version that introduced each field | `worker-generation.txt` | M |
| Older-Controller tolerance in the Machine | `machine_plugins/operations.rs`, `plugin_install/interrupted.rs`, opaque settings in `src/core.rs` | Controller rollback | Two-generation rollback window holds no pre-change release | Deployment history | M / C |

## Persistence and migrations

| Path | Site | Keeps working | Exit condition | Proof | Boundary |
|---|---|---|---|---|---|
| `migration_compat` (+ `migration_policy` test) | `src/migration_compat.rs`, `src/store.rs`, `src/store/sqlite.rs` | Pre-squash databases get the baseline marker (PostgreSQL 34, SQLite 8) | Every database carries the marker and no rollback below the squash release | `_sqlx_migrations` query on every deployment; release floor | C |
| Inbound Hub tombstones | `src/core.rs` `SetSessionAutoResume`, `ResumeTurn`, `SetSetting`; `web/src/protocol.ts` | Stale installed PWAs | Stated: accepted and ignored "before it updates" | No such inbound type in client telemetry over a window; service-worker `VERSION` past the retiring release | C+W |
| Outbound `Settings` tombstone | `src/core.rs`, `server.rs`, `web/src/protocol.ts`, `web/src/store.ts` | Cached clients read an empty snapshot as auto-resume off | As above | As above | C+W |
| Retired synthetic continuation purge | `src/core.rs`, `src/agent_model.rs` | Rows from older Controllers | No such rows | Store query returns zero | C |
| `COWBOY_POSTGRES_URL` flag | `src/cli.rs` | Deployed callers | Stated: "while the backend-neutral flag becomes canonical" | No NixOS module or tool sets it (true for Columbus on 2026-11-05). Removal must refuse startup when the variable is set: silently ignoring it would start an external deployment in memory | C |
| Passkey storage bridge | `src/core_passkeys.rs` | One-time import | Import done everywhere | Reader floor in `core-security-storage-bridge.md` | C |
| Hostless OIDC | `src/auth_plugins.rs` | Bootstrap migration | Stated: "only during bootstrap migration" | Every host policy is Catalog-only | C |
| Telemetry legacy admission | `src/telemetry_binding.rs`, `telemetry_binding/background.rs` | Schema-one byte compatibility | Stated: "until the first explicit resolution" | Every deployment's journal has its first row | C |
| Legacy Draft directory import | `src/store/workspace_documents.rs` | One-time import | No legacy Draft directory remains | Host scan | C |
| Legacy JSONL log reader | `src/logs/cli.rs` | Retained JSONL logs | Retained directories aged out | Host scan | C |
| Machine native-profile and runtime-state migration | `components/machine/src/machine_plugins.rs`, `native_profile.rs`, `extensions.rs`, `operations/installations.rs`, `account_stop/worker_exit.rs` | Legacy on-disk Machine state | No legacy slot or home remains | Machine-side scan | M |

## Plugins and releases

| Path | Site | Keeps working | Exit condition | Proof | Boundary |
|---|---|---|---|---|---|
| `LegacyProviderRelease` | `src/legacy_provider_release.rs`; `components/wire/src/machine_auth.rs`; readers in `provider_catalog.rs`, `machine_plugins.rs`, `auth_refresh.rs`, `installation.rs`, `provider_api.rs`; listed in `flake.nix` machine sources | Already installed Provider generations | Stated: "during the Plugin rollout"; no installed legacy envelope | Machine inventory shows none; retained worker is post-Plugin | M |
| Identity-based Provider fallback | `src/provider_behavior.rs`, `src/provider/mod.rs` | Sessions created before signed behavior contracts persisted | No session with a null generation or behavior contract | Store query (`SessionMeta`: `None` is reserved for package-less legacy sessions) | M |
| Pre-Catalog source storage | `src/plugin_catalog.rs`, `retention.rs`, `watch.rs` | Legacy root | Stated: once every ID has a release authority | Catalog listing | C |
| Registry schema 2 | `src/plugin.rs` accepts 2 and 3 | Historical releases | Slice 7.3 (archive) | Every referenced release at schema 3 or archived | M |
| Code-intelligence schema 1 | `components/plugin-sdk/src/lib.rs` | Published legacy adapters | Stated: "already published legacy adapters" | No schema-1 adapter in `components/registry.json` | M (signed component) |
| Catalog reader conformance | `tools/catalog_reader_conformance.py` | Legacy bytes in both readers | Reader floor | `catalog-reader-bridge.md` (floor 2026-10-02) | C |
| Legacy `default` account | `components/wire/src/provider_account.rs`, `portable_contract.rs`, `src/agent_api.rs`, `src/product_cli.rs`, `src/provider_service.rs` | Account `default` | No legacy default credentials | Account registry scan | M |

## Web client

| Path | Site | Keeps working | Exit condition | Proof | Boundary |
|---|---|---|---|---|---|
| `legacyRecordsNotice` | `web/src/legacyRecordsNotice.ts`, `store.ts`, `ProductSyncDataNotice.tsx`, `productSyncDatabase.ts` (`cowboy:sync:` keys) | Pre-dataset IndexedDB records | No device retains `cowboy:sync:` keys | VictoriaLogs `legacy_records_notice` with `retained:0` on every device | W |
| Calls dock rollback | `web/src/backgroundTasks.ts` `ui.calls_dock_legacy` | The old Calls dock | Stated: two Web releases or 14 days, the later of 2026-11-14 and the second Web release | Release log and the parity checklist in `background-tasks.md` | W |
| Fields older Controllers omit | `web/src/agentTools.ts`, `mobile/review/codeApi.ts`, `callApproval.ts`, `components/provider-ui/src/index.ts`, `sessionErrorNotice.ts` (`session_id: null` before 2026-11-05) | Web against an older Controller | The Controller rollback window holds no release without the field | Deployment history | W |

## Build and release

| Path | Site | Keeps working | Exit condition | Proof | Boundary |
|---|---|---|---|---|---|
| Schema-one writer outputs | `flake.nix` `cowboy-machine-writer`, `writerSchema`, `writer-host-release` | Session deletion writer behind a root floor | A separately accepted migration (slice 7.2) | Every Machine has the writer or the reader floor is admitted; recovery conformance | M |
| Bootstrap outputs | `flake.nix` bootstrap and host variants | First install without a desired generation | No host needs a signed bootstrap | Recovery and startup-admission conformance | M |
| `pedantic = allow` | `Cargo.toml` workspace lints | Clippy debt that predates the machine baseline | Stated: "reduced separately" | Zero pedantic warnings | M (Cargo.toml) |
| Lint allowance groups | `too_many_lines` 71, `dead_code` 44, `too_many_arguments` 28, `unused_imports` 10, others ≤ 3 | — | Target ≤ 60 (plan §7); the two `dead_code` allows in `components/plugin-sdk` wait for a component release | `tools/structure-baseline.json`; `just feature-check` | C (M for plugin-sdk and wire) |

## Permanent, not exits

The dataset socket gate (`server/sync_dataset.rs`, 426 for old browsers), the
retained-worker interface pin and the retired `--compat-socket` guard in
`flake.nix`, readers of historical transcript shapes (`web/src/derive.ts`,
`tools/result.ts`, `attachments.ts`, `sessionProject.ts`,
`pluginHost/inventory.ts`) and device storage format readers
(`composerExpand.ts`, `turnNotify.ts`, `vimSetting.ts`, `readingSettings.ts`,
`themeDefault.ts`, `composerToolbarModel.ts`) protect durable data or
boundaries and stay.

## Retired

- 2026-11-06: Controller gates on `PROVIDER_ACCOUNT_PROTOCOL_VERSION` (36)
  and `NATIVE_USAGE_PROTOCOL_VERSION` (37) wherever the protocol is a
  connected Machine's negotiated one (`connection_protocol`, the handshake
  value): named-account login, cancellation, registration, readiness, usage
  collection, Provider auth scope, reconnect grant reconciliation and the
  runtime encryption key, plus the remote-wipe, temporary-credential,
  worker-tree and account-transition `ensure!` gates, and the managed-call,
  managed-image, managed-environment and Agent control gates (28–33).
  `MACHINE_PROTOCOL_FLOOR`
  makes them unreachable.

- 2026-11-05: the protocol-v4 `GET /api/machines/{id}/providers` inventory
  adapter. Every client in the repository reads `/plugins`.

## Next step

Most rows wait on one decision: a Machine maintenance that raises
`MIN_MACHINE_PROTOCOL_VERSION` and `MIN_PROTOCOL_VERSION` to what every
enrolled Machine and the retained worker already speak. The Controller and Web
rows with stated conditions (the artifact route, the `COWBOY_POSTGRES_URL` flag,
the Calls dock rollback after 2026-11-14) can retire independently once their
proofs are recorded.
