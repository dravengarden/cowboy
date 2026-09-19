# Session read routes — accepted Controller release

This delivers the finite [Session read-route binding](../plugin-session-read-routes.md),
not completion of the whole Plugin refactor. Core owns the logical Session and
its original execution route; no Plugin, SDK, native ABI or Machine protocol
version changes. No SQL baseline, durable codec, private policy or installation
is changed. Only the Controller is activated.

## Delivered boundary

All eleven buffered filesystem/Git readers now retain a privately constructed
`SessionReadScope`: the original Hub Session observation, core registry and
authenticated Machine connection. Typed dispatch, manifest readiness/selection,
complete HTTP response checks, file continuations and diff continuations use
that same route. Equal Machine names or epoch strings cannot renew it. Stale
buffered responses become `410/no-store` without stale bytes or an ETag; stale page
continuations refuse before another Machine command.

A disconnected named colocated Machine can no longer fall back to local I/O
using a saved database flag. The obsolete name-based locality APIs were removed,
not retained behind warning suppression. Standalone core-owned `local` Sessions
still work without a Machine. Logical/native resource lifetimes remain separate;
this does not adopt, release or restore native owners on reconnect.

Implementation: `4270c4fc`, followed by obsolete-API removal `a07ab9ce`.
Final accepted, published and activated source:
`9d20c97efb98819161ff4d5a945246443bb59a34`. It preserves remote main through
`01658d18`, including the already active `c82cdb04` permission/background-activity
changes. The integrated source was rebuilt and revalidated, not inferred from
the first candidate's results.

## Accepted gates and negative evidence

- Complete pinned `just check-compact`: **1,530 main Rust / 34 explicitly
  ignored**, **375 standalone Machine / 4 ignored**, **26 core Code adapter**,
  **126 private adapter / 2 ignored**, **1,833 Web**, and **18 isolated
  PostgreSQL** tests. Formatting, strict lint/types, feature boundaries, SDK/
  Plugin/component closure, 86 structural link vectors, dependency, native-shell,
  website, IndexedDB-harness and optimized shipped build checks pass. Existing warnings and
  intentionally ignored tests were not suppressed.
- **13 new Session lifecycle tests** cover foreign owners, same-epoch connection
  replacement, remote/colocated direction changes, both caches, cwd ABA,
  independent scopes, pre-enqueue refusal, parked replies, cancellation and
  manifest readiness, including the actual standalone Unix socket contract.
  Two pre-fix source regressions failed as expected; six focused relay/HTTP
  helper tests also pass, including the two new fixture/budget restrictions.
- Clean immutable `.#cowboy-controller-release` passes its **1,147 default-feature
  Rust tests / 18 ignored** and **3 bridge tests**.
- The new connected **v7** gate rejects the supplied pre-fix Controller
  `51c1a674`: after real connection replacement the old file cursor dispatches
  a **third** `coreFile` command and returns **502**. The failing receipt records
  `stage=connection_replacement`, `accepted=false`, `cleanup=true`; this is an
  isolated negative test, not a production outage.
- Corrected candidate `a07ab9ce` passes all **20** actual-process checks in
  **170.88 s**. The final integrated Controller passes all **20** again in
  **170.80 s**, with `stage=complete`, `failure=null`, `cleanup=true` and
  `accepted=true`. Both have exactly **two** core file commands across three
  authenticated connections: initial Unicode pages reconstruct the complete
  text; reconnection and Controller restart refuse the old continuation with
  `410/no-store/no-ETag` and no additional dispatch. All earlier native
  cancellation/lost-reply/no-replay checks remain enabled.
- Exact publication coverage passes for all **six** source Agent Plugins using
  the actual public Catalog, artifact bytes and original publication receipts.

