# Structural refactor plan (November 2026)

Status: proposal. Baseline `origin/main` `c57a29455` (2026-11-05). Every slice
below is behaviour-preserving; a slice that needs a behaviour change is not a
refactor slice and must be filed as a feature or fix instead. The measurements
were taken by the author and cross-checked by an independent Astra Max
analysis of the same tree on 2026-11-05; an Astra Max design review of this
document followed (§9) and its findings are folded in.

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
- Lifecycle contracts (reboot convergence, retained workers, write-behind
  persistence, rolling updates) are documented and tested; they are the asset
  every slice below must keep.

What is decaying (ranked in §3): the base crate has become one dependency
cycle; the entry-point files grew into god files with god functions; the
persistence layer is implemented twice; the wire protocol and the release
source closures are mirrored by hand (and the Machine release build was broken
on `main` because of it); several hand-bumped ledgers are the most-edited files
in the repository; the web app keeps its largest surfaces in four files and one
coordinating store; the Hub owns too many interacting lifetimes; compatibility
paths accumulate without exit conditions; and a class of tests asserts on
source text rather than behaviour. Additions outpace deletions four to one
(Jun→Oct added lines 37.7k, 86k, 184k, 256k, 280k per month against 12.4k,
26k, 44k, 32k, 43k deleted), and only about 35 commits in five months were
labelled refactors.

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
| Base crate `src/` | 107 top-level modules, 135k production lines; one strongly connected component of **37 modules** in production code (`acp_bridge`, `admin`, `agent_api`, `cli`, `core`, `machine_control`, `managed_calls`, `oidc`, `plugin_catalog`, `store`, `usage`, …; the first textual scan reported 42 because it also counted rustdoc links); the dependency closure of `core`, `store`, `acp` or `supervisor` is 84 modules / 123k lines |
| Feature gates in the base crate | `cfg(feature = "full")` at 161 sites in 20 modules plus 59 in `lib.rs`; `plugin_runtime_args.rs` alone has 62, `cli.rs` 41 |
| Crate API discipline | `components/server/src/lib.rs:6` and `components/machine/src/lib.rs:6` re-export the whole base crate (`pub use cowboy::*`); `src/controller_hooks.rs` installs process-global reverse callbacks so base code can call back into the server |
| `components/server/src/server.rs` | 28,342 lines: 21,369 production and 6,973 test lines in **30 inline test modules interleaved with production code**; `handle_machine_ws` 1,111 lines (10 nesting levels), `serve` 837, `handle_command` 682 (44 `Inbound` arms), `handle_ws` 508, `serve_axum` 480, `api_new_session` 397. 258 distinct co-change partners, the most of any file |
| `src/core.rs` | 13,077 lines; `impl Hub` spans 5,600 lines with 169 public methods covering sessions, configuration reconciliation, provider updates, history reduction, background tasks, folders, fan-out and write-behind dispatch; several lock-order requirements live only in comments |
| Persistence | `Store` exposes 264 public methods; **129 dispatch through `dispatch_storage!` to one Postgres body in `src/store.rs` and one SQLite body in `src/store/sqlite.rs`**; 300 identical 10-line windows between those two files; a further 111 `StorageBackend`/`dispatch_storage!` sites inside `store/{managed_calls,draft_documents,provider_accounts,…}.rs` |
| `components/machine/src` | `machine_plugins` module 20,770 production lines; `machine_broker::spawn_worker` 559 lines, `machine_cli::handle_machine_command` 881; module cycles `machine_plugins ↔ machine_code_plugins` and `machine_broker ↔ machine_cli` |
| Web | `App.tsx` 9,183 lines, 53 components, 165 imports, 202 hook calls: `SessionList` 2,530 lines, `App` 2,400, `SettingsShell` 1,087. `Composer.tsx` 8,277: `ComposerWorkspace` 2,522, `PendingRow` 1,322, `PendingPanel` 1,017. `Transcript.tsx` 6,236: `Transcript` 2,626, `ToolDetailsBrowser` 904. `store.ts` 6,107 lines, 62 exports, owning canonical and presented state, sockets, authentication pauses, hydration, caches, optimistic delivery and watchdogs. `mobile/review/ReviewApp.tsx` 4,423: `ReviewApp` 2,254, `DocumentView` 1,681 |
| Function sizes (repo) | 116 functions over 200 lines, 50 over 400, 17 over 800 |
| Protocol mirror | `web/src/protocol.ts` (585 lines) hand-mirrors `core::Inbound`/`Outbound`; the only mechanical check is a discriminant test over a handful of fixtures; `JSON.parse(...) as Outbound` performs no runtime validation. Co-changes since August: `server.rs ↔ protocol.ts` 28, `core.rs ↔ store.ts` 28, `server.rs ↔ App.tsx` 31, `acp_bridge.rs ↔ protocol.ts` 20 |
| Release source closures | `flake.nix` enumerates the files of `machine-src` and `code-adapter-src` by hand (about 70 explicit paths) and composes release variants from three booleans; a full-checkout `cargo` success does not imply a buildable release. **On the baseline, `nix build .#cowboy-machine` fails with `E0583` because `src/lib.rs:239-240` declared `task_notifications` and `web_push` unconditionally and neither closure carries those files** (since the 2026-10-22 split; the last Machine release, `af27fbec`, predates it) |
| Hand-bumped files (60-day commits) | `web/public/sw.js` **498** (manual `VERSION` increment), `docs/INDEX.md` 197, `flake.nix` 145, `plugins/claude-code/plugin.json` 140, `components/registry.json` 133 |
| Release ledger | `components/registry.json` (schema 3) is 39,518 lines / 1.2 MB, an append-only record of 101 releases growing about 480 lines per release, `include_str!`-ed into the binary (`src/plugin.rs:98`) and parsed on first use |
| Compatibility surface | 1,351 mentions of "legacy"; `migration_compat.rs`, `legacy_provider_release.rs`, `legacyRecordsNotice.ts`, tombstones in `core.rs`, identity-based provider fallback in `remote_runtime.rs`; 8 Nix release outputs (`cowboy-release`, `-web-`, `-controller-`, `-machine-`, `-machine-host-`, `-machine-writer-host-`, `-machine-bootstrap-`, `-machine-host-bootstrap-`) that encode distinct capabilities (compile-time deletion writer, retained worker runtime, bootstrap without `--desired-generation`); 193 `allow(dead_code | too_many_* | large_*)` sites; 11 documented deletion triggers |
| Tests bound to source text | at least 28 web test files read component source (`Composer.tsx`, `App.tsx`, …) and assert on substrings or positions, e.g. `web/src/composer/mobileEditorFocusContract.test.ts` |
| Commit mix | fix share of non-merge commits: 67% (Aug), 39% (Sep), 38% (Oct); lifetime non-merge commits touching `App.tsx` 640, `Composer.tsx` 563, `server.rs` 451 |
| Documentation | 193 entries under `docs/`; `architecture/00-overview.md` and `06-server-api.md` still pointed at `src/server.rs`, `src/machine_*.rs` and `src/bin/cowboy-machine.rs`, which moved on 2026-10-22 (fixed with this plan); `docs/history/pluginization-handoff.md` (2,000+ lines) still lives at the repository root |

