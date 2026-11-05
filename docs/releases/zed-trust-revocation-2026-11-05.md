# Zed 1.21.0 native trust revocation — Hawk, 2026-11-05

## Why

At the pinned Zed revision `RestrictWorktrees` only records a restriction, so
turning "Project language servers" off left servers started while trusted
running. Adapter `1.21.0` restricts the worktree at once, refuses every language
query on it (including owned-buffer reads), and once no lease or buffer remains
removes the native worktree, dropping the servers rooted only there, and adds it
back restricted. See [14](../architecture/14-zed-code-provider.md#trust).

## Source and artifacts

- Source `be2e6c781` on `main`; complete `just check-compact` green; adapter
  crate 128 tests, including two new trust-revocation tests.
- `just zed-plugin-runtime-build https://cowboy.stormbird.xyz/plugin-artifacts`
  from the clean commit:

| Component                   | Version           | SHA-256                                                            |
| --------------------------- | ----------------- | ------------------------------------------------------------------ |
| `cowboy-zed-adapter`        | 1.21.0            | `eabe864af684cffbe45f3124f9292de20c7c8159d221f441d197f5d1c4867992` |
| `cowboy-zed-server`         | 1.6.0 (unchanged) | `d84c0516b2dc872b2c3ac97a23a972056532249cea7daa6414274fee877014c3` |
| Package                     | 1.21.0            | `07ac9f7e4a294b01745c6b56b52edf6acc9a33d7a87947b1985f70aa14337def` |
| Signed artifact (composite) | 1.21.0            | `74c77d02c57f1f76e3020dccba94b8dcfa5f287ccc6eb486b9af5cf79af451a1` |

## Acceptance before publication

| Gate                                                                             | Result                                            |
| -------------------------------------------------------------------------------- | ------------------------------------------------- |
| `zed-plugin-conformance` (signed install, leases, uninstall drain, reactivation) | `released_zed_runtime_installs_and_drains` passed |
| `zed-native-navigation-conformance`                                              | all native sync/navigation/close stages passed    |

Both native gates must run with `CARGO_BUILD_BUILD_DIR` unset: the fixture
locates `examples/navigation_lsp` beside the test binary, which a separate Cargo
build directory moves.

## Publication and installation

- Signed with `cowboy-first-party`, independently verified, rehearsed on a copy
  of `/var/lib/cowboy/plugins/catalog` (exactly five files added, none changed),
  then published there. Both new public URLs serve the exact bytes.
- `cowboy operator converge --machine hawk --plugin zed --apply`: operation
  `hawk-zed-1-21-0-converge`, phase `completed`, Machine receipt `applied`
  (`installation-3443961aadeef80c8f5f25bb328fe2ac0fccf2e0ee0b52ec4eea252657cb6706`);
  inventory reports Zed `1.21.0` active. Zero Zed session leases at upgrade; no
  Machine or Controller restart.
- Falcon and macbook-air have no Zed installation and were not changed.