Both successful connected runs use supplied Machine `7269199e` at
`/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`, adapter
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
and server
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`.
Receipts bind executable hashes and provenance, not merely version labels.
Disposable product authentication, enrollment and signed Code installation are
real; forced fixture teardown is not production recovery or native resume.

## Actual readers and activation

Pre-activation and settled snapshots each pass **8 cold Catalog reads and 4
actual-Service host-policy preflights**, covering all **94** releases from both
configured public Catalog roots. Candidate, active, next-transaction recovery
and actual cold readers agree. Pre-activation active/recovery is `c82cdb04` at
`/nix/store/6ng73d25ga5342xmmn6krg3726yb6n0z-cowboy-controller-release`; settled
active/recovery is the new candidate. Actual cold remains `94382b94` at
`/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release`.
Managed telemetry writer/background policies remain `unconfigured`; legacy
selection is `not_checked`. No private environment or credential was copied.
No journal codec/writer changed, so historical journal matrices are not recounted.

- Controller release:
  `/nix/store/gg0011rjg731fkxzyrsz5w7afa3f75n6-cowboy-controller-release`.
- Running executable:
  `/nix/store/3rai553rlirn7p9bdqbm62a92k1fpfx8-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `e1a340916cb9d794fa9384efc2cc7e6691ab6a8914de99319b4bff01c26eae7c`.
- Transaction: `1789805512929024487-9d20c97efb98`,
  `outcome=succeeded`, `phase=committed`, `published=true`, `maintenance=false`.
  Committed at **2026-09-19T08:12:07.869659045Z**.

Only `cowboy.service` restarted. During the bounded
**08:11:22.979–08:13:06.253 UTC** observation window, all **16 original ACP
worker PID/start/executable identities**, resident Machine and three Victoria
processes were retained. Controller PID changed from `3332222` to `3403154`,
whose executable is the accepted artifact. Machine is connected/online on the
unchanged `worker-3a3c8b33f96774b1981e` generation. This does not prove native
resume or uninterrupted activity outside the observation window.

Machine/Web profiles and receipts, host closure, all eight Plugin version/
digest/installation identities and available authentication generations are
unchanged across this activation. Web retains the parallel task's `01658d18`
release `/nix/store/89i2bv8vjsdswyys0r30xmyf9mzydkia-cowboy-web-release`.
Local/public health and version endpoints and exact index/service-worker bytes
pass. Their shell cache policy is `no-store`; SPA version remains
`c55ccf4bd908e771500737febf26491e`. No Web or Machine activation is part of this
release, and no production Plugin publication/installation occurs.

## Evidence and limits

Private evidence root: `/tmp/cowboy-session-read-routes.U7loU59H`.
Failed initial builds remain recorded: release-mode warnings exposed the unused
locality APIs, which were removed before both final gates. One coverage attempt
used the reader-only Catalog copy (which intentionally omits runtime artifacts
and original receipts); canonical coverage was rerun against the actual public
Catalog and passed. Neither failed attempt was used as release acceptance.

| Evidence | SHA-256 |
| --- | --- |
| `check-integrated.log` | `6049cc879127d6b2e669765ec141073adbf32a02912bf89b101c49d7cf17baed` |
| `build-integrated.log` | `4db5668ee7597a811c55e2454c1865c25ee25e003c235a5ced537ba9a1327133` |
| `connected-baseline-receipt.json` | `5bbe779c7bdfa115052dedcdc9d5e862d92f59d87f9a718f3aa0a560b23d7f2b` |
| `connected-integrated-receipt.json` | `a3778a2f7ac8345312cf9acf2f4cbb8000de582edfba4f7743847289f8f0d278` |
| `integrated-floor/floor.json` | `b82577fb8ecf649c5db129178cf5ac9a2d5a2ad878e6f464ffcd1120fb6361ce` |
| `settled/floor.json` | `a474ab9072050c2123a1fb18274f73c4509b7fcc48ae0e7889cb9b08b7b11f2c` |
| `observed-before.json` | `dbaae466ccc06a0be42ebc793eaaaca3e853710b5d8158d2db53bbc6e0ec8392` |
| `observed-after.json` | `c028d4ed2e8a748910114bd8b2e56881ab81cb0a1b55c7bfa3654ea42c08102c` |
| `coverage-final.log` | `247d2d13beb97601e7c89a76845108db6315dca120d7288e4c7acc1fc871f573` |

Still open: general verified graph resolution, continuous Machine-owned
Workspace/Session/security-domain identity, state reader/writer leases and
coexistence, Agent/native-generation acceptance, remaining native/history
budgets, supported-device consumers and independently authorized post-effect
restoration. This read observation is not an authority, filesystem/inode proof,
atomic HTTP-delivery fence or generic revertible effect. The
[completion ledger](../plugin-refactor-completion.md#code-work-still-required)
retains those exits.
