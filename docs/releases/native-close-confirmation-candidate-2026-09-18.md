# Original-peer native close candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not production
Catalog publication, Plugin installation or component activation**. Production
owned navigation remains closed. This does not complete the Plugin refactor or
establish independently authorized recovery.

Accepted implementation/test source:
`059314bf488a3a3de70e1574e483826970d1e536`, descended from published
`c241a88d0af1ff870c6c918d472fa56654238030`. The
[close contract](../plugin-native-close-confirmation.md) specifies the exact
ownership guarantee and remaining uncertainty.

## Change

Zed Plugin/private adapter advances `1.14.1` → `1.15.0`; private server advances
`1.1.0` → `1.2.0`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, all third-party pins, component
release `3.11.0`, Plugin SDK `1.8.1`, Machine protocol 21 and adapter API 1 are
unchanged. No historical registry or applied SQL bytes change.

- Add a closed private native protocol with an effect-free instance probe and
  one synchronous peer-map removal for a strictly sorted set of at most 33 IDs.
  Every original sender-peer member is validated before any is removed; foreign
  instances and incomplete sets cannot release a prefix or another peer.
- Derive a non-cloneable adapter release plan under the original active-owner
  lock, counting all same-native-ID aliases and independent typed owners. A
  navigation group closes one complete set. Shared-only releases remain local.
- Before the single effect await, retain original pins/mirrors and mark ordinary
  Unknown or navigation ReleaseUnknown. Only the exact original instance/set's
  Closed reply permits synchronous local retirement. Cancelled probes have no
  effect; wrong/lost/late replies and the normal deadline never authorize replay.
- Keep closing-ID reads/effects and new acquisition fenced. Independently known
  owners can still release, without clearing another owner's uncertainty.

Closed proves native peer-map/shared-handle removal, **not** global allocation
reclamation, LSP delivery/quiescence, stopped background work, filesystem undo
or recovered Agent state. The native protocol has no saved operation journal
or recovery query. Adapter one-use dispatch is not an independent server-side
deduplication claim for arbitrarily replayed private frames.

## Exact candidate artifacts

