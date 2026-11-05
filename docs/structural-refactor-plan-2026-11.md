# Structural refactor plan (November 2026)

Status: proposal. Baseline `origin/main` `c57a29455` (2026-11-05). Every slice
below is behaviour-preserving; a slice that needs a behaviour change is not a
refactor slice and must be filed as a feature or fix instead.

## 1. Question and verdict

Cowboy has received about 5,300 commits since 2026-05-27 and is now roughly
690k lines of Rust and TypeScript plus 74k lines of Markdown. The question
asked was whether that pace has turned the code into a big ball of mud.

Verdict: the trend is real but concentrated, and nothing requires a rewrite.

What is healthy:

- The gate is strict: `warnings = "deny"`, `unsafe_code`, `todo!` and `dbg!`
  denied, Clippy `all` on, cargo-deny and cargo-machete, oxlint and `tsc` on
  the web, around 3,170 Rust tests, 442 web test files and 42 browser
  conformance suites. Only 5 `TODO`/`FIXME` markers exist; 11 reverts in five
  months.
- Copy-paste duplication is low: 1.5% of significant lines repository-wide
  (10-line windows, tests excluded). Almost all of it is one known pair, the
  two persistence backends.
- The leaf crates and packages extracted so far (`cowboy-wire`,
  `cowboy-plugin-sdk`, `cowboy-provider-sdk`, `components/app-shell`,
  `state-sync`) have no measured duplication and clear dependency direction.

What is decaying (ranked in §3): the base crate has become one dependency
cycle; the three entry-point files grew into god files with god functions; the
persistence layer is implemented twice; the wire protocol is mirrored by hand
into TypeScript; several hand-bumped ledgers are the most-edited files in the
repository; the web app keeps its largest surfaces in four files; and
compatibility paths accumulate without exit conditions. Additions outpace
deletions four to one (Jun→Oct added lines 37.7k, 86k, 184k, 256k, 280k per
month against 12.4k, 26k, 44k, 32k, 43k deleted), and only about 35 commits in
five months were labelled refactors.

## 2. Measured baseline

Method: production lines exclude `#[cfg(test)]` modules and files whose name
contains `test`, `fixture` or `conformance`; function length is measured to the
indentation-matched closing brace, and the few outliers caused by brace
characters inside literals were checked by hand and dropped. Co-change counts
are commits since 2026-08-01 that touched both files (merges and commits with
more than 25 source files excluded). Churn counts are commits in the 60 days
before the baseline.

