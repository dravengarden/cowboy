# Native sync/reload replacement candidate — 2026-09-18

Status: verified source and immutable Linux x86_64 candidate, **not production
publication, installation or activation**. This bounds two native writers, not
every history/background resource or completion of the Plugin refactor.

Accepted implementation/test source:
`66f3fd3eccf6cf56ce30f53569982d92aca539fa`. The later integration commit
`cc251a3f6e603becaa31029298c53103151c3d57` preserves that ancestor and merges
remote `739277bfb514681d1f9a100f885fedaa37a212fa`. Its only changes from the
accepted source are desktop run-config label layout and the service-worker
version. Both commits have Plugin subtree
`81838b1376e7f2525285af2cec1156ae9910bc71`. The complete source gate ran on
`66f3fd3e`; the frontend gates were repeated after integration as detailed below.
See the [replacement contract](../plugin-native-replacement-budgets.md).

## Change and regression

Zed Plugin/private adapter advances `1.16.0` → `1.17.0`; private server advances
`1.3.0` → `1.4.0`. Upstream revision
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, third-party pins, component release
`3.11.0`, Plugin SDK `1.8.1`, Code payload schema 2, adapter API 1 and Machine
protocol 21 remain unchanged. No applied SQL or historical release bytes change.

The preceding immutable `1.3.0` server reproduced the gap: a reload with 1,025
independent changed lines succeeded. That expected negative run is separate
development evidence, not acceptance of the new 1,024-edit bound.

- One application-wide pool admits four combined sync/reload jobs across stores
  and worktrees. Actual loaders, completed byte results, diff workers and their
  completed results retain shared RAII charges, independently of observers.
- Admission scans actual retained text history before loading and again before
  mutation: 4 MiB current/new text, 8 MiB base plus inserted history text, 4,096
  edit/undo operations, 16,384 aggregate parts, 256 dense clock slots including
  the writer replica, and no causally deferred text operations.
- The existing diff algorithm refuses more than 1,024 edits before allocating
  another result string. Its consuming result binds exact entity and version,
  including edit/undo ABA. Prospective history checks precede undo finalization,
  text/encoding mutation and saved-state changes. No partial diff, history
  pruning, whole-file fallback, reopening or automatic replay is introduced.

Private sync protocol 1 adds a closed `Budget` refusal. The native operation
retains its terminal result under the original ID after capacity returns. The
adapter validates the exact paired response, but the public/core owner codec
is deliberately unchanged: unsupported budget reconciliation retains Unknown
and its original fence. It does not pretend the reason was Source, resend Apply
or permit Retire. Public budget-result projection and independently accepted
reconciliation remain separate work.

## Exact candidate

