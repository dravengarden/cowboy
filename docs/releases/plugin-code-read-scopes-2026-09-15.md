# Scoped code reads — Controller release, 2026-09-15

The [finite code-read repair](../plugin-code-read-scopes.md) is published on
main and active on Hawk. Zed calls retain one exact Session observation; diff
pages bind their actual read context and a unique cache-entry cursor, with
UTF-8-safe offset validation. This closes a finite read/response boundary, not
the full Plugin refactor or reversible execution.

## Source and activation

- Source: `0b2d879ef5134b68790cfba561232b44169090ca`, clean and published.
- Controller release:
  `/nix/store/7a1rjwg135k1zxf286y1b1kr55pv1h74-cowboy-controller-release`.
- Executable:
  `/nix/store/bzni2n92ha8111wqvs59wjgz0ypgvg60-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `38d8ca75a0da990027809e4c524b7f83fba4cc3560c69370d6e6a087c9be8a6c`.
- Transaction: `1789450479484454492-0b2d879ef513`, `outcome=succeeded`,
  `phase=committed`, `published=true`.
- Accepted predecessor:
  `/nix/store/qvr3sm7a4h1bbx54z1vqmjwsdf5sp03h-cowboy-controller-release`,
  source `fa1fd219bb46ed6675ea538f8d202c65cd6e61e7`.

The machine-owned component activator restarted only `cowboy.service`. No Plugin
package, public Catalog, SDK, wire protocol, SQL migration, persistent format,
host policy, Machine/native generation or Web source changed. Rollback
compatibility concerns only unchanged persistent formats and transient cursors;
a Controller restart already expires its in-memory diff cache.

## Verification

The pinned `just check-compact` passed: **1,211 Rust library tests, 285
standalone Machine tests, 1,481 Web tests and 17 isolated PostgreSQL tests**,
plus the 86-vector Rust/TypeScript structural link gate, formatting, strict
Clippy, dependency audits, feature/native-source/Plugin closure checks and
shipped builds. Existing ignored tests and Web/advisory-policy warnings were not
suppressed. The clean committed Nix Controller release build also passed.

Twelve tests were added. The two initial regressions failed before the repair:
equal-content files selected the wrong continuation path, and a cursor within a
Chinese character panicked. The targeted 17-test run includes Session cwd ABA,
delete/recreate, same-name independent Hubs/Sessions, identity-axis mismatch,
unrelated metadata continuity, cursor eviction, concurrent snapshot coalescing,
page reconstruction and both real Unix socket and fixture Machine-channel reply
timing. These are core fixtures, not an actual installed Zed generation or a
production login.

This read-only scope/cursor release does not change installation or telemetry
journals, writers or configuration. Their previous immutable 807-role matrix is
historical evidence for the preceding release; it is not recounted as a new gate
for this commit. No managed Victoria cutover, Plugin install, compensation or
native restoration was attempted.

## Production observations

The bounded observation window was **13:34:09–13:35:10 +08:00** (61 seconds, not
an outage measurement). Controller PID changed from `1937491` to `2020262`, and
the running executable matched the accepted artifact. All **15 worker** PID and
start-time pairs, resident Machine and three Victoria processes remained
unchanged. The Machine stayed online on `worker-48ad34f5c4615668b75f`, with its
workspace revision and identity hash unchanged.

Web/Machine profiles and receipts, host closure/unit hashes, cold roots and the
system/user failed-unit sets were unchanged. The existing
`xdg-desktop-portal-gtk.service` failed state remains; it was not cleared or
repaired. No new failed service appeared in the window.

Local and public HTTPS checks accepted `/healthz`, `/version`, exact index,
admin, service-worker and both entry-asset bytes, including cache headers. Web
stayed at `92997a83`, SPA version `9dcf7c01602e4bf519e697e619761b03`; no new Web
bundle or PWA update was needed for this release. This bounded process evidence
does not prove physical-device behavior or a native-generation swap.

Private evidence is retained at `/tmp/cowboy-code-scopes-ruqIqE7t`: baseline
regression failures, targeted and complete gates, immutable build, bounded
before/after snapshots, activation receipt/journal and exact public/local HTTP
checks. Source gate SHA-256:
`5a8b4f258d77dca6f6eb392aa696c4a22aa7a4e74baf1f6515441c705d3d0163`. Targeted
gate SHA-256:
`aa1753f85f05c468eb05248896495325d7c3c0a4586f877ca9cc5e8bb6ed9dc9`.
Activation/HTTP audit SHA-256:
`8b5ab2796c50e5a4674d75e59d0e42e9f6f694b46f9f3280e671099bb39bcb1c`. An
intermediate test compile error was repaired without changing the production
response's debug/serialization surface; its failed log remains separate.

Continuous Machine-owned Workspace identity, remaining readers, effect fences,
state leases, independent post-effect/native recovery and supported-device /
account acceptance remain in the
[completion ledger](../plugin-refactor-completion.md). Discarding a stale reply
does not undo an already-dispatched effect.