| Area | Measurement |
|---|---|
| Size | `src/` 185k lines, `components/` 213k, `web/src/` 263k, `plugins/` 19k (Rust + TS/TSX) |
| Base crate `src/` | 107 top-level modules, 135k production lines; one strongly connected component of **42 modules / 96k lines** (`acp`, `acp_bridge`, `admin`, `agent_api`, `cli`, `core`, `machine_control`, `managed_calls`, `oidc`, `plugin_catalog`, `provider`, `remote_runtime`, `store`, `supervisor`, `usage`, …); the dependency closure of `core`, `store`, `acp` or `supervisor` is 84 modules / 123k lines |
| Feature gates in the base crate | `cfg(feature = "full")` at 161 sites in 20 modules plus 59 in `lib.rs`; `plugin_runtime_args.rs` alone has 62, `cli.rs` 41 |
| `components/server/src/server.rs` | 28,342 lines: 21,369 production and 6,973 test lines in **30 inline test modules interleaved with production code**; `handle_machine_ws` 1,111 lines (10 nesting levels), `serve` 837, `handle_command` 682 (44 `Inbound` arms), `handle_ws` 508, `serve_axum` 480, `api_new_session` 397. 258 distinct co-change partners, the most of any file |
| `src/core.rs` | 13,077 lines; `impl Hub` spans 5,600 lines with 169 public methods |
| Persistence | `Store` exposes 264 public methods; **129 dispatch through `dispatch_storage!` to one Postgres body in `src/store.rs` and one SQLite body in `src/store/sqlite.rs`**; 300 identical 10-line windows between those two files; a further 111 `StorageBackend`/`dispatch_storage!` sites inside `store/{managed_calls,draft_documents,provider_accounts,…}.rs` |
| `components/machine/src` | `machine_plugins` module 20,770 production lines; module cycles `machine_plugins ↔ machine_code_plugins` and `machine_broker ↔ machine_cli` |
| Web | `App.tsx` 9,135 lines, 53 components, 165 imports, 202 hook calls: `SessionList` 2,530 lines, `App` 2,400, `SettingsShell` 1,087. `Composer.tsx` 8,277: `ComposerWorkspace` 2,522, `PendingRow` 1,322, `PendingPanel` 1,017. `Transcript.tsx` 6,236: `Transcript` 2,626, `ToolDetailsBrowser` 904. `store.ts` 6,107 lines, 62 exports. `mobile/review/ReviewApp.tsx` 4,423: `ReviewApp` 2,254, `DocumentView` 1,681 |
| Function sizes (repo) | 116 functions over 200 lines, 50 over 400, 17 over 800 |
| Protocol mirror | `web/src/protocol.ts` (585 lines) hand-mirrors `core::Inbound`/`Outbound`; co-changes since August: `server.rs ↔ protocol.ts` 28, `core.rs ↔ store.ts` 28, `server.rs ↔ App.tsx` 31, `acp_bridge.rs ↔ protocol.ts` 20 |
| Hand-bumped files (60-day commits) | `web/public/sw.js` **498** (manual `VERSION` increment), `docs/INDEX.md` 197, `flake.nix` 145, `plugins/claude-code/plugin.json` 140, `components/registry.json` 133 |
| Release ledger | `components/registry.json` is 39,518 lines / 1.2 MB, an append-only record of 101 releases growing about 480 lines per release, `include_str!`-ed into the binary (`src/plugin.rs:98`) and parsed on first use |
| Compatibility surface | 1,351 mentions of "legacy"; `migration_compat.rs`, `legacy_provider_release.rs`, `legacyRecordsNotice.ts`; 8 Nix release outputs (`cowboy-release`, `-web-`, `-controller-`, `-machine-`, `-machine-host-`, `-machine-writer-host-`, `-machine-bootstrap-`, `-machine-host-bootstrap-`); 193 `allow(dead_code | too_many_* | large_*)` sites; 11 documented deletion triggers |
| Commit mix | fix share of non-merge commits: 67% (Aug), 39% (Sep), 38% (Oct) |
| Documentation | 193 entries under `docs/`; `architecture/00-overview.md` and `06-server-api.md` still pointed at `src/server.rs`, `src/machine_*.rs` and `src/bin/cowboy-machine.rs`, which moved on 2026-10-22 (fixed with this plan); `PLUGINIZATION-HANDOFF.md` (2,000+ lines) still lives at the repository root |

## 3. Diagnosis

Ranked by the expected cost of leaving each alone.

### R1. The base crate is one dependency cycle

`cowboy` (`src/`) was meant to become the shared base after the 2026-10-22
split of `cowboy-wire`, `cowboy-server` and `cowboy-machine`, but 42 of its
modules reference each other in one cycle, so `core`, `store`, `acp`,
`supervisor` and `machine_control` all have the same 84-module closure.
Consequences: every edit to the base recompiles and retests the whole
controller and Machine host; nothing in the cycle can be unit-tested without
constructing the rest; and `cfg(feature = "full")` is used 220 times to carve
the Machine-only subset out of the same files, so one crate compiles as two
different programs. Edge evidence: `store` → `core` is 53 references — plain
types and constants (`SessionMeta` 14, `settings_keys` 8, `SessionOrigin` 4,
`Status` 3, hot-tail limits and SQL constants 9) plus the write-behind
contract (`Hub` 11, `StoreWrite`/`StoreSink` 2); `core` → `store` is 2;
`cli` fans out to 17 modules.

### R2. God files and god functions at the entry points

