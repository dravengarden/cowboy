# Build & deploy

cowboy ships as a **single self-contained binary**: the React SPA is built and
**embedded** into the Rust executable, so there is no static-asset directory to
deploy alongside it. It runs as a NixOS systemd service on hawk (`:3333`).

## The build graph

```mermaid
flowchart TB
    SRC["web/src + deno.lock"] --> DEPS["deps FOD<br/>(vendored npm cache)"]
    DEPS --> WEB["cowboy-web<br/>buildDenoViteApp → dist"]
    WEB --> EMB["preBuild: cp dist →<br/>web/dist (embed dir)"]
    EMB --> BIN["cowboy<br/>buildRustPackage<br/>(rust-embed)"]

    style WEB fill:#eef2ff,stroke:#6366f1
    style BIN fill:#dcfce7,stroke:#16a34a
```

- **`cowboy-web`** uses the shared `buildDenoViteApp` builder from the
  `shared-utils` flake — a deps-only FOD (vendored npm cache, keyed by
  `depsHash`) plus a normal offline build. The builder also stages the
  `@shared-utils/ui` SDK into `web/src/_shell`. Refresh `depsHash` only when
  `web/deno.lock` or `web/package.json` change.
- **`cowboy`** is a `buildRustPackage`. Its `preBuild` copies the web output into
  `web/dist`, which the `#[folder = "web/dist"]` `rust-embed` macro embeds at
  compile time. It pins crates via **`cargoHash` / `fetchCargoVendor`** (not
  `cargoLock`): crates.io now 403s the download endpoint for requests with no
  User-Agent, and the plain-fetchurl `cargoLock` path sends none. `git` is on the
  build's `nativeBuildInputs` because the memory store's unit tests `git init` a
  temp store offline.

## Dev workflow

The `justfile` is the task surface (run inside `nix develop`):

| Task | Does |
|---|---|
| `just install` | `deno install` → `web/node_modules` |
| `just dev` | `cargo run -- serve` (foreground daemon) |
| `just dev-web` | Vite dev server with HMR, proxying `/ws` + `/healthz` to the daemon |
| `just build-web` | build the SPA bundle |
| `just build` | `build-web` then `cargo build --release` |
| `just check` | `fmt` + `lint` (clippy `-D warnings` + deno lint) + `typecheck` |

All Rust builds in the dev shell go through **sccache** (`RUSTC_WRAPPER`), with
`CARGO_INCREMENTAL=0` because incremental compilation and sccache conflict —
disabling it maximizes cache hits. The hermetic `nix build` uses Nix's own
crane/cargo caching instead.

## CLI / daemon flags

`cowboy serve` is the daemon. The flags that shape a deployment:

| Flag | Default | Purpose |
|---|---|---|
| `--bind` | `127.0.0.1:3333` | listen address |
| `--workspace-root` | `.` | root the session pickers scope to |
| `--postgres-url` | — | enable persistence ([Storage](05-storage.md)); in-memory if absent |
| `--memory-enabled` | off | turn on the memory subsystem ([Memory](08-memory.md)) |
| `--memory-root` | `~/.agents/memory` | the memory store path |
| `--memory-janitor-provider` | `codex` | which agent runs the janitor |

Other subcommands: `serve-acp` (the ACP server face for Zed), `try-agent`
(one-shot provider smoke test), `mem` (the memory write CLI).

## NixOS service shape

cowboy is consumed by the hawk config via a `git+file://` flake input from this
repo. To ship a change: commit here, then on hawk `nix flake update cowboy` +
`nixos-rebuild`. The unit runs **as the human SSH user** (so the agent and a
Zed-over-SSH session share one identity and one view of the files), points
`--workspace-root` under that user's home, and sets
`CLAUDE_CODE_REMOTE_MEMORY_DIR` so spawned agents' auto-memory lands in the
machine store.

## The deploy gotcha

**A `nixos-rebuild switch` that restarts cowboy kills the approval channel for
any in-flight turn you are driving Claude *through*.** The switch tool reports
"rejected" even though the rebuild actually **completed** — verify via
`/run/current-system`, don't re-run. Web/bundle changes additionally need a PWA
hard-reload (a WS reconnect alone keeps the stale JS); bump `sw.js`'s `VERSION`
to trigger the auto-reload onto the fresh bundle.
