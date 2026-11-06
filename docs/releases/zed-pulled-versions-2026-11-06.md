# Zed 1.25.0 pulled diagnostics attributed to their version — Hawk, 2026-11-06

## Why

After the [worker pin advance](worker-pin-code-routing-2026-11-06.md) put Zed
1.24.0 into service, `gopls` evidence was always `unknown`/`unversioned`: Zed
merged pulled diagnostics (`textDocument/diagnostic`) without a document
version. Private server `1.10.0` records the version each server last received
before the pull is sent and merges the result as published for that version; the
server answers after every earlier change notification. Adapter `1.25.0` no
longer disables evidence for the whole process after one slow reply.

## Source and artifacts

- Source `7f6868b08`; complete `just check-compact` green; adapter 139 tests;
  private server `cowboy_` suite including
  `cowboy_frontier_attributes_pulled_diagnostics_to_the_version_pulled`.

| Component                   | Version | SHA-256                                                            |
| --------------------------- | ------- | ------------------------------------------------------------------ |
| `cowboy-zed-adapter`        | 1.25.0  | `76d642a958effc2a34f4f7383e1df2c3beef57bce26b3a004c0d24157fa391e9` |
| `cowboy-zed-server`         | 1.10.0  | `12178c9eb342d95d1762976d4417c2fc61720bf590b0b343b1117b42e15789e9` |
| Package                     | 1.25.0  | `ab3519eae547453235ed9a68c23f2daa37f0459da3d60a07fd243892dd9d2385` |
| Signed artifact (composite) | 1.25.0  | `b43c2c4cd018adc75a5c3f44ee6086afe67e8d4f8713f1e0351f70f109d0c198` |

## Acceptance and installation

- `zed-plugin-conformance` and `zed-native-navigation-conformance` passed on the
  exact pair.
- Rehearsed on a Catalog copy (six files added, none changed), published at
  `2026-11-06T04:03:58Z`; all three public URLs serve the exact bytes.
- `hawk-zed-1-25-0-converge`: HTTP 204, phase `completed`, Machine receipt
  `applied`; zero Zed session leases.

Production verification is recorded below once the idle route has moved.
