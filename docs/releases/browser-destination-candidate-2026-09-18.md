# Browser destination handoff candidate — 2026-09-18

This source-only candidate implements [typed destination handoff](../plugin-browser-buffer-destinations.md).
It does not enable a Review navigation entrypoint, publish/install a production
Plugin, activate an application component or replace any resident Machine.

Core reserves ordinary capacity before the one-use original-index POST; opaque
page-local targets and a closed actual Service response control adoption into
that same slot. Observer cancellation and lost replies cannot drop ownership,
retry acquisition or create a path fallback. Open and child cleanup remain
explicit and independent from the parent. Historical Prepared receipts cannot
reset a live or released child. An invalid destination receipt cannot acknowledge
HTTP 202 no-admission or rearm group Release.

Remote main was fast-forwarded from `4e92c84c` to `0caf9119` before implementation,
preserving its credential-store/SDK and no-remote workspace changes. This slice
changes no production Rust behavior, Machine protocol or native algorithm/dependency.
The added shared wire fixture belongs to Controller/test
sources and is explicitly absent from the Machine source closure.

Input review found a separate upstream release-pin defect: Plugin/adapter
source `1.13.1` still declared private adapter runtime `1.13.0`. The package-owned
builder would reject its Nix output, while the source gate missed that edge.
An independent source candidate `1.13.2` aligns the manifest, contract, Cargo
package/lock and private runtime version without rewriting the `1.13.1`
release. The component baseline remains `3.11.0`, and the server stays `1.0.0`.
The source gate now refuses missing, duplicate or stale private adapter pins.
No native algorithm, protocol or dependency changes; no Catalog publication or
Machine installation is implied. Final native acceptance uses matching bytes,
not the mismatched intermediate `1.13.1` build.

Implementation: `e392da8162d4ac2b1d672529d4e126bf0c16e5af`.
Runtime-pin correction and final clean acceptance source:
`f2d41397c04f64ca1cb1565841ca6d3c9bc85891`.

## Source gates

The focused browser-owner gate passes **118 tests**, including 15 destination
tests. The actual Rust handler fixture and Web typecheck pass. Compile-only
tests reject numbers, serialized IDs and fabricated target objects as handoff
authority, and reject any public ordinary-owner adoption API. Receipt preflight
tests cover collisions with another ordinary owner before any sibling adoption.
The added component-source test rejects stale, missing and duplicate adapter
runtime pins; the complete component graph and independent package builds pass.

Final pinned `just check-compact` passes: formatting, lint, dependency,
contract/component/feature checks, **1,478** all-feature Rust tests, **367**
Machine tests, **26** core-adapter tests, **102** private-adapter tests,
**1,782** Web tests, **18** isolated PostgreSQL tests, the Web bundle and
optimized Rust builds. Dedicated ignored process gates are recorded separately
below. Existing transitive `spin 0.9.8` yanked-version and bundle-size warnings
remain; no dependency was upgraded or warning suppressed to pass this slice.

## Browser acceptance

All seven final isolated Firefox suites pass, **62 checks** total. The owner
suite now requires 24, including six destination cases. These use actual
browser clicks, Response streams, cancellation, structured cloning and
WebCrypto, with synthetic HTTP and the existing development StrictMode harness.
They do not exercise the intended destination view or a physical device.

| Suite | Checks | Final fixture SHA-256 |
| --- | --- | --- |
| Code owner | 24 | `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f` |
| Product context | 6 | `fcd8c031bfb4e8324d8cde8d3c3ab15bd127c50d397213a39fd89d623773ba92` |
| Cleanup | 7 | `bb93243818d657ab64416c0d949e6d32e711f8f90d888f7d3fa8498e9efe5f2f` |
| Synchronization | 8 | `b8d0bc6c23c926bf47072fb2f450f81f889183b3b201c1cfcf6904d94e87d35e` |
| Review source | 6 | `ff582fc66c9a557d88e47457eaf2e7332b86acf74ef01e123ebfc0a9c8c141b5` |
| Review working diff | 6 | `ff2393f9a74dd441c675198c9ad141e37236dfddb4051be3031a3d0785ea3128` |
| Review document refresh | 5 | `306e1b5c594c1c69ded77b060f917d7e143b51b03babc22f65a72f02a643c4d0` |

Browser: `/nix/store/bvshrp9mdjbmx1asq5ndikxim24jfxfv-firefox-151.0.1/bin/firefox`.
Every suite uses a fresh profile and private loopback namespace, with no product
credentials, storage or endpoints. The canonical release skill and runner now
require the expanded owner suite; historical 18-case receipts do not accept the
handoff extension.