The source-owned native gate ran before the server output became available.
The official runtime builder subsequently verified both static executables with
isolated credential-free probes on clean accepted source `66f3fd3e`.

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.17.0` | `/nix/store/ri6116zl79f0hcj6jshmgjpzzpd7r1hc-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.17.0` | `c1f239964df6b4fd56de5e9212a12dd568791fb5af890e8197b12500b4582df2` |
| Server `1.4.0` | `/nix/store/88hccb0s3csayvay7qwisn755pbscp4y-cowboy-zed-server-x86_64-unknown-linux-musl-1.4.0` | `1bfd5b9556f61545906a0f24b08bdfcb96c194a900fc54b8d96992a012dec283` |

The generic SDK bound both exact components after the complete gate. The retained
release envelope has an **empty production signature**; its content-addressed
HTTPS URLs are planned locations, not evidence of publication or an installable
Catalog release. Disposable fixture signing and installation remain separate.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `311b82406872dd23ae231efa453ad52a9444775a6c905c5777884befce737146` |
| Composite artifact | `82063f2555b6d69e601c41d57b3ca0e6673400b92cf5fbbc56639494ab1a06de` |
| Contract fingerprint | `c6e66c5de91b0e70e357f897c2cd7c5cc816e9c772dfbe4e1e339151c088b1a8` |
| Build receipt file | `3e890e86016cdebffe1bb3d8d03258f597fa4ed0e4400a5ded0050350adbf3cd` |
| Runtime matrix file | `8bf41e7c25ff55b3148f54b65a523fc412905f5b1aa4179e47ec2f5de255bc53` |
| Unsigned release envelope file | `5eb6b6410eb612b3bb15374634459da46a759b829011208af6d91b8344135149` |

## Accepted gates

All commands used the pinned shell. Final runtime, pair, lifecycle and connected
gates used clean committed source. Acceptance docs were written afterward.

- `just check-compact` on `66f3fd3e`: Rust main 1,479 passed / 34 explicitly
  ignored; bridge 3; standalone Machine 368 / 4 ignored; core Code adapter 26;
  private adapter 126 / 2 ignored; Web 1,800; isolated PostgreSQL 18. Formatting,
  Clippy, strict types, dependency/component/feature checks and optimized builds
  passed. Existing upstream and Vite warnings were not suppressed.
- Nix native gate: **43 tests**, filesystem 3, LSP 4 and project 36. Eleven new
  groups cover shared capacity, actual loader/result and cross-thread lifetimes,
  cancellation, exact entity/version ABA, inclusive history/operation bounds,
  undo retention, aggregate parts, dense/deferred histories, whole-diff refusal,
  saved-ID deduplication and unchanged saved state. The build first completed
  on pre-rebase `fcbee35482e5baa7d8aa44e08d5fe3e17db0f92a`; unchanged Plugin
  inputs select the same immutable output in the official `66f3fd3e` builder.
- `zed-native-navigation-conformance`: final static pair passed in 5.64 s.
  Actual oversized reload refuses; conditional sync returns typed Budget;
  duplicate Apply retains that result; original text/version/source stay intact.
  A separately requested smaller reload succeeds. Existing input/acquisition,
  one-use Open, confirmed Close, five-kind navigation and handoff checks pass.
- `zed-plugin-conformance`: temporary signed installation, uninstall drain,
  retained-generation reactivation and independent navigation/read/release
  passed in 10.15 s. Fixture teardown is not product recovery.
- `code-buffer-connected-conformance`: all **18 v5** checks passed in 130.85 s.
  Disposable login/enrollment and signed installation include three actual lost
  replies through normal 40-second deadlines. Receipt: `stage=complete`,
  `failure=null`, `cleanup=true`, `accepted=true`; source is `66f3fd3e` and
  package/native digests match the exact candidate above.
- Firefox 151.0.1 browser ownership suite: **24** checks passed; fixture SHA-256
  `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f`.
  This does not accept a physical device.
- Nix source boundary passed:
  `/nix/store/953k8nq1z6psf6848gh5i12qgpk10kzz-cowboy-source-boundary`.
- After the frontend-only merge `cc251a3f`, Web typecheck, lint, all **1,800**
  tests and optimized build passed again, as did all **24** browser checks and
  the source boundary. This is not a second complete source gate on that merge.

The connected test supplied Controller
`/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` and
Machine `/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`,
both from `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`. They are not relabelled
as this source or accepted as active production roles. Core adapter SHA-256:
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
The explicit test-only stdio LSP SHA-256 is
`83c2e05dde83249f68d5d1cb6dc3c501111905e899a03d304f983068af5b0bc5`;
its synthetic answers are not production language semantics or an ambient
dependency of the published contract.

## Evidence and exclusions

Private evidence is retained at `/tmp/cowboy-native-replacement-5fBzQd`, including
the exact input, runtime matrix, build receipt, unsigned package/envelope and logs.

| Evidence file | SHA-256 |
| --- | --- |
| `full-integrated.log` | `9f28a3e23d92eb5893af15b582a7645d65177e949faaa30ce515076d037f0cbb` |
| `native-final-build.log` | `a19fc9e53ca9105c16a666b13341965dffc2198d16e642740328d9a53b82ca68` |
| `runtime-build.log` | `4aea95bcd29aa658e11042f3e3162d63b0d804e2ae05de9f45fae50a899eae62` |
| `native-pair-integrated.log` | `2930fd626b4ae0aa3c4405a9fcac193a5dd9a5df53bb3bff9a1b3179bf41d2b9` |
| `lifecycle.log` | `d24d8a10bdb63c72e4e79e37f9f633b12b762571e0f4dcc581cc9e4d33c0eef0` |
| `connected.json` | `5a9811ec4f1b9dfd3b62bf484f7b91d8461ab07ba18736e79bbe237475861204` |
| `connected.log` | `7ef0056f7b9f552f9a2c6ed3898a4e0f670d95e160b7a3e9e466755d78172a10` |
| `browser-boundary-integrated.log` | `7ff9f1200ac4414ff11578882a0a7ec58fe0970d836fa74d28bc2f7a861214f6` |
| `merge-web.log` | `065fc42f4b0e2c9725b4934a1a69ff987d6439e86ba152c223ef7082fd39b634` |
| `bind.log` | `a355261a05f1176b9b07ad942f0935dd5d2f3720626656eb637cfb301821fbbd` |

Development evidence includes the preceding server's negative run and an initial
native compile failure from two unqualified error macros, repaired before the
accepted build. Earlier source runs are retained separately and do not substitute
for the final integrated gate. No third-party dependency upgrade was needed.

This bounds sync/reload growth, not remote edit/undo or LSP writers at their own
mutation boundaries, all constructors, process-global history bytes, detached
snapshots, parsing/serialization jobs, worktree scanning, RSS or CPU deadlines.
Close, observer cancellation and fixture teardown do not establish background
drain or recovery. Public budget reconciliation, general graph/site/state leases,
independently authorized post-effect restoration, actual deployed native-generation
and supported-device acceptance remain separate
[completion exits](../plugin-refactor-completion.md). No production Catalog,
account, policy, installation, Machine, component pointer or session changed.
