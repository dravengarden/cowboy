# Native reload input/cleanup candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not production
publication, installation or activation**. Owned navigation admission remains
closed. This does not complete the Plugin refactor or independent recovery.

Accepted implementation/test source:
`fe68eca8ac8997cbfeec7199ab30c3e04ce18d9b`, descended from published
`61031f4dac250263d0e467daa3b0fe2375231bc4`. The
[reload contract](../plugin-native-reload-bounds.md) describes the finite bounds.

## Change and regression

Zed Plugin/private adapter advances `1.15.0` → `1.15.1`; private server advances
`1.2.0` → `1.2.1`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, all third-party pins, component
release `3.11.0`, Plugin SDK `1.8.1`, Machine protocol 21 and adapter API 1
remain unchanged. No applied SQL or historical registry bytes change.

The previous immutable `1.2.0` server reproduced the gap: a file opened while
small, then grown beyond 4 MiB, still received a successful native reload
transaction. This expected negative result is retained, not counted as a pass.

- Route the existing LocalFile byte reader through the bounded descriptor
  primitive: 4 MiB plus one overflow sentinel, no symlink/nonregular fallback.
- Check raw bytes before decoding and decoded UTF-8 before diff/CRDT mutation,
  including forced encodings. Oversized or binary input cannot apply a prefix.
- Retire completed local reload tasks on all exits. Compare the original
  nonserialized invocation identity so old cleanup cannot cancel a replacement,
  including one started by a completion observer.

Refusal preserves original text/vector and disk bytes. Task cleanup does not
retry, clear ownership Unknown, release a native owner or grant synchronization.
The headless watcher only emits ReloadNeeded; no automatic reload bridge or
owned-read fallback is introduced.

## Exact candidate

The repository-owned runtime builder ran from the clean committed source above.
Both executables passed static ELF checks and isolated credential-free probes.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.15.1` | `/nix/store/pfjapl3syz6mhjbz56vxd3239z9ppvfg-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.15.1` | `c52d60f670e2e87c0bf9d1d2343c1e331d72f47e6e90a1066eed58fcab339317` |
| Server `1.2.1` | `/nix/store/smp6xxzam6qbgp0sh4zvgqf77nsia67n-cowboy-zed-server-x86_64-unknown-linux-musl-1.2.1` | `14009143a11891891f2c0b789e37dd44ee2d2369b008cd1276f306a8e1cf80e3` |

The generic SDK built the package and bound both runtime components. The
retained envelope has an **empty production signature**. Its content-addressed
URLs are planned locations, not publication receipts or installable Catalog
entries. Temporary fixture signing is separate.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `d5bbb86dcd339d6db72c147d77fabfa40cefa9228a029cd13baa6e22e9b4e060` |
| Composite artifact | `e686b012148d94f9f284f1239c273d573ddba96c76697bb3af95e5a3b513720b` |
| Contract fingerprint | `9a92c558b5721620401ce225503796477869cbe01711e64539b5fdb22fc3a22d` |
| Build receipt file | `6c8d61ae83364831f29d1c04b8c8816578286098086f094a94babaf361e3afc2` |

## Accepted gates

All build/test commands used the pinned shell. Final connected checks and runtime
packaging ran on clean source; no acceptance documentation was edited mid-gate.

- `just check-compact`: Rust main 1,479 passed / 34 explicitly ignored; bridge 3;
  standalone Machine 368 / 4 ignored; core Code adapter 26; private adapter
  124 / 2 ignored; Web 1,796; isolated PostgreSQL 18. Formatting, Clippy,
  strict types, dependency/component/feature boundaries and optimized builds
  passed. Existing dependency and Vite warnings were not suppressed.
- Final Nix native build: **24 tests** (filesystem 3, LSP 4, project 17).
  Five new reload groups cover raw growth, UTF-16/forced-encoding expansion,
  binary input, exact-limit acceptance, absent/untitled files, lost observers,
  replacement and completion-observer reentrancy. Original version/text and
  independent later operation remain correct after rejection.
- `zed-native-navigation-conformance`: exact static pair passed. The source-test
  adapter additionally sends real legacy reload RPCs to the exact server,
  requiring actual error replies for raw/decoded/binary/symlink refusal,
  unchanged original vectors/text/files, effect-free conditional preparation
  after failure, and a separately initiated successful reload. That legacy call
  exists only in the isolated test, never as a product read fallback. Existing
  five-kind navigation, handoff, input bounds, one-use Open and confirmed Close
  regressions remain accepted.
- `zed-plugin-conformance`: temporary signing/installation, uninstall drain,
  reactivation, nonempty navigation, retained destination reads and independent
  release passed. Synthetic authority and forced fixture teardown are not
  production authority or native recovery.
- `code-buffer-connected-conformance`: all **18 v5** checks passed in 130.76 s.
  Actual disposable login, enrollment and signing/installation were used.
  Three real replies were discarded through the unchanged 40-second deadlines;
  original-ID observation did not replay effects. Receipt:
  `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`.
- Firefox 151.0.1 browser ownership suite: **24** checks passed; unchanged wire
  fixture SHA-256 `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This is not physical-device acceptance.
- Source boundary passed:
  `/nix/store/bbnajmw0rf9wg15lmgwb5dljk3sjx8si-cowboy-source-boundary`.

The connected gate supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and
Machine `/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`,
both from `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled
as this source or accepted as active host roles. Core adapter SHA-256 remains
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicit test-only LSP SHA-256 is
`701a16575a39aa45a0943c98f950fe6bfa6b4f76b21315ede0b7a2b4d6d23833`;
its synthetic answers are neither production language semantics nor an ambient
or packaged dependency.

## Evidence and remaining boundary

Private evidence is retained at `/tmp/cowboy-native-reload-mmhj8e`, including
the exact input, runtime matrix, build receipt, unsigned package/envelope and logs.

| Evidence file | SHA-256 |
| --- | --- |
| `full-accepted.log` | `a560529d2ab210b8ce84062c071c1e822a2e359c1af75624d2c2723177dfd286` |
| `native-final-build.log` | `d7f54ede085d3ea920e55d1fbdc03e0aa974b2e1af89e5d5d997a1bb92be0012` |
| `native-accepted.log` | `c303330a930c4b4da44e9695df71256569adb68e0f144a1e444cca9c37aba839` |
| `lifecycle-accepted.log` | `889b3291dee41e8fd5b1ce58ed1fbd284a4b6aa6e4fc363d74e4531b0ffe3359` |
| `connected.json` | `8dbed5ad54c92ba8a8d4a618e09013e2d1ef8e64befe5cdd9352bdbdf9e02ed2` |

Earlier development compilation/lint failures, the old-server negative test
and preliminary builds are retained separately, not substituted for these final
gates. No production Catalog, account, policy, installation, component pointer,
Machine or session changed.

These are per-input and per-invocation limits, not global retained-history,
background-concurrency, decoding-scratch or process-RSS bounds. Independent
post-effect recovery, signed publication/installation, actual deployed
native-generation and supported-device acceptance, and general graph/site/state
leases remain in the [completion ledger](../plugin-refactor-completion.md).