`server.rs` (21k production lines, 30 interleaved test modules),
`core.rs` (`impl Hub`, 169 public methods) and `store.rs` carry most of the
controller. Functions such as `handle_machine_ws` (1,111 lines, handshake +
connection install + frame loop + one arm per Machine event) and `serve` (837 lines,
nine startup phases ordered by comments) cannot be reviewed as units, and the
inline test modules mean a test-only change rebuilds the server crate and
shows up in the same hunks as production edits. `server.rs` has 258 co-change
partners: it is the file every feature touches, so it is where every merge
conflicts.

### R3. Persistence implemented twice

129 `Store` methods have a Postgres and a SQLite body that differ mainly in
placeholders (`$1` vs `?1`), timestamp types and lock strategy. The 300
identical windows show the bodies are copied, and the submodule-level
`StorageBackend` matches show the pattern is still spreading. A fix landed in
one backend is not automatically landed in the other, and the SQLite path is
the one most development runs exercise while Postgres is production.

### R4. The wire protocol is mirrored by hand

`protocol.ts` is written to "mirror src/core.rs"; the only mechanical check is
a discriminant test (`protocol_contract.rs` / `protocolContract.test.ts`) over
a handful of fixtures. The co-change data shows the mirror is maintained by
editing both sides in the same commit 20–31 times per quarter per pair.

### R5. Hand-bumped ledgers are the most-edited files

`sw.js` is edited in 498 of the last 60 days' commits because a human writes
the next integer into `const VERSION`; parallel worktrees conflict on exactly
that line. `components/registry.json` grows forever and is compiled into the
binary; `plugin.json`/`provider.json` version bumps and `docs/INDEX.md`
descriptions follow the same pattern. None of these edits carry design
intent, yet they dominate churn and conflicts.

### R6. The web keeps its largest surfaces in four files

`App.tsx`, `Composer.tsx`, `Transcript.tsx` and `store.ts` hold 30k lines with
single component functions of 2,400–2,600 lines and 130–200 hook calls per
file. `App.tsx` is additionally a 4-space formatting outlier that `dprint`
must skip (`AGENTS.md`), so it cannot be auto-formatted and every extraction
has to re-indent. The desktop/mobile split under `web/src/desktop`,
`web/src/mobile`, `web/src/explore` shows the intended feature-directory
layout; the four files predate it.

### R7. Compatibility paths without exit conditions

Legacy provider releases, migration compatibility, legacy browser records,
the schema-one deletion writer, bootstrap and writer-host release variants and
193 lint allowances all exist for a reason, but only 11 of them state when
they may be removed. Without an exit condition each one is permanent, and the
release matrix (8 outputs) is what new contributors must learn first.

### R8. Documentation drift

The architecture chapters describe `src/` paths that moved three weeks ago;
the plugin-refactor handoff and completion ledgers (2,000+ and 140+ commits)
are kept as living documents although the work they track has shipped.

## 4. Rules for every slice

1. **No behaviour change.** A slice moves, splits or deduplicates code; it does
   not change wire formats, ACP handling, SQL semantics, startup order, route
   classification, or UI behaviour. Where order matters (startup phases,
   route tables, message dispatch) the slice adds a golden test of the order
   before moving anything.
2. **Pure moves stay pure.** Git must detect the rename; `pub use` shims keep
   old paths compiling for one release when another crate depends on them.
   Moving a file also updates the Nix source closures (`cowboy-src`,
   `machine-src`, `code-adapter-src`, `worker-generation-files`,
   `retained-worker-interface-files` in `flake.nix`), `tools/remote_check_map.json`
   with `tools/remote_impact_test.py`, and documentation paths, in the same
   commit.
3. **Guard first, then move.** The size and layering ratchets in Phase 0 land
   before the first split so the problem cannot grow back while the split is
   in progress.
4. **Gate per slice.** `nix develop -c just check-compact` for everything;
   `just test-postgres` whenever `store` changes; `just feature-check` whenever
   a `cfg` boundary changes; `just idb-conformance-check` and the Desktop
   browser gate whenever a web shell file moves; `just desktop-perf-profile`
   before and after any Composer/Transcript extraction (web/AGENTS.md).
5. **Build the narrowest affected release output** (`.#cowboy-web-release`,
   `.#cowboy-controller-release`, `.#cowboy-machine-host-release`) for every
   slice that moves files, so fileset omissions fail before deployment.
