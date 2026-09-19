# Native text-snapshot lifetime release — 2026-09-19

Status: Zed Plugin/private adapter **1.19.0**, selecting private server **1.5.0**,
is verified, signed and published. The exact release is advertised as `ready`
for Linux x86_64. **Hawk still has Zed 1.18.0 installed**; this release did not
perform a Machine upgrade, restart an existing process or migrate native owners.
This is not whole-refactor completion.

Implementation and clean final build/sign/publication source:
`819e6e4a893e8fe3defbb00bbb06b2c7a24b4e09`. This record is a documentation
descendant. The [design](../plugin-native-snapshot-lifetimes.md) records the
exact scope and remaining limits.

## Change and fixed boundary

The native application still admits **64 independent acquisition lineages**.
The original charge now follows native text Buffer/BufferSnapshot ownership,
including language/raw snapshot clones, branches and background previews.
Closing the language entity and clearing its indexes no longer returns
capacity while one of those original holders survives. The last holder drops
the charge once, even off the GPUI thread, after its retained text/trees.

Snapshot clones share one slot; this is not a cap on all snapshot counts,
historical versions or total bytes. Detached Rope/string/syntax-only data,
unrelated constructors, all edit writers, general background work and RSS
remain outside this charge. No history is pruned, unknown owner evicted,
operation replayed or cross-site inverse authorized.

Previous published Plugin/adapter is `1.18.0`, private server `1.4.0`.
Upstream remains exactly `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`.
Third-party pins are unchanged. Plugin component release `3.11.0`, SDK `1.8.1`,
Code component `1.2.0`, payload schema 2, outer release schema 1, adapter API 1
and Machine protocol 21 remain unchanged. Ordinary Zed is untouched.

## Exact artifacts

| Artifact | SHA-256 |
| --- | --- |
| Adapter ELF | `4dfa7a170b212a904574939476949bfb1eaca25fae8c687ebab2b70dd3615974` |
| Private server ELF | `1fe3329ff99ac55830014310850cddc9defdb86662b5b15cee1fd1a688e385fb` |
| Package | `fa25ebeb6c53a47798ccc581c01fbb98b38e2b688c956c5dceccee52fce63e42` |
| Composite artifact | `04bec5bfe03fbc6aeabe1701102dbbe24d3840007d7bbdbf1087bee83664279a` |
| Contract fingerprint | `0a51b97cb213887bd9b95a8e52230bcf5172a255e242597180b77a1f9e2a6503` |

Immutable outputs:

- `/nix/store/2g6iw11c0qr893d4ni4inc794n6cv7l3-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.19.0`
- `/nix/store/ypslmmdb95blwxz1xczzimmhlbg0z7si-cowboy-zed-server-x86_64-unknown-linux-musl-1.5.0`

The package-owned runtime builder verifies the exact declared versions, no
ELF interpreter or shared-library requirements, and copied-byte probes in
temporary credential-free homes. The existing `cowboy-first-party` publisher
signed the composite release. The independently selected immutable SDK verifier
`/nix/store/bdjawvlvkv9w0hfbq8aljwb9rsk5da8c-cowboy-plugin-pack-1.8.1/bin/cowboy-plugin-pack`
verified it against the configured trusted public key. There was no key
rotation, new publisher or Service login.

## Repeated acceptance

- Complete `just check-compact` against the implementation bytes: main Rust
  **1,483 passed / 34 explicitly ignored**, SDK **371 / 4 ignored**, Provider
  SDK **26**, private adapter **126 / 2 ignored**, frontend **1,815**, and all
  **18** isolated PostgreSQL tests. Package/component, source-copy, native-shell,
  website, strict types, composition, formatting, lint, dependencies and
  optimized builds pass. `just plugin-check` also passes independently.
- Native source-owned Nix derivation: **48 tests**, comprising 3 filesystem,
  4 LSP and 41 project tests. Five new groups cover retained raw/language
  snapshots, cross-thread last-clone release, cross-store detached-snapshot
  saturation with pre-I/O refusal, completed/unobserved and cancelled previews,
  and text branches/edited snapshots. The clean final builder selects this
  exact tested result; it does not imply a second native test run.
- Final immutable pair: native sync/navigation conformance **5.71 s**, including
  the actual two-worktree 64-acquisition limit, refusal, original mirror
  preservation, confirmed close, one-use operations and retained target reads.
- Temporary signed Plugin installation/uninstall/drain/reactivation lifecycle:
  **10.10 s**, including nonempty navigation and independent retained targets.
- All **19 connected v6 checks**, **170.74 s**, with actual disposable password
  authentication, Machine enrollment and installation through the supplied
  Controller/Machine and exact new pair. Four discarded real replies use the
  normal deadlines. Receipt: `stage=complete`, `failure=null`, `cleanup=true`,
  `accepted=true`. Forced fixture teardown is not production recovery evidence.
- Firefox 151.0.1: **24 buffer-owner regressions**. Fixture SHA-256
  `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`.
  Physical devices are not checked.
- Final immutable source-boundary gate passes at
  `/nix/store/phvbdyn0s1pmlqgbxglq28qhlxc2j2ky-cowboy-source-boundary`.

