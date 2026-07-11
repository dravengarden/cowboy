# cowboy — agent guide

Drive Codex over ACP from anywhere. A single Rust (axum) process serves both
the API and the embedded React SPA, with
one agent subprocess per session. Deployed as a NixOS service on hawk (:3333).
Frontend specifics live in `web/AGENTS.md`; this is the cross-cutting layer.

## Deploy (read before deploying)
- cowboy is a NixOS service (`services/cowboy` on hawk), consumed via a
  `git+file://` flake input from this repo. To ship: commit here, then on hawk
  `nix flake update cowboy` + rebuild.
- **Deploying restarts the daemon you may be driving Codex through.** A
  `nixos-rebuild switch` that restarts cowboy kills the approval channel for the
  in-flight turn — the switch tool reports "rejected" even though the rebuild
  actually COMPLETED. Verify via `/run/current-system`; don't re-run. Web/bundle
  changes also need a PWA hard-reload (a WS reconnect keeps stale JS).
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
- cowboy-spawned agents inherit `CLAUDE_CODE_REMOTE_MEMORY_DIR` (set on
  `cowboy.service`) → their CC auto-memory lands in the machine-level mnemosyne
  store, keyed by the session's cwd-slug. The New Session picker
  (`GET /api/workspaces`) lets you open a session in any columbus project's
  worktree, which is what makes per-project memory + AGENTS.md load.

## Frontend
Composer (mdlive / CM6), optimistic-send, transcript (column-reverse scroll),
confirm-detection, the Tauri native shell — see `web/AGENTS.md` and the
`cowboy-*` memories.
