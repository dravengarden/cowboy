Cowboy pluginization started from commit `69defc1d` on branch
`cowboy/sess-1788279284752`. This candidate contains the completed
core-extraction implementation and the subsequent design-review corrections.
Preserve the task history and continue from
**Completion record** and **Design review addendum** below. The earlier
starting-state inventory is retained only to explain what was removed.

## Goal

Cowboy core (server, SPA shell, Machine, native shell, DB) must stay business-agnostic. Adding a conforming Provider or login method must not require a new core ID branch, card, icon, collector, overlay, parser, widget, or SQLx table in core. First-party plugins already ship `plugins/*/host.json` and `examples/authentication/*/host.json`. That contract is source of truth.

Normative contract: `docs/requirements.md` (especially CR-0/CR-1/CR-2). Plugin lifecycle: `.agents/skills/release-cowboy-plugin/`. Frontend: `web/AGENTS.md`. Cross-cutting: `CLAUDE.md`.

## How to work

- Run from repo root in the pinned shell:
  `nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH just check-compact` for
  the full gate; for a slice use
  `nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked <filter>`
  and Deno tests from `web/` when they need `@cowboy/provider-ui`.
- Do not use host `cargo`/`deno` as a preliminary check.
- Do not `deno fmt` `web/src/App.tsx` (4-space outlier). Match 4-space when editing it.
- Do not edit applied SQLx migrations (bytes are checksummed). Add a new migration.
- `PluginHostSpec` lives in `components/plugin-sdk/src/host.rs` and has
  `deny_unknown_fields`. New `host.json` keys belong in that shared schema;
  update any narrow `src/plugin_runtime_args.rs` projection that consumes the
  field. The latter is compiled for Machine (`machine-host`).
- Isolation: `claude-deepseek` / `codex-deepseek` must not read ordinary Claude/Codex homes. Tests live in `src/provider/mod.rs`.
- Core extension test for every slice: a new conforming Plugin ID with only
  package manifests, bounded `host.json` data, and optional non-browser
  `collector/**` sidecars must not require editing `src/provider/mod.rs` match
  arms or SPA `if (id === ...)` branches.

## Historical starting point — superseded below

This section records the state inherited at `69defc1d`. Some named files and
approaches were deliberately removed by the completed implementation; it is
not a current architecture inventory.

- Host singleton `globalThis.__COWBOY_PLUGIN_HOST` + `PluginSlot` (`web/src/pluginHost.ts`, `components/plugin-api/`).
- `PluginHostSpec` in `src/plugin_host.rs`.
- Occupancy, visuals, usage maps, loopback, isolation, CLI auth from `host.json`.
- `lookup()` isolation from `isolated_home_env` / `isolated_shell`, not `id == "codex-deepseek"|"claude-deepseek"`.
- `builtin()` launch table from `launch_plugin_ids()` + payload args/env + occupancy CMD sharing.
- Machine PATH/CLI/adapter CMD/`EXECUTABLE`/loopback inventory from `path_detect()`, `occupancy_slots()`, `cli_auth`, `loopback_*`.
- Adapter staged command from `entrypoint` via `adapter_entrypoint()`.
- Cache-protection policy from the first `host.json` with `usage.cache_protection`.
- Model catalog path from `loopback_catalog` (`CODEX_DEEPSEEK_CATALOG` removed).
- SPA usage/visual/occupancy maps from `web/src/bundledHostPlugins.ts`.
- Password/OIDC/passkey **UI** is plugin-owned. Core `/api/auth/*` as Controller protocol drivers is allowed by CR-0 — do not blindly delete it. Unhook **UI** from compiling plugin TSX into the SPA and from plugins importing `web/src/auth/authApi.ts`.

Key files at that starting point included `src/plugin_host.rs`,
`src/plugin_runtime.rs`, `src/plugin_runtime_args.rs`, `src/plugin_storage.rs`,
`build.rs`, the first-party `host.json` files, `web/src/pluginHost.ts`, and the
now-removed `web/src/bundledHostPlugins.ts`.

## Original remaining work (completed below)

### 1. Drop-in first-party discovery

`src/plugin_runtime_args.rs` still `include_str!`s every `plugins/*/provider.json` and `host.json` (`ADAPTER_HOSTS`, `NPM_PAYLOADS`). `src/provider_catalog.rs` `EMBEDDED_SOURCES` and `src/provider_behavior.rs` `LEGACY_SOURCES` do the same. `build.rs` already discovers bundled **hosts**. Extend discovery so adding `plugins/<id>/{provider.json,host.json}` does not edit a Rust table. Self-test: discovered set equals on-disk first-party plugins.

### 2. Named usage collectors still compiled

`src/usage.rs` still switches `UsageCollectorKind::{OpenaiAppserver,XaiBilling,DeepseekStore}`. Implementations: `src/provider_info/{openai,xai,deepseek}.rs`. `collector_argv` already exists for `Command`/`Unknown`. Move first-party collection behind plugin-owned argv or a signed collector so a new account does not add an enum arm. Reset has the same shape.

### 3. Parser / widget / error / overlay / CLI-auth probes still in host

Closed enums in `src/plugin_host.rs`: `UsageLimitParserKind`, `UsageWidgetKind`, `UsageErrorKind`, `UsageSessionOverlay`. SPA: `web/src/usageLimits.ts` `USAGE_LIMIT_PARSERS`, `web/src/usageWidget.ts`. Overlays: `src/provider_info/mod.rs` → `anthropic::overlay` / `gemini::overlay`. `cli_auth` kinds `GeminiEnv`/`GrokJson` still compile `probe_gemini_auth` / `probe_grok_auth` in `src/machine_cli.rs`. Goal: plugin-owned or generic data-driven code so `xai-credits` / `anthropic-rate-limit` / `deepseek-balance` / Gemini file probes are not new core branches.

### 4. DeepSeekDetails and PasskeysPanel still SPA-compiled

`web/src/pluginHost.ts` imports `plugins/claude-deepseek/ui/DeepSeekDetails.tsx` into `host.components`. Plugin `ui/index.js` renders `components.DeepSeekDetails`. The TSX imports SPA modules (`activityUsage`, `usageLimits`, `ObservabilityFilters`, `Sheet`). Do not claim this fixed by moving the file. Options: (a) rewrite with only `host.ui` / `host.call` in plugin JS; (b) a real plugin TSX bundle loaded from `/api/plugins/<id>/ui/` that does not import `web/src/*`. Update `web/src/pluginSlot.test.ts` (it currently requires the host kit to compile DeepSeekDetails).

Same for `examples/authentication/passkey/ui/PasskeysPanel.tsx` importing `web/src/auth/authApi.ts`, and host kit `PasskeysPanel: ProductPasskeysPanel`.

### 5. Login HTTP vs plugin RPC

