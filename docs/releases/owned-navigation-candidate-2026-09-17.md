# Owned native navigation candidate — 2026-09-17

This accepts a **private source/candidate slice**, not production activation,
Catalog publication, Plugin installation or public navigation. The
[finite contract](../plugin-owned-navigation.md) preserves original target
identity/content/epoch and never retries an ambiguous native acquisition.

## Immutable inputs

Clean source: `8578887866d5e4b5216be95542ec57e33d1bd393`, rebased onto
`60ab09c0` including the independent credential-observation repair. Subsequent
documentation and the upstream documentation-only merge do not replace the
tested candidate bytes. Only Linux x86_64 is declared. No third-party dependency
pin, server source, shared component version or historical registry entry
changed.

| Candidate                            | SHA-256                                                            |
| ------------------------------------ | ------------------------------------------------------------------ |
| Zed adapter `1.10.0` ELF             | `c508bdaee13a4f84a9e689b1da3f05dd22d63cff2be04576edd7688c8bc9d366` |
| Unchanged private server `1.0.0` ELF | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Machine release entrypoint           | `f5e11c5b039051ad69db348bd484f9a5314d7e2c533803a8bbfb58a4646e1b20` |
| Machine wrapped ELF                  | `10f839704039ba25c6ae01cade0b681d014b663fb290d03e63734f6d9d9a97e1` |

Immutable outputs:

- Adapter:
  `/nix/store/d859d9kxkp7nzdf2rgnvvrbifn2y9w3m-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.10.0`.
- Server:
  `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0`.
- Machine: `/nix/store/y3r98cq00nbx8xy1rir2bg23iblj3v34-cowboy-machine-release`,
  candidate worker generation `worker-14de5ccda3070c5d5108`.
- Machine wrapped ELF:
  `/nix/store/yh814z1qda1vj042hqa2gzh4xi6x0qq0-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.

The package-owned runtime builder verifies static ELF dependencies and runs
credential-free probes. Its first invocation could not locate child `nix` in the
worker PATH; the corrected caller supplies `/run/current-system/sw/bin`. The
final clean-source build passes, without changing the build recipe or dependency
pins. Generated artifact URLs are candidate metadata, not evidence that these
bytes were published at those URLs.

## Verified behavior

The final pinned `CARGO_INCREMENTAL=0 just check-compact` passes: **1,429 main
Rust, 340 standalone Machine, 26 core-adapter, 88 private-adapter, 1,730 Web and
18 isolated PostgreSQL tests**, plus three Codex bridge tests, formatting,
strict Clippy, dependency/type/feature checks, component/package isolation,
composition conformance and release builds. The 13 new private-adapter tests
cover bounded preparation, source/target identity, edit/undo, duplicate targets,
alias ownership, deleted paths, disconnected observers, cancellation, invalid
results and failure to enqueue release. ReleaseUnknown retains the original
target evidence as well as possibly live pins. Explicit opt-in tests remain
ignored in the ordinary suite; the relevant process gates below run separately.
Existing Vite chunk warnings are not suppressed.

Three process gates pass against the exact native pair:

1. `zed-plugin-conformance`: temporary signed installation, mixed ownership,
   uninstall drain, original resources after path removal, retained-generation
   reactivation and the existing core synchronization checks.
2. `code-buffer-connected-conformance`: all **11** authenticated HTTP/control/
   native checks, using the Machine candidate above and supplied Controller
   `/nix/store/4yj5b7fpnmjb65jnrsls116g6shimg1g-cowboy-controller-release`
   (source `e8f2c249328964c9a22bd536bedbb80339ce27b6`). Receipt v3 reports
   `accepted: true`, `cleanup: true`, no failure, three connections, six held
   replies, one discarded reply and one connection cut. This includes the real
   40-second lost-Apply timeout without replay. These are supplied artifacts,
   not acceptance of actual host roles, production accounts or policies.
3. `zed-native-sync-conformance`: the existing native synchronization regression
   plus all five real plaintext navigation kinds, one-use execution, retained
   source after parent release/deletion, shared-owner synchronization refusal
   and path-free local release. Plaintext has no LSP destinations: **nonempty
   navigation targets are covered by deterministic protocol fixtures only**.

The Machine source gate rejects all three private navigation commands before
generic runtime selection, with and without a forged worktree. No new Machine
wire protocol, Service endpoint, Web consumer or native bridge capability is
enabled. Fixture cleanup and successful CloseBuffer enqueue are not verified
native recovery, filesystem restoration or production resource reclamation.

## Still open

This is not ready to expose merely by adding a forwarding route. Besides core
acquisition authority and original-generation handoff, actual nonempty-LSP
acceptance must address bounded native acquisition and ambiguous outcomes. An
oversized, cross-worktree or invalid result can currently retain Unknown and its
conservative process-wide admission fence. Adapter result limits and native
worktree identity are not an OS isolation or native allocation-budget proof.
There is no independent recovery for a lost native LSP result.

Service/principal/Session and browser destination ownership, exact target text
presentation, supported-device acceptance, separately authorized native rollout,
general graph/state leases and post-effect recovery remain unfinished. No
production Catalog write, installation or component activation was performed by
this task. No claim is made about concurrent host changes from other tasks.

Raw local evidence is under `/tmp/cowboy-navigation-Ju28y7Xo`:
`check-final.log`, `runtime-build-final.log`, `machine-build.json`,
`machine-build.log`, `process-gates.log`, `connected-input.json` and
`connected-receipt.json`.