The repository-owned runtime builder ran from the clean committed source above.
Both executables passed static ELF checks and credential-free isolated probes.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.15.0` | `/nix/store/4096f9g6n0zay1r0jmlwj7vpis1dvxj6-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.15.0` | `341ff761f0c38b52400b0e8c5f0a90ae7274ba55604d2c895aa2b619395aef3b` |
| Server `1.2.0` | `/nix/store/kxcxb4vy6xsy2wkk6vx6x9ifykyj07cc-cowboy-zed-server-x86_64-unknown-linux-musl-1.2.0` | `89ed2b45b780e9890c0e8224957443df73f4adb349cf9a0303c041ce2d19a573` |

The generic SDK built the package and bound its complete two-component matrix.
The retained envelope has **an empty production signature**. Content-addressed
URLs are planned locations, not public availability receipts or installable
Catalog entries. Temporary fixture signing is separate.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `dcc9bd7786c65e2345322cbc85dcefbaedaa5774d90f7cfbfc2c1bac096ddb39` |
| Composite artifact | `74fc4642311912cfdd0770f402b973f441e625624fd5327a250e478f7a8e2720` |
| Contract fingerprint | `d34829e38403d95f81a0a23b36bd83a49b83a598467bf3faa8f92b665a5ded4b` |
| Build receipt file | `f0b52ae0399feffde18b731cf318452597a63396a33235333c6801e60eadc5de` |

## Accepted gates

All build/test commands used the repository's pinned shell.

- `just check-compact`: Rust main 1,479 passed / 34 explicitly ignored; bridge 3;
  standalone Machine 368 / 4 ignored; core Code adapter 26; private adapter
  124 / 2 ignored; Web 1,796; isolated PostgreSQL 18. Formatting, Clippy, strict
  types, dependency/component/feature boundaries and optimized builds passed.
  Existing dependency and Vite chunk warnings were not suppressed.
- Nine new adapter tests plus updated private-frame fixtures cover exact
  instance/set/protocol/outcome validation, alias ownership, unsupported probes,
  ordinary/group cancellation, malformed/late replies, the genuine 30-second
  close deadline and independent-owner progress. No upstream close fallback is
  accepted by the fixtures.
- The final server's Nix build runs **19 native tests**: filesystem 3, actual LSP
  input/parser 4, synchronization/navigation/close 12. Four new close groups
  cover complete atomic removal, other-peer preservation, missing-member and
  malformed-input refusal, instance replacement and discarded native results.
- `zed-native-navigation-conformance` against the exact static pair passed.
  Its source-test adapter additionally discards a **real** native Closed reply,
  observes native peer removal without treating it as recovery, retains original
  Unknown/no-replay, and confirms an unrelated owner's release. That drop hook
  is `cfg(test)` only and absent from the shipped adapter. The final immutable
  socket pair separately passes confirmed ordinary/group release, all five
  nonempty navigation kinds, path-free independent handoff/text reads, actual
  input/whole-query refusal and single-use Open regressions. The explicit test
  LSP's document close observations are not production language semantics or
  universal background-drain evidence.
- `zed-plugin-conformance`: actual temporary signing, installation, uninstall
  drain/reactivation, nonempty navigation, retained original-target reads and
  independent release passed. Synthetic Machine authority and forced fixture
  teardown are not production authority or native recovery.
- `code-buffer-connected-conformance`: all **18 v5** checks passed from clean
  source, including actual disposable password login, enrollment and signed
  installation. Three genuine replies were discarded with their unchanged
  40-second transport deadlines. Original-ID Query settles completed upper-layer
  outcomes without replay; this does not recover a lost native close ACK.
  `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`; 130.68 seconds.
- Firefox 151.0.1 `code-buffer-browser-conformance`: all **24** cases passed;
  unchanged fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This is not physical-device acceptance.
- `checks.x86_64-linux.cowboy-source-boundary` passed:
  `/nix/store/f46h107s9hmclg0gw7bpmbgxa8kay1sa-cowboy-source-boundary`.

The connected gate deliberately supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and Machine
`/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`, both from
`f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled as this source
or accepted as active host roles. Core adapter SHA-256 remains
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicit test-only LSP SHA-256 is
`f3d811542ed2af3e8a772c1461600fc8315745e75a6a81a468570c82f5d85bc3`;
it is neither an ambient nor a packaged dependency.

## Evidence and remaining boundary

Private evidence is retained at `/tmp/cowboy-native-close-XTHAkk`, including
the exact input, build receipt, runtime matrix, unsigned package/envelope and
native, lifecycle, browser and source-boundary logs.

| Evidence file | SHA-256 |
| --- | --- |
| `full-first.log` | `12101d6be7a71ab56b2a319aeee97eddbcb24275bdc37d6d503b6a99b18b7b49` |
| `native-final-nix.log` | `8dc1bbcbdb6bfa659397ddc44a03e5861e536c45bfbd3fcde8de0f0ce480aa77` |
| `native-final.log` | `7cfb31a78730b6a6a85aff6687d86334a767452d8744969e9c3a1fcdaa7e0323` |
| `lifecycle-final.log` | `4865eaa9927fa46c29cf747eb5556b4dd7ec8111f92c02312e92a2e42c738e61` |
| `connected.json` | `be69f92e530fa83386152b0fcd3622b8655b35cdbf19711c5592d1b8bfe7c909` |

Earlier development compilation/lint failures and the first connected attempt's
clean-tree precondition refusal are retained, not counted as acceptance. The
successful connected log is `connected-second.log`; the receipt above is
create-only and binds the accepted source and exact artifact hashes.

No production Catalog, account, policy, installation, component pointer, Machine
or session changed. Global retained-history/background-effect limits,
independently authorized recovery, signed publication/installation and actual
deployed consumer/native-generation and supported-device acceptance remain
open. General graph/site/state leases remain in the
[completion ledger](../plugin-refactor-completion.md).
