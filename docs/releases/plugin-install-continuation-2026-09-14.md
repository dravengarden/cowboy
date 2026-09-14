# Core Plugin installation continuation release: 2026-09-14

Controller and Web source `a1bfd754c5e0157cc2da4f617f5f0793d3feb890` is
published to remote `main` and activated on Hawk. It includes freshly fetched
`94aa2af8` and preserves the incoming mobile keyboard/swipe change. No Plugin
package, SDK, protocol, database format, native ABI or Machine release changed.

## Fix and acceptance

The [core-owned install/upgrade attempt](../plugin-install-continuation.md)
survives cancellation of its HTTP observer. It owns the shared lifecycle fence,
rechecks the original purpose-bound Operator, exact trusted Catalog envelope,
applicable compatibility and original authenticated connection at effect
boundaries, and retains uncertainty after dispatch. Generic rejected ACKs do
not prove activation/authentication was undone. Only a proven unsent command
releases that pre-dispatch fence; no forward retry or inverse is fabricated.
Post-install authentication failure reports installation plus pending auth,
not a failed installation that can safely be replayed.

The independent admin entry was calling a nonexistent `/install` suffix. It now
uses the registered Plugin resource route and refuses unbound/missing-digest
releases before fetching. Both new Web regression tests failed before this fix.
Eight installation tests and one nominal-authority test cover request binding,
cancellation/panic, each authority boundary, transport certainty, real
connection replacement/late ACKs and fourteen authority/identity changes.

This attempt is process-local, not a durable install protocol. Restart recovery,
installation receipt correlation, independent post-effect restoration and the
other [refactor exits](../plugin-refactor-completion.md) remain unfinished.

## Gates and exact artifacts

`nix develop -c just check-compact` passed: format, Clippy, dependency and
independent-feature checks, **1,125 Rust library tests** (23 explicitly ignored),
**272 standalone Machine tests** (2 ignored), binary/adapter tests, **1,436 Web
tests**, **15 isolated PostgreSQL tests**, and release builds. The first full
run exposed two stale SW assertions inherited from `main`; these now enforce
the required minimum SW revision rather than a stale exact version. No runtime
failure oracle was weakened. The actual legacy Catalog location
`/var/lib/cowboy/plugin-catalog` covers all six exact embedded Agent releases.

Both Nix releases came from the clean committed source:

- Controller: `/nix/store/6yvnjqc5m9pjkpji34azcb054fa42zlq-cowboy-controller-release`.
  Actual ELF SHA-256: `d8854c83698c6a84aef413f308a1056be31445ba02fecea1fadd7017fe15b603`.
- Web: `/nix/store/nyp0190ssjsfy4n5xkdjlfadw7yyhkl9-cowboy-web-release`.

Candidate configuration-only preflight passed as the Service owner in an
isolated network namespace. It reports writer/background policies
`unconfigured` and legacy selection `not_checked`; it did not open production
storage or authorize managed export.

| Immutable gate | Checks | Result |
| --- | --- | --- |
| Populated Service/Machine readers | 96 | Accepted |
| Independent writer admission | 294 | Accepted |
| Managed startup/local recording | 78 | Accepted |
| Connected protocol/fault flows | 45 | Accepted |
| Real disposable Victoria database pairs | 9 | Accepted |

The matrix uses the new Controller, the actual pre-transaction active Controller
`/nix/store/3wbj6ky4kqp5b88fiv7p3h2pxb28lifn-cowboy-controller-release` as this
transaction's recovery target, and the actual cold Controller. The three Machine
roles are unchanged. After activation the new Controller is also the next
transaction's automatic recovery target; historical `previousRelease` is not
that role. Exact manifests, executable chains and real database ELFs were
independently hashed. All six runtime roles and all nine role pairs were tested.

Connected acceptance includes 144 delivery rounds and 324 correlated RPC/HTTP
exports. Real database acceptance adds 18 query rounds and 36 exports, with
exact stored logs/metrics/traces, database reopen and no host-restart replay.
These use isolated fixture credentials/databases, not production Operator
authority or production managed Victoria acceptance.

