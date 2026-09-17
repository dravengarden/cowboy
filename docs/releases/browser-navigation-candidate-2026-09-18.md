# Browser navigation continuation candidate — 2026-09-18

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

The complete pinned `just check-compact` gate passes: formatting, lint,
dependency/contract/component/feature gates, 1,474 all-feature Rust tests,
365 Machine tests, 26 core-adapter tests, 102 private-adapter tests, 1,767 Web
tests, 18 isolated PostgreSQL tests, the Web bundle and optimized Rust builds.
It retains the existing telemetry lint, transitive `spin 0.9.8` yanked-version
and Web bundle-size warnings; no unrelated dependency was upgraded or warning
suppressed. Dedicated process gates remain separate from ignored unit cases.

Committed-source connected/source-boundary acceptance is recorded below when
completed; source/browser success alone is not a substitute for those gates.

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
| Product context | 6 | `aacf2294dd040189565252fb960ba498a670d5a0a230ee61a96b73d788e4e405` |
| Cleanup | 7 | `30185e24156871de8430a9b66bb96c15c00e8081c82cff9afc693ef45f42c719` |
| Synchronization | 8 | `cf581542b3e687758031f89a92f8bd0ac73bcb97673a00082c60af677f169a4b` |
| Review source | 6 | `e74d866fec8c5a79a45ed301173031a9cbb3bb7d5868309e87c4f2fe7e208075` |
| Review working diff | 6 | `5702df40d496804501543a1dbcc55de1c9cce32a855b4c910dc15e7379bb41a3` |
| Review document refresh | 5 | `414b930f4fd7aa9c59a75b1d8d4bffd16cb638109427ad8625173a5ad4c62c5e` |

The exact browser executable is
`/nix/store/bvshrp9mdjbmx1asq5ndikxim24jfxfv-firefox-151.0.1/bin/firefox`.
The canonical release skill and browser runner now require the eighteen-case
owner gate. Historical eleven-case receipts remain historical; they cannot
accept this navigation extension. There is no PWA version bump or deployment
receipt because the candidate enables no product entrypoint.