Keep Controller protocol drivers (`/api/auth/*`, `web/src/auth/authApi.ts`) if they stay generic. Remaining: `pluginHost.ts` `auth.login/register/setup` wrapping `authApi`; password UI uses `host.auth`; passkey UI uses `authApi` directly. Prefer `host.call(pluginId, body)` + `rpc_argv` for plugin-specific ops. Do not move product session/user tables into a plugin unless you also do item 7.

### 6. Signed host UI as the runtime source

`tools/write-plugin-host-bundle.ts` and `just plugin-build` already emit `.hostbundle.json`. Controller `src/plugin_runtime.rs` stages compile-time bundled hosts and can install catalog host bundles. SPA still compiles first-party UI from the source tree. Make signed host bundles the live UI (load `/api/plugins/<id>/ui/index.js` from staged generation, not Vite-bundled copies). Keep `plugins/provider-usage-slot.js` as the single shared usage implementation.

### 7. SQLx business tables still core

Plugin storage exists (`src/plugin_storage.rs`, schema `plugin_<id>`). Core still owns `users`, `user_sessions`, `user_passkeys`, `provider_usage_*` (`migrations/` and `migrations/sqlite/`). **Never edit applied migration files.** New plugin-owned data → new plugin migrations. Moving product-auth tables is a real migration; fail closed. Passkey already has plugin storage plus core import (`src/plugin_passkeys.rs`).

### 8. Isolated home implementations still product-shaped

Dispatch is generic (`CODEX_HOME` / `CLAUDE_CONFIG_DIR` / mkdir fallback). `prepare_codex_deepseek_home_at` and `prepare_claude_deepseek_config_dir_at` in `src/provider/mod.rs` still contain Codex/Claude linking and config.toml/settings.json writers. Move those into plugin-owned data (payload `runtime.arguments` already feeds `isolated_config_toml`) or keep only as env-key conventions, not plugin-id matches.

### 9. Mac / native ABI

`native_capabilities` is on `PluginHostSpec`. The Mac app / native shell has no plugin ABI for host UI or provider cards. Web `PluginSlot` does not cover this. Separate slice; do not recycle Cowboy sessions with a Machine/native release.

## Original acceptance target

A new first-party Provider can be added by dropping a plugin tree + host.json + UI, without a core ID branch; DeepSeekDetails and PasskeysPanel are not compiled into the SPA host kit; collectors/parsers/widgets/overlays/cli_auth probes are plugin-owned or generic; signed host bundles are what the running controller serves.

## Completion record — 2026-09-05

Status: the original nine-item core-extraction target and the code corrections
below are implemented. The initial extraction record preceded the commits and
clean builds in **Candidate verification**. Production signature, publication,
activation and platform/real-login acceptance remain separate prerequisites.
This is not a completed end-to-end migration of every Plugin kind: the final
runtime review also identified Zed's legacy execution path, recorded below.

1. `build.rs` now discovers every first-party Provider and host source. The
   runtime-argument, Catalog, and legacy-behavior readers consume that generated
   inventory; tests prove it equals the on-disk plugin trees.
2. First-party usage collection and reset behavior now runs through signed,
   plugin-owned `collector/**` programs and generic argv execution. Core no
   longer contains the OpenAI/xAI/DeepSeek collector implementations.
3. Usage parser, error, widget, activity, overlay, and CLI-auth policy is
   bounded host data. Business-specific enum variants and Gemini/Grok auth
   probes were replaced by opaque typed IDs, generic strategies, and a closed
   auth-rule evaluator.
4. Browser plugins are data-only. All plugin/example `ui/**` sources and the
   shared executable usage module were removed. Signed `ui.renderers` values
   select only Cowboy-owned closed renderers, including activity details and
   Passkeys.
5. Product login remains Controller-owned as required by CR-0. Password,
   Passkey, and OIDC integrations contribute bounded host data and slot claims;
   the closed Cowboy renderers call generic Controller protocol drivers without
   importing plugin code.
6. Release-bound host bundles are the runtime source when a trusted Catalog
   release exists. They contain `host.json` plus optional `collector/**`, reject
   browser-executable content, and feed the validated `/api/plugins` platform
   inventory. The Controller no longer exposes a plugin UI-file route. The
   compile-time first-party bootstrap fallback is recorded as transitional
   below.
7. Product user/session records remain deliberately Controller-owned. Plugin
   storage is capability-selected, independently migrated, checksum-protected,
   and downgrade-safe. Provider usage remains generic, and new PostgreSQL
   `0043` / SQLite `0017` migrations replace the reset-provider allowlist with
   a bounded plugin-ID constraint while preserving existing rows.
8. Isolated homes are selected by signed `isolated_home_env` data. Codex and
   Claude variants receive plugin-authored config/settings without reading or
   linking ordinary user homes; generic and symlink-boundary cases are tested.
9. Native shells now expose native-host API `1.0.0`. Web invokes a capability
   only when both the signed plugin generation and shell declare it; unknown or
   version-mismatched capabilities fail closed.

The source-compiled first-party SPA inventory was removed. Usage, visual, and
Machine-occupancy maps are populated only from the validated runtime inventory;
tests explicitly load package-authored host fixtures when first-party behavior
is under test. Prompt-origin, account-usage namespaces, telemetry/currency, and
managed configuration dispatch were also made generic.

Release metadata now appends component release `2.4.0`, retaining the earlier
`2.3.2` record. The six Agent Plugins are `3.1.10`; Zed is `1.1.10`;
Plugin contract/SDK are `1.5.0`, Plugin API remains `1.1.1`, and Provider SDK/UI
are `3.1.10`. Password and Passkey have independent `1.0.0` source manifests
under `examples/authentication/`. These are local, unsigned build results—not
published releases. Provider version tests now derive current/future versions
from the source/SDK instead of breaking at the next coordinated version bump.

Verification:

- `nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH just check-compact`
  passes, including component/plugin graph checks, all seven independent plugin
  builds, isolation checks, Rust/Zed/Web format and lint, dependency audits,
  feature builds, tests, and release builds.
- `git diff --check` passes.
- `find plugins examples/authentication -path '*/ui/*' -type f -print`
  returns no files.
- Production-source audits find no `bundledHostPlugins`, executable plugin UI
  import/loader, fixed account-usage provider enum, or static usage/visual/
  occupancy fallback inventory.

The dependency audit still reports the pre-existing allowed warning that
`spin 0.9.8` (transitive through `sqlx-sqlite`) is yanked; advisories, bans,
licenses, and sources all pass.

## Design review addendum — 2026-09-05

The extraction direction is sound: Cowboy owns a closed typed host, Plugins
own data and private implementation, and a conforming Provider no longer needs
an ID-specific core branch. The review found several places where the first
solution met that extensibility target but did not yet preserve the stronger
immutable-Plugin and exact-generation invariants.

Corrections implemented in this working tree:

1. `/api/plugins` now serializes an explicit public descriptor instead of the
   internal activated-host type. Collector/reset/RPC argv, storage state,
   reset/session policy, and generation paths cannot leak when an internal
   field is added.
2. Web accepts only bounded Plugin IDs and lowercase 64-hex host generations,
   and usage, visual, occupancy, auth, and renderer registries consume the same
   once-validated inventory. Invalid or duplicate rows cannot partially mutate
   different registries.
3. Once any trusted external release exists for a Plugin, an absent,
   ambiguous, or invalid released host can no longer fall through to stale
   source-bundled behavior. Catalog ingestion validates the complete host
   schema and every referenced signed runtime file.
4. Host data is no longer a parallel publisher-signed sidecar. Release schema
   2 binds the exact `.hostbundle.json` bytes into the outer Plugin composite
   digest and its single signature. Build, sign, verify, publish, coverage, and
   Catalog ingestion all enforce that same identity; schema 1 explicitly
   forbids an attached host bundle.
5. `cowboy-plugin-pack build` now creates and binds the host bundle itself.
   The repository-only Deno writer was removed, and the isolation check proves
   a Plugin with host behavior builds outside Cowboy using only its source and
   the matching SDK CLI.
6. Catalog publication writes package/runtime/host bytes first and installs the
   release envelope last as the atomic visibility marker. Catalog refresh
   ignores orphan package bytes but fails closed once a complete release marker
   is visible.
7. Plugin JavaScript runs with no inherited Controller environment, config,
   remote modules, npm, import, or write permission. Bare read/run/net grants
   are rejected; signed typed environment bindings resolve only exact paths,
   executables, and HTTP origins. Input/output/time are bounded, and every exit
   path reaps the invocation's cgroup/process-group descendants.
8. A declared Plugin storage capability now fails host activation if migration
   fails instead of exposing a UI/RPC host whose required namespace is absent.
9. The core `--codex-command`/`${USAGE_CLI_COMMAND}` special case was removed.
   The Codex host owns its optional `COWBOY_CODEX_COMMAND` binding through the
   same typed executable-grant mechanism available to any Plugin.
10. Controller host generations now use one immutable Catalog/runtime snapshot
    keyed by `(plugin_id, plugin_version, artifact_digest)`. Refresh validates
    and stages the entire candidate set, runs required Plugin migrations, and
    swaps Catalog entries plus runtime hosts only after all work succeeds. Exact
    old and new generations coexist; requests capture one snapshot, while usage,
    auth, RPC, Machine cards, session visuals, renderer slots, and occupancy
    resolve either the recorded exact identity or one explicit unambiguous
    default. A persistent private Catalog-authority marker prevents a transient
    empty scan or restart from reviving source-bundled behavior after a trusted
    release has existed.
11. Released usage collection, reset, and activity transforms now execute by a
    typed Machine protocol 7 request against an active exact Plugin and auth
    generation. Catalog installation carries the exact release-bound host
    bundle bytes; the Machine independently verifies and stages them beside the
    package/runtime, selects signed argv locally, binds exact component commands
    and authentication homes, and never records sensitive responses in event
    history. DeepSeek's two balance lanes use signed `collector_sidecars` data
    to resolve exact Codex/Claude Provider gateways on dynamic loopback ports;
    Controller-configured endpoint order is no longer authoritative. The
    source-bundled Controller executor remains only for pre-Catalog bootstrap.
12. Machine proof v3 now binds a strict generic `PluginContractInventory`
    independently of the Agent-specific Provider inventory. Catalog releases
    derive exact Plugin SDK, manifest, package, release, payload-kind,
    host-bundle, and host-contract requirements; Controller and Web reject an
    incompatible target before package bytes cross the control channel. New
    install/upgrade requires protocol 7, while protocol-5 uninstall and exact
    retained-generation compensation remain available for rolling upgrades.
13. The versioned `PluginHostSpec`, CLI-auth rule schema, SQL containment,
    process-grant grammar, and signed runtime-file validation now live in
    `cowboy-plugin-sdk`. The independent SDK builder, Controller Catalog, and
    Machine staging all parse and validate that same type at their own trust
    boundaries. Core retains only Machine-side CLI-auth I/O/evaluation and a
    compatibility re-export; the weaker SDK JSON-shape-only check was removed.
14. Password and Passkey previously had only bootstrap hosts, so there was no
    package that could actually replace them in a signed Catalog. Both now
    build independently through the same Plugin lifecycle. Authentication
    payload schema 2 selects closed `local_password`/`webauthn` Controller
    drivers with exactly empty public configuration; schema-1 OIDC remains
    supported. The shared SDK validates host capability ownership at build,
    bind, sign, verify, Catalog, and Machine boundaries: local protocols require
    a bound host, Authentication hosts contain only `host.json`, and renderer,
    native capability, and storage must match the protocol. Executable/RPC,
    unrelated Agent capabilities, schema downgrade, and secret/policy fields
    fail closed. Hermetic SDK signing and Catalog migration tests cover exact
    takeover, SQLite credential preservation, signed protocol-confusion
    rejection with snapshot retention, and no fallback resurrection on restart.
    `just example-auth-build-all` discovers every login host and joins the
    full gate; a future bootstrap host without a manifest fails that gate.
15. Controller host activation now has a separate private
    `COWBOY_PLUGIN_HOST_CONFIG` / `--plugin-host-config` document, schema
    `dravengarden.cowboy.plugin-host-activation/v1`. Exact stable release pins
    are immutable for a Controller lifetime; existing OIDC driver selections
    merge into that policy and conflicts fail startup. `catalog_only` disables
    every bundled host and requires matching signed hosts for enabled login
    methods plus exactly one WebAuthn storage host whenever Product Passkeys
    or durable admin authentication require it. Startup preflight runs before
    the core DB connection/migrations or host staging. Legacy non-Plugin OIDC
    must migrate first; hostless signed OIDC remains bootstrap-only.
    Unselected Authentication releases do not become defaults, and public auth
    status filters out unconfigured methods/panels. Refresh checks every exact
    pin before staging/migrations; a newer publication cannot advance login
    behavior or Passkey schema. Missing selections preserve the current
    Catalog/runtime snapshot and explicitly selected activation failures abort
    startup rather than installing an empty runtime. After successful staging
    and migration, a durable `plugins/.catalog-only-v1` marker prevents later
    omitted configuration or an empty Catalog from restoring any bootstrap
    hosts. The operator configuration, bounds and one-way restart semantics
    are documented in `docs/plugin-packages.md#controller-host-activation`.
    Eight new regression tests cover closed/private config, exact OIDC/UI
    identity, signed cutover preserving SQLite credentials, unselected future
    migrations, admin-only readiness, missing-pin refresh retention, migration
    failure, and corrupt/symlink/downgraded cutover receipts. This slice changes
    no Authentication config schema, public SDK, Plugin version, or production
    configuration.