## Exact immutable inputs

Controller/Machine and native build receipts all name the final clean source
above. The Controller and Machine inputs are newly built to include upstream
SDK 1.8.1; no earlier binary is relabelled as this source.

| Input | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Controller | `/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release` | `96555b634d6dc9aa6cb30559dd056d0e8d96c05681aa02eab528df45b6865eba` |
| Machine launcher | `/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release` | `1737aefb874b64b0e0874cee4e704c0eb3e9ddaf8661fe272529340ef5042cc7` |
| Zed adapter | `/nix/store/2cg1qw0jhmb8w5v6864jbfxxficdhrq7-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.13.2` | `b5d4afd9bfe6aebf208beeea1f889ebbaa5755040ee6230f7d66849e8c1222b3` |
| Private server | `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0` | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |

The receipt also binds the complete Machine wrapper/executable chain and core
Code adapter (SHA-256 `38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`).
The package-owned native builder checks static ELF boundaries and isolated
probes; both final native artifacts pass. The committed Nix source-boundary
check passes at `/nix/store/sf1qav345bggfd5fkn8ifkg5wvjmhjs2-cowboy-source-boundary`,
including the new Controller-only fixture and existing component boundaries.

## Native and connected acceptance

The final pair passes `zed-native-navigation-conformance`, including actual
native synchronization, all five nonempty navigation kinds, Unicode positions,
lost handoff reply, original-ID reconciliation and independent reads/releases
after parent/path removal. `zed-plugin-conformance` also passes actual temporary
signing, installation, retained-generation/uninstall behavior and nonempty-LSP
handoff. These gates install only disposable fixtures, not a registered Machine.

The v5 Controller/Machine connected receipt passes all **18 checks**:
`accepted: true`, stage `complete`, no failure and confirmed fixture cleanup.
It executes disposable product login/enrollment, signed Code installation and
uninstall, original-owner reads, synchronization, all five navigation kinds,
destination handoff and multi-page Unicode text after parent/path removal.
Nine actual replies are held and three discarded through the normal 40-second
timeouts; no replay repairs the lost Apply, Execute or Release. Six navigation
executions and five releases are observed. Connection replacement and Controller
restart refuse old identity adoption. The run completes in 130.75 seconds.

Its explicit test-only stdio LSP has SHA-256
`b024cf866047f5e9a6e941ab232d9a6a655930ba1e3e23baecef9c686a215dec`.
Synthetic answers and forced fixture containment do not establish production
language semantics, native pre-allocation budgets, physical Close acknowledgement,
restoration, intended-consumer or supported-device acceptance.

## Audit artifacts

Temporary audit directory: `/tmp/cowboy-browser-destination-7DFaoT`.
The final `*-accepted.log` browser receipts bind the fixture hashes above.

| Evidence | SHA-256 |
| --- | --- |
| `full-final.log` | `93d82d8b7368313eef47695c4d5b24082e5fafc77b4d8ac9f5c484b85f5a8eca` |
| `owner-check.log` | `91eceb29b9273332dc2c3bbeb894fda77b1b3624edc71bca76edcf85a2a822d2` |
| `native-final.log` | `373f156474792256f709439301cdfcf1672a73e92efcbfcead8dbdbbe3b0d24a` |
| `lifecycle-final.log` | `7ab721584dc4c752bfc853f0be3cd470f8c8a5a013ecc31cd08165b860551218` |
| `nix-final.log` | `71737506ebde23d6d9292e3303f8fdf8268673af4655c622359f5ec92ba86f1c` |
| `input-final.json` | `c19f83a7ca6e3aec1788067f574acf5d07b0580b0bee95c118130bd7e64a2a42` |
| `connected-final.json` | `b995a98afd275ea83a8febaca17c8d7b897a064c73f3bd08db128f5a63a03c20` |
| `connected-final.log` | `893300b95048a6741dc160a55f61f4ac86f165b2af09a8e6cbbc678eca38db9c` |

No production account, policy, Catalog, installation, component, worker or PWA
activation changed. There is no SW version bump or production deployment receipt.

Production navigation acquisition remains default closed. Intended destination
view integration, native acquisition budgets, supported-device acceptance,
runtime rollout and independently authorized post-effect recovery remain
separate exits in the [completion ledger](../plugin-refactor-completion.md).
