# Colocated execution needs permission, not a claim — 2026-09-24

Closed the trust gap recorded in the two preceding root-identity slices.
`colocated` decides **which** of those two fences applies — whether a Machine's
Code reads execute on the Machine or on the Controller's own filesystem — so it
is itself a trust boundary.

It was taken verbatim from the Machine's self-declared
`hello.connection_mode`, with no transport, peer or enrollment check anywhere.
Enrollment deliberately records `outbound_wss`; the connect path then
overwrote it from the hello. An enrolled remote Machine could therefore declare
local mode, advertise Controller-host paths, and have them read back through
the authenticated Code read API.

A peer-address check cannot separate the cases: `cowboy.stormbird.xyz` is
reverse-proxied to `localhost:3333`, so every proxied Machine looks local. The
declaration is now a **request**, and an explicit operator permission decides:

```
colocated = declared local && named in COWBOY_COLOCATED_MACHINES
```

Both halves are required and neither is sufficient. Naming a Machine never
converts a remote connection into a local one; declaring local mode never
reaches this host's filesystem without the permission. Empty permits nothing.
A Machine that asks without permission is logged and served remotely rather
than silently downgraded.

This is a trust-boundary crossing, not an unauthenticated attack surface: the
Machine is enrolled and challenge-signed. It was never exploited.

## Ordering, and why it was safe in both directions

The permission is an environment variable rather than a flag precisely so the
two repositories could land in a safe order:

1. **Host unit first** (Columbus `2256897eeedefae070c70c7f6e5aabef53b2d2cc`,
   published): adds `COWBOY_COLOCATED_MACHINES=hawk`. An older Controller
   ignores an unknown environment variable, so this could not break the running
   service — whereas an unknown *flag* would have made it refuse to start.
   `cowboy.service` is a retained service, so nothing restarted.
2. **Controller second** (Cowboy `ce9107b4dfce6bdd9eddcdd57896e23aec0823b5`):
   enforces it. Because the unit already carried the permission, activation
   preserved the deployed Machine's existing local execution.

At no point was the deployment in a state where hawk lost local execution.

## Exact releases

- Columbus: revision `2256897eeedefae070c70c7f6e5aabef53b2d2cc`, closure
  `/nix/store/6nh2lmgr14rik7bxsw94ivw2rggnmsi8-nixos-system-hawk-26.05.20260731.5b4f72e`,
  transaction `1790262650138150642-2256897eeede`, outcome `succeeded`.
  `changedUnits.system` is exactly `["cowboy.service"]`; `retainedProcessPids`
  records `cowboy.service: 250339` **before and after** with no violations, so
  the unit changed without restarting the Controller. Health checks passed for
  `cowboy`, `claude-deepseek` and `codex-deepseek`.
- Controller: `/nix/store/mpq91i6fn36fpxwhhbga6qdhr7yp6chg-cowboy-controller-release`,
  source `ce9107b4dfce6bdd9eddcdd57896e23aec0823b5`, pushed to remote main
  before activation. Previous release
  `/nix/store/3drs6cdhjdh97lyvya2a9qb0s0214ypj-cowboy-controller-release`.
  Activation `1790263182722826946-ce9107b4dfce` committed at
  `2026-09-24T15:19:56.72133633Z`: published, succeeded, non-maintenance, no
  recovery.

No Plugin/SDK version, Machine wire protocol, native binary, SQL baseline,
durable format, production role, credential, Machine generation or Web release
changed.

## Gates

- Complete pinned-shell `just check-compact`: 1,635 main Rust tests (34
  ignored), 384 standalone Machine tests (4 ignored), 26 core adapter tests,
  126 private adapter tests (2 ignored), 1,933 Web tests and all 18 separately
  isolated PostgreSQL tests, plus formatting, lint, types, dependency,
  feature/build gates and all 86 structural-link vectors.
- The decision is a named function with its own test covering all four
  combinations, including the two that must be refused: an enrolled remote
  Machine claiming local mode, and a permitted Machine that did not claim it.
  A separate test pins the default — with nothing named, no Machine may be
  read on this host.
- The merged unit was inspected before activation and differed from the
  running one by exactly one line, with `X-RestartIfChanged=false` and
  `X-StopIfChanged=false` preserved.

## Bounded production observation

Between `2026-09-24T15:19:26.172Z` and `2026-09-24T15:20:10.575Z`, all **29**
ACP workers retain their exact PID and kernel start ticks. The resident Machine
remains PID `3706189`, kernel start `349734872`. Both resident Code native
processes and every installed Plugin generation are unchanged. Only
`cowboy.service` restarted, from PID `250339` to `3187789`; `hawk`, `falcon`
and `macbook-air` all reconnected within the window. Local and public
`/healthz`, `/version`, `/` and `/sw.js` return 200 with byte-identical SPA and
service-worker output, and the Controller journal has no warnings after
activation.

**hawk remains colocated.** The running process carries
`COWBOY_COLOCATED_MACHINES=hawk`, the machine record still declares `local`,
and the Controller logs a warning whenever a Machine declares local mode
*without* permission — no such warning was emitted after the restart. `falcon`
and `macbook-air` declare `outbound_wss`, so neither asked for nor received
local execution.

No authenticated production read was driven through the deployed Controller;
that remains a gap here as in the two preceding records.

## Evidence

Private evidence root: `/tmp/cowboy-colocated.evidence`.

| Evidence | SHA-256 |
| --- | --- |
| `check.log` | `4aa9d85048426b567c518fa064df07a3a1e0df72841555345a63a6f54eb24336` |
| `sys-build.log` | `396d3d5ce0b7de1a4131a02cc5fbdda0ffb9053ef22f3142be39220a9f75236d` |
| `sys-activate.log` | `d0345587eec3a2c46968d3eac1af6e070ee8ccd1e7d0e3520bccc030c5142e98` |
| `activation.log` | `2f4b3d52c1952f8fa03ca95d4a006c8adeca17127c9e7f09656d2eb7f7550ad0` |
| `observed-before.json` | `97d171a9acd0fbd36969f6e9dfbf57d742cf9dd158314e95cf045c970f7cfc66` |
| `observed-after.json` | `dbea228e639ccdd6c5910031b49682724ec5903741e2f5729bc24ed84fd1218c` |

## What this unlocks, and what it does not

Honest connected coverage of the colocated branch is now *possible in
principle*: a fixture Controller can name its fixture Machine. It is **not**
done here, so the colocated branch of
[Controller-owned local root identity](plugin-local-root-identity-2026-09-21.md)
still has source-test evidence only.

This does not close Machine-owned Session or security-domain identity, state
reader/writer leases, general graph contracts, independently authorized
post-effect restoration, native-generation replacement or supported-device
acceptance. Those remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
