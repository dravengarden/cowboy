# Continuous Workspace reads — Controller release, 2026-09-19

The [continuous Workspace observation](../plugin-workspace-read-scopes.md) is
published on main and active on Hawk. It binds the existing eleven core
filesystem/Git readers, their buffered responses and file/diff continuations to
the original authenticated Machine connection and continuously advertised root.
It is a finite read fence, not whole-refactor completion or a filesystem lease.

## Scope and exact source

Only the live registry constructs the opaque, process-local identity. Removal,
path change, duplicate ID, invalid/over-budget observation, disconnect and even
same-epoch connection replacement end that original lifetime. Re-adding the
same strings cannot revive it. Unrelated roots and display metadata preserve
continuity. Persisted enrollment must remain non-revoked and agree with the
unambiguous live root; persisted paths alone cannot mint a scope.

The closed Code executor derives Machine, adapter and root from that scope,
checks it under the dispatch lock and checks again after awaiting a reply.
The existing response guard discards stale bodies, ETags, errors and conditional
responses. Old file/diff cursors cannot be adopted by a recreated root.
Cancellation releases only an RPC waiter; it does not retry or clean up a
Workspace, Session or previously dispatched effect. Session compatibility
routing is unchanged.

The new live map is bounded to 1,024 roots per Machine, 4,096 overall and 8 MiB
of logical identity strings. It is not a bound for the existing event history
or native process memory. No Plugin/SDK/native/wire version, SQL baseline,
persistent journal, private policy, account or production installation changed.

Implementation: `d6c29310`. Final clean accepted and deployed source:
`51c1a6743b364e797be85ddbdd9e6603f0375dca`, including remote main through
`6e7dfc5b`. Concurrent upstream Web/core sync changes were preserved and the
final integrated source was revalidated, not inferred from the earlier run.

The integrated component check exposed an upstream app-shell source change
without its independent release entry. `eba97386` records app-shell **1.1.4**
and appends component registry **3.13.0**. All historical registry entries are
unchanged; the shell has no Plugin consumers, so no Plugin manifest, version,
pin, artifact or dependency was changed to repair this metadata. This does not
publish a Plugin or activate a new Web bundle.

## Accepted gates

- Complete pinned `just check-compact`: **1,512 main Rust / 34 explicitly
  ignored**, **375 standalone Machine / 4 ignored**, **26 core Code adapter**,
  **126 private adapter / 2 ignored**, **1,825 Web** and **18 isolated
  PostgreSQL** tests. SDK, component/Plugin closure, the 86-vector structural
  link gate, formatting, lint, strict types, dependency, native-shell, website,
  IndexedDB and optimized shipped builds pass. Existing warnings and ignored
  tests were not suppressed.
- **14 new Workspace tests** cover actual SQLite enrollment/resolution,
  connection and root ABA, independent scopes, revoked/mismatched inventories,
  count/byte bounds, pre-enqueue refusal, parked replies, cancellation,
  buffered headers and both continuation caches. The initial remove/re-add and
  same-epoch reconnect regressions failed before the repair. These are core
  source fixtures, not production filesystem identity acceptance.
- Clean immutable `.#cowboy-controller-release` build passes, including its
  **1,131 default-feature Rust tests / 18 ignored** and **3 bridge tests**.
- Connected Code **v6, all 19 checks**, **170.84 s**, with this exact Controller,
  supplied Machine `7269199e`, private adapter **1.20.0** and server **1.6.0**.
  Receipt: `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`.
  Actual disposable authentication, enrollment, installation, native reads,
  lost replies, cancellation and no-replay pass. This is the existing Session/
  buffer regression chain, not an additional connected Workspace fixture,
  production Plugin installation, native restoration or supported-device test.
- Exact signed-publication coverage passes for all **six** source Agent Plugins.