6. **Separate commits for defects.** A bug found while refactoring is fixed in
   its own commit with a test, before or after the move, never inside it.
7. **Small slices.** Target under 2,000 moved lines per commit; a slice that
   needs more is split by submodule.
8. **Machine protocol and worker generation are frozen** unless a slice is
   explicitly scheduled as a Machine maintenance boundary (`AGENTS.md`
   deploy section). Phase 0 and Phase 1 require none.

## 5. Sequence

### Phase 0 — guards and friction removal

| Slice | Change | Invariant protected | Proof | Guard that remains |
|---|---|---|---|---|
| 0.1 Size ratchet | `tools/check-structure.ts` reads `tools/structure-baseline.json` (per-file max lines, per-function max lines, per-file hook count for TSX). New Rust file ≤ 1,500 lines, new TS/TSX file ≤ 800, new function ≤ 150 lines; existing offenders enter the baseline at their current size and the script fails when any number grows. `--update` may only lower a baseline. Wired into `just lint`. | none (adds a check) | script unit tests on fixtures; `just check` | the ratchet |
| 0.2 Service-worker version from the build | A Vite `generateBundle` plugin rewrites `const VERSION = "…"` in the emitted `sw.js` to `cowboy-<16 hex of sha256(sorted output file names + sw.js source)>`. Source keeps `cowboy-dev`. `mobileUpdateVersion.ts` already reads the string by regex; `brandAssets.test.ts` and `index.html`'s `manifest.webmanifest?v=` use the same derived value. | installed PWAs still see a new `VERSION` for every changed bundle and the same `VERSION` for an unchanged rebuild | web tests; build `.#cowboy-web-release` twice and diff `sw.js`; deploy a web release and confirm the foreground update fires | the manual step disappears from `web/AGENTS.md`; a test asserts the placeholder is never shipped |
| 0.3 Dead and test-only code | `src/migration_policy.rs` (631 lines of test constants compiled into production) becomes `#[cfg(test)]`; audit the 193 lint allowances and delete items that are dead under `--all-features`. | none | `just check`, `just feature-check` | `allow(dead_code)` count enters the ratchet baseline |
| 0.4 Documentation | Fix the moved paths (done with this plan); move `PLUGINIZATION-HANDOFF.md` and `docs/plugin-refactor-*.md` under `docs/history/` with links updated; add a check that every `docs/**/*.md` is listed in `docs/INDEX.md`. | none | `rg` for stale `src/server.rs`, `src/machine_*`; link check | index completeness check |
| 0.5 Module layering ratchet | `tools/check-module-layers.ts` extracts `crate::<module>` edges per crate, compares with `tools/module-edges-baseline.json`, and fails on any new edge or any new cycle. Baseline = today's edges. | none | script tests | prevents new back-edges while Phase 2 removes old ones |

Phase 0 needs no Machine release and no protocol change. 0.1, 0.3 and 0.5 are
each one commit; 0.2 is one web release.

### Phase 1 — decompose the server entry points (mechanical)

All slices are within `cowboy-server`; no public API or wire change.

| Slice | Change | Proof |
|---|---|---|
| 1.1 Tests out of `server.rs` | Move the 30 inline `mod *_tests` blocks into `components/server/src/server/tests/<name>.rs` (`#[cfg(test)] mod tests;`). Zero production diff. | `cargo test -p cowboy-server` count unchanged (698 lib tests at the last split) |
| 1.2 `handle_command` | Keep the `match Inbound` as a thin dispatcher; move the 44 arms into `server/commands/{sessions,queue,drafts,review,config,connection}.rs` grouped by `Inbound` family, one function per arm with the same signature. | existing command tests; a golden test listing every `Inbound` variant and its handler so an unhandled variant fails to compile |
| 1.3 `handle_machine_ws` | Split into `machine_ws/handshake.rs` (challenge, hello, signature, protocol negotiation), `machine_ws/install.rs` (writer tasks, lanes, runtime tunnel, control install), `machine_ws/frames.rs` (the receive loop) and `machine_ws/events/*.rs` (one function per `MachineEvent` arm). Effects keep their order; the loop keeps its single `select!`. | existing Machine control/binding/matrix tests; a connection-level test that records the ordered effects of a scripted handshake before and after |
| 1.4 `serve` | Extract the nine startup phases into `server/startup/*.rs` behind a `Startup` builder that records phase names; the final `serve` reads as the ordered list. | golden test on the recorded phase order; `/healthz` and `/version` smoke in `tools/agent_api_smoke.py` |
| 1.5 Routes | `serve_axum` route table and `classify_route` move to `server/routes.rs` as one table; a test asserts every registered route has a classification and vice versa. | route table test |
| 1.6 Session creation | `api_new_session` (397 lines) splits into validation, workspace preparation and worker start steps in `server/sessions/create.rs`. | session creation tests and the browser conformance suite |

