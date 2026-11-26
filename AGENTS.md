# cowboy — agent guide

Drive Codex over ACP from anywhere. A single Rust (axum) process serves both
the API and the embedded React SPA, with
one agent subprocess per session. Deployed as a NixOS service on hawk (:3333).
Frontend specifics live in `web/AGENTS.md`; this is the cross-cutting layer.

## Toolchain

- Use the pinned Biome formatter and the existing Rust-based linters. Do not
  introduce dprint, Deno runtimes, or libraries maintained by the Deno
  organization (including its standard-library packages). Tests use Bun and
  Node's built-in assertions; retain type checking as a separate gate.
- Run Rust, Bun, build, lint, and test commands from the repository root in
  the pinned shell, for example `nix develop -c just check-compact`. Ordinary
  interactive Cargo commands retain incremental state. The complete gate keeps
  it for the shared test graph, its feature slices and Clippy, where an edit's
  rebuild drops from about 114s to 25s for the graph, and compiles everything
  else through one sccache shared across worktrees. Only registry dependencies
  hit that cache across worktrees: workspace crates and crates with build
  scripts carry checkout paths in their keys, and so does any `CARGO_*`
  variable holding one, so pass directories as Cargo flags or profiles, never
  as `CARGO_TARGET_DIR`/`CARGO_BUILD_BUILD_DIR`. Do not use host `cargo` or
  `bun` as a preliminary check; a missing tool or stale Rustup linker wrapper
  is an environment failure, not a product-code failure.
- Keep the gate cheap to rerun. A rerun without source changes must not
  recompile the Rust graphs. Rust test recipes share `rust_test_args` and
  `rust_test_env` because another package or feature selection recompiles the
  workspace; a recipe that needs a built CLI takes the shared graph's
  executable (`test-graph-bin`) instead of a separate `cargo build`. A build
  script must not watch build outputs such as nested Cargo `target`
  directories or `node_modules`. `check` first starts the shared test graph's
  compile, workspace Clippy, the feature-sliced Rust test builds,
  `release-sources-check` and the Zed adapter (each with a Cargo profile or
  build directory of its own), the script suites and the frontend build in the
  background, then runs the second-scale static checks and the rest in the
  foreground; a failed background job stops the gate at the next foreground
  step. The shared test graph links its binaries into `target/test-graph`;
  `target/debug/cowboy-execution-host` is the Machine-feature host that
  `claude-remote-prepare` links for the script suites, and nothing else in the
  gate rewrites it. While iterating on one failure, rerun that recipe first,
  then run the complete gate once before handing off.
- In a fresh worktree, start `nix develop -c just warm` in the background
  before reading the code. It installs the dependencies and compiles the
  gate's Rust lanes through the gate's compiler cache (a few minutes cold)
  while you work, so the first `check-compact` starts warm.
- Iterate on Rust tests with `nix develop -c just test-rust-filter <filter>`
  (nextest name filters or `-E` expressions) and on lints with `just clippy`.
  Do not run ad-hoc `cargo test -p …`, `cargo nextest run -p …` or
  `cargo clippy -p …` selections: each compiles the workspace again in another
  configuration, for minutes and then again on every edit, and leaves another
  multi-gigabyte copy in the build directory, which the host evicts under disk
  pressure.
- JavaScript and TypeScript run on the pinned Bun. The
  repository root is one Bun workspace (`web` plus the `components/*` packages
  it links): `just install` creates `node_modules`, `bun.lock` is the only
  lockfile, and a dependency change needs a new `depsHash` in `flake.nix`.
  Tests import `test` from `bun:test`. Repository-local tests reuse assertions
  backed by `node:assert/strict` in `tools/test-assert.ts`; tests shipped inside
  component packages import Node's built-in assertions directly to keep the
  package self-contained.
  Scripts use `node:` and `Bun` APIs and spawn through `tools/lib/command.ts`.
  Do not use `jsr:`/`npm:` specifiers.