## 3. Diagnosis

Ranked by the expected cost of leaving each alone.

### R1. The base crate is one dependency cycle with weak API discipline

`cowboy` (`src/`) was meant to become the shared base after the 2026-10-22
split of `cowboy-wire`, `cowboy-server` and `cowboy-machine`, but 42 of its
modules reference each other in one cycle, so `core`, `store`, `acp`,
`supervisor` and `machine_control` all have the same 84-module closure.
The server and Machine crates re-export the whole base (`pub use cowboy::*`)
and the base calls back into the server through process-global hooks, so the
Cargo boundary reduces rebuilds but does not constrain what depends on what.
Consequences: every edit to the base recompiles and retests the whole
controller and Machine host; nothing in the cycle can be unit-tested without
constructing the rest; `cfg(feature = "full")` is used 220 times to carve the
Machine-only subset out of the same files, so one crate compiles as two
different programs. Edge evidence: `store` → `core` is 53 references — plain
types and constants (`SessionMeta` 14, `settings_keys` 8, `SessionOrigin` 4,
`Status` 3, hot-tail limits and SQL constants 9) plus the write-behind
contract (`Hub` 11, `StoreWrite`/`StoreSink` 2); `core` → `store` is 2;
`cli` fans out to 17 modules. Behavioural back-edges exist too: `store`
calls `core_passkeys`, `auth_plugins` and `product_auth` (namespace
attachment in `store/core_security.rs`), `supervisor` imports ACP and
provider behaviour, `code_adapter.rs` needs `code_review`, and Machine launch
paths need `managed_calls::host`.

### R2. God files and god functions at the entry points

`server.rs` (21k production lines, 30 interleaved test modules),
`core.rs` (`impl Hub`, 169 public methods) and `store.rs` carry most of the
controller; `machine_cli::handle_machine_command` (881 lines) and
`machine_broker::spawn_worker` (559) carry the Machine. Functions such as
`handle_machine_ws` (1,111 lines: handshake, connection install, frame loop and
one arm per Machine event) and `serve` (837 lines, nine startup phases ordered
by comments) cannot be reviewed as units; the inline test modules mean a
test-only change rebuilds the server crate and lands in the same hunks as
production edits. `server.rs` has 258 co-change partners: it is the file every
feature touches, so it is where every merge conflicts.

### R3. Persistence implemented twice

129 `Store` methods have a Postgres and a SQLite body that differ mainly in
placeholders (`$1` vs `?1`), timestamp types and lock strategy (compare the
transactional event writes around `store.rs:6020` and `sqlite.rs:5059`). The
300 identical windows show the bodies are copied, and the submodule-level
`StorageBackend` matches show the pattern is still spreading. A fix landed in
one backend is not automatically landed in the other, and the SQLite path is
the one most development runs exercise while Postgres is production.

### R4. Two representations maintained by hand: wire types and release inputs

`protocol.ts` is written to "mirror src/core.rs"; the co-change data shows the
mirror is maintained by editing both sides in the same commit 20–31 times per
quarter per pair, and matching variant names can hide field, nullability and
serialization drift. The release side has the same shape: `flake.nix` lists
the Machine and code-adapter source files by hand, and the ordinary gate never
compiles those filtered trees. That is how `nix build .#cowboy-machine` came
to fail on `main` for two weeks without any test noticing.

### R5. Hand-bumped ledgers are the most-edited files

`sw.js` is edited in 498 of the last 60 days' commits because a human writes
the next integer into `const VERSION`; parallel worktrees conflict on exactly
that line. `components/registry.json` grows forever and is compiled into the
binary; `plugin.json`/`provider.json` version bumps and `docs/INDEX.md`
descriptions follow the same pattern. None of these edits carry design
intent, yet they dominate churn and conflicts.

### R6. The web keeps its largest surfaces in four files and one coordinator

`App.tsx`, `Composer.tsx`, `Transcript.tsx` and `ReviewApp.tsx` hold 28k lines
with single component functions of 2,250–2,650 lines and 130–200 hook calls
per file, so unrelated UI changes share effect lifetimes, refs and render
state, and an extraction can change focus, mounting, scroll or gesture timing
by accident. `store.ts` is an application coordinator (transport, persistence
and rendering timing in one module with module-global state), which makes
isolated tests and deterministic teardown hard. `App.tsx` is additionally a
4-space formatting outlier that `dprint` must skip (`AGENTS.md`). The
desktop/mobile split under `web/src/desktop`, `web/src/mobile`,
`web/src/explore` shows the intended feature-directory layout; the four files
predate it.

### R7. The Hub owns too many interacting lifetimes

`impl Hub` combines sessions, configuration reconciliation, provider updates,
history reduction, background tasks, folders, fan-out and write-behind
dispatch behind one lock, with ordering requirements recorded in comments.
Local changes can break recovery or delivery elsewhere, and splitting the file
alone would not change that: the fix is one owner per effect, which is why
Phase 4 exists separately from the mechanical splits.

### R8. Compatibility paths without exit conditions

Legacy provider releases, migration compatibility, legacy browser records,
Hub tombstones, identity-based provider fallback, the schema-one deletion
writer, bootstrap and writer-host release variants and 193 lint allowances all
exist for a reason, but only 11 of them state when they may be removed.
Without an exit condition each one is permanent, removing an apparently
obsolete branch can break retained workers or cached clients, and the release
matrix (8 outputs) is what new contributors must learn first.

### R9. Documentation drift

The architecture chapters described `src/` paths that moved three weeks ago;
the plugin-refactor handoff and completion ledgers (2,000+ lines and 140+
commits) are kept as living documents although the work they track has
shipped.

### R10. Tests bound to implementation text

At least 28 web test files read component source and assert on substrings or
source positions. A safe move fails them for reasons unrelated to behaviour,
while a preserved string can pass without the code ever running. They stand
exactly in the way of the extractions in Phase 3 and must be converted first.

