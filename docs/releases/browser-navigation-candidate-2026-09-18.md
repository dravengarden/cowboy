# Browser navigation continuation candidate — 2026-09-18

Implementation and clean connected harness source:
`5f4e2281db64dd8e24c78ac293e7df9ae7813575`.
This source-only candidate implements the finite
[browser navigation continuation](../plugin-browser-buffer-navigation.md).
It does not publish/install a Plugin, activate application components, enable
production acquisition or replace a resident Machine/native generation.

## Scope and review

Core owns the original buffer, closed navigation transport, identity lifetime,
capacity and cleanup. The captured LF text/UTF-16 point and five query kinds
cross a checked Service contract; native references and serialized destination
IDs never become browser authority. Execute is one-use, lost Execute/Release is
query-only, and view cancellation drains the original continuation. Known
Unknown/Retained outcomes cannot become inert Expired. A pending or unresolved
group keeps the source fence; ordinary cleanup performs no implicit navigation.

Local capacity includes all 32 pending/retained groups. The recovery projection
has only original-handle Query/Release, with synchronous redaction after identity
loss. No product navigation or recovery panel is activated. Destination
preparation/adoption, intended Review destination views, native pre-acquisition
budgets, signed runtime rollout, device acceptance and independently authorized
post-effect recovery remain distinct exits.

No production Rust behavior, Machine protocol, native source/dependency or
Plugin manifest changed. The Controller source closure includes one additional
shared test fixture; the Machine closure explicitly excludes it.

## Source gates

The focused owner gate passes 103 tests, including 19 new navigation tests.
Web typecheck and lint pass. The actual Rust Prepare/Execute/Query/Release
handler test matches the complete shared JSON fixture, normalizing only random
lookup IDs and preserving no-store headers. Compile-only tests reject forged
captures, exchanged identity domains, generic operations and destination import.

The first typecheck caught a misplaced `@ts-expect-error` comment after automatic
line wrapping. The directive was moved to the exact rejected object property;
the type boundary was not weakened. Review also added the explicit source
cleanup-busy guard and its regression before the final browser runs.

The complete pinned `just check-compact` gate passes on the implementation:
formatting, lint,
dependency/contract/component/feature gates, 1,474 all-feature Rust tests,
365 Machine tests, 26 core-adapter tests, 102 private-adapter tests, 1,767 Web
tests, 18 isolated PostgreSQL tests, the Web bundle and optimized Rust builds.
It retains the existing telemetry lint, transitive `spin 0.9.8` yanked-version
and Web bundle-size warnings; no unrelated dependency was upgraded or warning
suppressed. Dedicated process gates remain separate from ignored unit cases.

The final follow-up exposes the existing passive `navigations` projection at the
product root, without `ready()` or discovery. Expanded product tests prove
unauthenticated subscribe/get cause no discovery, storage or HTTP, and that
ready consumers get the same projection. Web typecheck/lint, all 1,767 tests,
the production bundle and all seven browser suites pass again. This follow-up
changes no Rust, transport or native behavior; the full and process acceptance
below retain their stated source identities.

## Actual browser acceptance

All seven final isolated Firefox suites pass: **56 checks**. The owner suite
requires 18, including seven new navigation cases. The browser is immutable
Firefox `151.0.1`, using a fresh temporary profile and private loopback network,
with no production account, cookies, storage or endpoints. These fixtures use
actual WebCrypto, Response streams, clicks, cancellation and the existing React
development StrictMode harness, not the intended navigation consumer or a device.