## Activation and bounded host observations

The independent machine-owned activators committed:

- Controller transaction `1789362169352547720-a1bfd754c5e0`.
- Web transaction `1789362238763628176-a1bfd754c5e0`.

Both receipts say `succeeded`, `committed`, `published: true` and name the exact
candidate and predecessor. Controller PID/start changed from `1814627` /
`3029815133421` to `2408697` / `3045103285534`, and its live ELF matches above.
No Machine, worker-generation, NixOS, Catalog or host-policy activation occurred.

Captures at `13:02:16`, `13:05:01` and `13:07:10` (+08:00) preserve all **13
worker PID/start pairs**, Machine PID/start `2131179` / `3036770249940`,
Machine profile/receipt, all three Victoria process/executable identities, host
closure and workspace projection. Machine is online at generation
`worker-240c2080a8bf9eb8968f`. There are no new failed system/user units. This
bounded continuity window does not prove a generation swap or native resume.
Production Machine writer policy and managed binding ledger remain absent.

Public HTTPS and local HTTP serve the exact new admin document and SW bytes.
HTML/SW remain `no-store`; the actual new admin JS is content-addressed and
`immutable`. The SW is **`cowboy-v1683`**. `/version` correctly remains
`20f7d02a362993ce2435824446506366`: it hashes `index.html`, which is byte-identical
because this Web fix changes the separate admin entry and SW only. The initial
post-activation audit incorrectly required a changed main-SPA hash; its failed
result is retained. The corrected audit verifies exact served index/admin/SW
bytes, not an assumed global version bump; after and settled audits both pass.
An already-open native/PWA client still needs its normal update or hard reload.

## Retained evidence

Private evidence is under `/tmp/cowboy-plugin-completion.3fpsxn/`. It retains
both full-gate logs, builds, Catalog/preflight results, five immutable gate
receipts, activation logs and host captures. The premature receipt-audit attempt,
empty canonical Catalog lookup and failed SPA-hash assumption are not acceptance
and were not overwritten. `audit-receipts-2`, `audit-after-2` and `audit-settled`
record successful independent checks.

| Receipt | SHA-256 |
| --- | --- |
| `reader.json` | `ba7cbf48214eb37b6c100b8091bfdd834b90ae18a72aaef853e9eb36b3f5dc8b` |
| `writer.json` | `9e7f387420c125456593001f5ef595471073686852fdc52001d8d625485577b7` |
| `startup.json` | `b8b46600389eb7cf1c723daaf91d3042e27329c1a6ab1016b20f462da14fdab6` |
| `connected.json` | `123968f7d11fdfe4f42c904d3b1175ae30e41ccc443e686bef126880c534a87b` |
| `victoria.json` | `0ce7e8e247b1881f8ec298ff5a4e0843ba8d531d37edc88460820fdf4a4ac4fd` |

Matrix SHA-256: `8c953c09c6213b00b32d64c6a60ad806dcafb469c78deb26d36a808264d08459`.
Preflight: `8f54098a30f83d5f5fe4f8dc8b2131a0d2aee4a35d6587fffe68b850f5cd08f6`.

The subsequent core-only [Rust/TS structural linker](../plugin-composition-checker.md)
adds its differential gate without changing these production artifacts. It
does not satisfy live verified resolution, durable installation, general state
coexistence, recovery or actual client/Operator acceptance. The
[completion ledger](../plugin-refactor-completion.md) remains authoritative.

The subsequent full `check-compact` also passed (`quality-3.log`), including all
86 complete-report differential cases. Its Web `dist` is byte-for-byte equal
to the active Web root; Nix still evaluates the identical Controller package
derivation `fpy53dpygcg9wy9d6c7gi8qx30ynfrlv-cowboy-0.1.0.drv`. No additional
process restart is needed for the core-only checker, gate and documentation.
The first TS nominal-brand compile failure, reproduced property-order projection
failure and inherited build-environment spawn refusal are retained in the
private test logs. The accepted implementation uses an actual private Symbol,
stable projection constructors and an empty inherited CLI environment; it does
not bypass type checking or broaden subprocess permission.