The connected inputs use Controller
`/nix/store/ygyk7dd8r0c92fxmk6a2zb47rrw52ndh-cowboy-controller-release`
and Machine
`/nix/store/03li1x7fh85ga3ycqiwwhz47l89af442-cowboy-machine-release`.
These are supplied fixtures, not a production installation. Their active role
and unchanged processes are observed separately below.

## Publication and actual reader floor

Before publication, all **88 releases** in the complete staged public Catalog
pass two cold process reads for each actual Controller role, plus each role's
actual-Service host-policy preflight. The same six reads and three host checks
pass against a complete public Catalog snapshot after publication:

- Active **and next-transaction recovery**:
  `/nix/store/ygyk7dd8r0c92fxmk6a2zb47rrw52ndh-cowboy-controller-release`.
  The component activator captures the currently active profile for recovery;
  this is not its historical `previousRelease` field.
- Actual NixOS cold pin:
  `/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release`.

The cold pin comes from the current host activation program, not an assumed
candidate. Host closure remains
`/nix/store/g612grcjk53982zsl0sl3yp2v84filgx-nixos-system-hawk-26.05.20260731.5b4f72e`.
Managed telemetry writer/background policies remain `unconfigured`; legacy
selection remains `not_checked`. No prior Catalog release was removed or
rewritten, and host policy is unchanged.
This does not rerun or relabel the prior host refresh's journal-reader matrix.

Canonical `just plugin-publish` independently verifies the signature before
writing immutable Catalog bytes. Publication completed at
**2026-09-19T03:10:36.837Z** in `/var/lib/cowboy/plugin-catalog`, receipt:
`receipts/zed-1.19.0-04bec5bfe03fbc6aeabe1701102dbbe24d3840007d7bbdbf1087bee83664279a.json`.
All three public HTTPS downloads match their signed hashes and immutable cache
headers/digest ETags. The existing delegated Operator refresh succeeds and its
Catalog advertises the exact version, kind, component release, package/composite
digests, fingerprint and Linux x86_64 platform as `ready`. Anonymous
`/api/plugins` remains HTTP 401; no browser credential is copied.

## Unchanged production and remaining work

The post-publication observation retains all **20 original Cowboy process
PID/start/executable identities**, including **18 ACP workers**. Controller,
Machine and Web profiles/receipts, the host closure, three Victoria process
identities, local/public health and SPA version remain unchanged. Resident
Machine generation is still `worker-795a7ae472286bc7993b`. Other Plugin
installation identities are unchanged.

Hawk Zed remains active `1.18.0`, composite
`92b1078ab8bd8030cda30f32e13c7c7b355ffbd4e9a9dda0a8085c23c0ef89a1`.
No install/upgrade operation was submitted for `1.19.0`. Publishing availability
does not apply this fix to an existing native process. Installation needs the
separate exact-release Operator step; retained native owners are not adopted.
Production private navigation remains closed. Global byte/history/background
budgets, independent restoration, supported-device acceptance and the other
[completion exits](../plugin-refactor-completion.md) remain open.

## Evidence

Private evidence directory: `/tmp/cowboy-native-snapshot.CcnGKI`. It retains
source/native/package logs, public Catalog snapshots, original runtime/signature
artifacts, bounded host projections and the connected receipt. No private key
or Service environment was written into these receipts.

| Evidence file | SHA-256 |
| --- | --- |
| `check-compact.log` | `b2e51936ce7be647957c2158bd0ec99444445ae33cb0b23a59b952defa9868e3` |
| `native-build.log` | `b9109a94b47b2d4c95536dee0c724a91c5e76aefe211ffc1f669400ce5b9c844` |
| `native-pair.log` | `2853aa7a82257ac1cb693f117979c923b4b187b9408f863605fdd215da28242e` |
| `lifecycle.log` | `f763e903e91511e64e2bfbd99a27a7796697498158d337f857ffd34d6d469db9` |
| `connected.json` | `15144885b606840abe4b34a79007edfafeb3e5f608edf8fac4227a0e765aeb38` |
| `browser.log` | `7758f63eb22d246c16492fc285cd89666cf9cda15d90951a3619085b6c822264` |
| `runtime-build-receipt.json` | `720e20ff4de6854a48e1f8c74ca599005f8d71b217b7b6a245f53f4a54695d9a` |
| `signed-release.json` | `e8798220c209f7ca6afe3be5a02982f8ae5e89baa2559316f469f949a0378c05` |
| `staged-floor.json` | `67663881727a9d5d2a85e7e3c43b690cbcf3b67623b98359b3562dc9dec53c07` |
| `published-floor.json` | `88804fc63f813dfb39312022c9b4141532aeeb1c3a2e39f647bbae07ace07a97` |
| `public-downloads.json` | `c88a4bd4a854e72ebfe28cfaf3c84ec8c7c7d36dc3f86dd8d2d1b395dce7ece6` |
| `audit-published.json` | `dcd57dbfa9c1be1d81d2633a1712e3b1fecbd324070be951c262dc0ce17d3df4` |

Private audit-helper setup initially required Deno `/proc` permission and an
explicit process-observation type; corrected helper checks pass. An initial
combined source-boundary/helper command failed only at that helper type check;
the independently repeated source-boundary command succeeds. These are not
product failures or relaxed acceptance conditions.