- Upstream packages Cowboy builds from source live in `third_party/<name>/`
  as a pinned release plus a delta series. Change them only through
  `tools/third-party.ts` (checkout, export, upgrade); never hand-edit a patch.
  Upstream first: drop a delta once upstream covers it. See
  `docs/third-party.md`.
- Signed Plugin collectors (`plugins/*/collector/`) run on the Machine's
  bundled Bun with no host-provided globals. A package is self-contained, so
  a collector that starts its provider CLI carries its own `process.js`.

## Deploy (read before deploying)
- User preference: completed product fixes include verification, integration into
  remote `main`, and production activation by default. Do not stop at local
  edits or ask again for routine merge/deploy approval. Preserve the component
  and active-session boundaries below and report actual release receipts.

iOS Xcode 27 builds use the build-local Swift runtime export adapter described
in `apps/native-shell/README.md`. Keep registry dependency checksums and the
single-runtime-owner invariant; do not fork swift-rs through
`[patch.crates-io]`. Source checks alone never prove an iOS release shipped:
require the exact successful build receipt and App Store Connect / TestFlight upload receipt.
- Cowboy application releases use project-owned Nix artifacts rather than a
  full NixOS generation. From a clean committed task worktree, build the
  narrowest affected output: `.#cowboy-web-release`,
  `.#cowboy-controller-release`, or `.#cowboy-machine-release`. Hand each
  immutable result to the machine-owned Cowboy component activator. Publishing
  the Cowboy commit is independent from local activation; the target pins an
  unpublished successful revision until the task pushes it or deploys a
  descendant revert. `/etc/nixos` and Columbus stable checkouts are never
  deployment sources.
- Release outputs do not rerun the Rust suite (`checks.cowboy-tests` keeps it
  hermetic). Build a release only from the exact commit whose complete gate
  passed; after merging or rebasing onto newer `main`, rerun the gate on the
  merged commit before building it.
- Resident Machine fixes use `.#cowboy-machine-host-release` to retain the
  separately accepted worker/Code/Zed bundle pinned by the `cowboy-workers`
  flake input. Its source and exact generation remain in the artifact's
  `retained-worker-source.json`; native wire/SDK/dependency source changes
  refuse this output. Advancing that pin or using `.#cowboy-machine-release`
  changes the pool/adapter boundary and needs separate maintenance acceptance.
  Host releases also retain that pin's native execution runtime while using the
  current keeper. Recovery must preserve saved executor digests; do not silently
  advance the target executor when only upgrading the resident Machine.
- Targets with an active schema-one Session deletion writer use
  `.#cowboy-machine-writer-host-release` for resident fixes. This retains the
  same accepted worker/Code/Zed bundle and the installed writer capability.
  Its startup admission requires the fixed root component selection, exact
  native build and existing Hawk reader floor. Ordinary/bootstrap builds stay
  read-only; reader-only artifacts remain compatible explicit recovery targets.
- Host configuration that adds mandatory Machine arguments also advances its
  cold recovery input. Use `.#cowboy-machine-host-bootstrap-release` to retain
  the accepted worker bundle while supplying the new resident CLI; it remains
  a read-only bootstrap and never enables the deletion writer.
- Web releases atomically move only `/run/cowboy-web` and restart no process.
  Controller releases restart only `cowboy.service`. Machine releases are a
  separate explicit maintenance boundary for the resident Machine, worker
  generation, and isolated Zed adapter. Do not use a controller or Web release
  to recycle active sessions.
- **A controller deployment restarts the daemon you may be driving Codex
  through.** The machine activator runs in an independent root systemd unit that
  survives this restart. Follow its journal and verify the component receipt,
  `/healthz`, `/version`, the SPA version/cache headers, and Machine presence.
  Web/bundle changes also need a PWA hard-reload (a WS reconnect keeps stale JS).
  (memories: cowboy-switch-restarts-approval-channel, cowboy-v1-deploy)
