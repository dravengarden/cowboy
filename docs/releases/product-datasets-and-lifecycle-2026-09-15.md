# Product dataset bridge and durable lifecycle diagnostics

Source `a0956a42cda6aa5ead11fc16089c28572ab72642` was published to Cowboy
`origin/main` and its Controller was activated on Hawk on 2026-09-15. This is
**Controller bridge acceptance**, not the completed browser migration, Machine
maintenance, native acceptance or whole Plugin refactor.

## Scope

- [Product datasets](../product-sync-datasets.md): authenticated immutable
  Service/principal binding, selected WebSocket protocol, closed Service/Session
  keys, exact IDB v2 fencing and explicit bounded legacy export. Component
  registry 3.6.0 adds state-sync-idb 1.7.0 only; all Plugin source/version/2.9.0
  pins remain unchanged. `REQUIRE_BOUND_BROWSER=false` still admits old Web.
- [Core lifecycle history](../plugin-lifecycle-history.md): a bounded typed
  install/uninstall/resolution projection, shared Product/Admin view and exact
  Rust/Web fixture. Reads carry no effect authority and do not query the Machine.
- [Rustls advisory](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc):
  the newly published handshake issue caused the final dependency gate to reject
  0.23.40. The minimum/lock now use patched 0.23.45 and WebPKI 0.103.15, with the
  verified Nix vendor staging hash. No advisory ignore or TLS-policy relaxation.

## Immutable artifacts

| Component | Exact release | Status |
| --- | --- | --- |
| Controller | `/nix/store/02hzpfbjvg8cxz5pfmb6q1lgm0g312z2-cowboy-controller-release` | Activated |
| Web | `/nix/store/ldjxcjvmz9wr7v8l3ym3p4b25fdnpy2d-cowboy-web-release` | Built/verified, not activated; SW v1688 |
| Machine | `/nix/store/59w3sk0g3abpcp7md3ywnw64p0v21nfr-cowboy-machine-release` | Built/verified, not activated; `worker-48ad34f5c4615668b75f` |

The running Controller ELF is
`/nix/store/5zad3h063gg14bq013ygka6wk9m4716g-cowboy-0.1.0/bin/cowboy`, SHA-256
`764a156b4b17d9bacb61d31fa8eb5e7b20cbc4c0e77ba4097ba46777d2445ab8`.
The deferred Web/Machine outputs have task-owned GC roots under the proof
directory. They are exact acceptance evidence, not permission to activate an
ancestor after later main commits; rebuild an integrated descendant when the
maintenance boundary is approved.

## Gates

`just check-compact` passed: 1182 Rust tests (29 intentionally ignored), 285
Machine tests (2 ignored), 1481 Web tests, 17 PostgreSQL cases, 86 complete-report
Rust/TypeScript differential vectors, native/package/feature checks, format,
lint, dependency/security checks and release builds. The pre-existing SQLx/flume
spin 0.9.8 yank warning remains non-failing; it is not the Rustls vulnerability.

Clean committed source additionally passed the real pinned Firefox suites:
eight IDB connection cases, sixteen outbox cases and six React lifecycle-history
cases. This includes original-order v2/v1 fencing, independent Workers,
transaction abort/reopen, principal ABA, target replacement and late UI results.
No browser timeout was enlarged. A native-IDB-only development reproduction
motivated the product's transaction-lifetime connections; there is no claim of
an identified upstream Firefox bug or physical-device/power-loss acceptance.

Exact immutable Controller acceptance passed all six role/cold-open checks.
The candidate's two cold opens additionally checked genuine disposable login,
anonymous/Viewer denial, Operator no-store lifecycle reads, two domain-disjoint
attempts with the same ID, independent pre-effect resolution and unchanged
journals after three repeated HTTP reads. Recovery/cold legacy readers were
tested for explicit new-client refusal and their retained old-client behavior,
**not** labeled usable dataset-aware recovery.

All 168 Service installation reader and 72 Machine installation reader cases
also passed. Before the Controller transaction, recovery was the then-active
Controller `6fd7f2ca`, Machine recovery was the active `4817bc7f`, and cold roles
were the actual `2f7fa237` Controller and Machine bootstrap in the active host
closure. Full artifact/provenance/executable-chain hashes were checked. These
are reader/diagnostic gates, not a new production Plugin installation, retained
worker TLS update, managed Victoria cutover or native-generation restoration.

## Activation

The machine-owned transaction
`1789404104621576469-a0956a42cda6` completed with `outcome=succeeded`,
`phase=committed`, `published=true`. Only `cowboy.service` restarted. During
00:41:25–00:42:52 +08:00, all 14 worker PID/start pairs, the Machine and three
Victoria process identities, host files/system closure, Web/Machine profiles,
and failed-unit sets remained identical.

Local and public `/healthz`, `/version`, five exact SPA/admin/asset files and
their no-store/immutable cache headers passed. Web remains `a00aac6f`, SW v1687,
version `a5a575af00bf603ea4bdab3e66cdee38`; Machine remains `4817bc7f` and reports
online/connected with `worker-240c2080a8bf9eb8968f`. No production login,
Plugin installation, private policy read/change, SQL migration rewrite, browser
record deletion, forced Session rebind or Machine/worker restart was performed.

## Remaining boundary

Hawk cold recovery is still Columbus `8e358bab` / Cowboy `2f7fa237`. An owned
host recovery-floor maintenance transaction is required before the IDB v2 Web
cutover; then activate the accepted Web and a separately gated descendant that
requires browser/native-shell binding. Do not lower the database version,
delete old records or call a legacy reader's refusal usable recovery. The next
Controller transaction now recovers to **a0956a42**, not this receipt's historical
`previousRelease` 6fd7f2ca.

Rustls also changes the Machine/worker build generation. Their independently
approved maintenance and supported-session acceptance are not implied by this
Controller update. Real account/device security checks, managed Victoria
Operator/policy acceptance, general graph/state resolution, capability-specific
acceptance, independently authorized post-effect recovery and archival remain
in the [completion ledger](../plugin-refactor-completion.md). The iPhone pasted
image caret issue remains unrelated and unsolved.

## Evidence

Private proof root: `/tmp/cowboy-plugin-completion-Ax6aBV`; fixture credentials
and disposable browser profiles were not published. `attempts.md` retains failed
expectations, source-check failures, the new security advisory and vendor-hash
diagnostics. Earlier ddae188d artifacts are not approved after that advisory.

| Evidence | SHA-256 |
| --- | --- |
| `check-compact-06.log` | `8ad88225c8d762dba56d91f59dc3ef21287c3afcec6b7f565e249b6135ab5225` |
| `browser-clean-02.log` | `588774e17bf9a423fe86fc010c588f900a1776e6cb9187df89da418c13dea602` |
| `final-gates-audit.json` | `50b02093205637d26c2e6e913e14e83897db032cdfb68daad1ddbcef65fae15f` |
| `activation-audit.json` | `df202e292412afa968e4068a56f187af1cb46cde726492faf68c4de975d74034` |
