# Native acquisition lifetime candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not production
publication, installation or activation**. Production navigation remains closed;
global retained-history/background budgets and the complete Plugin refactor
remain open.

Accepted implementation/test source:
`e74a8eba7e6f63344945cc27d26f0232b05eb147`, descended from published
`0eeb2f4ef3789fd8efeffe42a05b02e7e38b0eb9`. See the
[acquisition contract](../plugin-native-acquisition-budgets.md).

## Change and regression

Zed Plugin/private adapter advances `1.15.1` → `1.16.0`; private server advances
`1.2.1` → `1.3.0`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, third-party pins, component release
`3.11.0`, Plugin SDK `1.8.1`, Code payload schema 2, adapter API 1 and Machine
protocol 21 remain unchanged. No applied SQL or historical release bytes change.

The preceding immutable `1.2.1` server reproduced the gap: after 64 real buffer
opens distributed across two worktrees, the 65th still returned a new native ID.
That expected negative run is retained separately, not counted as acceptance.

- One native-application pool admits at most 64 combined in-flight/live local
  file and untitled acquisitions across BufferStores and worktrees. Same-path
  pending/open reuse does not reserve again. New loads reserve before I/O and
  loading-task insertion; untitled creation reserves before spawning.
- Shared RAII charges follow the worktree loader, its loaded text result,
  background CRDT construction/result and final Buffer entity. Dropping an
  observer/store or receiving Close cannot free another holder's charge.
- Actual entity release removes matching weak opened/path/entry-ID indexes and
  non-searchable markers. Delayed old-entity cleanup preserves replacement
  indexes, including an equal native ID under a different GPUI entity.

Exhaustion neither evicts existing buffers nor clears adapter Unknown. The
native internal error does not become wire-level no-effect proof for a larger
operation. No new retry, read fallback, restoration, public effect or admission
policy is introduced.

## Exact candidate

Both static executables passed the repository-owned runtime builder's isolated,
credential-free probes from the clean accepted commit. The server's source-owned
Nix test gate ran before that output became available.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.16.0` | `/nix/store/g9l69pxppyym83q89342jdhjvyfg5crz-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.16.0` | `61db229f46b861010fa0d22241a924e9cc28190a0aecf84863001a6cbb5d7dd5` |
| Server `1.3.0` | `/nix/store/k08rbiwh8gc85hnn0cykna72c4ynr8fi-cowboy-zed-server-x86_64-unknown-linux-musl-1.3.0` | `40790dec0bee10a545609bcbae81bcbcd4962be1c3a96395e1bfb9d96ffcdb0a` |

The generic SDK bound both exact components. The retained release envelope has
an **empty production signature**. Its content-addressed HTTPS URLs are planned
locations, not evidence of publication or an installable Catalog release.
Disposable fixture signing and installation remain separate.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `f64071cf9e6372a2fab2e6681d358c171b551370314a9e6282fe30733e84c84f` |
| Composite artifact | `004716fd43a8e8913b56e59617bb33cd107d54700ea2528cf00b120cfc546df5` |
| Contract fingerprint | `ac66ce888bbea8e5b0730c2ea6ce9c844a2b210202fab882e6797468e158855e` |
| Build receipt file | `541ab210d18758c2a346cbd10d85eb0b023b397d4c0774a3cedb815f44b9cd0a` |
| Runtime matrix file | `c66012b91c2f21b468003d213dc26d6bb04b50eeede2c6f377c8dd050ab078ab` |

## Accepted gates

All commands used the pinned shell. The final runtime builder, connected gate
and installation lifecycle ran on clean committed source; acceptance docs were
written only after completion.

- `just check-compact`: Rust main 1,479 passed / 34 explicitly ignored; bridge
  3; standalone Machine 368 / 4 ignored; core Code adapter 26; private adapter
  124 / 2 ignored; Web 1,796; isolated PostgreSQL 18. Formatting, Clippy, strict
  types, dependency/component/feature checks and optimized builds passed.
  Existing dependency and Vite warnings were not suppressed.
- Nix native gate: **32 tests**, filesystem 3, LSP 4 and project 25. Eight new
  groups cover saturation across stores, same-path reuse, pending deduplication
  with a lost observer, loaded-result retention, failure/untitled cancellation,
  store teardown, cross-thread shared charges, 128 release/index cycles,
  replacement ABA and a Close ACK with other native holders.
- `zed-native-navigation-conformance`: final static pair passed in 5.61 s.
  The source-test adapter additionally saturates the exact server across two
  real worktrees: 64 successful opens, actual capacity errors on both roots,
  unchanged original mirrors, and a separate open after the fixture releases
  its native handles. Existing input/reload, one-use Open, confirmed Close,
  five-kind nonempty navigation, handoff and no-replay checks remain accepted.
- `zed-plugin-conformance`: temporary signing/installation, uninstall drain,
  retained-generation reactivation, nonempty navigation and independent target
  read/release passed in 10.07 s. Fixture teardown is not product recovery.
- `code-buffer-connected-conformance`: all **18 v5** checks passed in 130.67 s.
  Disposable actual login/enrollment and signed installation were used; three
  actual replies were lost through normal 40-second deadlines. Receipt:
  `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`.
- Firefox 151.0.1 browser ownership suite: **24** checks passed; fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This does not accept a physical device.
- Nix source boundary passed:
  `/nix/store/rhc9afgw6ls6ccz0czffmaf6sqr83464-cowboy-source-boundary`.

The connected test supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and
Machine `/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`,
both from `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled
as this source or accepted as active production roles. Core adapter SHA-256:
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicit test-only stdio LSP SHA-256 is
`d6c373f362ed2778029d5527630ae5087c63f7e401ae74eabfe3c150c7f74c34`;
its synthetic answers are not production language semantics or an ambient
dependency of the published contract.