- Authorized Plugin upgrades may use `cowboy operator` on the Controller host
  as its Service account after explicit host delegation is enabled. Follow
  `docs/agent-driven-plugin-release.md`: select exact version/digest, retain one
  operation ID, and inspect receipts after failures. Never substitute browser
  credential copying, live database edits or installation-pointer writes.

## Review risk

Managed reviews pick their reviewer by declared risk
([`docs/review-routing.md`](docs/review-routing.md)); declare it from this
list and the affected paths, and give a reason for `high`.

- **high** keeps only what can lose or corrupt durable data, weaken
  credentials or isolation, or touch money:
  - Machine durable namespaces and their writers
    (`components/machine/src/machine_broker/{deletions,incarnations,cleanups,namespace}.rs`),
    store migrations and ledger CAS (`migrations/`, `components/store/src/store/`), and
    deletion paths;
  - authentication, permissions, Operator delegation and account isolation
    (provider accounts, account sandboxes, credentials, browser profiles);
  - anything touching money or usage limits.
- **routine**: everything else, including managed-call lifecycle and
  routing, the Controller/Machine/worker wire, release pins, large or
  mechanical refactors, Web (`web/`), docs, tests, model evidence
  (`models/`), Plugin presets and copy, and CLI help. Deterministic gates
  (`just check`, the structure and layering ratchets, wire generation
  checks) carry those areas; size alone is not risk.
- Routine reviews stay on Sol Max (Codex) or Opus (Claude); simple changes
  never need Astra Max (`preset_outside_tier` refuses stronger presets on
  `routine`).
- On `risk: high` (with a concrete `reason`), Codex dynamically switches
  automatic priority between **Astra Max** (when fresh quota headroom is ample
  relative to reset time) and **Sol Max** (when quota is tight, behind pace, or
  stale). Agents may also self-assess and explicitly name `astra-max` (or
  `fable-max`) with `risk: high` and a concrete reason whenever an important
  change warrants the strongest reviewer (for example durable deletion,
  incarnation or cleanup protocols in the Machine broker, store migrations,
  account sandboxes, credential storage, or Operator delegation scope).
- A fix-confirmation round is routine. Continue the review child when it
  runs a routine-tier preset; a child on Astra Max refuses a routine round, so
  start a fresh routine review with the same task and base (it reuses the
  captured input). Declare `risk: high` on a round, with a reason, only when
  the fixes themselves changed a high-risk design, not because the first
  round was high.
- A review child takes at most three completed rounds
  (`review_rounds_exhausted`). Findings that survive them call for
  redesigning that mechanism, not another round.

## Architecture gotchas
- **Structure ratchet** (`just structure-check`, part of `just check`): no
  production file, function or TSX hook count over its limit (Rust file 1,500
  lines, TS/TSX file 800, function 150, hooks 40) may appear or grow beyond
  `tools/structure-baseline.json`. When a change needs room in an offender,
  first extract something from it in the same change; do not raise the
  baseline. After moving an offender unchanged into a new file, record the move
  with `bun tools/check-structure.ts --update`, which only admits it while the
  crate/package total does not grow. The Nix web build runs this ratchet and
  the web unit tests too, and the Controller release depends on
  `cowboy-structure-check` (this ratchet plus module layering), so neither
  release can be built from a commit that fails them. The refactor sequence this guards is
  `docs/structural-refactor-plan-2026-11.md`.
- **Module layering ratchet** (same recipe): no Rust module may join or enlarge
  a dependency cycle, and no new edge may be added between two modules of the
  same cycle (`tools/module-edges-baseline.json`; the base crate's former
  37-module cycle is down to 8 (store), 6 (Plugins) and 4 (Hub) modules). Depend on a narrower module or move the shared
  type into a contract module instead. Run `bun tools/check-module-layers.ts
  --update` after a change that removes edges.
