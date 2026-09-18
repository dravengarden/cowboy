# Native input-bounds candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not a production
Catalog publication, Machine installation or component activation**. Production
navigation admission remains closed. This accepts individual input limits, not
aggregate target acquisition or completion of the Plugin refactor.

Accepted implementation/test source:
`cee58a044c1701df79211a6cddce12ac33dcff89`, descended from the published Review
destination reader at `298209b45aca6fd3663481f3f6e1d4a110b2e993`.
The [input contract](../plugin-native-input-bounds.md) records the precise bounds
and remaining exits.

## Change

Zed Plugin/private adapter advances `1.13.2` → `1.13.3`; private server advances
`1.0.0` → `1.0.1`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, all third-party dependency pins,
component release `3.11.0`, Plugin SDK `1.8.1` and private protocol remain
unchanged. No historical registry or applied SQL bytes are edited.

- Complete LSP headers are limited to 8 KiB while reading, including an
  unterminated line. Exactly one bounded decimal Content-Length is required;
  a body above 2 MiB is rejected before body allocation/read. Header values
  cannot enter these errors. Existing queue backpressure is retained.
- Actual native text-source reads use the existing no-symlink regular-file
  descriptor primitive, bounded to 4 MiB plus one overflow sentinel. A growing
  file cannot bypass an earlier stat. Decoded UTF-8 must also fit 4 MiB before
  constructing a native text buffer. Source files are never changed.
- Tests require actual native budget refusals, not a timeout/transport failure.
  No retry, reload, acquisition-Unknown retirement or replacement owner is added.

## Exact candidate artifacts

Built with the repository-owned `zed-plugin-runtime-build` from the clean
accepted source above. Both executables passed static ELF checks and isolated
credential-free probes.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.13.3` | `/nix/store/7gy2kfprd1mjq9899pr6n3cmph3l38kb-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.13.3` | `c391c0d5179dc55659009e14e701b5adaf55d0c6966893c24ba6de2a980751ab` |
| Server `1.0.1` | `/nix/store/968ma2zvpvqh32q4vqin24fficilkbi4-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.1` | `2a68f77bafce9c60b81a2efa342be085ee64f43edae2cb90648cc061944da055` |

The generic SDK built the package and bound the complete two-component runtime
matrix. This envelope has **an empty production signature** and is not available
for UI installation. Its content-addressed URLs are planned publication locations,
not availability receipts.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `60aa4e805f6aefaeb981fcb1b8e9c08e260a742e84195be2e56ce174c6df0f9a` |
| Composite artifact | `36b0624e637666061c106c3d48c537a295776cc994dcad0e2ebb109cf7b161a3` |
| Contract fingerprint | `7ab0145ee15ba4de58ad2c3b1d0a1c8c1676867bea6008281731ccaf5a55698a` |
| Build receipt file | `2f6c09a36bbe85935cbac762d2202580bec4c2913ac1f6532a1770861451b445` |

## Accepted gates

All commands ran from the repository root in its pinned Linux shell.

- Final `just check-compact`: Rust main 1,478 passed / 34 explicitly ignored;
  bridge 3; standalone Machine 367 / 4 ignored; core Code adapter 26;
  private adapter 102 / 2 ignored; Web 1,794; isolated PostgreSQL 18. Formatting,
  Clippy, strict types, dependencies, components, feature slices and optimized
  builds passed. Existing upstream/Vite/dependency warnings were not suppressed.
- Native Nix build: 11 tests passed (3 filesystem, 4 actual LSP reader/parser,
  4 native GPUI synchronization groups). This includes the one-byte growth
  sentinel, exact header/length bounds, redacted errors and the actual dispatcher
  rejecting an oversized declared body before reading it.
- `zed-native-navigation-conformance` against the exact pair: inclusive 4 MiB
  open; over-budget raw and decoded text refusals; unchanged files and surviving
  native process; synchronization/ABA/lost-reply checks; five nonempty navigation
  kinds; original-target handoff/read/release after parent/path removal.
- `zed-plugin-conformance`: real temporary signing, installation, uninstall
  drain/reactivation and independent original-target reads. Synthetic Machine
  authority and language answers are not production installation acceptance.
- `code-buffer-connected-conformance`: all **18 v5** checks passed, with actual
  disposable password login, enrollment and signed installation. Three genuine
  replies were discarded; normal 40-second deadlines were retained. Original-ID
  observation did not replay Apply, Execute or Release. `stage=complete`,
  `failure=null`, `cleanup=true`, `accepted=true`; elapsed 130.67 seconds.
- Firefox 151.0.1 `code-buffer-browser-conformance`: all **24** cases passed;
  browser fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This is synthetic browser ownership evidence, not a physical-device result.
- `checks.x86_64-linux.cowboy-source-boundary` passed:
  `/nix/store/5i6r3imlabmmhr5hxb72bci9lfl293ik-cowboy-source-boundary`.

The connected fixture deliberately reused supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and Machine
`/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`, both actually
built from `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled as
this candidate. Core adapter hash remains
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicitly supplied test-only LSP hash is
`fff92d5cdc6146c1fc8f1b46c6121bda035ebaad1aa4234fa05a71093e9edbc0`;
it is never an ambient or packaged dependency.

## Evidence and delivery boundary

Private evidence is retained at `/tmp/cowboy-native-input-bounds-wrUPf6`.
`full-accepted.log` SHA-256:
`0263f1b854a13e1164915ee812fa16be6eca2a63e9f4f690e9a6f3dd19708ce4`.
`connected.json` SHA-256:
`806089f09448d6c2bd209a4478cd8afc7454083313bedaa393d2a60ad5f4fcb6`.
The final build log, exact input, browser/native/lifecycle logs and unsigned
package/envelope snapshots are retained separately. Earlier failed/intermediate
build logs are preserved, not treated as final acceptance.

No production Catalog, policy, account, component pointer, installed Plugin,
Machine or session was changed by this candidate's gates. No production process
continuity claim is inferred from fixture cleanup. Signed publication, the
actual reader floor, registered Machine/exact native-generation rollout and
supported-device acceptance remain separate boundaries.

Still required: aggregate budgets across all participating LSPs before any
target opens; typed whole-query refusal instead of upstream omission of failed
responses; external-worktree admission before acquisition; aggregate retained
history/background-effect/deadline limits; independently authorized recovery.
Single-input limits do not close those gaps or enable navigation. General
graph/state-lease work remains in the [completion ledger](../plugin-refactor-completion.md).