16. Startup readiness can now be checked before restarting the Controller:
    `cowboy serve --check-plugin-hosts` uses the same Service environment and
    arguments and returns a data-only
    `dravengarden.cowboy.plugin-host-preflight/v1` JSON report. Catalog reading
    is separated from directory initialization, and the shared check precedes
    Service identity, cache, listener, database and host initialization in both
    modes. Neither successful inspection nor a failed startup preflight creates
    any state. Missing Catalog roots are empty inventories; refresh no longer
    creates a missing root. The report lists merged exact pins, login renderers,
    WebAuthn storage requirements, Catalog defaults and the observed cutover
    marker, but explicitly marks runtime bytes, staging, migrations, credential
    import and live authentication as unperformed. It is not an activation
    receipt. Four added regression tests exercise the actual CLI/serve dispatch, signed
    selections and bad signatures, startup/check failure equivalence, existing
    cutover receipts, unchanged file bytes/modes and an unreachable database
    URL that must never be contacted or printed. Private host/Auth/legacy OIDC
    JSON parse errors now expose only category and line/column, not invalid
    input values or unknown names that might contain misplaced credentials.

17. PostgreSQL coverage is now a reproducible gate, not an external-database
    prerequisite left ignored. `nix develop -c just test-postgres` compiles the
    library test binary once, discovers ignored `postgres_` tests and runs each
    against its own empty database in a temporary PostgreSQL cluster from the
    existing Nix lock. `just check` / `check-compact` include this gate. The
    cluster has no TCP listener, its Unix socket and root are private, and all
    inherited PostgreSQL settings/test URLs are cleared; password/service files
    belong to the fixture. Success and failure stop the server and clean only
    the exact owned fixture, including read-only Plugin generations; a failed
    shutdown retains the fixture and fails the gate. Five tests now cover the
    full storage contract, legacy migration-ledger restoration, namespace
    isolation with upgrade/downgrade and failed-migration rollback, user/admin
    Passkey import and CRUD without re-importing deleted credentials, and the
    same signed Catalog-only cutover/publication-not-activation contract as
    SQLite. Real execution exposed two stale test assumptions: a hardcoded
    migration count and byte equality for PostgreSQL jsonb serialization. The
    tests now compare the full ledger including checksums and semantic JSON
    plus all ceremony metadata, respectively. No deployed migration bytes,
    production database, Plugin release identity, or activation policy changed.

18. Controller storage authorization is now generic, not Authentication-only.
    In both source modes, every release-bound host declaring `storage` needs
    an exact host-policy pin before it can become an ID default or migrate a
    namespace. An unselected release remains addressable by exact Catalog
    identity but cannot acquire storage through publication, refresh or restart.
    Stateless Agent/Code defaults still follow the unambiguous latest release;
    a newer release that adds storage cannot inherit that permission or fall
    back to an older stateless default. The source-only bootstrap storage path
    remains for pre-Catalog deployments. Bootstrap preflight now checks required
    WebAuthn storage against selected releases and still-eligible source hosts,
    including persistent Catalog-authority markers, before core DB connection
    or state creation. Publishing a replacement without a required selection
    rejects refresh while retaining the current credential runtime. Five added
    tests cover all Plugin kinds and both source policies, signed non-Auth
    SQLite/PostgreSQL publish/pin/upgrade/restart sequences, stateless-to-storage
    upgrades, and actual startup/check parity with an unreachable DB and no
    filesystem mutations. The signed non-Auth fixture uses a code-intelligence
    payload; its Machine adapter is never fetched or executed. Historical
    migrations, public SDKs, Plugin versions and production configuration are
    unchanged by this slice.

19. Release-source review caught gaps hidden by building in the complete
    checkout. The narrow Machine source now includes `build.rs`, required by
    its new generated first-party inventories. The Controller's filtered source
    includes the Authentication package manifests/payloads needed by its signed
    cutover tests, and the Nix check phase supplies the pinned Deno runtime.
    Both real Rust package directories now contain `cowboy-plugin-js`: placing
    it only beside a release symlink does not satisfy `current_exe()` after
    symlink resolution. The Nix source-boundary check asserts these inputs and
    runtime executables. Worker generation hashing also includes the extracted
    managed-config/discovery code, SDKs and package-authored runtime inputs, so
    their later changes cannot reuse a worker generation with different behavior.
    The clean Nix builds and source-boundary check below pass. Refreshing the
    staged Cargo vendor hash was also necessary after the local SDK versions
    changed; a normal checkout build did not exercise that fixed-output lock
    check. None of these build results constitute activation evidence.

20. Zed's Nix adapter package still advertised `1.1.2` while its owned Cargo
    manifest was already `1.1.10`. `84ce854d` derives the Nix version from that
    manifest instead of maintaining another version literal. The clean Nix
    adapter build passes all 11 tests and the real adapter/server integration
    check. This fixes package metadata drift, not the separate live-runtime
    ownership gap below.

Remaining architecture work (historical snapshot; see the September 6 follow-up below):

1. **Finish Zed's exact Plugin runtime ownership (P1).** The generic installer
   stages a code-intelligence Plugin, but `supervise_zed_adapter` still selects
   its adapter/server from the legacy `ComponentStore` through
   `selected_zed_pair`, not from that installed Plugin generation. Its current
   published matrix binds only the adapter. Bind and validate the complete
   adapter/server dependency set, route execution and leases through the exact
   installed generation, and prove upgrade/uninstall/rollback and legacy drain.
   Resolve selection semantics for more than one code-intelligence Plugin
   explicitly; do not silently replace the existing shared Code service.
   The Nix-built ELF also requires its Nix loader/closure: a passing probe on
   Hawk is not evidence for an arbitrary Linux Machine. Zed's `1.1.10` candidate
   is deliberately still unbound, rather than claiming these gaps are solved.
2. **Finish immutable runtime release acceptance.** All six Agent matrices
   have now been built and bound for Linux x86_64 and macOS aarch64. Their
   declared CLI/help probes pass on both platforms. Keep the required broader
   conformance evidence, then sign and publish each Plugin independently under
   the requested release authority. Unbound or unsigned local envelopes are
   not production release receipts.
3. **Complete native and real integration acceptance.** An actual arm64 Mac
   with Xcode is reachable and its eight Agent component probes pass. That does
   not build the changed Apple shell or exercise native Passkey/bridge behavior.
   Use the registered host and committed Git exchange for that build, then
   obtain native acceptance, authorized real authentication and real ACP/sidecar
   old/new generation coexistence checks. Schema validation and version/help
   output are not substitutes for those results.
4. **Verify an authorized Catalog-only cutover (P1).** The local signed
   sources, exact host configuration and startup readiness path are implemented
   and exercised with hermetic Catalogs. Production still needs independently
   verified Password/Passkey and configured OIDC releases, an exact private
   host policy, migration of legacy OIDC config if present, a read-only check
   with the candidate binary's exact Service arguments/environment, and an
   explicitly authorized Controller deployment/restart. Preserve Plugin IDs and
   historical migrations for existing Passkey storage; verify actual login, admin auth,
   `/api/auth/status`, `/api/plugins`, and old-generation session drain. Follow
   AGENTS.md and the canonical release skill; publication never authorizes
   Machine installation or credential mutation. None of these production
   signing/publication/activation/deployment actions was performed here.
