# Authenticated immutable telemetry pair acceptance: 2026-09-13

The actual Hawk active, next-transaction rollback and cold Controller/Machine
artifacts passed the new
[connected conformance gate](../telemetry-connected-conformance.md): all nine
role pairs, four flows each, **36/36 accepted**. This closes authenticated
synthetic cross-process binding/recovery acceptance, not the complete production
configuration, real Operator account, managed-export cutover or P2 exit.

## Source and gates

Accepted harness source: `f46727db48a68563dd200764a836204fefb67cbe`. Changes are
test-only Rust helpers, the recipe, ignored generated receipts and
contract/skill documentation. Runtime code, wire protocols, durable schemas,
applied SQL baselines, signed Plugin/SDK publications, native ABI and worker
inputs are unchanged. Under the canonical release workflow, this does not
require activating a new runtime just to update its embedded source revision.

`nix develop -c just check-compact` passed: format, lint, dependencies, feature
slices, **1,082 library tests** (22 explicitly ignored), binary/adapter tests,
**1,426 Web tests**, all **15 separately isolated PostgreSQL tests**, and
release builds. The four new ordinary fixture/relay/purpose tests are included.
The existing Vite chunk-size warning remains non-fatal.

All immutable gates ran from the clean committed source above, offline inside
loopback-only namespaces with cleared child environments and disposable state:

| Gate                                              | Required checks | Result   |
| ------------------------------------------------- | --------------- | -------- |
| Authenticated connected pairs                     | 36              | Accepted |
| Independent writer policy / finite effects        | 294             | Accepted |
| Populated two-Site readers                        | 96              | Accepted |
| Managed Controller startup / local OTel recording | 78              | Accepted |

Each connected pair uses real fixture password login, a genuine product cookie,
an enrolled temporary Machine identity, protocol 18 and an actually installed
temporary signed Victoria release. There is no forged cookie, local-auth
shortcut or production credential. Only each flow's necessary writer purposes
are open; the recovery flow leaves ordinary binding closed on both Sites.

Select/revoke/restore, logout, lost previews, replacement connections,
independent Machine recovery and Service resolution, and exact durable receipt
reads after both processes restart passed. Proxy counters and journal evidence
agree: no resend, implicit resolution, worker/session dispatch or export
command. The measured binding command-to-fallback query interval was
**45,000–45,001 ms**; recovery was **15,002–15,004 ms**. The real runtime
deadlines were not shortened. The complete connected matrix took 236.75 seconds.

Authenticated local OTLP logs, metrics and traces still grew private JSONL files
after reopen without remote attempts. This does not verify external delivery.

## Failed attempts and the corrected harness

Three unsuccessful matrices are retained, never overwritten or counted as
acceptance. The first two had 18 failures; the third had 19. Lost-ACK flows
returned uncertainty in under a second instead of reaching the real deadline.
Bounded HTTP and relay diagnostics identified the cause: the proxy rejected the
normal Core `SetDesiredGeneration` startup frame, closing the connection. The
third run also caught one ordinary round-trip in that same startup race. Every
rejected frame in that run was this initialization frame.

The correction is confined to the harness. The relay permits exactly one
initialization per connection, only for the authenticated Machine's own
generation, with no worker executable. Readiness now waits for it. Tests reject
foreign generations, executable overrides, duplicates, wrong direction and
session/Provider commands. The relay still forwards original bytes and cannot
manufacture an acknowledgment. No production runtime fix was necessary.

## Actual role and continuity evidence

Read-only captures at `2026-09-13T22:13:17+08:00` and
`2026-09-13T22:37:01+08:00` agree on the active closure, component profiles,
receipts, next ordinary rollback targets and cold outputs. No component
activation was in progress. Cold roots came from the actual active closure's
absent-profile activation script, not substituted candidate artifacts.

Columbus revision remains `1da44eb88883b52a7bff266da0c07ede35193659`, active
closure:

`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`

| Site / role                       | Immutable release                                                              | Embedded revision |
| --------------------------------- | ------------------------------------------------------------------------------ | ----------------- |
| Controller active / next rollback | `/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release`        | `c1a7752f`        |
| Controller cold                   | `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`        | `c1a7752f`        |
| Machine active / next rollback    | `/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`           | `faa0451c`        |
| Machine cold                      | `/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release` | `c1a7752f`        |

The receipts hash exact manifests, launchers and ELF chains. Independently
resolved matrix SHA-256:
`8643187cfc2e7db257612cacab3dec506c59a4bc1ba68a7eb92ad8fe60be3a8d`.

Controller PID/start remains `252472` / `2974253925088`; Machine remains
`110025` / `2971136905262`. All thirteen worker PID/start pairs are unchanged,
snapshot SHA-256
`3c0d2852cf7db32792896e104ecec0c55baac7eecd33a02b92a745e7302f7d64`.
Health/version, Machine presence, workspace projection, Web root and cache
headers/ETags agree; no new failed system or user unit appeared. This is a
bounded-run comparison, not a guarantee about future independent tasks.

Machine writer policy and binding journal remain absent, including no dangling
symlink. No production policy, endpoint/token, managed namespace, Provider
state, component profile or host configuration was changed. No restart or PWA
reload is required for this test/documentation publication.

## Retained receipts and next boundary

Private evidence: `/tmp/cowboy-connected-conformance.MK45jX/`, including quality
and matrix logs, baseline/after captures and the final independent aggregate
audit. Its initial audit stopped because it expected 18 failures in the third
negative run; the actual 19 and their shared relay rejection were verified
before the final audit passed. No receipt or host capture was rewritten.

All receipts below are atomic create-only files, mode 0600:

| Receipt under `dist/`                                                         | SHA-256                                                            |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `telemetry-connected-conformance/20260913-actual-pairs-4.json`                | `37f01e0ea89cdc34e80983f7a660ef55ef0950faf9bb001106a53028fce0e350` |
| `telemetry-writer-conformance/20260913-connected-regression.json`             | `e3be357222cb1a8c4c95a7939bdf4490cc7de6df59c90e1867b45c012877defb` |
| `telemetry-reader-conformance/20260913-connected-regression.json`             | `3d4cf0d205f36363c2a559d1b47a6a72e77cd0e338b8a0b44704fc44e1fd9124` |
| `telemetry-background-startup-conformance/20260913-connected-regression.json` | `afd3f7a1796dfb905911aa4c6f522128f8e514e14feb8abd86a4e8dd290b6d5a` |
| Negative `telemetry-connected-conformance/20260913-actual-pairs-1.json`       | `b8baf00e0b57441d0ccda5f504a5d418de053a63cf4b98c146af698a8a5f3e25` |
| Negative `telemetry-connected-conformance/20260913-actual-pairs-2.json`       | `07b462ecab6bfe08ad571ff26c724f878c8082b6c12995e5cfe730d4f16e0100` |
| Negative `telemetry-connected-conformance/20260913-actual-pairs-3.json`       | `a97be62cacfe574ecf18c066f6fe436a72d4e61e6a56ee39e764190153b67e78` |

Still separate: full intended Service/Machine production configuration,
production Operator authorization, configured external Victoria delivery and the
owned writer/background-policy cutover with production failure/restart
acceptance. Fixture password login is not registration or production identity
acceptance. Bounded proxy faults are not arbitrary network faults or physical
power-loss testing. The first managed Service intent permanently fences legacy
export, even on refusal or abort. Never open production writers alone or erase
evidence to regain legacy export; emitted telemetry remains `NoRestore`.
