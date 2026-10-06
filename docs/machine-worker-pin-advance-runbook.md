# Advancing the retained worker pin on Hawk — runbook

Written from what was run and read during the October 5–6 work, to make the next
advance quick. It authorizes nothing: advancing the pin needs the user's explicit
go, and the consequence below must be accepted first. Items marked *expected* were
reasoned, not observed.

## What it does

The Controller takes a Machine's desired worker generation from its Active
`AcpRuntime` component and sends it on every connection (`SetDesiredGeneration`).
The Machine then marks every worker whose generation differs for `Drain`: idle
workers are replaced at once and busy ones after their current turn, restored
through native resume. So a pin advance is a **real native-generation replacement of
every live Hawk worker**, including concurrent tasks, not a host-only fix. Record the
live worker list before activation.

## When it is needed

`cowboy-machine-host-release` and `cowboy-machine-writer-host-release` assert that
`Cargo.toml`, `Cargo.lock`, `runtime_wire.rs`, `execution_protocol.rs`,
`execution_environment.rs`, `machine_protocol.rs` (and its directory) and the SDK
sources are byte-identical to the `cowboy-workers` input
(`retained-worker-interface-compatible`). Any change to them refuses a host-only
build until the pin advances. Landing such a change on `main` blocks every other
task's resident-only Machine release until then, so land it together with the pin
commit, not ahead of it.

## Steps

1. **Choose the revision `R`** (the main commit whose worker generation is wanted,
   with the wire changes included). *Expected:* the pin is a `git+ssh` URL to the
   GitHub repository, so `R` must exist there before the pin commit is built.
2. **Independent acceptance of `R`**, all against `.#cowboy-machine-release` of `R`,
   disposable loopback fixtures, no production credential, a **new receipt path every
   run** (reusing one fails the input check, not the Provider):
   - `just execution-worker-conformance` (Codex; Claude needs its own input). Input:
     `native_cli`, `sha256`, `version`, `keeper` (the release's `cowboy-execution-host`)
     and `adapter_launcher`, which is the packaged `app/cowboy-launch.mjs` of the active
     Codex generation, not `codex-acp`.
   - `just execution-session-conformance`. Input needs `"browser_device": true`
     (otherwise the Controller refuses the HTTP origin and the harness reports a
     timeout) and the native `.cowboy-machine-wrapped` ELF as `machine`, because the
     release wrapper already passes `--desired-generation`.
   - `just agent-worker-conformance` for Codex and for Claude coexistence. The harness
     wants `<id>-<version>.release.json`, its sibling `.cowboy-plugin`, and
     `artifacts/artifacts/<digest>/<file>` as **regular files** (the artifact root is
     that outer `artifacts` directory), and an absolute, symlink-free worker path.
   - `just logs-conformance`.
   - Not yet done and needed: Claude native execution for the repository's current
     Claude plugin (its runtime has to be built through the release skill) and connected
     Code with the exact native Zed pair.
3. **Pin commit.** Change the `cowboy-workers` revision in `flake.nix`, update
   `flake.lock`, keep it a small commit on top of fresh `main` (as `48d3054d` did).
4. **Build** `.#cowboy-machine-writer-host-release` and `.#cowboy-machine-host-release`
   from the committed tree (dirty or revisionless sources are refused). Check each
   `source.json` for the new `workerGeneration` and `retained-worker-source.json`
   for `R`.
5. **Native production conformance** (`just session-deletion-production-conformance`)
   requires the old writer, new writer and reader-only fallback to share **one worker
   generation** (the script asserts it), so the currently active old-generation writer
   cannot be the "old" side of an advance. Use a writer and a reader-only release built
   at two different revisions that carry the new pin, as done on October 6
   with builds of `48d3054d`, `331ed2d1` and later commits, which all carry the same pin. Always pass literal store roots and a fresh
   receipt path (a root-written receipt cannot be removed from the sticky `/tmp`).
6. **Dispatch** through the installed owner from a current Columbus worktree:
   `nix develop -c just --justfile machines/justfile cowboy-machine-activate <writer store root>`.
   The owner refuses a Cowboy revision that does not contain freshly fetched
   `origin/main`; integrate, rebuild and dispatch in one tight sequence. Dispatch
   success is not deployment success.
7. **Verify** the component receipt (`succeeded`, `committed`), the root unit journal,
   startup logs (`writer_enabled=true`, the incarnation namespace line), `/healthz`,
   `/version`, the SPA and service-worker cache headers, Machine deployment-health, and
   compare worker/keeper IDs, PIDs and states, the Controller PID, the deletion and
   incarnation floors, sudoers digest, host source and the Controller/Web receipts
   before and after. A pin advance will change worker PIDs by design; record which.

## Boundaries

A Controller release restarts only `cowboy.service`; a Web release moves only
`/run/cowboy-web`. Neither authorizes this. Do not edit sudoers, delete local-auth
pins or authority markers, or replace a worker by hand to make a step pass.