5. **Remove transitional bootstrap code after cutover evidence.** Default
   bootstrap behavior and the pre-Catalog Controller executor deliberately
   remain for existing deployments. `catalog_only` retires them at runtime,
   not from the binary. Physical deletion should follow verified migration
   coverage and legacy session drain, not silently strand old installations.

Post-review verification:

- `nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH just check-compact`
  passes after correction 18. Cowboy core has 642 tests: 636 pass in the
  ordinary all-features library suite, and all six ignored PostgreSQL tests
  pass in the separate isolated-database gate. The complete gate reports both
  groups separately. This also includes 18
  Plugin SDK tests, 17 Provider SDK tests, Zed tests, 1105 Web tests,
  Web typecheck/lint/build, feature builds, dependency audits, seven Machine
  Plugin builds, five Authentication Plugin builds, the SDK-only isolation
  build, and release builds.
- `just test-postgres` also passed with intentionally invalid inherited host,
  port, database, user, service/password-file settings, options and test URL,
  confirming that fixture connections do not use caller-selected databases.
  A failed test preserved its nonzero result while stopping/cleaning the
  cluster; successful runs left no temporary database directories. Script
  syntax and rejection of external database arguments were checked as well.
- The newly built release CLI was also run against temporary local inputs:
  bootstrap inspection returned a configuration-only JSON report with exit 0;
  Catalog-only inspection without required login selections returned exit 1.
  Neither created its missing data/Catalog directories or altered the private
  policy. The temporary fixture was removed; no production input was used.
- The gate still emits non-fatal warnings for the transitive yanked
  `spin 0.9.8` dependency and oversized Web chunks. This host-policy slice did
  not change dependency pins or Web bundling; do not confuse a passing gate
  with resolution of those warnings.
- The final public Authentication schema is self-contained rather than relying
  on resolving another schema by URL. `just plugin-check` passed again after
  this packaging-only change, including the new offline-reference regression.
- A complete Authentication Plugin lifecycle was exercised against temporary
  local state: SDK build, immutable URL binding, ephemeral Ed25519 signing,
  verification, and Catalog publication all passed; the temporary key and
  Catalog were deleted. No production publication or activation occurred.
- `git diff --check` passes. No plugin/example `ui/**` file, old host-bundle
  writer reference in production, `${USAGE_CLI_COMMAND}`, core
  `codex_command`, or bare Plugin JavaScript read/run/net grant remains.

## Candidate verification — 2026-09-05–06

The complete extraction and corrections were committed in `4347a22a`.
The Cargo vendor fix is finalized in `78d06754`; `d4ca9a15` updates only the
canonical release skill's paths, schema-2 host requirements and cutover routing.
The skill was validated with its owned validator using PyYAML from Cowboy's
locked Nixpkgs, without changing the shared Python environment.
`84ce854d` then fixes the Zed package version drift without changing SDK,
Plugin, dependency or host-contract sources.

The final clean source `84ce854df9230f72c82c5e3d5892825e58d759c7` successfully built:

| Output | Local GC-root link | Immutable Nix result |
|---|---|---|
| Controller | `result-pluginization-verified` | `/nix/store/7krwlfdrprdczj314jr460xnd5fiir88-cowboy-controller-release` |
| Web | `result-pluginization-verified-1` | `/nix/store/xhxb3ackmm2s2nfrh7bmnwzy8px0f4mq-cowboy-web-release` |
| Machine | `result-pluginization-verified-2` | `/nix/store/qn41ynv45k7mhssnx60g62asi8hprdfz-cowboy-machine-release` |
| Source boundary | `result-pluginization-verified-3` | `/nix/store/b3a60n9az6kaly9fj67n1kb5s1j3q4xl-cowboy-source-boundary` |
| Zed integration | `result-pluginization-verified-4` | `/nix/store/b1rgpzsmwm5fxqxkj0m4yhmhz782p454-cowboy-zed-integration` |

All three component `source.json` receipts contain that exact revision and
`dirty: false`. Machine's desired worker generation is
`worker-d6e3554fa38944e72167`. These are candidate artifacts, **not deployment
receipts**. The Controller's hermetic Nix check phase passed 538 default-feature
library tests (six PostgreSQL tests remain in the separate gate) and three
binary tests. The checkout's complete all-features gate passed again from the
clean extraction commit: 636 ordinary library tests, all six isolated
PostgreSQL tests, 1105 Web tests and the other package/lint/build gates above.
Both real Rust package directories expose pinned Deno 2.9.5 through
`cowboy-plugin-js`, verified with an empty process environment.
The earlier `78d06754` build remains under `result-pluginization*`; use the
explicit `result-pluginization-verified*` links above for the final candidate.
The final Controller executable is byte-identical to the one used for the
Service-input preflight below:
`sha256:602535ba7fe21f22adf5c65a705cdaeb027d61309d00949e65ceeb21a2b16f1a`.

All seven Machine Plugin packages and five Authentication examples were also
rebuilt from the clean `d4ca9a15` source. They are local artifacts under
`dist/plugins/`, with no production signatures or Catalog writes.

The repository-owned Agent runtime builder completed for each of the six
Agents using the unchanged exact dependency pins and npm/Git locks. Each matrix
contains Linux x86_64 and macOS aarch64. The SDK assigned digest-bound HTTPS
package URLs and bound each runtime matrix to its own unsigned schema-2 release.
Password/Passkey also have their own digest-bound candidate package URLs;
assigning a URL does not publish its bytes. A final local audit re-hashed every
package, host bundle and bound runtime artifact: all matched, with 16 distinct
runtime artifacts across the two platforms. Zed remains data-only/unbound.

Linux executable probes passed in the owned runtime builder. All eight unique
macOS components were additionally tested on the registered `macbook-air`
(macOS 26.4.1 build 25E253, arm64), not merely downloaded or cross-built:

| Component | Exact dependency version | Declared probe result |
|---|---|---|
| Claude CLI | `2.1.231` | `--version`, exit 0 |
| Claude Agent ACP | `0.63.0` | `--version`, exit 0 |
| Codex CLI | `0.147.0` | `--version`, exit 0 |
| Codex ACP | `1.1.7` | `--version`, exit 0 |
| Gemini CLI | `0.55.1` | `--version`, exit 0 |
| Grok CLI | `0.2.117` | `--version`, exit 0 |
| Claude DeepSeek gateway | `0.1.0` | `--help`, exit 0 |
| Codex DeepSeek gateway | `0.2.0` | `--help`, exit 0 |

