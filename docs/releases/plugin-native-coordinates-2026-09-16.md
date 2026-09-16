# Zed native-coordinate candidate — 2026-09-16

**Verified source/runtime candidate, not a Catalog publication or installation.**
Zed Plugin and private adapter are `1.5.0`; upstream server stays `1.13.0`.
The owned-read API remains `language | symbols`, and ordinary Review has not
switched to the new owner API. See the
[coordinate design and remaining boundaries](../plugin-native-buffer-coordinates.md).

## Source and integration

Implementation: `66f25f84`; checksum-preserving Nix download correction:
`a2566f72`; corrected real-server expectation: `a3d84152`; empty-base and
per-author causal-history fixes: `70da2ad031524c7903999126dfa10d5b5bafdb55`.
The task first rebased onto `d0232313`, preserving both incoming Composer fixes.
Subsequent merge `b424f1106f54f7e6e6545caf3d5eaca2d5d86f32` integrates remote
`ff733ee5` (Composer source mode). Its only incoming changes are Web files;
`src` and `plugins/zed` are byte-identical to the fully checked `70da2ad0`.
The final runtime build receipt names clean committed `b424f110` and reproduces
the same accepted native binaries. The enclosing follow-up only records evidence.

## Candidate artifacts

- Package SHA-256:
  `45e824bdef4368ec10511d038357501c7a0d4b71adb40d92f18b191b83cbfe54`.
- Bound artifact identity:
  `sha256:eb3e42085bdb5d416449fc835f48a5873c132ee4fd85600b90a3539cc4b6c87f`.
- Adapter: `/nix/store/xvhf838m9hsqkbhgbilzf0kqyiknmq31-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.5.0/bin/cowboy-zed-adapter`,
  SHA-256 `e7cb2ca30fc558192b30173b79383b11d8717fa850750c794531bac104e16e4c`.
- Server: `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0/bin/cowboy-zed-server`,
  SHA-256 `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5`.

The package, unsigned bound envelope and runtime matrix are under
`dist/plugins/zed`. Their HTTPS destinations are planned immutable URLs, **not
proof of upload or public availability**. The envelope signature is empty.
Both binaries pass isolated probes and ELF checks for no `INTERP`/`NEEDED`.
There is no runtime shared-library dependency introduced by the text engine.

## Accepted checks

The complete `nix develop -c env RUST_TEST_THREADS=1 just check-compact` gate
passed on `70da2ad0`: 1,341 main Rust tests, 305 standalone Machine tests,
26 core-adapter tests, 51 private Zed tests, 1,555 Web tests and 17 isolated
PostgreSQL checks, plus lint, types, features, dependency/Plugin/component
contracts and release builds. GNU and musl dependency audits pass. The static
Nix build independently ran all 51 private tests.

After the Web-only integration, Web lint and the typecheck/build passed again;
all **1,561 Web tests** passed. No Composer source was edited by this task.

`just zed-plugin-conformance` passed against the exact final binaries above
(8.39 seconds). It performs genuine temporary signing/install, native worktree
and buffer opens, disk-only coordinate refusal, retained-original reads,
uninstall admission refusal, reads after file deletion/worktree rename, final
owner drain and reactivation. The two independent native owners and legacy
owner retain distinct release obligations. `.txt` has no configured LSP, so
this is not evidence of nonempty real LSP results or real disk-to-native edits.
Nonempty coordinates and edit/undo/redo use pinned-engine and protocol fixtures.

## Failures retained rather than hidden

- Nix's crates.io API downloads returned HTTP 403. Direct official static
  downloads worked with unchanged lockfile checksums. Using `extraRegistries`
  first caused a duplicate Cargo source definition; the final recipe overrides
  only the fetch URL, not registry identity or checksums.
- The first real-process expectation of automatic reload timed out. Fixture
  tracing showed `UpdateBufferFile`, not native edit/reload operations. Reads
  now explicitly prove that disk text is not adopted as native coordinates;
  no hidden `ReloadBuffers`, close/reopen or owner replacement was added.
- A parallel full-gate run failed the existing local-Operator endpoint lock
  assertion at `src/server/local_operator/tests.rs:21` with EAGAIN. Its isolated
  rerun passed; the final complete gate uses `RUST_TEST_THREADS=1`. This does not
  establish the cause or fix that parallel-test intermittency. No production
  lock behavior or assertion was weakened.

Evidence root: `/tmp/cowboy-native-coordinates-EOFWnqQQ` (local scratch, not a
permanent public artifact). Selected SHA-256 checksums:

| Evidence | SHA-256 |
| --- | --- |
| `check-accepted.log` | `6d1cdd4c436612f63c37e8c445d7368dcaaa6b4f84c32dea35f402450b34d547` |
| `native-conformance-complete.log` | `219375a015a83f952504e7a41fc05df39ad1e8842700c676c66d561a8b70f86e` |
| `integration-web-tests.log` | `38535083acd29693c01382863d16a2535a32e7e3639e0c089a9e08f0aac9d9d1` |
| `native-runtime-integrated.log` | `9039961855b14183cd832494d84156a38ae1297d875dc795a2a6f791e63a511f` |
| `dist/plugins/zed/runtime/build-receipt.json` | `5bed13c194d113e7281e4296d506069100dfb3524ec67f87dd39a8bb8e7c6cbb` |
| `dist/plugins/zed/zed.release.json` (unsigned) | `9395ee870464742af773087d845931de192f2a0fcb90674a0090a300e6f6847c` |

No Catalog write, live Plugin install, delegation/policy mutation or application
component activation was issued by this task. Publication would still require
signing and actual-reader acceptance; installation remains a separate action.
Review additionally needs explicit content synchronization authority,
browser/native position certificates and navigation destination ownership.
Machine rollout, supported-device behavior, abandoned-browser cleanup and
independent post-effect recovery remain unclaimed; the Plugin refactor is not
complete.