## Evidence and exclusions

Private evidence is retained at `/tmp/cowboy-native-acquisition-8JOcem`, including
the exact input, runtime matrix, build receipt, unsigned package/envelope and logs.

| Evidence file | SHA-256 |
| --- | --- |
| `full-final.log` | `97eb3273555c386d0009e77ca719493b92095fbe51f4d27593cc3d42b7aa05ec` |
| `native-final-build.log` | `29e9235a2a20902c2fbf9c13a24e910b34b1df8eeaf6bff33a0dacc4f64cd416` |
| `native-pair-final.log` | `02f3331e85c50d2357a78a08d4c92efd92447bc3f35f152deff4c5b8c210e7dc` |
| `lifecycle-final.log` | `21937330c4aa97ed75d096993c4eb33ca0ec5bb62ccece51ec4baf1b2856b7c4` |
| `connected.json` | `40c9055900d9156c36af09b843d662911cbeae4565bb2550436d856420625349` |
| `browser-boundary-final.log` | `95c97d307273926fd1cbaa3ad51f0015c082a6ae0156bfeb0a108962bc239b17` |
| `bind-final.log` | `a1abc788f85175189b3f37378e228e6c041fe75ef38a2f433c3b1bec5d6f8a80` |

Development evidence includes the old-server negative test, an initial fixture
failure that tried to reuse capacity with an absent file, and an interrupted
duplicate build. The fixture now explicitly creates that independent input;
the admission logic was unchanged. The complete source gate was rerun on the
final commit. An initial bind refused a gate-regenerated unbound package URL;
the final separate bind supplies its planned HTTPS location. None of these
earlier attempts substitutes for final acceptance.

This bounds acquired buffer/load lifetimes, not all text constructors, CRDT
history bytes, detached snapshots, parsing/LSP jobs, worktree scanning or process
RSS. Close remains peer-ownership confirmation, not background drain. General
graph/site/state leases, independently authorized post-effect recovery, actual
deployed native-generation and supported-device acceptance remain separate
[completion exits](../plugin-refactor-completion.md). No production Catalog,
account, policy, installation, Machine, component pointer or session changed.
