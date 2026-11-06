# Zed 1.24.0 open admission confined to the target — Hawk, 2026-11-06

## Why

A buffer being closed or reserved for synchronization refused every new open on
the Machine, since another path could alias its native ID. Private server
`1.9.0` answers `CowboyOpenTarget` with what an open of a path would return,
using `open_buffer`'s own lookup and opening nothing; adapter `1.24.0` refuses
only opens whose target is a reserved ID, a load in flight or unknown. See
[single-use native buffer acquisition](../plugin-native-open-once.md#reservations-confined-to-the-open-target-zed-1240).

## Source and artifacts

- Source `0f366f312`; complete `just check-compact` green; adapter crate 139
  tests (admitted unopened path and unrelated buffer; refused alias and loading
  target, lease kept Prepared); private server `cowboy_` suite including
  `cowboy_open_target_reports_what_an_open_would_return`.

| Component                   | Version | SHA-256                                                            |
| --------------------------- | ------- | ------------------------------------------------------------------ |
| `cowboy-zed-adapter`        | 1.24.0  | `39f520b9c526ffd885f0cf44a26c6dbd5cf97e98e28445659d20300f163b52f4` |
| `cowboy-zed-server`         | 1.9.0   | `1b11e9eb27e018f51f64daed0a0e0b66b0e8d3ef1f6f978ef1cbbc34c172b762` |
| Package                     | 1.24.0  | `c9e294943f9213e934eb09f3d1b053fb1c1d96f9c246b80eabf798ff3844f642` |
| Signed artifact (composite) | 1.24.0  | `1e7e3adce4b283245eea5eff6464b22bacc2d7256243bb89f05f5641e9c65740` |

## Acceptance before publication

| Gate                                | Result                                                                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `zed-plugin-conformance`            | `released_zed_runtime_installs_and_drains` passed                                                      |
| `zed-native-navigation-conformance` | passed: the real server refuses a reserved buffer's own path and admits an unopened one; 1.23.0 checks |

## Publication and installation

- Signed with `cowboy-first-party`, verified, rehearsed on a Catalog copy (six
  files added, none changed), published at `2026-11-06T01:14:36Z`; all three
  public URLs serve the exact bytes.
- `cowboy operator converge --machine hawk --plugin zed --apply`: operation
  `hawk-zed-1-24-0-converge`, HTTP 204, phase `completed`, Machine receipt
  `applied`
  (`installation-623abbb09b5631643f23842f79c50ba37d5a4b3387f4cb2c568cf229866afe59`);
  Zed `1.24.0` active, rollback generation 1.23.0
  (`sha256:52ab45cc23db021050f27215b3d318330f1582292df31ca27675e05fded1292a`).
  Zero Zed session leases; no Machine or Controller restart.

## Still open

Unresolved navigation and synchronization Unknowns keep process-wide or
per-buffer fences without same-process recovery; interactive priority and
per-Machine language-server budgets
([20](../architecture/20-agent-code-intelligence.md#acquisition-gated) gates 3
and 4) are not built. Automatic acquisition stays off.

## Correction (2026-11-06)

Installation did not put this generation into service for existing worktrees:
their code-runtime routes stayed on the generation they started with. It served
only after the [worker pin advance](worker-pin-code-routing-2026-11-06.md)
restarted the Machine with idle routes following the active generation.
