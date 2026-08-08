# cowboy — agent guide

Drive Codex over ACP from anywhere. A single Rust (axum) process serves both
the API and the embedded React SPA, with
one agent subprocess per session. Deployed as a NixOS service on hawk (:3333).
Frontend specifics live in `web/AGENTS.md`; this is the cross-cutting layer.

## Toolchain

- Run Rust, Deno, build, lint, and test commands from the repository root in
  the pinned shell, for example `nix develop -c just check-compact`. Ordinary
  interactive Cargo commands retain incremental state; the complete gate avoids
  growing another incremental generation. Keep sccache opt-in until its
  cross-worktree Rust hit rate is proven on the active host. Do not use host
  `cargo` or `deno` as a preliminary check; a missing tool or stale Rustup linker
  wrapper is an environment failure, not a product-code failure.

## Deploy (read before deploying)
- Cowboy application releases use project-owned Nix artifacts rather than a
  full NixOS generation. From a clean committed task worktree, build the
  narrowest affected output: `.#cowboy-web-release`,
  `.#cowboy-controller-release`, or `.#cowboy-machine-release`. Hand each
  immutable result to the machine-owned Cowboy component activator. Publishing
  the Cowboy commit is independent from local activation; the target pins an
  unpublished successful revision until the task pushes it or deploys a
  descendant revert. `/etc/nixos` and Columbus stable checkouts are never
  deployment sources.
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

## Architecture gotchas
- **One process serves frontend + backend** → daemon-down = white screen. The
  robustness layers (SW shell cache, AppErrorBoundary, ConnectionBanner, store
  NUL-strip / skip-bad-row, parking_lot mutex) exist to catch that — keep them.
  (memory: cowboy-white-screen-robustness)
- **`web/src/App.tsx` is a 4-space-indent outlier** — never run `deno fmt` on it
  (it would reformat 3700 lines). Match 4-space when editing it.
  (memory: cowboy-web-app-tsx-4space)
- A **fresh worktree is missing `web/src/_shell`** (`harness shell link` no
  longer covers cowboy) — symlink it manually before building.
  (memory: cowboy-shell-symlink-fresh-worktree)
- SQLx migration files are immutable after deployment, including comments and
  whitespace because their exact bytes are checksummed. Add a new migration;
  never edit an applied file or alter stored checksum records. If startup
  reports a modified migration, restore its exact historical bytes before
  diagnosing later service symptoms.

## Memory / sessions
- cowboy does not own agent memory. Standard Codex uses its native local-memory
  feature through the normal user `CODEX_HOME`. Provider variants such as
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
  but a selected Machine must fetch the remote default branch and prepare or
  reuse a session-owned worktree before starting the ACP worker. The legacy
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
confirm-detection, the Tauri native shell — see `web/AGENTS.md` and the
`cowboy-*` memories.
