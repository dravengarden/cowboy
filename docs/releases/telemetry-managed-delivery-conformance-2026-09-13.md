# Immutable managed OTLP delivery acceptance: 2026-09-13

Hawk's actual active, next-transaction rollback and cold Controller/Machine
artifacts passed the extended
[connected gate](../telemetry-connected-conformance.md): **45/45 results**, five
flows over all nine role pairs. The new managed-delivery flow completed
**144/144 rounds**, with **324** correlated real Machine RPC receipts and
isolated HTTP requests. This accepts the real immutable runtime path with
synthetic identities and a protocol receiver, **not a real Victoria database,
production Operator account, owned policy cutover or P2 exit**.

## Source and verification

Accepted clean harness source: `6e059779311cc459b3268bd7a2075f680971079f`.
Changes are test-only Rust helpers, recipe comments and contract/skill docs.
There is no runtime, protocol, journal schema, applied SQL baseline, signed
Plugin/SDK, native ABI, Provider or worker-generation change. The canonical
release workflow therefore requires no runtime activation just to replace its
embedded source revision.

`nix develop -c just check-compact` passed: format, lint, dependencies, feature
slices, **1,084 library tests** (22 explicitly ignored), binary/adapter tests,
**1,426 Web tests**, all **15 separately isolated PostgreSQL tests**, and
release builds. The existing Vite chunk-size warning remains non-fatal. Targeted
reader conformance unit tests and all-targets/all-features Clippy also passed.

Every immutable gate ran from that committed source, in an offline loopback-only
namespace with cleared child environments and disposable private state:

| Gate                                                      | Required checks | Result   |
| --------------------------------------------------------- | --------------- | -------- |
| Authenticated connected pairs, including managed delivery | 45              | Accepted |
| Independent writer policy / finite effects                | 294             | Accepted |
| Populated two-Site readers                                | 96              | Accepted |
| Managed Controller startup / local OTel recording         | 78              | Accepted |

The complete connected matrix took **312.97 seconds**. Each managed flow took
28.10–28.30 seconds; its lost-export-ACK round took **15,221–15,255 ms**,
including the real 15-second runtime deadline and the quiet no-retry check.
Existing binding/recovery ACK fallback intervals remained **45,000–45,001 ms**
and **15,001–15,004 ms**. No runtime timeout was shortened.

## Accepted managed-delivery boundary

The harness uses actual product password login, a genuine cookie, an enrolled
temporary Machine identity, protocol 18 and an actually installed temporary
signed Victoria release. It does not borrow production credentials, forge an
Operator cookie or enable local-auth fallback. Only ordinary binding admission
is enabled for this flow on either Site; recovery and Service resolution remain
separate purposes exercised by other flows.

All nine managed pairs completed the same sixteen stages:

- Unconfigured and binding-only intake record all signals locally, with no
  implicit exporter. Explicit private policy for the exact current binding
  activates logs, metrics and traces.
- The real Controller queue and typed bound RPC reach the real Machine exporter.
  Its HTTP requests use the three Victoria-compatible OTLP routes and a
  synthetic bearer token. The receiver decodes official protobuf, checks
  cumulative metrics and records only signal/count/hash/response class.
- Official protobuf partial success, HTTP 503, 429 and 307 produce exact
  rejection/failure counters without retries or redirect following. Local JSONL
  recording survives all failures.
- One actual success ACK is dropped; another delivery is followed by a forced
  disconnection. The Controller records uncertainty, never resends, and a new
  connection exports only new batches.
- Revoke and restore advance the durable binding head and policy epoch. The
  original policy cannot export under either new head. Both processes reopen
  with that stale policy: optional export stays stopped and local intake works.
- A new exact-head policy reactivates delivery. Both processes reopen again
  without replaying local files or an old queue. A final metrics-only policy
  keeps logs/traces local while sending only metrics.

Each pair admits 58 batches and deduplicates their 58 identical repeats before
aggregation/export. Only 36 batches reach RPC and HTTP. The proxy and receiver
independently agree on signal, item count and payload hash for every attempt;
actual Machine receipts correlate to unique typed request digests. The four
partial-success batches account for exactly four rejected items. Service/Machine
journals stay unchanged during recording and export rounds; the only binding
writes are the separately confirmed select/revoke/restore. All three explicitly
created background policies retain exact private file bytes and inode/ownership
metadata. Telemetry already emitted is `NoRestore`; restoring configuration
cannot undo it.

