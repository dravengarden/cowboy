# Execution binding readers, 2026-10-01

Controller revision `45c89f797e1e172582664122dddc694e7e26db8f` activated on
Hawk through the machine-owned component activator. Transaction
`1790860814671440389-45c89f797e1e` committed successfully at
`2026-10-01T13:20:37Z`.

- Release: `/nix/store/1d11izxmsknn9708qlplz6navwg2sdgs-cowboy-controller-release`.
- Scope: binding persistence and strict readers, target Code routing, unsupported
  runtime recovery fences, and uncached rejection of unavailable routes.
- `just check-compact` passed: 1,695 Rust library tests, 22 isolated PostgreSQL
  tests, feature/lint/Provider checks, Web checks and the release build. The final
  Code-read fix also passed its 44 focused tests and all-target/all-feature lint.
- [Connected receipt](../experiments/execution-bound-code-2026-10-01.json): all
  35 checks passed against this exact Controller and the retained Machine
  artifact, using disposable authenticated and enrolled fixtures. This includes
  target reads with the runtime disconnected, explicit malformed-binding refusal,
  and stale-route refusal after reconnect and Controller restart.
- Production `/healthz` returned `ok`. The unchanged Web `/version` and index
  ETag were `30bcbc0bc1c03cf3b8747c7e762ce3bb`; the index retained `no-store`.
  Hawk and OVH both reported `connected: true` with their previous worker
  generations and workspace identities.
- Before/after Hawk worker unit/PID snapshots were byte-identical (SHA-256
  `ffb25f0dda8ffc3a87ee7a1d38101018493e3ad5b7563ff342317f50341191cc`).
  This release did not activate a Machine, Provider or Web component.

The component receipt records `published: false` because activation preceded
publication. The task publishes this revision and its evidence commit to `main`
separately; the original machine-owned receipt remains unchanged.

This is a reader deployment. Bound-session creation and native remote Agent
execution remain disabled. It does not establish the active/recovery/cold reader
floor for a future binding writer, native conversation resume, Claude tool
dispatch, subscription billing, or model-token/turn comparisons. See the
[execution environment contract](../execution-environments.md) for the remaining
acceptance work.