The Mac verified the exact artifact digests before extraction. Archive path,
entry-type and size bounds passed; probes used private temporary homes, closed
environments and a 30-second timeout. No Provider credential or ordinary
Codex/Claude state was used. The remote fixture was removed after all probes
exited; the local build artifacts remain. These probes do not establish native
shell, authenticated ACP, sidecar drain or actual Passkey acceptance.

Zed's final Nix adapter is
`/nix/store/0digkp9a7x8pq0mqgchbq20ws9mbpyy1-cowboy-zed-adapter-1.1.10/bin/cowboy-zed-adapter`,
with executable digest
`sha256:4e49cb6941c4972e3dcd49208472f562afd51b3d4a0be38bb63ce84e3c89dfe3`.
Its owned `--help` exits successfully with an empty environment. The Nix
integration starts the actual pinned Zed server, verifies adapter health, and
opens/closes a trusted worktree and a file-buffer lease. It does not install a
Plugin or prove the remaining Plugin-generation selection path.

`dist/plugins/pluginization-candidate-verification.json` records the exact
local candidate digests, source revisions, Nix outputs, Mac probe observations
and explicit limitations. It is an ignored local verification record, not a
production publication, installation or deployment receipt. The durable
completion/remaining-work summary is this committed document.

Read-only inspection of Hawk's actual Service established the concrete cutover
prerequisites, rather than assuming a fresh deployment:

- The running unit uses `/var/lib/cowboy`, the legacy-compatible Catalog at
  `/var/lib/cowboy/plugin-catalog`, and protected
  `/var/lib/cowboy/authentication.json`. Password and Passkeys are enabled;
  OIDC exact-selects `cardea@1.1.0`.
- The candidate Controller ran `serve --check-plugin-hosts` with the running
  process's original environment, arguments and working directory, without
  printing private inputs. It returned `configuration_valid` for `bootstrap`,
  with no Catalog-only marker and WebAuthn storage required. This did not
  connect to the database, stage hosts, migrate, authenticate or restart.
- The installed Cardea selection is a trusted schema-1 release with **no host
  bundle**. Catalog-only needs an independently released Cardea successor from
  its owning repository; do not attach host bytes to the immutable 1.1.0
  envelope or disable the configured login method to make preflight pass.
- Password and Passkey are not yet published. Current Agent Catalog releases
  are 3.1.8 without host bundles. The actual
  `just provider-release-coverage /var/lib/cowboy/plugin-catalog` check rejects
  all six required 3.1.10 releases as unpublished, correctly blocking Controller
  deployment until that separate release work succeeds.

No production signing key was opened, credential changed, Catalog refreshed,
Machine installed/upgraded, component activated, or legacy data/path deleted.
The final read-only check still returned `/healthz: ok`, the existing Web
version `d60d4931b93b674abc4982ec4bda3092`, and `cowboy.service` active with
`NRestarts=0`.

## Owned-runtime and independent publisher follow-up — 2026-09-06

The preceding candidate receipts are historical, not inputs for the next
deployment. `3b1dfaf9` integrates fresh `origin/main` (`c293e0e9`), retaining
its manager health, Provider Reload, transport resilience, updated dependency
locks, website and native-shell changes together with the extraction.

The next component release is **2.5.0**: Plugin SDK/contract **1.6.0**, Code
Intelligence contract **1.2.0**, six independent Agent Plugins **3.1.12**, and
Zed **1.2.0**. Historical component releases remain unchanged. In particular,
the integrated Codex CLI/ACP pins are **0.153.4 / 1.10.0**; the older Mac probe
table above is not acceptance evidence for these new bytes.

Completed implementation and deterministic evidence:

- Code Intelligence schema 2 owns the complete adapter/server matrix, exact
  dependency/version/command bindings, typed socket/state arguments and a
  bounded readiness command. Authentication cannot smuggle Machine runtimes.
- Machine dispatch uses the explicit Plugin ID and the exact verified installed
  generation. The current Code client still selects `zed`; there is no implicit
  first-installed/latest engine selection. Worktree/buffer leases retain old
  generations during upgrade/uninstall; new worktrees use the new selection.
  After owned activation, a durable marker prevents ambient legacy fallback.
  Pre-migration legacy sessions retain their existing path.
- Runtime integrity covers every extracted file, not only the entrypoint.
  Isolated probes recheck bytes before activation. Closed runtime environments,
  private homes/sockets and process-group guards cover failure, cancellation and
  teardown, including descendant processes.
- Zed's private Nix builder now lives under `plugins/zed/runtime/`. Its Linux
  adapter is static musl, with build-time rejection of an ELF interpreter or
  shared-library dependency. Its exact private Zed server is also bound into
  the release. Both actual binaries passed the new temporary signed-package
  conformance: install, readiness, worktree/buffer open, uninstall while leased,
  close/drain, failed new selection, and retained-generation reactivation.
- The full `just check-compact` gate passes: **664 ordinary Rust library
  tests**, **six isolated PostgreSQL tests**, **1142 Web tests**, **21 Plugin
  SDK tests**, **17 Provider SDK tests**, **11 Zed adapter tests**, site tests,
  lint/type/format/audit checks and feature/release builds. Two other ignored
  tests are a subprocess fixture and the separately executed real Zed
  conformance. No registered Machine was installed or upgraded.
- The shared SDK now has an immutable `.#cowboy-plugin-pack` Nix output, which
  passes its 21 tests and supplies pinned OpenSSH for verification. Its owned
  `version` command reports `1.6.0`. The Cargo vendor hash was regenerated from
  the exact updated lock rather than weakening offline/locked builds.
- Cowboy publication now uses atomic create-only hard links, not POSIX rename
  replacement. Four focused tests pass for identical retries, concurrent
  differing/identical writes, and rejecting source/target symlinks.
- Cardea independently owns its new **1.2.0** host-bound release pipeline in
  commit **d54b994**, branch `codex/cowboy-plugin-host-20260906`. Its clean task
  worktree is `/srv/storage/fast0/agent/worktrees/cardea/cowboy-plugin-host-20260906`.
  Its complete Rust/verifier/tools/wasm/lint/format gate and dependency audits
  pass. The package owns only OIDC login presentation; issuer, PAR, client
  authentication, manual approval and account policy are unchanged. The Cardea
  Worker was not redeployed. Its final package will use the independently built
  immutable SDK, not Cowboy source or an ambient CLI.
- The canonical release skill now requires owned Code runtime conformance and
  portable bytes, and documents the immutable SDK for external publishers.
  The skill-creator validator passes against the locked Nix Python/PyYAML.

Still required before declaring release/cutover complete:

1. Build the final immutable outputs from the clean committed follow-up source;
   rebuild/bind every current Agent matrix and Zed's complete matrix. Run actual
   current macOS probes and required ACP/sidecar generation conformance.
2. Build the changed Apple shell from committed Git source on the registered
   Mac and obtain the applicable native/passkey acceptance. No physical-device
   result has been claimed.