The receiver is an isolated protocol fixture, not a Victoria server. These tests
do not prove database ingestion, retention, queries, actual credentials or the
external network. No production endpoint or token is copied into receipts.

## Actual roles and production continuity

Independent read-only captures at `2026-09-13T23:28:24+08:00` and
`2026-09-13T23:41:16+08:00` agree on host closure, source, component profiles,
receipts and next ordinary rollback targets. Cold roots are resolved from the
actual active closure's absent-profile activation script; no candidate artifact
is substituted. No activation was in progress.

| Site / role                       | Immutable release                                                              | Embedded revision |
| --------------------------------- | ------------------------------------------------------------------------------ | ----------------- |
| Controller active / next rollback | `/nix/store/gz5vcgfjrm2zqa22pc08i6pbdi57j6p2-cowboy-controller-release`        | `c1a7752f`        |
| Controller cold                   | `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`        | `c1a7752f`        |
| Machine active / next rollback    | `/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`           | `faa0451c`        |
| Machine cold                      | `/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release` | `c1a7752f`        |

Columbus remains `1da44eb88883b52a7bff266da0c07ede35193659`; host closure
remains
`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`.
The independently resolved matrix SHA-256 is
`8643187cfc2e7db257612cacab3dec506c59a4bc1ba68a7eb92ad8fe60be3a8d`. Receipts
retain exact manifest, launcher and ELF-chain hashes.

Controller PID/start stays `252472` / `2974253925088`; Machine stays `110025` /
`2971136905262`. All thirteen worker PID/start pairs are unchanged, snapshot
SHA-256 `3c0d2852cf7db32792896e104ecec0c55baac7eecd33a02b92a745e7302f7d64`.
Health/version, Machine presence, Web root and cache headers/ETags agree. No new
failed system/user unit appeared. This is a bounded-run comparison, not a
guarantee about later independent tasks.

Machine writer policy and binding journal remain absent, including no dangling
symlink. No production host policy, endpoint/token, managed namespace, component
profile or session was changed. There is no restart or PWA reload for this
test/documentation publication.

## Retained evidence and remaining work

Private evidence directory: `/tmp/cowboy-managed-egress.axhA3l/`. It contains
the quality/gate logs, before/after role and continuity captures and independent
aggregate audit. All four gate receipts are atomic create-only files, mode 0600:

| Receipt under `dist/`                                                                | SHA-256                                                            |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `telemetry-connected-conformance/20260913-managed-delivery-1.json`                   | `ef46585326f24ff68e748120bd513ac035300e78c4537ba34d33be211e0e561d` |
| `telemetry-writer-conformance/20260913-managed-delivery-regression.json`             | `787d22ca5efff82033f361762f2eca1992a173170e88b3cb4fdd661dc38cbaf0` |
| `telemetry-reader-conformance/20260913-managed-delivery-regression.json`             | `c22f024822167a688dbada951e7ecc6f2d2cd90f0250cec0195a7b312a949ea3` |
| `telemetry-background-startup-conformance/20260913-managed-delivery-regression.json` | `21b6347f0c9019f49f43cc250ba20ae973f8e74947017260d81cf56b591096ff` |

The immutable matrix passed on its first run. Earlier unit testing corrected a
receiver test's assumption about SDK fixture order. The independent aggregate
audit initially expected a bare request hash; the actual protocol's
`BindingDigest` requires `sha256:` plus 64 lowercase hex digits. The audit was
corrected to require that exact format and then passed. Failed unit/audit logs
remain retained; no runtime, gate oracle, receipt or host capture was rewritten
to manufacture acceptance. Prior connected-gate attempts remain recorded in the
[earlier acceptance](telemetry-connected-conformance-2026-09-13.md).

Next boundary: accept the complete intended Service/Machine production
configuration, actual Operator authorization, configured real Victoria delivery
and the owned writer/background-policy cutover with production failure/restart
acceptance. The first managed Service intent permanently fences legacy export,
even if refused or aborted. Do not open writers alone, silently migrate private
configuration or erase evidence to regain legacy export. This closes another P2
acceptance gap, not the generic Plugin DAG/refactor or later P3/P4 migration.