| Suite | Checks | Final fixture SHA-256 |
| --- | --- | --- |
| Code buffer owner | 18 | `14dac2a35a01a2a9a98a583bb71550b512508daa74cc872856dd3e6f221f5929` |
| Product context | 6 | `e6bb6ed6ef9bf8212189651fa9d471b6121ff2b9a499b134e8d8b2d0d8087fea` |
| Cleanup | 7 | `c82460f18281aa1b15dadd43a2201c03e439b7da3a9d01c1931629d4dda8d8ac` |
| Synchronization | 8 | `1fdc8230c47a16a98549cff4bcaaf95295b3edfcd280f875afb6c26dd78caf70` |
| Review source | 6 | `27f298a5b0aa89a7dfdba4d92230d6a997c54a799bdacbac45ea9667a2e81a66` |
| Review working diff | 6 | `dda5c05c5ad2754a06363752a44054f22cfe796f3cfba37da57fa108b694085b` |
| Review document refresh | 5 | `91c057c52f27a4a1509254e9801276e0849fe339e83e91fe1f3204fb0b4fd95d` |

The exact browser executable is
`/nix/store/bvshrp9mdjbmx1asq5ndikxim24jfxfv-firefox-151.0.1/bin/firefox`.
The canonical release skill and browser runner now require the eighteen-case
owner gate. Historical eleven-case receipts remain historical; they cannot
accept this navigation extension. There is no PWA version bump or deployment
receipt because the candidate enables no product entrypoint.

## Connected and Nix acceptance

The v5 connected gate passes all **18 checks**, with `accepted: true`, stage
`complete`, no failure and verified fixture cleanup. It uses the exact immutable
Controller/Machine and Zed 1.13.0 static pair from the
[native-text candidate](native-text-reads-candidate-2026-09-18.md#exact-inputs).
Their executable hashes were independently rechecked before this run. No
runtime changed in this browser slice, so rebuilding or activating them merely
to change a source label is unnecessary.

Actual disposable login/enrollment, signed Code installation/uninstall, all
five nonempty navigation kinds and ordinary destination reads pass. Nine real
replies are held and three discarded, with the original 40-second transport
timeouts. Lost Execute/Release is reconciled by original ID without replay;
the opened destination retains two-page Unicode text after parent release and
path removal. Six navigation executions and five releases are observed. The
test-only explicit stdio LSP has unchanged SHA-256
`93742240881b5d45acf317a4a9927e8832f3d6110106d49b2bdde747746a1d76`.
These synthetic answers and forced fixture containment are not real language
acceptance, intended-consumer acceptance, restoration or product cleanup proof.

The clean committed Nix check
`checks.x86_64-linux.cowboy-source-boundary` also passes, output
`/nix/store/2mv88ghcyh2s7c0rzs6py4qi63bps693-cowboy-source-boundary`.
It builds the cropped Controller and proves the shared navigation fixture is
included there and absent from Machine sources, while preserving the existing
component executable/dependency boundaries. An initial invocation used the
nonexistent package attribute; selecting the existing `checks` attribute fixed
that command error without a source or dependency workaround.

## Audit artifacts

Temporary audit directory: `/tmp/cowboy-browser-navigation-ReTlBQ`.

| Evidence | SHA-256 |
| --- | --- |
| Complete `full-check.log` | `11efafa0da0ff854533676c31289fa6251dea2065de014dd2542534270db569e` |
| Focused `owner-check.log` | `db41a3eae8f44c0ba14903f1c5367e4d04b393de677afee6c567363f1816e8c3` |
| Final `web-final.log` | `e81fd76534a7ab40580fdb8054c780c34bdb816e3234ceb4e64ed1011d98bee5` |
| Accepted `connected-1.json` | `e44fde2b6d41752d4dd790d9c7314cb789e27a6397b02f52876ad0936add8677` |
| `connected-1.log` | `37c75bbbdd2f04185e7f123762475fb096c9d733e85d5635ba5030a1561eac46` |
| `source-boundary-2.log` | `854e0463dc971d8749c46d01724caa6e230c9712b4ba596dae1b3ef436e794fb` |

The seven final `*-product.log` browser receipts bind the fixture hashes above.
No production account, policy, installation, Controller, Machine, native worker
or PWA activation was changed. The separate acquisition budget, intended view,
supported-device and recovery exits in the
[completion ledger](../plugin-refactor-completion.md) remain open.
