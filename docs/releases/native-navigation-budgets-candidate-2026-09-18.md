# Native whole-query navigation candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not a production
Catalog publication, Machine installation or component activation**. Production
navigation admission remains closed. Finite per-query bounds do not complete
the Plugin refactor or establish native recovery.

Accepted implementation/test source:
`cadbe37b3a3537652afc2ba2083e7e9484e183b4`, descended from published
`2a6da8a7edf11755ac161c29183c1be01e355677`.
The [whole-query contract](../plugin-native-navigation-budgets.md) records the
precise bounds and exclusions.

## Change

Zed Plugin/private adapter advances `1.13.3` → `1.14.0`; private server advances
`1.0.1` → `1.1.0`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, all third-party dependency pins,
component release `3.11.0`, Plugin SDK `1.8.1`, core Machine protocol 21 and
adapter API 1 remain unchanged. No historical registry or applied SQL bytes
are edited. The new private navigation codec is protocol 1, tags 1002/1003;
the existing private synchronization codec remains unchanged.

- Select at most four capable registered servers from a bounded registration
  table, without manifest discovery. Losing a registered participant refuses
  the whole query before dispatch; an LSP error is not omitted from success.
- Collect and validate all responses before any target opens: at most 256
  locations including duplicates, 32 distinct paths, original worktree only.
  Every distinct path opens once through that original `ProjectPath`.
- Recheck original source peer ownership, native vector, file and worktree at
  acquisition boundaries. Validate exact target UTF-16 ranges instead of clipping.
- Distinguish Supported, Complete and typed Refused, with strict reply decoding
  and no upstream query fallback. All five refusal reasons preserve the adapter's
  Unknown state and fence after Execute; no retry or invented rollback is added.

## Exact candidate artifacts

Built with the repository-owned `zed-plugin-runtime-build` from the clean source
above. Both executables passed static ELF checks and isolated credential-free
probes. The upstream server's version string is not the private distribution's
signed identity.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.14.0` | `/nix/store/809l36hh0y07pnswpapfzc0gf8y5c0hb-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.14.0` | `ce9cf0a6820a26b2dea3cfe31949eb15d02a344d70a72a43a70e5767c4c83487` |
| Server `1.1.0` | `/nix/store/5psz7qz6ly9y4rpjmx526xh7d51vvf8q-cowboy-zed-server-x86_64-unknown-linux-musl-1.1.0` | `39bbf25f202da105c41238a87ee5374beb30ed8b592798fc6f1030cde9799e4e` |

The generic SDK built the package and bound the complete two-component runtime
matrix. The retained envelope has **an empty production signature**; it is not
available for UI installation. Content-addressed URLs are planned publication
locations, not availability receipts. Temporary fixture signing is independent.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `d3e3c9049d493b651a83ef679a6435c2e0bdd4bb009afa2b379875b0d7c70072` |
| Composite artifact | `14baa60dc79419e5b693690af2ddb10a8ecc33435c9a19accfeedc98b0ac352e` |
| Contract fingerprint | `217a1619f72894237b4613062e3379b1ef2968a593e4382fb0711bf8a1ee7c40` |
| Build receipt file | `3eede1530b37179fac1b767a14a3717655c85903db40b4bc9d8f05d7e4b0b75a` |

## Accepted gates

All commands ran from the repository root in its pinned Linux shell.

- Final `just check-compact`: Rust main 1,478 passed / 34 explicitly ignored;
  bridge 3; standalone Machine 367 / 4 ignored; core Code adapter 26;
  private adapter 106 / 2 ignored; Web 1,794; isolated PostgreSQL 18. Formatting,
  Clippy, strict types, dependencies, components, feature slices and optimized
  builds passed. Existing upstream/Vite/dependency warnings were not suppressed.
- Native Nix build: 15 tests passed (3 filesystem, 4 actual LSP reader/parser,
  4 native synchronization and 4 navigation groups). A GPUI fixture uses two
  actual request handlers: 128 + 129 locations refuse with zero target opens;
  128 + 128 complete with one distinct target open; one server error refuses
  the whole query. Invalid selection and lost source ownership dispatch nothing.
  The split-UTF-16 case explicitly proves a refusal can follow target acquisition.
- `zed-native-navigation-conformance` against the exact pair: all five nonempty
  kinds and original-target handoff; location/target budgets, external target,
  invalid UTF-16 and actual LSP `content modified` typed refusals, without partial
  result. Prior inclusive/over-budget input, synchronization, lost-reply and
  no-replay checks remain green. Private queries do not consume upstream query IDs.
- `zed-plugin-conformance`: real temporary signing, installation, uninstall
  drain/reactivation, nonempty navigation and independent original-target reads.
  Synthetic Machine authority and language answers are not production acceptance.
- `code-buffer-connected-conformance`: all **18 v5** checks passed, with actual
  disposable password login, enrollment and signed installation. Three genuine
  replies were discarded; normal 40-second deadlines were retained. Original-ID
  observation did not replay Apply, Execute or Release. `stage=complete`,
  `failure=null`, `cleanup=true`, `accepted=true`; elapsed 130.71 seconds.
- Firefox 151.0.1 `code-buffer-browser-conformance`: all **24** cases passed;
  fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  Browser source is unchanged; this is not a physical-device result.
- `checks.x86_64-linux.cowboy-source-boundary` passed:
  `/nix/store/fzr2jfpls4078b4l9a8zgi6v5pb4m6ji-cowboy-source-boundary`.

The connected fixture deliberately reused supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and Machine
`/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`, both actually
built from `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled as
this candidate. Core adapter hash remains
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicitly supplied test-only LSP hash is
`3c42959665be982dd5b786d2b994241a21b911355723c230a7c42e1b41f057ee`;
it is never an ambient or packaged dependency.

## Evidence and delivery boundary

Private evidence is retained at `/tmp/cowboy-native-navigation-budget-23yuPo`.
`full-final.log` SHA-256:
`191866dda058a9f007dea8a43d6598b658fa8e82a929ad8d2ca2704526a3dded`.
`connected.json` SHA-256:
`20d38bb459f463c0d68b4911dd5eb246d693f4e7a88f00fd66d5f334e8d92cf0`.
The build receipt, exact input, native/build/lifecycle/browser logs and unsigned
package/envelope snapshots are retained separately. Earlier failed/intermediate
build logs are preserved and are not final acceptance. One complete-gate attempt
exited 139 in the TypeScript process without a type diagnostic; standalone
typecheck and the final complete gate passed afterward. Its cause is unproven.

No production Catalog, policy, account, component pointer, installed Plugin,
Machine or session was changed by these candidate gates. Fixture cleanup does
not prove production process continuity. Signed publication, the actual reader
floor, registered Machine/exact native-generation rollout and supported-device
acceptance remain separate boundaries.

Still required: aggregate native live-buffer/history and background-effect
limits, independently authorized uncertain-acquisition recovery and verified
native close. Observation deadlines and one admitted handler do not guarantee
that background loads have stopped. General graph/state-lease work remains in
the [completion ledger](../plugin-refactor-completion.md).