Supplied immutable Machine:
`/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`.
Supplied adapter/server:
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
and
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`.
The receipt binds exact executable hashes and provenance, not just versions.

## Catalog and host boundaries

Both pre-activation and settled snapshots cover all **94** releases from the
two configured public Catalog roots. Each snapshot passes **8 cold Catalog
reads and 4 actual-Service host-policy preflights**: candidate, active,
next-transaction recovery and actual cold Controller. All readers agree.
Recovery means the current profile a new transaction would capture, not an
older receipt's historical predecessor.

Pre-activation active/recovery is `6e7dfc5b` at
`/nix/store/9zlqspbvc318084n5w6hr84c4rj9h6h9-cowboy-controller-release`;
settled active/recovery is the accepted candidate below. Actual cold remains
`94382b94` at
`/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release`.
Managed telemetry writer/background policies remain `unconfigured`; legacy
selection is `not_checked`. No private environment or credential was copied to
the evidence. No persistent codec/writer changed, so the historical 807-role
journal matrix is not recounted as a new gate.

## Actual activation and continuity

- Controller release:
  `/nix/store/j8nvm8yqs2yx3m97y9dz37izz5fhii8m-cowboy-controller-release`.
- Executable:
  `/nix/store/aw7qk01lyfm0p9qf6f2iavdz568d8ccw-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `08591ba4df90ae00b9aa6f522958fe5b2ff810ad6e0b89a4d46029e0ae77c70a`.
- Transaction: `1789800599812996193-51c1a6743b36`,
  `outcome=succeeded`, `phase=committed`, `published=true`, `maintenance=false`.
  Committed at **2026-09-19T06:50:23.966433564Z**.

Only `cowboy.service` restarted. In the bounded
**06:49:36.096–06:50:48.354 UTC** observation window, all **16 original ACP
worker PID/start/executable identities**, resident Machine and three Victoria
processes were retained. Controller PID changed from `2977326` to `3051194`,
whose executable is the accepted candidate. Machine remains online on
`worker-3a3c8b33f96774b1981e`. These observations do not prove native resume or
continuous absence of interruption outside that window.

Machine/Web profiles and receipts, host closure, all eight Plugin version/
digest/installation identities and available authentication generations remain
unchanged. Web stays at `6e7dfc5b`, profile
`/nix/store/hb7b9f7fw74z4337vd7zv0ddd9mfrkci-cowboy-web-release`.
Local and public `/healthz`, `/version`, exact index and service-worker bytes
pass; index/service-worker cache policy is `no-store`. SPA version remains
`eb4f844f5d9603f7efabfbac21942701`. No new PWA bundle is part of this release.

## Evidence and remaining work

Private evidence: `/tmp/cowboy-workspace-scopes.e8ax2bCR`.

| Evidence | SHA-256 |
| --- | --- |
| Final complete gate, `check-current.log` | `c6cffd156238b0ee1e3d587ad73319d5b61bb14bb819f069ac95eab061f1caa0` |
| Immutable build, `build-current.log` | `4e71ae4004ac2b6f4618fecfc228704aef75eaa34c568f147136c559464be0da` |
| Connected v6 receipt, `connected.json` | `eae8346b935282e3a3acfd2da2469470821a412a51a9da63c9b2549c40a0af45` |
| Accepted continuity summary, `audit-after.json` | `78a9c278b6492ad77c4830d5e669a7bae87412ca3c59f682778a595d3786e72e` |
| Full bounded post-observation, `observed-after.json` | `d329ea7a91ed81c0b581096d0b589364fad38f1b810fe1d08df907c712dc3c7e` |

Earlier successful source/build runs, the integrated component-metadata
failure, and superseded runs intentionally stopped for newer upstream source
remain separate logs. They are not relabeled as final acceptance. An ad-hoc
broad test selection inherited `COWBOY_PROVIDER_PACKAGE_PATH`; the complete
recipe's existing environment isolation passes those Supervisor tests without
changing product code or skipping tests.

Controller-observed continuity cannot detect an unreported filesystem or
Machine configuration replacement. Continuous Machine-owned Workspace/Session/
security-domain identity, general graph linking, state reader/writer leases,
independent post-effect recovery, aggregate native lifetimes and supported
client/account acceptance remain open in the
[completion ledger](../plugin-refactor-completion.md). No cancelled observation
or stale-response refusal is described as undoing an already-dispatched effect.