- **Reboot convergence is a core requirement**: after any Machine (Hawk, OVH,
  Falcon) reboots successfully, Cowboy must return to service with no human
  step. Units start at boot, always restart (exit 75 included) and never stay
  start-limited; cross-host readiness is retried, not fatal; transient state is
  rebuilt and lost lifetimes are recovered without replaying uncertain effects.
  See [reboot convergence](docs/requirements.md#reboot-convergence) and verify
  without rebooting production hosts.
- Storage moves must preserve `/tmp/codex-daemon-<uid>` as a real user-owned
  `0700` directory. Codex's native sandbox rejects a symlink there, even with
  a correctly permissioned target. See the
  [sandbox directory recovery contract](docs/execution-environments.md#reserved-sandbox-directories-during-storage-moves).
- Agents use Cowboy MCP first for Cowboy state and control. Load only the
  needed help topic/domain, reuse its revision, compose reads, and return a small
  projection. Use a stable operation ID and inspect its receipt after uncertainty.
  CLI remains supported for unavailable live MCP capabilities and the explicit
  native/host/identity boundaries in `docs/agent-api.md`. Do not replay an unknown
  effect under a new ID or interpret accepted submission as confirmed completion.
- **Long-wait pre-dispatch triage (Phase 2B)**: before delegating work from an
  active session, match the waiting path to expected duration and mutability so
  long waits do not keep a parent turn open across prompt-cache expiry:
  - Long (>3–5 min heuristic) **read-only** work (code/design reviews, broad
    codebase or transcript analyses): dispatch via `calls.start` / `review.start`
    with `notify:true` and end the parent turn cleanly instead of polling
    `calls.wait`; read `calls.result` when the durable `[Cowboy]` completion
    notice arrives (Phase 2A guarantees recovery across restarts and rollovers).
  - Long **write** work: route to an independent isolated session in its own
    worktree (`sessions.create` + `queue.submit` with `notify:true`, or
    `cowboy session prompt --notify`), then end the parent turn.
  - **Short critical-path** work (<3–5 min): reserve native subagents (Codex
    `spawn_agent`+`wait_agent`, Claude `Task`) for work that belongs inside the
    current turn.
  - Never abort, reset, or force-compact an in-flight native turn to dodge
    cache expiry. Measure decoupling with ordinary resume first before enabling
    opt-in `rollover_on_notify` (Phase 2C). See
    [`docs/wait-cache-observability.md`](docs/wait-cache-observability.md) and
    [`docs/agent-api.md`](docs/agent-api.md#pre-dispatch-triage-long-waits-and-prompt-cache).
- **Configuration**: the Service and each Device own one TOML file through the
  unified framework in `src/config.rs`; see `docs/configuration.md`. Decide
  every new setting with its "Where a setting belongs" rule (wiring/secret →
  CLI/env, UI-edited runtime state → database, everything else → config file,
  scoped to the enforcing process). Never add a behavioural environment
  variable or a separate JSON policy file; validate edits with
  `cowboy config check` before applying or rolling out.
- New configurable behavior must expose typed discovery and scoped AI control
  through Cowboy MCP by default, or document a concrete exception. Reuse its
  existing read/execute/receipt surface and domain writers; discover schemas on
  demand rather than expanding resident tool prompts. Preserve each setting's
  DB or Service/Device config ownership, permissions and activation rules. See
  `docs/agent-api.md` for the account preference contract.
- The normative Provider-platform contract is `docs/requirements.md`. The
  canonical dependency-audit and release workflow is the repository-owned
  `.agents/skills/release-cowboy-plugin/`; keep it versioned with Cowboy and
  do not fork it into a user-home skill.
- **One process serves frontend + backend** → daemon-down = white screen. The
  robustness layers (SW shell cache, AppErrorBoundary, ConnectionBanner, store
  NUL-strip / skip-bad-row, parking_lot mutex) exist to catch that — keep them.
  (memory: cowboy-white-screen-robustness)
- **`web/src/App.tsx` is a 4-space-indent outlier** — never run `biome format --write` on it
  (it would reformat 3700 lines). Match 4-space when editing it.
  (memory: cowboy-web-app-tsx-4space)
- Web app-shell primitives and the Bun/Vite builder are owned by Cowboy under
  `components/app-shell` and `nix/`; fresh worktrees must build without
  links to another repository.
- The consolidated PostgreSQL and SQLite SQLx baselines are immutable after
  deployment, including comments and whitespace because their exact bytes are
  checksummed. Add a new migration;
  never edit an applied file or alter stored checksum records. If startup
  reports a modified migration, restore its exact historical bytes before
  diagnosing later service symptoms.

## Memory / sessions
- Cowboy does not own the memory store. Standard Codex and Claude can enroll
  in the separately versioned Matrix service through their signed Provider
  adapters; enrolled sessions disable native long-term memory and use explicit
  project/executor bindings. See `docs/architecture/08-memory.md`.
  Unenrolled runtimes retain their native behavior. Provider variants such as
  `codex-deepseek` use a fully separate provider-owned `CODEX_HOME` and must not
  read, link, or mutate standard Codex config, auth, history, memory, rules,
  plugins, or skills. Required project guidance stays in `AGENTS.md`, docs,
  tests, hooks, and canonical skills.
- `claude-deepseek` follows the same stronger boundary for Claude Code: use a
  provider-owned `CLAUDE_CONFIG_DIR`, remove every inherited `ANTHROPIC_`,
  `CLAUDE_`, and `DEEPSEEK_` variable before applying the closed provider
  environment, mark routing as host-managed so settings cannot override the
  endpoint or authentication, and never read, link, or mutate ordinary Claude
  settings, credentials, history, projects, plugins, cache, or instance
  metadata. Sharing the adapter executable is allowed; sharing its mutable
  instance state is not.
- The Web New Session picker (`GET /api/workspaces`) lists stable source roots,
  but a selected Machine must fetch the remote default branch (or use committed
  HEAD when no remotes are configured) and prepare or reuse a session-owned
  worktree before starting the ACP worker. A configured remote that fails must
  never fall back to local HEAD. The legacy
  WebSocket creation path fails closed. Direct API/ACP callers retain their
  caller-owned local workspace for compatibility. Never turn the stable
  checkout or `/etc/nixos` back into a Web task workspace.
- If a project task needs Hawk NixOS integration, create a second isolated
  Columbus worktree from freshly fetched `origin/main`. Commit the full machine
  configuration there, integrate the active deployment revision, and use the
  machine-owned build/activation commands. A clean commit may deploy before it
  is pushed; the task that deployed it retains responsibility for publication
  or a committed revert.
- The picker also projects matching central Columbus work items. Selecting one
  sends a resume prompt into a native Codex task; Cowboy never stores item
  lifecycle or binds it to the session.

## Frontend
Efficiency is the core UX criterion. Follow [the core UX principles](docs/ux-principles.md):
Mobile keeps frequent controls in a bottom workbench and content above;
Desktop keeps commands visible and keyboard reachable. Verify rendered task
flows, not just static source or visual tidiness.

Composer (mdlive / CM6), optimistic-send, transcript (column-reverse scroll),
the Tauri native shell — see `web/AGENTS.md` and the
`cowboy-*` memories.

Mobile horizontal swipe (Sessions/Review drawers and the Agent↔Review
pager) **must not jank**. That is a core product requirement, not
polish: a wrap-off CodeMirror file must track the finger as cheaply as
README. The contract is
[`docs/mobile-spatial-presentation.md`](docs/mobile-spatial-presentation.md).

**OPEN TODO:** physical iPhone caret after a pasted composer image is
unsolved. 2026-08-15 user: same shape in Obsidian; likely WeType (first
Return on native Pinyin, first few on WeChat IME). Ledger, failed
attempts, and what not to retry: `web/src/mdlive/PITFALLS.md` pitfall
**#69**. Do not claim it fixed.
