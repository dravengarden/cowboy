# Zed 1.23.0 same-process native open recovery — Hawk, 2026-11-06

## Why

One unresolved native open (cancelled caller, lost or invalid reply, missing
initial share) latched a process-wide fence that refused every later acquisition
on the Machine until the adapter process ended. That made any automatic
acquisition unsafe. The owner chose automatic, query-only recovery (2026-11-05).
Private server `1.8.0` records each peer's `OpenBufferByPath` by message ID and
answers `CowboyOpenAttempt`; adapter `1.23.0` resolves a latched fence from that
record before the next acquisition. See
[single-use native buffer acquisition](../plugin-native-open-once.md#recovery-from-the-same-native-process-zed-1230).

## Source and artifacts

- Source `2302cd5f3`; complete `just check-compact` green; adapter crate 138
  tests (orphan release, failed/unreceived, alias and pending recovery cases);
  private server `cowboy_` suite including the open-attempt registry.
- `just zed-plugin-runtime-build https://cowboy.stormbird.xyz/plugin-artifacts`:

| Component                   | Version | SHA-256                                                            |
| --------------------------- | ------- | ------------------------------------------------------------------ |
| `cowboy-zed-adapter`        | 1.23.0  | `45b19baf9c3ff6d505cde82e076828faa82618df48d347234751bd5d0cbd36c6` |
| `cowboy-zed-server`         | 1.8.0   | `611b3bca6eebd0c04f9db15ba5a7233ffcb26e062671bf7cd52553c301b47097` |
| Package                     | 1.23.0  | `22749005a4eb573c3a5978607062cddb7e68e31eb2f3859ec05bfe77a47c7de8` |
| Signed artifact (composite) | 1.23.0  | `52ab45cc23db021050f27215b3d318330f1582292df31ca27675e05fded1292a` |

## Acceptance before publication

| Gate                                                                             | Result                                                                                                                 |
| -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `zed-plugin-conformance` (signed install, leases, uninstall drain, reactivation) | `released_zed_runtime_installs_and_drains` passed                                                                      |
| `zed-native-navigation-conformance`                                              | passed: an uncommitted real open is released as an orphan; a refused oversized open is recovered and replacements open |

The native gate's expectation changed with this release (`f70642a91`): a failed
open no longer refuses every replacement; the original lease stays Unknown and
nothing is replayed.

## Publication and installation

- Signed with `cowboy-first-party`, verified, rehearsed on a copy of
  `/var/lib/cowboy/plugins/catalog` (six files added, none changed), published
  at `2026-11-06T00:40:37Z`. All three public URLs serve the exact bytes.
- `cowboy operator converge --machine hawk --plugin zed --apply`: operation
  `hawk-zed-1-23-0-converge`, HTTP 204, phase `completed`, Machine receipt
  `applied`
  (`installation-d08878d849b1c1ff6d76c101148a6dd8e7235d604a224e125c6e5c920994924c`);
  Zed `1.23.0` active, rollback generation 1.22.0
  (`sha256:022d96ddd5340085eeeda6c462dcf9b02ac966bba610ad828d8646167b0a6966`).
  Zero Zed session leases at upgrade; no Machine or Controller restart.

## Still open

Navigation and synchronization Unknowns keep their own fences, and the admission
gate is still process-wide while nothing is unresolved
([20](../architecture/20-agent-code-intelligence.md#acquisition-gated) gates 1,
3 and 4). Automatic acquisition stays off.
