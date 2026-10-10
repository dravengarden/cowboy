# cowboy — agent guide

Drive Codex over ACP from anywhere. A single Rust (axum) process serves both
the API and the embedded React SPA, with
one agent subprocess per session. Deployed as a NixOS service on hawk (:3333).
Frontend specifics live in `web/AGENTS.md`; this is the cross-cutting layer.

## Toolchain

- Run Rust, Bun, build, lint, and test commands from the repository root in
  the pinned shell, for example `nix develop -c just check-compact`. Ordinary
  interactive Cargo commands retain incremental state; the complete gate avoids
  growing another incremental generation. Keep sccache opt-in until its
  cross-worktree Rust hit rate is proven on the active host. Do not use host
  `cargo` or `bun` as a preliminary check; a missing tool or stale Rustup linker
  wrapper is an environment failure, not a product-code failure.
- JavaScript and TypeScript run on the pinned Bun. The
  repository root is one Bun workspace (`web` plus the `components/*` packages
  it links): `just install` creates `node_modules`, `bun.lock` is the only
  lockfile, and a dependency change needs a new `depsHash` in `flake.nix`.
  Tests import `test` from `bun:test` and assertions from `@std/assert`;
  scripts use `node:` and `Bun` APIs and spawn through `tools/lib/command.ts`.
  Do not use `jsr:`/`npm:` specifiers.
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
require the exact successful build receipt and published SideStore version.
- Cowboy application releases use project-owned Nix artifacts rather than a
  full NixOS generation. From a clean committed task worktree, build the
  narrowest affected output: `.#cowboy-web-release`,
  `.#cowboy-controller-release`, or `.#cowboy-machine-release`. Hand each
  immutable result to the machine-owned Cowboy component activator. Publishing
  the Cowboy commit is independent from local activation; the target pins an
  unpublished successful revision until the task pushes it or deploys a
  descendant revert. `/etc/nixos` and Columbus stable checkouts are never
  deployment sources.
- Resident Machine fixes use `.#cowboy-machine-host-release` to retain the
  separately accepted worker/Code/Zed bundle pinned by the `cowboy-workers`
  flake input. Its source and exact generation remain in the artifact's
  `retained-worker-source.json`; native wire/SDK/dependency source changes
  refuse this output. Advancing that pin or using `.#cowboy-machine-release`
  changes the pool/adapter boundary and needs separate maintenance acceptance.
- Targets with an active schema-one Session deletion writer use
  `.#cowboy-machine-writer-host-release` for resident fixes. This retains the
  same accepted worker/Code/Zed bundle and the installed writer capability.
  Its startup admission requires the fixed root component selection, exact
  native build and existing Hawk reader floor. Ordinary/bootstrap builds stay
  read-only; reader-only artifacts remain compatible explicit recovery targets.
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

## Architecture gotchas
- **Configuration**: the Service and each Device own one TOML file through the
  unified framework in `src/config.rs`; see `docs/configuration.md`. Decide
  every new setting with its "Where a setting belongs" rule (wiring/secret →
  CLI/env, UI-edited runtime state → database, everything else → config file,
  scoped to the enforcing process). Never add a behavioural environment
  variable or a separate JSON policy file; validate edits with
  `cowboy config check` before applying or rolling out.
- The normative Provider-platform contract is `docs/requirements.md`. The
  canonical dependency-audit and release workflow is the repository-owned
  `.agents/skills/release-cowboy-plugin/`; keep it versioned with Cowboy and
  do not fork it into a user-home skill.
- **One process serves frontend + backend** → daemon-down = white screen. The
  robustness layers (SW shell cache, AppErrorBoundary, ConnectionBanner, store
  NUL-strip / skip-bad-row, parking_lot mutex) exist to catch that — keep them.
  (memory: cowboy-white-screen-robustness)
- **`web/src/App.tsx` is a 4-space-indent outlier** — never run `dprint fmt` on it
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