3. Sign with the existing configured publishers, independently verify and
   publish each exact Plugin, including Cardea and Password/Passkey. Verify
   immutable URLs, Catalog observations and complete embedded-Agent coverage.
4. Apply an exact protected host policy and Cardea selection, run the candidate
   preflight, then perform the authorized Controller/Web activation through
   the machine-owned activator. Verify migration, real login, health, versions,
   Machine presence and old-generation drain before deleting any transitional
   support. Authorization does not include Machine activation/installation,
   Service Provider login, or bypassing Cardea's human approvals.

At this checkpoint, no production signatures, Catalog writes, private policy
changes, credential mutations, component activation or bootstrap deletion have
occurred. The user's continuation authorizes gated publication and Controller
restart; implementation/build success alone is not a production receipt.

### Post-build behavioral findings — release candidate 2.6.0

`7c063918` committed and published the owned-runtime implementation on the task
branch, not `main`. Its clean Nix build produced Controller
`/nix/store/y1qcz0xv1l3kfkz525c7mj0s07a9mnxx-cowboy-controller-release`, Web
`/nix/store/24z159qmz7j906hxl25230pyi91hw995-cowboy-web-release`, source boundary
`/nix/store/igrpcj6m0hg713aj9fsy63g7wwihzv8w-cowboy-source-boundary`, Zed integration
`/nix/store/08krsf8lais3cm38xy49i1mb190vybvh-cowboy-zed-integration`, and SDK
`/nix/store/0x1jbcajy22n8yrj4ingq3mkrdkj1nia-cowboy-plugin-pack-1.6.0`.
**Do not activate that Controller candidate:** subsequent real-worker testing
found a detached-worker startup defect not exercised by its Nix unit suite.

The owned loopback-only worker harness exposed two concrete defects:

1. The detached worker did not install the TLS crypto provider before constructing
   its sidecar readiness HTTP client. Reqwest 0.13's provider-neutral build
   panicked even for plain loopback HTTP. Worker startup now installs ring
   explicitly before Provider preparation, independently of Controller startup.
2. `codex-acp@1.10.0` starts Codex App Server without forwarding its `-c`
   arguments. Exact installed packages therefore lost declared routing/config,
   unlike legacy homes containing prewritten TOML. The private runtime archive
   now owns an argument-forwarding launcher with four regression tests, while
   Cowboy core remains Provider-neutral. Standard Codex declares the upstream
   adapter's existing API-key initialization request for projected API keys.

These runtime packaging changes append component release **2.6.0**, bump
`cowboy.provider-runtime` to **1.1.2**, six Agent packages to **3.1.13**, and
Zed to **1.2.1**, following the existing coordinated component-release gate.
SDK **1.6.0**, Provider SDK/UI **3.1.10** and all upstream dependency pins are
unchanged. Earlier candidate signatures/digests must not be reused.

Behavior evidence so far, before final clean rebuilding:

- A temporary real-worker test passes for Claude DeepSeek **3.1.8 / 3.1.12**
  concurrently, with distinct released runtimes/sidecar ports. Both create
  native ACP sessions; stopping the old worker leaves the new one ready, then
  both fully drain their running descendants. This uses only fake declared
  credential files and a private loopback-only network namespace.
- With the new launcher, the Codex DeepSeek prototype passes real initialize,
  session-new, sidecar readiness and complete worker/descendant stop. Its final
  new-version and distinct-generation receipts still need rebuilding.
- The new harness initially omitted required credential-file fixtures and
  waited for a legacy Ready event instead of the current native-session-ID +
  Running events. Both harness mistakes were corrected; do not classify those
  failures as product defects. Gemini/Grok and normal-account startup still
  need appropriate isolated upstream/auth startup fixtures or separately
  authorized acceptance; they have not been declared passed.
- The registered Mac fetched the committed task branch into the isolated
  `/tmp/cowboy-plugin-native.bxw89h/source` worktree. All **41 native Manager
  tests** pass and the production Passkey bridge passes iOS Simulator syntax
  compilation, with an existing deprecated UIWindow fallback warning.
  A new owned `just native-plugin-conformance` compiles this bridge into a real
  WKWebView test app on a fresh disposable Simulator; record its execution
  separately. It does not claim a full Tauri or physical-device build.
- The Mac's existing `/Users/dravenchen/cowboy-shell` has no Git metadata. The
  tracked Cowboy source has only the native overlay, not the full shell crate.
  Its source ownership/location has been asked of the user; the existing shell
  was not overwritten or treated as reproducible release input.
- Cardea **d54b994** built **1.2.0** with the clean immutable SDK above:
  package `sha256:00c1db4e8111b6ed5216e629ecdebe7279cdde8ff90a8a56deb251b9f8065a2b`,
  host bundle `sha256:9dc62540150eee892d0ddccf3a0080083453c8a279083a0b814d8c365da2f91a`.
  Existing publisher public-key fingerprints match their independently selected
  Catalog trust keys. No production private key has been used for signing yet.

Production publication/cutover remains gated. No Catalog write, real Service
Provider login, Machine installation/activation, Controller restart or host
policy modification has occurred during this follow-up.

The complete `just check-compact` gate passes for this follow-up: **664 Rust
library tests**, all **6 isolated PostgreSQL tests**, **1,142 Web tests**, SDK
and adapter suites, audits, lint, formatting, type checks and release builds.
The conformance recipe now preserves the caller's non-root UID inside its
private network namespace. Each fixture also uses a unique session/cgroup ID
across concurrent runs and Plugins; previous parallel failures sharing the old
ID were invalid evidence. Six harness regression tests cover those isolation
requirements alongside archive and protocol safety.

After correcting those fixture identities, normal Claude, Gemini and Grok
**3.1.13** all pass real worker initialize/session-new and complete descendant
drain with fake auth and no external network. Their earlier interrupted
parallel attempts do not establish missing upstream startup fixtures. Repeat
these against the final clean-build worker and matrices for release receipts.

### Clean candidate acceptance and live-cutover blockers — 2026-09-06

Cowboy implementation **`3b281bea556dc2952a53b6e82b5551672c12c88c`** is
committed and pushed on `cowboy/sess-1788279284752`. All six Agent matrices,
Zed **1.2.1**, Password and Passkey were rebuilt/bound from that clean source.
Agent runtime builders ran with an explicit credential-free build environment;
actual execution probes use private homes. Immutable Nix outputs all pass:

- SDK: `/nix/store/rvjwyg4vhw0cpxvdjch6jhibzrw4iyh8-cowboy-plugin-pack-1.6.0`
- Controller: `/nix/store/kxwcnz1357hsv0xssw9hvi86crfzd297-cowboy-controller-release`
- Web: `/nix/store/s1wc9m7zm8w8xz2x15lzkfi6ap39d77a-cowboy-web-release`
- Source boundary: `/nix/store/f0ysd93apfbv31dh7izhmvyskzj7lpkb-cowboy-source-boundary`
- Zed integration: `/nix/store/f60hziklp2jsgdw30p8lj6sppv9x9apl-cowboy-zed-integration`