## 4. Rules for every slice

1. **No behaviour change.** A slice moves, splits or deduplicates code; it does
   not change wire formats, ACP handling, SQL semantics, startup order, route
   classification, lock order or UI behaviour. Where order matters (startup
   phases, route tables, message dispatch, middleware) the slice adds a test
   of the observable effects in order — not of labels — before moving
   anything.
2. **Pure moves stay pure.** Git must detect the rename; `pub use` shims keep
   old paths compiling for one release when another crate depends on them.
   Moving a file also updates the Nix source closures (`cowboy-src`,
   `machine-src`, `code-adapter-src`, `worker-generation-files`,
   `retained-worker-interface-files` in `flake.nix`), `tools/remote_check_map.json`
   with `tools/remote_impact_test.py`, the explicit package lists in
   `justfile` (`test`, `feature-check`, `tools/test-postgres.sh`), and
   documentation paths, in the same commit.
3. **Guard first, then move.** The ratchets and the filtered-source build check
   in Phase 0 land before the first split so the problem cannot grow back
   while the split is in progress.
4. **Narrow what an extracted module may see.** Each extracted server or
   Machine module gets an explicit import allowlist (no blanket `AppState` or
   `pub use cowboy::*` access); the layering check enforces it.
5. **Behaviour tests before moving the file they read.** A test that reads
   component source text is replaced by an equivalent behavioural test in the
   slice before the extraction, never weakened or deleted.
6. **Gate per slice.** `nix develop -c just check-compact` for everything;
   `just test-postgres` whenever `store` changes; `just feature-check` and the
   filtered-source build whenever a `cfg` or `lib.rs` boundary changes; the
   browser suites named in the slice's execution matrix (Phase 3) whenever a
   web shell file moves, in both pinned browsers and both themes, with a
   Desktop gate exit 3 treated as missing acceptance rather than a pass;
   `just desktop-perf-profile` before and after any Composer/Transcript
   extraction (web/AGENTS.md); physical iOS acceptance for mobile composer and
   gesture surfaces.
7. **Build the narrowest affected release output** (`.#cowboy-web-release`,
   `.#cowboy-controller-release`, `.#cowboy-machine-host-release`) for every
   slice that moves files, so fileset omissions fail before deployment.
8. **Separate commits for defects.** A bug found while refactoring is fixed in
   its own commit with a test, before or after the move, never inside it.
9. **Small slices.** Target under 2,000 moved lines per commit; a slice that
   needs more is split by submodule. A pure move carries its size allowance
   with the moved symbols (§5, slice 0.1) instead of waiving the guard.
10. **Machine protocol, worker generation, retained-worker interface files and
    applied migrations are frozen** unless a slice is explicitly scheduled as
    a Machine maintenance boundary (`AGENTS.md` deploy section). Any change to
    `Cargo.toml` or `Cargo.lock` is such a boundary, because
    `retained-worker-interface-files` compares them byte for byte and
    `worker-generation-files` hashes them. Phases 0–4 require none; Phase 5
    is one.

## 5. Sequence

The order is: evidence and guards, then the self-contained mechanical splits
(server, store, web leaves), then ownership separation, then crate
boundaries, then generated contracts and compatibility exits. The independent
analysis argued, and the author agrees, that extracting crates before the
modules inside them stop sharing broad state would produce boundaries that
look clean in `Cargo.toml` but constrain nothing; the crate split therefore
follows the seams instead of leading them.

### Phase 0 — guards, boundary evidence and friction removal

| Slice | Change | Invariant protected | Proof | Guard that remains |
|---|---|---|---|---|
| 0.1 Size ratchet | `tools/check-structure.ts` reads `tools/structure-baseline.json` (per-file max lines, per-function max lines, per-file hook count for TSX). New Rust file ≤ 1,500 lines, new TS/TSX file ≤ 800, new function ≤ 150 lines; existing offenders enter the baseline at their current size and the script fails when any number grows. `--update` may only lower a baseline. A pure move transfers the allowance of the moved symbols to the new file (`--transfer <from> <to> <symbol>` records the move in the baseline) while the aggregate limit per crate/package only decreases, so `SessionList` can leave `App.tsx` unchanged without a waiver. Wired into `just lint`. | none (adds a check) | script unit tests on fixtures including a transfer | the ratchet |
| 0.2 Filtered-source build check | Expose `machine-src` and `code-adapter-src` as flake outputs; a `just release-sources-check` recipe materialises them and runs `cargo check` with the matching features inside each tree; `just check` depends on it. | every release closure compiles from the files it actually ships | the recipe fails on the baseline and passes after the `lib.rs` gate fix | the check |
| 0.3 Service-worker version from the build | Replace the manual `const VERSION = "cowboy-vNN"` with a build-derived identity, but keep an ordered generation: `sw.js:58–64,156–164,182–183` discover and clean caches by parsing `cowboy-v<number>` and need the previous generation for offline fallback and rollback. The design is therefore: (a) a Vite `generateBundle` plugin computes `cowboy-<16 hex>` from a deterministic manifest of **final file contents** (every emitted file, including stable-name `index.html`, `device-proof.js`, manifest and public assets; the substituted `sw.js` hashed with its placeholder); (b) `sw.js` keeps a small `cowboy-generations` index cache recording install order, used for cleanup and previous-shell discovery instead of numeric parsing, with a one-time migration from existing `cowboy-v<number>` caches. Source keeps the `cowboy-dev` placeholder; `mobileUpdateVersion.ts` already reads the string by regex; `brandAssets.test.ts` and `index.html`'s `manifest.webmanifest?v=` use the derived value. | installed PWAs see a new `VERSION` for every changed release (including stable-name-only changes) and the same one for an identical rebuild; previous shell stays discoverable; cleanup keeps working across hashed generations | web tests for the generation index and migration; `serviceWorkerShell` tests for multi-release offline fallback, lazy-chunk handoff and rollback; build `.#cowboy-web-release` twice and diff `sw.js`; change only `index.html` and confirm the digest changes; one staged web release confirming the foreground update fires | the manual step disappears from `web/AGENTS.md`; a test asserts the placeholder is never shipped. This slice needs a short design note before implementation and is one web release, not one commit |
| 0.4 Protocol fixtures across languages | Rust serialises one fixture per `Outbound`/`Event` family into `contracts/protocol/*.json`; a generator turns them into `web/src/protocol.fixtures.generated.ts` with literal object types checked by `satisfies Outbound`/`satisfies Event` under a dedicated `tsc -p web/tsconfig.contracts.json` step in `just check` (`web/tsconfig.json:44` excludes `*.test.ts` and `bun test` does not type-check, and `JSON.parse` yields `any`, so a test file alone proves nothing). Fixtures cover present, absent and `null` optional fields. One message family per commit. | existing wire bytes stay accepted | both suites; deliberately changing a required field or nullability in Rust makes the contract step fail | drift in a field, nullability or tag fails the gate |
| 0.5 Module layering ratchet | `tools/check-module-layers` resolves imports syntax-aware (a small `syn`-based Rust binary under `tools/`, or `cargo modules` where it suffices): grouped imports, aliases, `super::*` (already used by `store/sqlite.rs`), and cross-crate re-exports all produce edges; graphs are kept per feature set. It compares with `tools/module-edges-baseline.json` plus the per-module import allowlists of Rule 4, and fails on any new edge or cycle. Baseline = today's edges. | none | fixture tests for each import form | prevents new back-edges while later phases remove old ones; cannot be bypassed by changing import syntax |
| 0.6 Dead and test-only code | `src/migration_policy.rs` already compiles only under `cfg(test)`; audit the 193 lint allowances and delete items that are dead under `--all-features`. | none | `just check`, `just feature-check` | `allow(dead_code)` count enters the ratchet baseline |
| 0.7 Documentation | Fix the moved paths (done with this plan); move `docs/history/pluginization-handoff.md` and `docs/plugin-refactor-*.md` under `docs/history/` with links updated; add a check that every `docs/**/*.md` is listed in `docs/INDEX.md`. | none | `rg` for stale `src/server.rs`, `src/machine_*`; link check | index completeness check |

