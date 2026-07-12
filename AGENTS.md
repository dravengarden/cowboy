# cowboy — agent guide

Drive Codex over ACP from anywhere. A single Rust (axum) process serves both
the API and the embedded React SPA, with
one agent subprocess per session. Deployed as a NixOS service on hawk (:3333).
Frontend specifics live in `web/AGENTS.md`; this is the cross-cutting layer.

## Deploy (read before deploying)
- cowboy is a NixOS service (`services/cowboy` on hawk), consumed via a
  `git+file://` flake input from this repo. To ship: commit here, then on hawk
  `nix flake update cowboy` + rebuild.
- **Deploying restarts the daemon you may be driving Codex through.** Build
  first, then use `/etc/nixos`'s `just sys-activate ./result`: it hands activation
  to an independent root systemd unit that survives the cowboy restart. Verify
  its journal and `/run/current-system`; never run a direct switch from cowboy.
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

## Memory / sessions
- cowboy does not own agent memory. Codex uses its native local-memory feature
  through the normal user `CODEX_HOME`; required project guidance stays in
  `AGENTS.md`, docs, tests, hooks, and skills.
- The New Session picker (`GET /api/workspaces`) opens sessions in a Columbus
  project's worktree so the correct project guidance and trusted config load.
  It also projects matching central Columbus work items. Selecting one sends a
  resume prompt into a native Codex task; Cowboy never stores item lifecycle or
  binds it to the session.

## Frontend
Composer (mdlive / CM6), optimistic-send, transcript (column-reverse scroll),
confirm-detection, the Tauri native shell — see `web/AGENTS.md` and the
`cowboy-*` memories.
