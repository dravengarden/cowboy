# Machine navigation continuation candidate — 2026-09-17

This accepts the protocol-21 Machine continuation and Zed `1.12.0` support
probe as a **source/immutable candidate**, not public navigation, production
installation or component activation. The [finite contract](../plugin-machine-buffer-navigation.md)
extends the [private destination handoff](owned-navigation-handoff-2026-09-17.md)
without treating a serialized resource ID as execution authority.

## Immutable inputs

Clean candidate source: `3b6448b28077e87b34c8dff5b24770d2f597628d`.
It includes the direct-navigation synchronization-expiry fix found in independent
review. The final complete gate and connected receipt use clean harness revision
`65cfc384deb3265aae9666459ddca051a5edca90`; that descendant changes only the
temporary lifecycle test's operation order and explanatory comment. It does not
replace the candidate Machine or native bytes below.

Only Linux x86_64 is declared. The adapter and consuming Plugin advance together
to `1.12.0`. Private server `1.0.0`, upstream Zed revision, third-party pins,
shared component versions and historical registry entries are unchanged.

| Candidate | SHA-256 |
| --- | --- |
| Static Zed adapter `1.12.0` ELF | `0843d5e24232220aa8e7188b03c578f5414adf2d749b37ec958ea7a4ff95acbc` |
| Unchanged private server `1.0.0` ELF | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Machine release entrypoint | `4309fb66697afd5387f3aab425f258e00eac78e16c51cb446dc7062f073f8407` |
| Machine wrapped ELF | `8ba1d81028ff147d12b8eb609e042dc80818513c161a2546dc4abc057626b651` |

Immutable outputs:

- Adapter: `/nix/store/7xbr0gcgh02lvz7m8msrmzp800gq05hc-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.12.0/bin/cowboy-zed-adapter`.
- Server: `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`.
- Machine: `/nix/store/3mn3lphcq80cm9r32a7shxljb4qsfkjz-cowboy-machine-release`,
  candidate worker generation `worker-338844f9f4884884cb33`.
- Machine wrapped ELF: `/nix/store/9mv8h8z2xxqimqj3h70sbrxjpblqp54j-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.

The canonical runtime builder passes its static dependency audit and closed,
credential-free probes. Generated artifact URLs are candidate metadata, not
proof that those binaries have been published.

## Verified scope

Machine-issued invocations bind the original Service/Machine connection and a
15-second monotonic command budget. Separate core navigation IDs retain the
exact original runtime, route, one-use acquisition and uncertain outcomes.
Only inert preparations expire. Destination reservations use the existing
ordinary buffer capacity and continuation, with saved lookup after observer
loss and no path or installation reselection. Released snapshots are bounded
historical evidence, not rollback or fresh runtime-liveness guarantees.

The change adds **22 core tests** and one private-adapter test. Coverage includes
closed codecs, Site/protocol admission, original-connection revocation, queued
expiry, cancellation, ambiguous replies, no replay, runtime death, capacity,
source synchronization/release exclusion and independent destination routes.
Direct navigation now clears expired inert synchronization fences itself;
pending/unknown effects remain protected.

The pinned `CARGO_INCREMENTAL=0 just check-compact` passes with
`RUST_TEST_THREADS=4` and the inherited Provider-package override unset. Final
counts: **1,451 main Rust, 361 standalone Machine, 26 core-adapter, 98 private
adapter, 1,736 Web and 18 isolated PostgreSQL tests**, plus three Codex bridge
tests, SDK/component/package isolation, composition conformance, formatting,
strict Clippy, dependency/type/feature checks and release builds. Opt-in process
tests run separately below. Existing dependency/chunk warnings are not suppressed.

Three isolated process gates pass:

1. The exact static adapter/server navigation gate passes twice. A test-only
   explicit stdio LSP supplies all five nonempty kinds, duplicate cross-file
   locations and non-BMP UTF-16 ranges. The actual private socket loses a handoff
   reply, queries the original lease and reads/releases the independent target
   after parent/path removal. Existing native synchronization checks also pass.
2. The temporary signed lifecycle gate passes twice after correcting fixture
   ordering, including once on the final clean harness. All five kinds run
   through Machine core, with complete target identities and one acquisition
   per kind. A destination prepared before uninstall opens on the retained
   original process afterward, then supports exact-content hover and independent
   release after parent/path removal. Each of three fixture documents has one
   observed LSP open/close. This uses synthetic core authority and language answers,
   not an enrolled protocol-21 Service/browser navigation consumer.
3. Connected-buffer receipt v3 accepts all **11** authenticated HTTP/control/native
   checks with the Machine above and supplied Controller
   `/nix/store/4yj5b7fpnmjb65jnrsls116g6shimg1g-cowboy-controller-release`
   (source `e8f2c249328964c9a22bd536bedbb80339ce27b6`). This older protocol-20
   Controller exercises existing buffer/synchronization regressions, not the new
   navigation command. The receipt reports `accepted: true`, `cleanup: true`,
   no failure, three connections, six held replies, one discarded reply and one
   cut. The unchanged real 40-second lost-Apply timeout dispatches only one Apply.
   Independent audit checks the eleven expected unique results, exact supplied
   inputs, nine executable digests and both source-metadata digests. Supplied
   artifacts do not constitute acceptance of actual production host roles.

## Corrections and remaining boundaries

Independent review found that direct navigation did not reap an expired inert
core synchronization reservation. The candidate fixes this and tests both
fresh Prepare and existing Execute, while refusing expiry of an uncertain
effect. The earlier candidate without this fix is superseded.

The first signed lifecycle attempt opened a navigation source after establishing
a live native synchronization reservation. The native process-wide exclusion
correctly refused it. The fixture now acquires navigation first and settles
synchronization before destination Open; no production guard was weakened.
An initial lint failure on an oversized test helper was resolved by extracting
the acquisition helper.

Service/principal/Session navigation ownership, an enrolled protocol-21 consumer,
Review integration and supported-device acceptance remain open. Native queries
may allocate before adapter result limits reject them: global allocation bounds,
OS filesystem isolation and independent saved-query recovery are still unproved.
Observed fixture close events and forced teardown are not universal native close
acknowledgements, production reclamation or rollback. General graph/state leases,
independently authorized restoration, signed publication/installation and resident
Machine maintenance remain separate.

Following the canonical release skill, this task verifies immutable candidates
and temporary signed fixtures only. It issues **no production Catalog write,
Plugin installation or component activation** and makes no claim about concurrent
host changes by other tasks.

Raw local evidence is under `/tmp/cowboy-machine-navigation-eizDQ63e`:
`check-final.log`, `private-check.log`, `core-check-2.log`, `review-fix.log`,
`runtime-build-final.log`, `machine-build-final.json`, `machine-build-final.log`,
`native-pair.log`, `native-pair-final.log`, `signed-lifecycle-2.log`,
`process-gates-final.log`, `connected-input.json`, `connected-receipt.json` and
`audit-receipt.ts`. Earlier attempts remain in `core-check.log`,
`check-compact.log` and `signed-lifecycle.log`. The accepted connected receipt
SHA-256 is `432a4599a111c03196040d2fb5d05f27ca70ffc24c606342d863b94a4019991e`.