The real worker is the Controller package's private build output, not a file
in the public Controller release root:
`/nix/store/gfsvwz374j1s0qzgjdl0pjbzchi8a1rk-cowboy-0.1.0/bin/cowboy-acp-worker`,
SHA-256 `b4f87b48f10682fecb6c1d5d1e97bf15d00b332ef82298095f2435ba0c203318`.
Each of the **six Agent Plugins** passes initialize/session-new and complete
worker/descendant drain with that immutable worker, fake declared auth and a
loopback-only namespace. Receipts are
`dist/plugins/<id>/runtime/worker-conformance.json`.

Claude DeepSeek **3.1.8 / 3.1.13** also passes distinct-generation coexistence,
separate exact sidecar executables/ports, old-worker stop while the new worker
stays ready, and final drain. Codex DeepSeek **3.1.13** passes alone, but its
published **3.1.8** predecessor fails fresh isolated startup with
`Authentication required`: its unchanged upstream adapter drops the declared
Codex configuration arguments. Consequently its old/new coexistence gate is
**unresolved**, not passed. Do not amend the old signed artifact, substitute a
new launcher into its bytes, or treat the dirty prototype as published-old
acceptance. Existing-session migration/drain still needs accepted evidence.

The registered arm64 Mac executed all **12 declared component probes across
six Agent packages** from these exact candidate bytes; all pass. Transfer used
only built artifacts (archive SHA-256
`41198460415105e190753ca6926894f9a8f8dc6e1fa831d20432b9733c126cc4`), while
harness source arrived via Git. Records are
`dist/native-plugin-conformance/<id>-receipt.json`. The production Apple bridge
also passes all **8 real WKWebView ABI tests** on a freshly created iOS 26.5
Simulator. Its receipt is
`dist/native-plugin-conformance/macbook-air-3b281bea.json`; the test Simulator
was removed. This is not a full Tauri App or physical-device/passkey login
receipt. The full shell's version-controlled source location is still needed;
the unversioned Mac shell was not modified.

Zed's owned static adapter and exact server pass the real temporary-package
install/open/uninstall-drain/reactivation gate. Adapter
`/nix/store/i3fqv92qm4badxf3jfn74a7jj2a64w9d-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.2.1`
has digest `207a5800fe5b29deab93ef621d47740fb1193c3dc7156e495d6d17fad113356b`;
server bytes remain `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5`.
No registered Machine was installed, upgraded or activated.

Unsigned candidate composite identities (all prefixed `sha256:`):

| Plugin | Version | Composite digest |
| --- | --- | --- |
| claude-code | 3.1.13 | `9a560b3954a0596bbc407a6bb1eb6f84cb5c583ae6dbccc3c8b78f199552b315` |
| claude-deepseek | 3.1.13 | `5463b29bc173c43b6cac91509a3f9e42ad6b78250848784e9101e1b526ea10a7` |
| codex | 3.1.13 | `94f78b5850da128ff3dc744a73e2242e8a9389e5d39e595d902a21a1d1fe8304` |
| codex-deepseek | 3.1.13 | `5eadb6e1263d03d17d27f79d0aaac90f5f263c9c8c5569b374a84b303e13505e` |
| gemini | 3.1.13 | `39d09b98e413cbd9a870d9653eaaf7f36bf50d1dbf74aa96a3ed9ad72f42ff8a` |
| grok | 3.1.13 | `2d02cf583f0727f14fc47ba6efb322bff9d095686e7eef30d6a70c51ac5fbe32` |
| zed | 1.2.1 | `ff7735efdbd7da0d78858f82d63e2840a667759734e85260cc938e91a05ebe28` |
| password | 1.0.0 | `8bc0829d805cf17debdf79a374b8ca5039b0d58d9ee9f6a6e059f6a9c7a45c85` |
| passkey | 1.0.0 | `3b4cc0e2bf9a6aeaa1b49bca0ac438c2906f51e66a949cca0d0f1b3993fa758f` |
| cardea | 1.2.0 | `6003c3d6a3b27cd5c077892881cf8c68dc2c722e617581c9b6b530c7b2d309b3` |

Cardea implementation **`d54b994`** is now pushed on
`codex/cowboy-plugin-host-20260906`. Its complete gate remains passed; only
its Plugin package/build/publisher tooling changed. Cardea Worker, client,
accounts, credentials and human approvals were not changed.

**Live-state correction and deployment blockers:**

1. The active Controller release receipt is Cowboy **`c293e0e9`**, with public
   `/version` `d60d4931b93b674abc4982ec4bda3092`. That public value is not its
   Git revision. Its SDK accepts only release schema 1 and scans every package;
   adding schema-2 packages to its live Catalog would break refresh/startup.
   Automatic restoration of only the old Controller profile would therefore
   not be a safe rollback. Production publication must wait for an accepted
   reader bridge or transactional Catalog/configuration cutover.
2. The actual systemd unit has **Product authentication enabled**, unlike the
   stale stable Columbus checkout. Its `ExecStartPre` regenerates private
   `authentication.json` from protected Cardea inputs and pins Cardea **1.1.0**.
   It has no Plugin-host-config hook. Hand-editing generated JSON would be
   reverted at restart. Preserve the current enabled authentication policy;
   change only the owning committed generator/host configuration through the
   separate authorized NixOS maintenance boundary. No temporary drop-in or
   stable checkout edit is acceptable. Fresh Columbus `origin/main` was
   `a8722843904e17462add70af0f9743d2d4815546` when inspected; refetch and
   integrate both live provenance floors before any future machine task.
3. Candidate `serve --check-plugin-hosts`, using the actual enabled-auth
   Service environment/arguments/cwd, passes only **bootstrap** with hostless
   Cardea 1.1.0 and required WebAuthn storage. This is not catalog-only,
   migration or real-login acceptance. Production release coverage correctly
   reports all six **3.1.13** versions unpublished.
4. Full native-shell provenance/acceptance, Codex DeepSeek predecessor drain,
   reader-compatible publication/rollback, and the separate host-policy
   activation authority remain unresolved. Controller restart authority alone
   does not authorize a NixOS policy activation or real Provider login.

At this checkpoint there have still been **no production signatures, Catalog
writes, host-policy changes, credential changes, Controller/Web/Machine
activations, real logins, or bootstrap deletions**. Health remains `ok` with
the pre-existing Controller and zero service restarts. The canonical release
skill now calls out live/predecessor Catalog-reader compatibility explicitly.
The disposable Mac artifact directory and transfer-only tar were removed after
all receipts were copied back; exact candidate packages and local receipts are
retained. The isolated Mac Git worktree remains available for continuation.