Phase 0 needs no Machine release and no protocol change. 0.1, 0.2, 0.5 and
0.6 are each one commit; 0.3 is one web release preceded by a design note;
0.4 is a handful of small commits.

### Phase 1 — decompose the server entry points (mechanical)

All slices are within `cowboy-server`; no public API or wire change. Each
extracted module declares what it imports (Rule 4), starting from the exact
set it uses today.

| Slice | Change | Proof |
|---|---|---|
| 1.1 Tests out of `server.rs` | Move the 30 inline `mod *_tests` blocks into `components/server/src/server/tests/<name>.rs` (`#[cfg(test)] mod tests;`). Zero production diff. | `cargo test -p cowboy-server` count unchanged (698 lib tests at the October split) |
| 1.2 Persistence writer and notifications | The write-behind consumer, incident classification and notification fan-out leave `server.rs` first; they have the fewest callers. | writer and notification contracts |
| 1.3 `handle_command` | Keep the `match Inbound` as a thin dispatcher; move the 44 arms into `server/commands/{sessions,queue,drafts,review,config,connection}.rs` grouped by `Inbound` family, one function per arm with the same signature. | existing command tests; a test listing every `Inbound` variant and its handler so an unhandled variant fails to compile |
| 1.4 Authentication and router assembly | `serve_axum`, the route table, `classify_route` and the product-auth router move to `server/routes.rs` as one table; a test asserts every registered route has a classification and vice versa, and that middleware order is unchanged. | route table test |
| 1.5 `handle_machine_ws` | Split into `machine_ws/handshake.rs` (challenge, hello, signature, protocol negotiation), `machine_ws/install.rs` (writer tasks, lanes, runtime tunnel, control install), `machine_ws/frames.rs` (the receive loop) and `machine_ws/events/*.rs` (one function per `MachineEvent` arm). Effects keep their order; the loop keeps its single `select!`. | existing Machine control/binding/matrix tests; a connection-level test that records the ordered effects of a scripted handshake before and after |
| 1.6 `serve` | Extract the nine startup phases into `server/startup/*.rs` behind a `Startup` builder; the final `serve` reads as the ordered list, including shutdown flushing. The ordering contracts at `server.rs:1322–1326` (restored admission fences before spawned work), `1347–1349` and `1993–2022` (persistence closes only after dispatcher requeue) are protected by an isolated Controller lifecycle test that starts the server with an in-memory store, observes completed startup dependencies, interrupts a dispatch and proves it survives shutdown and reopen — a golden list of phase names is not sufficient. | the lifecycle test; an HTTP smoke that requests `/healthz` and `/version` from the started instance (added to `tools/`; `agent_api_smoke.py` exercises MCP only) |
| 1.7 Session creation | `api_new_session` (397 lines) splits into validation, workspace preparation and worker start steps in `server/sessions/create.rs`. | session creation tests and the browser conformance suite |

Exit criterion: `server.rs` under 3,000 lines, no function in `cowboy-server`
over 200 lines, no inline test module in a production file, every extracted
module on an import allowlist.

### Phase 2 — persistence: one domain per directory, both backends tested

| Slice | Change | Proof |
|---|---|---|
| 2.1 Differential store tests | A `#[store_test]` macro expands each store test into a SQLite test and an `#[ignore]`d `postgres_*` sibling that `tools/test-postgres.sh` already discovers by name and runs against its private cluster through `COWBOY_TEST_POSTGRES_URL`; the Postgres sibling fails (never silently passes) when the URL is absent while it is run explicitly. Existing tests are converted module by module; a count assertion in the runner proves both backends executed the same number of cases. | the converted tests pass on both backends; divergences found become separate fix commits |
| 2.2 Domain layout | Each store domain moves to `store/<domain>/{mod.rs,postgres.rs,sqlite.rs}` with the row mapping (`From<Row>`) and validation shared in `mod.rs`; settings first, event persistence second. SQL text does not change during a move. | 2.1 tests; transaction scope, watermarks, NUL handling and corrupt-row tolerance covered by the matrix; duplication measured per directory |
| 2.3 Dialect templating (optional) | For methods whose bodies differ only in placeholders and timestamp expressions, a small `Dialect` helper renders one SQL template for both backends. Methods that differ in locking or JSON semantics stay explicit. | 2.1 tests |

Applied migrations stay byte-identical throughout (`AGENTS.md`: baselines are
checksummed). Slice 2.3 is optional: it is only worth doing for domains where
2.1 found divergences, because the differential tests already remove the
"fixed in one backend only" failure mode.

### Phase 3 — web decomposition, leaves first

`just idb-conformance-check` only formats and type-checks the runner files
(`justfile:177–179`); the suites themselves run through
`bun tools/idb-browser-conformance.ts <browser> <suite> <theme>` and the
Desktop browser gate (`desktop-composer`, `draft-documents`, `session-fold`,
`sheet-keyboard`). Each slice therefore names the suites it executes:

| Slice | Change | Suites executed (both pinned browsers, both themes) |
|---|---|---|
| 3.1 Behaviour tests replace source-text tests | For each of the 28 source-reading test files, add the equivalent behavioural test (render, focus, geometry) and delete the substring assertion, before touching the file it reads. | both old and new tests pass on the unchanged tree, then the old one is removed |
| 3.2 Settings and tool details | Settings panels (`App.tsx` 6,596–7,726) move to `web/src/settings/`; `ToolDetailsBrowser` leaves `Transcript.tsx`. Keys, DOM structure, refs and effect dependencies unchanged. | settings-recovery, provider-management, tool-details; Desktop gate |
| 3.3 `App.tsx` | Extract `SessionList` (+ row memo, badges, spotlight) to `web/src/sessions/`, `CreateDialog`, `MachinesContent`/`ProvidersContent` to `web/src/machines/`, the delete/rename/info shells to `sessions/shells/`. `App.tsx` becomes the composition root. New files are 2-space and formatted by `dprint`, which shrinks the 4-space exception with every extraction. | session-fold, session-context, draft-documents, IndexedDB suites; Desktop gate; `just desktop-perf-profile` before/after |
| 3.4 `Composer.tsx` | `ComposerWorkspace`, `PendingPanel`/`PendingRow`, `ComposerSheet` and the `Session*Section` components move into the existing `web/src/composer/` directory. | desktop-composer, sheet-keyboard, managed-calls; Desktop gate; perf profile; physical iOS composer acceptance (web/AGENTS.md pitfall #69 stays open) |
| 3.5 `Transcript.tsx` and `ReviewApp.tsx` | Row renderers leave the `Transcript` component while the column-reverse scroll contract stays in one file; `DocumentView` leaves `ReviewApp`, which keeps only routing. | transcript and review-context suites, mobile review conformance; perf profile; swipe jank check (`docs/mobile-spatial-presentation.md`) |

The desktop and mobile layout duplication is intentional (web/AGENTS.md) and
is not a target.

### Phase 4 — one owner per effect

| Slice | Change | Proof |
|---|---|---|
| 4.1 `store.ts` | Extract connection/liveness, transcript cache and delivery coordination into separate modules with injected clock and transport; `store.ts` keeps the `useSyncExternalStore` surface and re-exports so imports do not move in the same commit. | deterministic reconnect, timeout, replay and restart traces; browser conformance runners |
| 4.2 Hub queue policy vs. effects | Separate the queue/drain decision from the dispatch, persistence and fan-out effects without changing lock order; each effect gets one owner and a test that asserts it. The 169-method `impl Hub` is split by concern (`core/queue.rs`, `core/fanout.rs`, `core/restore.rs`, …) only after the ownership is explicit. | ordering, acknowledgement and duplicate-suppression tests; `tokio::test` traces with an injected clock |
| 4.3 Machine procedures | `handle_machine_command` and `spawn_worker` split around the existing lifecycle boundaries (launch preparation, command families) while cancellation and lease ownership stay where they are. | standalone-host tests, isolated lifecycle conformance; account isolation, generation identity, worker survival and reboot convergence remain covered |

### Phase 5 — crate boundaries that follow the seams (a Machine maintenance boundary)

Every slice that adds or moves a crate changes `Cargo.toml` and `Cargo.lock`,
which `flake.nix:677–688` compares byte for byte with the retained worker
source and which `worker-generation-files` hashes. Phase 5 is therefore
scheduled as one explicit worker-generation maintenance with its own
acceptance, not as a series of host releases; the interface check is never
weakened and the pin is never advanced silently to make a move deployable.
Moving `worker` itself changes the hashed worker inputs as well.

| Slice | Change | Proof |
|---|---|---|
| 5.1 Edge inventory and behaviour ports | Export the 37-module cycle's edge list from the Phase 0.5 tool; classify each back-edge as type-only, constant-only or behavioural. The behavioural ones known today — `store` → `core_passkeys`/`auth_plugins`/`product_auth`, `supervisor` → ACP and provider behaviour, `code_adapter` → `code_review`, Machine launch → `managed_calls::host` — each get a port (trait or shared contract module) specified before 5.4–5.6, so no slice recreates a cycle or breaks a standalone package. | report committed under `docs/history/`; ports reviewed |
| 5.2 Replace wildcard re-exports | `pub use cowboy::*` in the server and Machine crates becomes explicit `pub use cowboy::{…}` lists, one module family per commit; `controller_hooks` shrinks to the entry points that genuinely need late binding. | `just check`; the layering tool reports the explicit surface |
| 5.3 `cowboy-model` | New crate for plain types and constants shared by `core` and `store`: `SessionMeta`, `SessionOrigin`, `Inbound`/`Outbound`/`Envelope`, `settings_keys`, hot-tail limits, store row/record types. No `tokio`, no `sqlx`. `store` → `core` drops from 53 references to the write-behind contract (`Hub`, `StoreWrite`, `StoreSink`), which becomes a sink trait in `model` implemented by the Hub, so the store depends on the trait and never on `Hub`. | `just check`; the layering tool shows `store` no longer references `core` |
| 5.4 `cowboy-store` | `store`, `store/*`, `persistence`, `migrations/` and `migration_compat` become a crate depending on `model` and the 5.1 ports only. | `just check`, `just test-postgres`; migrations unchanged byte-for-byte |
| 5.5 `cowboy-hub` | `core`, `supervisor`, `runtime_router`, `remote_runtime`, `scheduler`, `session_context`, `session_folders` move together (they are one cycle today). | hub tests; the layering tool shows `hub` → `store` → `model` only |
| 5.6 Owner-specific modules leave the base | `cli`, `product_cli`, `admin`, `oidc`, `passkey`, `product_auth`, `client_auth*`, `local_operator`, `agent_api`, `usage` move into `cowboy-server`; `machine_*`, `worker`, `execution_target_environment`, `plugin_runtime_args::machine` move into `cowboy-machine`; `code_review` and `managed_calls::host` split into the shared contract that `code_adapter` and the Machine need (stays in the base) and the Controller implementation (moves). The remaining base is provider/plugin contracts, ACP and those shared contracts. | `just feature-check` and the filtered-source build; `cfg(feature = "full")` sites in the base crate fall from 220 to 0 because feature selection becomes crate selection |
| 5.7 Machine-side cycles | `machine_plugins ↔ machine_code_plugins` and `machine_broker ↔ machine_cli` are cut the same way (shared types down, behaviour up). | layering tool |

Expected effect (measured after the October split): incremental `cargo check`
after a `store` edit fell from 34s to 25s and the Machine-host cold check from
40s to 29s when `wire` left the base; isolating `store`, `hub` and the
server-only modules removes the rest of the shared closure.

### Phase 6 — generated protocol types

| Slice | Change | Proof |
|---|---|---|
| 6.1 Generate | Derive JSON Schema for `Inbound`, `Outbound`, `Envelope` and the typed `Event` payloads from Rust (the composition contract already uses a generated, checked-in schema and TypeScript file via `tools/generate-composition-contract.ts`); generate `web/src/protocol.generated.ts`; `protocol.ts` re-exports the generated types and keeps only hand-written helpers. | `just composition-check`-style drift check fails when Rust changes without regeneration; the Phase 0.4 fixtures |
| 6.2 Retire the mirror comments | The "mirrors src/core.rs" contract becomes the generator. | co-change `server.rs ↔ protocol.ts` falls to zero |

Runtime validation of incoming frames is **not** part of this plan: Web and
Controller deploy independently and `protocol.ts` deliberately tolerates
fields absent from older Controllers, so a validator derived from current
types could reject a previously accepted frame and strand a client. It is a
compatibility feature in its own right, with an unknown-field policy and
old/new Controller–Web positive fixtures, and is filed separately.

### Phase 7 — compatibility exits and release surface

| Slice | Change | Proof |
|---|---|---|
| 7.1 Exit table | A table in `docs/compatibility-exits.md` listing every legacy path (`LegacyProviderRelease`, `migration_compat`, `legacyRecordsNotice`, Hub tombstones, identity-based provider fallback, schema-one writer outputs, bootstrap outputs, each lint allowance group) with its removal condition and the check that proves the condition. Nothing is removed merely for being old. | reviewed table; historical fixtures keep passing |
| 7.2 Release matrix | Describe the eight outputs in one checked table (writer capability × retained runtime × bootstrap input) that `flake.nix` reads instead of three booleans, so the matrix is reviewable and `nix flake check` proves every cell builds. The variants themselves stay: passing the fleet reader floors does not make them equivalent — ordinary and bootstrap artifacts must remain read-only and active-writer targets must keep their writer (`AGENTS.md`). Retiring a variant is a separately accepted migration with recovery and startup-admission conformance, recorded in 7.1. | `nix flake check`; release-manifest comparison before and after |
| 7.3 Registry ledger | `components/registry.json` keeps **schema 3** (normative in `docs/requirements.md`; `src/plugin.rs` accepts 2 and 3) and the active release plus every release still referenced by a `plugins/*/plugin.json` or the retained worker pin; older releases move to `components/registry-archive/<version>.json` in the same schema. `src/plugin.rs` validation and `tools/check-plugin-components.ts`, `worker-registry-input.ts`, `plugin-component-closure.ts` read both, preserving ordered historical validation and immutable identity checks across the archive. Because `worker-generation` hashes the registry digest, this is a Machine maintenance boundary. | tool tests over existing component-release references; a Machine release with the worker pin unchanged |

### Phase 0 status (2026-11-05)

Phase 0 is landed except where noted; every guard runs in `just check`.

| Slice | Outcome |
|---|---|
| 0.1 | `tools/check-structure.ts` + `tools/structure-baseline.json` (`structure-check`); seeded with 58 files, 276 functions and 7 TSX files over their limits |
| 0.2 | `release-sources-check` type-checks the exact `machine-src` and `code-adapter-src` trees; reintroducing the October module gap fails with E0583 |
| 0.3 | Simplified: instead of a content hash and a new generation index, the Nix web build sets `VERSION` to `cowboy-v<flake lastModified>`. Numeric order, two-generation retention and rollback stay untouched; the source value is frozen and a test rejects hand bumps. Residual risk: a descendant commit whose committer time is older than the deployed one would get a lower generation |
| 0.4 | Event family: `contracts/protocol/event.json` from `src/protocol_contract.rs`, generated `web/src/protocol.fixtures.generated.ts` held by the ordinary web typecheck (`protocol-contract-check`); found the undeclared `turn_end.usage`. Extended to every Outbound variant (`contracts/protocol/outbound.json`), which found global errors serialized as `session_id: null` that the web hid (fixed on both sides). Inbound flows web → Controller, so the check runs the other way: `web/src/protocol.inbound.fixtures.ts` (one TypeScript-typed frame per variant) renders to `contracts/protocol/inbound.json`, and a Rust test parses every frame, rejects any dropped field and requires every variant; no drift found |
| 0.5 | `tools/check-module-layers.ts` + `tools/module-edges-baseline.json`: cycles may not grow or merge and no new edge may join two members of one cycle; ordinary new edges stay allowed. Per-module import allowlists wait until modules are extracted |
| 0.6 | 57 stale `dead_code` allowances and three dead functions removed; the two in `components/plugin-sdk` stay because changing that signed component needs a component release |
| 0.7 | `docs/INDEX.md` covers all 403 pages (`docs-index-check`); finished ledgers moved to `docs/history/` |

### Phase 1 status (2026-11-05)

Landed as behavior-preserving moves; every slice kept the server's 767 lib
tests, clippy and both ratchets green and was released to the Controller.

| Slice | Outcome |
|---|---|
| 1.1 | 30 inline test modules moved to `server/<name>_tests.rs` |
| 1.2 | Writer, notifications, provider reset timer, warm restore and startup steps left `server.rs` (`server/provider_reset_timer.rs`, `startup_restore.rs`, `startup_tasks.rs`) |
| 1.3 | `handle_command` 317 → 203: per-command functions in `client_commands.rs`; queue and draft arms in `queue_command`. The outer match stays exhaustive over `Inbound` |
| 1.4 | `serve_axum` 481 → <150: the route table is grouped by domain in `server/routes.rs`; the agent-API coverage and site tests read it there |
| 1.5 | `handle_machine_ws` 796 → 353: handshake in `machine_handshake.rs`, event ingestion in `machine_events.rs`, per-frame dispatch in `machine_connection/frames.rs` (`MachineLink`, `MachineInventory`) |
| 1.6 | `serve` 572 → 410: the remaining body is the ordered startup sequence and AppState assembly. The `Startup` builder waits for the Controller lifecycle test the slice names as its proof |
| 1.7 | `api_new_session` 398 → <150 (`resolve_new_session_defaults`, `create_machine_session`, `create_local_session`); `create_checked` mirrors it |

Also split: `handle_ws` 509 → 229 (`client_ws/fanout.rs`, `admit_active_client`),
`product_auth_api.rs` 4,587 → 1,323 lines (`product_auth_api/{login,oidc,
passkeys,devices,credentials}.rs`), `confirmed_reconcile_install` 328 → 206,
`MachineSnapshots::load` 230 → 192, `recovery::checked` 216 → 189.

Exit criterion status: `server.rs` is 2,775 lines and no inline test module
remains. Functions over 200 lines remain in two groups. Flat dispatch and
route tables (`agent_api::actions::execute`, `control::dispatch`,
`control::route`, `SessionHost::call`, `classify_route`,
`product_auth_router`, `handle_command`, `MachineLink::handle_frame`) are one
arm per operation; splitting them would only scatter the table, so they are
exempt from the 200-line target. The remaining sequential bodies (`serve`,
`handle_machine_ws`, `handle_ws`, `issue_product_session`,
`managed_calls::Coordinator::start`, `run_client_fanout`,
`spawn_provider_reset_timer`, `confirmed_reconcile_install`) are single
`select!` loops or ordered security sequences; each further cut needs the
lifecycle or connection-level test that 1.5/1.6 name first.

### Phase 2 status (2026-11-05)

| Slice | Outcome |
|---|---|
| 2.1 | `store.rs` test modules moved to files. Instead of a proc macro (a new crate is a Cargo maintenance boundary), a contract is an `async fn(&Store)` with a SQLite test and an ignored `postgres_` twin over shared fixtures (`store/storage_contract_tests/test_support.rs`). OIDC logout scoping, session replacement fencing, capacity fairness and compact restore now run on PostgreSQL; the gate runs 59 PostgreSQL tests, all passing, with no divergence found. The product-auth HTTP tests in `cowboy-server` followed (task-local backend choice in `test_store()`): 27 twins, so the gate runs 86 PostgreSQL tests, all passing |
| 2.2–2.3 | Not started; 2.1 found no backend divergence, which per the slice text lowers the value of 2.3 |

### Phase 3 status (2026-11-05)

| Slice | Outcome |
|---|---|
| 3.2 | `SettingsShell.tsx` and `ToolDetailsBrowser.tsx` extracted (Phase 0 batch) |
| 3.3 | `SessionList.tsx` extracted; `App` 2,406 → 2,191 lines through `useSessionFocusLifecycle`, `useDesktopSplitters` and `useMobileDrawerSurface`, each called where its effects ran so hook order is unchanged |
| 3.4 | `PendingPanel.tsx` and `ComposerSheet.tsx` extracted; `Composer.tsx` 4,911 → 3,292 lines. `ComposerWorkspace` itself (2,524 lines) is not yet split, and physical iOS composer acceptance was not run |
| 3.5 | `TranscriptItems.tsx` and `TranscriptPlaceholders.tsx` extracted; `Transcript.tsx` 5,042 → 3,117 lines with the scroll contract kept in place. `DocumentView` left `ReviewApp` |

Every slice ran the browser suites named for it in Firefox and Chromium, in
both themes; a flaky `sheet-keyboard` timeout passed on rerun. The
`dormant-session` suite had failed on main since 50b6dd1b4 (bisected): a
session-settings open request a mounted surface had already handled stayed
pending and reopened settings on a remount. Fixed in `sessionSettingsOpen.ts`.
App's own splitter and drawer bindings have no dedicated browser fixture.

### Phase 4 status (2026-11-05)

| Slice | Outcome |
|---|---|
| 4.1 | Not started. The queue, transport and socket code in `store.ts` form one call cycle (queue → send → socket → handle → queue), and `setState` publishes connection status, so a module split needs the injected transport and clock the slice names first; moving code across that cycle would only add an import cycle |
| 4.2 | The `impl Hub` file layout is split by concern without changing ownership or lock order: `core/hub_queue.rs` (send queue, drafts, dispatch), `core/hub_sync.rs`, `core/hub_provider_pins.rs` and `core/hub_config.rs`; core.rs's inline tests moved to files. `core.rs` falls from 8,037 to 5,150 measured lines. Separating queue policy from its effects is still open |
| 4.3 | Not started (Machine code: a Machine maintenance boundary) |

### Phase 7 status (2026-11-05)

7.1 landed as [`compatibility-exits.md`](compatibility-exits.md): about 45
paths, of which only nine state their own exit condition. Both protocol
floors are still 1 (current runtime 5, Machine 41), so every older-Machine
branch waits on one Machine maintenance that raises them. 7.2 and 7.3 are
Machine maintenance boundaries and wait for that explicit window.

## 6. Not worth doing

- A rewrite, a second controller, a microservices split or a wholesale state
  library replacement. The gate, tests and deployment contracts are the
  asset; the plan keeps all of them.
- Crate-per-file fragmentation or any split that leaves unrestricted shared
  state behind a new crate name.
- Replacing SQLx or using the `Any` driver to collapse the two backends:
  Postgres JSONB and advisory locks are used deliberately; 2.1 removes the
  real risk at a fraction of the cost.
- Merging desktop and mobile UI code (intentional duplication per
  web/AGENTS.md), or redesigning composer or gesture behaviour during an
  extraction.
- Reformatting `App.tsx` wholesale (3,700-line diff with no structural gain);
  extraction shrinks it instead.
- Rewriting applied migrations, or introducing a registry schema version as a
  side effect of a storage-layout change.
- Removing compatibility branches or release variants because they look old;
  only an exit condition from 7.1 justifies removal.
- Weakening a test to make a move easier; 3.1 replaces brittle assertions
  with behavioural ones, never with nothing.
- A repository-wide Clippy `pedantic` sweep as its own project; enable
  `pedantic` per crate as each crate leaves the base (new crates start with
  it on).
- Converting the hand-written `docs/INDEX.md` descriptions to generated text;
  the completeness check in 0.7 is enough.

## 7. Health metrics and quarterly targets

Reported by `tools/check-structure.ts --report` so the numbers are reproducible;
the time and conflict metrics come from CI timing and `git log`.

| Metric | Baseline (2026-11-05) | Target (end of Q1 2027) |
|---|---|---|
| Largest module cycle in the base crate | 37 modules (`tools/module-edges-baseline.json`) | ≤ 6 modules |
| `cfg(feature = "full")` sites in `src/` | 220 | 0 |
| Filtered release closures compile in the gate | no (`.#cowboy-machine` broken on baseline) | yes, every commit |
| Files over 3,000 lines | 21 | 0 |
| Functions over 200 lines | 116 | ≤ 20, all in the baseline allow-list |
| `server.rs` production lines | 21,369 | < 3,000 |
| Store methods with two bodies | 129 | ≤ 60, all covered by differential tests on both backends |
| Web tests that read component source | 28 files | 0 |
| `server.rs ↔ protocol.ts` co-changes per quarter | 28 | 0 |
| `sw.js` commits per 60 days | 498 | 0 |
| `components/registry.json` size | 1.2 MB, 101 releases | ≤ 100 KB |
| Wildcard crate re-exports | 2 | 0 |
| Deleted/added lines per month | 12–25% | ≥ 50% during refactor quarters |
| Median edit-to-green time (server / Machine / web change) | to be recorded in Phase 0 | −30% |
| Merge conflicts per landed change touching `server.rs`, `App.tsx` | to be recorded in Phase 0 | −50% |
| Fix share of commits | 38% | trend down; reported, not targeted |
| Lint allowances (`allow(dead_code|too_many_*|large_*)`) | 193 | ≤ 60 |
| Incremental `cargo check` after a `store` edit | 25s | ≤ 12s |

## 8. Defects noticed during this review

Fixed with this plan (separate commits, each with its test or build proof):

- **Machine and code-adapter release builds were broken on `main`.**
  `src/lib.rs` declared `task_notifications` and `web_push` unconditionally
  since the 2026-10-22 split, but `machine-src` and `code-adapter-src` in
  `flake.nix` do not carry those files, so `nix build .#cowboy-machine` failed
  with `E0583` while every `cargo` gate on the full checkout passed. Both
  modules are Controller-only and are now gated on `feature = "full"`; the
  filtered builds compile again. Slice 0.2 turns this into a permanent check.
- **Retrying a crashed turn dropped non-text prompt content.**
  `Hub::retry_turn` rebuilt the prompt from the *last* `user_message_chunk`
  only, text blocks only. The ACP worker echoes one chunk per content block,
  so an image-plus-text prompt was retried without the image and a prompt
  ending with an image was retried as nothing ("no prior prompt to retry").
  `retry_turn` now rebuilds the whole chunk group and resubmits every block.
  Because history keeps large images only as `/api/artifacts/…` URLs
  (`push_tagged` externalizes them), the retry restores the bytes from the
  artifact store before dispatch and, when an artifact is gone, refuses with a
  message on the failure card instead of silently dropping the attachment. The
  "already queued" check identifies the requeued prompt by its cmid when the
  live echo still carries one, by text otherwise, and by attachment shape for
  an attachment-only prompt, then sends that entry rather than the queue head.
  Five Hub tests cover these shapes.
- `docs/architecture/00-overview.md` and `06-server-api.md` referenced
  `src/server.rs`, `src/machine_*.rs` and `src/bin/cowboy-machine.rs`, all
  moved on 2026-10-22.

Queued (own commits, not refactor slices):

- The 193 lint allowances have not been audited since the split; several
  `#[cfg_attr(not(test), allow(dead_code))]` row types in `store.rs` suggest
  production code that only tests read (slice 0.6).

## 9. Review record

Astra Max design review, 2026-11-05 (managed call
`call-6c41abd1c1ca5a80e979c394`): verdict *needs-attention*; four high,
nine medium and two low findings, none against the diagnosis or the
seams-before-crates ordering. Each finding and its disposition:

| Finding | Disposition |
|---|---|
| Retry resubmitted externalized image blocks without their bytes | fixed in the retry commit: artifact restore before dispatch, visible refusal when the artifact is gone, test through persistence |
| Hashed `sw.js` versions break numeric cache discovery and rollback; filename-only digest misses stable-name assets | slice 0.3 rewritten: content-manifest digest plus an ordered generation index with migration and multi-release tests; requires a design note |
| Crate extraction changes `Cargo.lock`, which the retained-worker interface and worker generation pin | Phase 5 is now one explicit worker-generation maintenance boundary; Rule 10 extended |
| Release variants encode writer, retained-runtime and bootstrap capabilities that reader floors do not make equivalent | slice 7.2 keeps the variants and makes the matrix a checked table; retirement only through 7.1 |
| Retry dedupe by text alone misidentifies attachment-only prompts and may drain a different queue entry | fixed: cmid first, attachment shape for empty text, `request_send_queued` on the identified entry; test added |
| Protocol fixtures in `*.test.ts` are not type-checked | slice 0.4 now uses a generated fixture module under a dedicated `tsc` contract step in `just check` |
| `COWBOY_TEST_POSTGRES` does not match the runner's `COWBOY_TEST_POSTGRES_URL` and `postgres_*` discovery | slice 2.1 generates discoverable `postgres_*` siblings that fail without the URL and asserts counts |
| `idb-conformance-check` only type-checks runners; web slices named no suites | Phase 3 lists the suites each slice executes; Rule 6 treats Desktop gate exit 3 as missing acceptance |
| Behavioural back-edges (`store` → auth modules, `supervisor` → ACP, `code_adapter` → `code_review`, Machine → `managed_calls::host`) make the proposed DAG unreachable by moving DTOs | R1 records them; slice 5.1 defines ports before 5.4–5.6; 5.6 splits `code_review`/`managed_calls::host` into contract and implementation |
| Runtime protocol validation is a compatibility change | removed from the plan and filed as a separate feature (Phase 6 note) |
| Golden phase-name list does not protect startup/shutdown ordering; `agent_api_smoke.py` is MCP-only | slice 1.6 requires an isolated lifecycle test and a real HTTP smoke |
| Registry "schema 4" would be an unplanned format migration | slice 7.3 keeps schema 3 |
| Textual `crate::` edge scan is bypassable by import syntax | slice 0.5 uses syntax-aware import resolution per feature set |
| New-file size limits reject unchanged pure moves | slice 0.1 adds allowance transfer tied to moved symbols with decreasing aggregates |

## Appendix: how the numbers were produced

- Lines: `rg --files -g '*.rs' -g '*.ts' -g '*.tsx' <dir> | xargs wc -l`.
- Module graph: for each top-level module under a crate's `src/`, references
  of the form `crate::<module>` in production code form the edges; Tarjan's
  algorithm finds the strongly connected components; closure is the
  transitive edge set. (The Phase 0.5 tool replaces this textual scan with
  syntax-aware resolution; the baseline numbers are a lower bound.)
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
- Release build: `nix build .#cowboy-machine --no-link` on the baseline tree.