Exit criterion: `server.rs` under 3,000 lines, no function in `cowboy-server`
over 200 lines, no inline test module in a production file.

### Phase 2 — break the base-crate cycle and finish the crate split

Order follows the dependency direction so each step compiles on its own.

| Slice | Change | Proof |
|---|---|---|
| 2.1 Edge inventory | Export the 42-module cycle's edge list from the Phase 0.5 tool; classify each back-edge as type-only, constant-only or behavioural. | report committed under `docs/history/` |
| 2.2 `cowboy-model` | New crate for plain types and constants shared by `core` and `store`: `SessionMeta`, `SessionOrigin`, `Inbound`/`Outbound`/`Envelope`, `settings_keys`, hot-tail limits, store row/record types. No `tokio`, no `sqlx`. `store` → `core` drops from 53 references to the write-behind contract (`Hub`, `StoreWrite`, `StoreSink`), which becomes a sink trait in `model` implemented by the Hub, so the store depends on the trait and never on `Hub`. | `just check`; the layering tool shows `store` no longer references `core` |
| 2.3 `cowboy-store` | `store`, `store/*`, `persistence`, `migrations/` and `migration_compat` become a crate depending on `model` only. | `just check`, `just test-postgres`; migrations unchanged byte-for-byte (`AGENTS.md`: baselines are checksummed) |
| 2.4 `cowboy-hub` | `core`, `supervisor`, `runtime_router`, `remote_runtime`, `scheduler`, `session_context`, `session_folders` move together (they are one cycle today). | hub tests; the layering tool shows `hub` → `store` → `model` only |
| 2.5 Owner-specific modules leave the base | `cli`, `product_cli`, `admin`, `oidc`, `passkey`, `product_auth`, `client_auth*`, `local_operator`, `agent_api`, `managed_calls`, `code_review`, `usage` move into `cowboy-server`; `machine_*`, `worker`, `execution_target_environment`, `plugin_runtime_args::machine` move into `cowboy-machine`; the remaining base is provider/plugin contracts and ACP. | `just feature-check`; `cfg(feature = "full")` sites in the base crate fall from 220 to 0 because feature selection becomes crate selection |
| 2.6 Machine-side cycles | `machine_plugins ↔ machine_code_plugins` and `machine_broker ↔ machine_cli` are cut the same way (shared types down, behaviour up). | layering tool |

Slice 2.5 changes which crate compiles the Machine binaries. It is the only
Phase 2 slice that must be scheduled as a Machine host release
(`.#cowboy-machine-host-release`) and must keep the worker generation files and
the retained-worker interface list exact; the memory of the October split
records the fileset and `remote_check_map.json` updates this needs.

Expected effect (measured after the October split): incremental `cargo check`
after a `store` edit fell from 34s to 25s and the Machine-host cold check from
40s to 29s when `wire` left the base; isolating `store`, `hub` and the
server-only modules removes the rest of the shared closure.

### Phase 3 — persistence: one body per method

