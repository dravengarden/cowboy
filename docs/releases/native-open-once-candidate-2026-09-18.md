# Single-use native Open candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not a production
Catalog publication, Plugin installation or component activation**. Production
owned navigation remains closed. This repair does not complete the Plugin
refactor or establish native recovery.

Accepted implementation/test source:
`692d0bf16fb076a3313028ff1b6c5f56dc8f8df1`, descended from published
`cbf1028b3a64d242931ed4a74551d460e7130906` after integrating latest main.
The [single-use acquisition contract](../plugin-native-open-once.md) records
the exact ownership and admission boundary.

## Change

Zed Plugin/private adapter advances `1.14.0` → `1.14.1`. Private server `1.1.0`,
upstream revision `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, all third-party pins,
component release `3.11.0`, Plugin SDK `1.8.1`, Machine protocol 21 and adapter
API 1 remain unchanged. No historical registry or applied SQL bytes change.

- Remove implicit Close/reopen after missing initial sharing. A real already
  shared native buffer returns its original ID without another State/last Chunk;
  the old fallback could close the original peer. The normal five-second
  observation timeout now returns without another native effect.
- Arm a one-use, non-cloneable acquisition attempt before native I/O. Only the
  original active owner commit consumes it; cancellation/error cannot clear its
  process-local admission fence, even before a native ID is known.
- Fence new legacy/owned opens, navigation, synchronization preparation and both
  destination handoff stages after an unresolved Open. Known safe reads and
  independent release remain available; late replies, file changes and unrelated
  cleanup cannot authorize replay or replacement ownership.

## Exact candidate artifacts

Built by the repository-owned runtime builder from the clean source above.
Both executables passed static ELF checks and isolated credential-free probes.
The private server is byte-identical to the previously accepted whole-query
candidate, not rebuilt or relabelled as a new server version.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.14.1` | `/nix/store/g1pbb5x2l4izvs9s47ssncr817wdkld4-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.14.1` | `80ee9b20528e63efd06712b12446cf46ceb8e968763333f748ce43c555869e0d` |
| Server `1.1.0` | `/nix/store/5psz7qz6ly9y4rpjmx526xh7d51vvf8q-cowboy-zed-server-x86_64-unknown-linux-musl-1.1.0` | `39bbf25f202da105c41238a87ee5374beb30ed8b592798fc6f1030cde9799e4e` |

The generic SDK built the package and bound its complete two-component matrix.
The retained envelope has **an empty production signature**. Its content-
addressed URLs are planned locations, not availability receipts or UI-installable
releases. Temporary fixture signing is separate.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `098f26c05e8f95c950abc48380b25965fe37e19551eb06b3de330750c58bf2b4` |
| Composite artifact | `bf0d8d889b7336bc92ee8d5db4186b8026352c21b32da7b5d29de648c011cb56` |
| Contract fingerprint | `84ce3122e6e66f47d1c60bea1dda57a85afed969fd5ad3484d276007667dffab` |
| Build receipt file | `2abfd3dea24ba9a9d5c7353a2ef937c4c707fafe3b8aa045e2cb7fdf2ed521de` |

## Accepted gates

All commands ran from the repository root in its pinned Linux shell.

- `just check-compact`: Rust main 1,479 passed / 34 explicitly ignored; bridge 3;
  standalone Machine 368 / 4 ignored; core Code adapter 26; private adapter
  115 / 2 ignored; Web 1,796; isolated PostgreSQL 18. Formatting, Clippy, strict
  types, dependencies, component/feature boundaries and optimized builds passed.
  Existing dependency and Vite chunk warnings were not suppressed.
- Nine additional adapter tests cover all three native-await cancellation
  boundaries, late replies, malformed Open/registration replies, zero IDs,
  broadcast loss, real timeout, early shares, one-use commit, legacy admission,
  independent known text read/release and both destination handoff stages.
- `zed-native-navigation-conformance` against the exact static pair passed.
  The actual native duplicate Open hits the normal initial-share timeout, after
  which the original mirror and native peer ownership still pass. The immutable
  socket fixture additionally rejects a real oversized file, retains the same
  Unknown despite source repair, refuses changed-source and known-key replacement
  owners, and reads/releases another original owner. All five nonempty navigation
  kinds, typed whole-query refusals, handoff, input and synchronization regressions
  remain green. Deterministic language answers are not production LSP acceptance.
- The unchanged server's retained Nix build evidence contains its same 15 native
  tests (filesystem 3, actual LSP reader/parser 4, synchronization/navigation 8).
  This is reused evidence for identical bytes, not a new server test execution.
- `zed-plugin-conformance`: actual temporary signing, installation, uninstall
  drain/reactivation, nonempty navigation and independent original-target reads.
  Synthetic Machine authority and forced fixture teardown are not production
  authority or independently verified cleanup.
- `code-buffer-connected-conformance`: all **18 v5** checks passed, including
  actual disposable password login, enrollment and signed installation. Three
  genuine replies were discarded and their normal 40-second deadlines retained.
  Original-ID observations never replayed Apply, Execute or Release.
  `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`; 130.81 seconds.
- Firefox 151.0.1 `code-buffer-browser-conformance`: all **24** cases passed;
  unchanged fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This is not physical-device acceptance.
- `checks.x86_64-linux.cowboy-source-boundary` passed:
  `/nix/store/aj0845cmpbzclnwp1xgskaj5lkbygvi1-cowboy-source-boundary`.

The connected gate deliberately supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and Machine
`/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`, both from
`f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled as this source
or accepted as active host roles. Core adapter hash remains
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicit test-only LSP hash is
`4515aaa3078d2ff776be4ad2ec590f9adcf6b6b5bcff5cec88badb13301b8204`;
it is not an ambient or packaged dependency.

## Evidence and remaining boundary

Private evidence is retained at `/tmp/cowboy-native-open-once-suZ0MG`.
`full-final.log` SHA-256:
`8912558fff8bdf2748bb8b5818718359d14bbe5d1d624ca2f75b95080814487d`.
`connected.json` SHA-256:
`319fc0536b446d25de981ddf9cdbe377312ad04b1edbfec22eee5f04c580a1b6`.
The exact input, build receipt, unsigned package/envelope, native, lifecycle,
browser and source-boundary logs are retained separately. Earlier compilation
and lint failures are preserved, not counted as final acceptance.

No production Catalog, account, policy, installation, component pointer, Machine
or session changed. The conservative fence intentionally trades new-admission
availability for retained uncertainty. It is neither durable acquisition recovery
nor proof that background effects stopped. Native close acknowledgement, global
retained-buffer/history/background-effect budgets, independently authorized
recovery, signed publication and actual deployed consumer/native-generation and
supported-device acceptance remain open. General graph/state-lease work remains
in the [completion ledger](../plugin-refactor-completion.md).