| Slice | Change | Proof |
|---|---|---|
| 3.1 Differential store tests | A `#[store_test]` macro runs each store test against SQLite and, when `COWBOY_TEST_POSTGRES` is set, against a fresh Postgres database (`tools/test-postgres.sh` already provisions one). Existing tests are converted module by module. | the converted tests pass on both backends; divergences found become separate fix commits |
| 3.2 Domain layout | Each store domain moves to `store/<domain>/{mod.rs,postgres.rs,sqlite.rs}` with the row mapping (`From<Row>`) and validation shared in `mod.rs`. Still two SQL bodies, but adjacent and with one mapping. | 3.1 tests; duplication measured per directory |
| 3.3 Dialect templating (optional) | For methods whose bodies differ only in placeholders and timestamp expressions, a small `Dialect` helper renders one SQL template for both backends. Methods that differ in locking or JSON semantics stay explicit. | 3.1 tests; `EXPLAIN` parity is not required |

Slice 3.3 is optional: it is only worth doing for domains where 3.1 found
divergences, because the differential tests already remove the "fixed in one
backend only" failure mode.

### Phase 4 — web decomposition

| Slice | Change | Proof |
|---|---|---|
| 4.1 `App.tsx` | Extract `SessionList` (+ row memo, badges, spotlight) to `web/src/sessions/`, `CreateDialog` to `sessions/CreateDialog.tsx`, the settings components (lines 6,596–7,726) to `web/src/settings/`, `MachinesContent`/`ProvidersContent` to `web/src/machines/`, the delete/rename/info shells to `sessions/shells/`. `App.tsx` becomes the composition root. New files are 2-space and formatted by `dprint`, which shrinks the 4-space exception with every extraction. | `bun run typecheck`, web tests, `just idb-conformance-check`, Desktop browser gate |
| 4.2 `Composer.tsx` | `ComposerWorkspace`, `PendingPanel`/`PendingRow`, `ComposerSheet` and the `Session*Section` components move into the existing `web/src/composer/` directory. | same, plus `just desktop-perf-profile` before/after |
| 4.3 `Transcript.tsx` | `ToolDetailsBrowser` and the row renderers leave the `Transcript` component; the column-reverse scroll contract (web/AGENTS.md) stays in one file. | transcript conformance suites; perf profile |
| 4.4 `store.ts` | Slice by domain (connection, sessions, queue/drafts, calls, preferences) behind an unchanged `store.ts` re-export so imports do not move in the same commit. | browser conformance runners |
| 4.5 `ReviewApp.tsx` | `DocumentView` and the review repository/file tree already have their own files; `ReviewApp` keeps only routing. | mobile review conformance |

The desktop and mobile layout duplication is intentional (web/AGENTS.md) and
is not a target.

### Phase 5 — generated protocol types

| Slice | Change | Proof |
|---|---|---|
| 5.1 Generate | Derive JSON Schema for `Inbound`, `Outbound`, `Envelope` and the typed `Event` payloads from Rust (the composition contract already uses a generated, checked-in schema and TypeScript file via `tools/generate-composition-contract.ts`); generate `web/src/protocol.generated.ts`; `protocol.ts` re-exports the generated types and keeps only hand-written helpers. | `just composition-check`-style drift check fails when Rust changes without regeneration; existing `protocolContract` tests |
| 5.2 Retire the mirror comments | The "mirrors src/core.rs" contract becomes the generator. | co-change `server.rs ↔ protocol.ts` falls to zero |

### Phase 6 — compatibility exits and release surface

| Slice | Change | Proof |
|---|---|---|
| 6.1 Exit table | A table in `docs/compatibility-exits.md` listing every legacy path (`LegacyProviderRelease`, `migration_compat`, `legacyRecordsNotice`, schema-one writer outputs, bootstrap outputs, each lint allowance group) with its removal condition and the check that proves the condition. | reviewed table |
| 6.2 Release outputs | Once every fleet target passes the writer/bootstrap floors recorded in `AGENTS.md`, fold `-machine-writer-host-`, `-machine-bootstrap-` and `-machine-host-bootstrap-` into `-machine-host-release` and `-machine-release`. Target: 4 outputs. | fleet receipts; `nix flake check` |
| 6.3 Registry ledger | `components/registry.json` keeps schema 4: the active release plus every release still referenced by a `plugins/*/plugin.json` or the retained worker pin; older releases move to `components/registry-archive/<version>.json`. `src/plugin.rs` validation and `tools/check-plugin-components.ts`, `worker-registry-input.ts`, `plugin-component-closure.ts` read both. Because `worker-generation` hashes the registry digest, this is a Machine maintenance boundary. | tool tests; a Machine release with the worker pin unchanged |

## 6. Not worth doing

- A rewrite or a second controller. The gate, tests and deployment contracts
  are the asset; the plan keeps all of them.
- Replacing SQLx or using the `Any` driver to collapse the two backends:
  Postgres JSONB and advisory locks are used deliberately; 3.1 removes the
  real risk at a fraction of the cost.
- Merging desktop and mobile UI code (intentional duplication per
  web/AGENTS.md).
- Reformatting `App.tsx` wholesale (3,700-line diff with no structural gain);
  extraction shrinks it instead.
- A repository-wide Clippy `pedantic` sweep as its own project; enable
  `pedantic` per crate as each crate leaves the base (new crates start with
  it on).
- Splitting `machine_plugins` before Phase 2.5; it depends on where the base
  crate boundary lands.
- Converting the hand-written `docs/INDEX.md` descriptions to generated text;
  the completeness check in 0.4 is enough.

## 7. Health metrics and quarterly targets

Reported by `tools/check-structure.ts --report` so the numbers are reproducible.

| Metric | Baseline (2026-11-05) | Target (end of Q1 2027) |
|---|---|---|
| Largest module cycle in the base crate | 42 modules / 96k lines | ≤ 6 modules |
| `cfg(feature = "full")` sites in `src/` | 220 | 0 |
| Files over 3,000 lines | 21 | 0 |
| Functions over 200 lines | 116 | ≤ 20, all in the baseline allow-list |
| `server.rs` production lines | 21,369 | < 3,000 |
| Store methods with two bodies | 129 | ≤ 60, all covered by differential tests |
| `server.rs ↔ protocol.ts` co-changes per quarter | 28 | 0 |
| `sw.js` commits per 60 days | 498 | 0 |
| `components/registry.json` size | 1.2 MB, 101 releases | ≤ 100 KB |
| Deleted/added lines per month | 12–25% | ≥ 50% during refactor quarters |
| Fix share of commits | 38% | trend down; reported, not targeted |
| Lint allowances (`allow(dead_code|too_many_*|large_*)`) | 193 | ≤ 60 |
| Incremental `cargo check` after a `store` edit | 25s | ≤ 12s |

## 8. Defects noticed during this review

Fixed with this plan:

- `docs/architecture/00-overview.md` and `06-server-api.md` referenced
  `src/server.rs`, `src/machine_*.rs` and `src/bin/cowboy-machine.rs`, all
  moved on 2026-10-22.

Queued (own commits, not refactor slices):

- `src/migration_policy.rs` is a `pub mod` compiled into every production
  binary although it contains only test constants and tests (slice 0.3).
- The 193 lint allowances have not been audited since the split; several
  `#[cfg_attr(not(test), allow(dead_code))]` row types in `store.rs` suggest
  production code that only tests read (slice 0.3).

## Appendix: how the numbers were produced

- Lines: `rg --files -g '*.rs' -g '*.ts' -g '*.tsx' <dir> | xargs wc -l`.
- Module graph: for each top-level module under a crate's `src/`, references
  of the form `crate::<module>` in production code form the edges; Tarjan's
  algorithm finds the strongly connected components; closure is the
  transitive edge set.
- Function sizes: regex on `fn`/`function`/arrow-component declarations,
  length to the first closing brace at the same indentation.
- Duplication: blake2b hash of every window of 10 consecutive normalised
  significant lines (whitespace collapsed; imports, comments, attributes and
  lines under 12 characters skipped); a window hash seen twice marks its lines
  duplicated.
- Co-change: `git log --since=2026-08-01 --no-merges --name-only`, pairs of
  `.rs`/`.ts`/`.tsx` files per commit, commits with more than 25 source files
  skipped.
- Churn: `git log --since=60.days --name-only | sort | uniq -c`.
