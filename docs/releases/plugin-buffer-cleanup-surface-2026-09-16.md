# Core Code cleanup Web delivery — 2026-09-16

**Published on main and activated as Web only.** The
[Settings cleanup surface](../plugin-buffer-cleanup-surface.md) exposes retained
original owners without opening resources, polling, replaying uncertain release
or closing active consumers. This is not the ordinary Review cutover, a Plugin
release, native recovery or completion of the refactor.

## Source and artifacts

Clean implementation/delivery source:
`7d871abfe28a1c7588fffa1db0badb67aadd35b0`, based on freshly integrated
`768877f23c6e8e0704ba9b69cc28c6da0888e575`. The independent Markdown
workspace-link changes are preserved. Core Rust, Plugin sources, components,
locks and Nix definitions are unchanged. Service worker is `cowboy-v1698`. The
enclosing follow-up only records evidence; it does not change application bytes.

- Web release: `/nix/store/si6cn8hmskjg88qcnvzfd6ckg83rgwn1-cowboy-web-release`.
- Immutable Web assets:
  `/nix/store/pa10myv0gc8s0pq1nnszlyz85a60a1qp-cowboy-web-0.1.0`.
- SPA version: `f8a2cca0b11e64f997983e27aa3541ec`.

## Accepted checks

The complete pinned-shell
`nix develop -c env RUST_TEST_THREADS=1 just check-compact` passed on the exact
committed source: **1,346 main Rust, 308 standalone Machine, 26 core-adapter, 56
private Zed, 1,595 Web and 17 isolated PostgreSQL tests**, plus strict Clippy,
types, formatting, dependency audits, feature/Plugin/component contracts and
release builds. Main/Machine suites retain their 30/two explicit ignored tests;
this is not acceptance of every optional immutable/native gate. The separate
immutable Web build also passed. Existing dependency-policy, Web lint/chunk and
Nix hardlink-limit warnings remain visible; no host workaround was applied.

Nine new cleanup unit cases join the 56-test focused Code client suite. They
cover immutable stable observations, independently owned subscriptions,
late-added subscriber boundaries, listener teardown, pending cleanup, ambiguous
DELETE, foreign/serialized/retired handles, observer cancellation and
synchronous authority loss. Compile-only checks reject string/object cleanup
handles and a nonexistent open operation. Pre-auth product-context observation
is explicitly checked to create no discovery, storage or HTTP request.

Actual Firefox `151.0.1` runs from a fresh profile and private loopback
namespace passed all **30 cases** across four suites:

| Suite               | Cases | Fixture SHA-256                                                    |
| ------------------- | ----- | ------------------------------------------------------------------ |
| New cleanup UI      | 7     | `db0d29aaf60fe94854a97ef86dc68b544ee5956db8c3a47262f7c1ec32420d81` |
| Owner/content reads | 8     | `28e944fbdcf72a08471e2f7862ed2bdc5a8325aa0e9752084f08e983687dc6b3` |
| Product context     | 6     | `6d74748011061c0f4aa4689808174daf63602ca6ba09405d10945e62ffb3e2c2` |
| Settings recovery   | 9     | `3defc3d7fc05d6497481089f6db7598783b8675d13419d7e766816270e755be2` |

The new UI gate uses actual React/MUI/StrictMode and original core owners:
mount/disclosure stays local; confirmation cancellation and double clicks do not
queue/repeat release; unmount/remount retains admitted work; lost release and
404 stay unresolved; same-stack context loss rejects stale confirmation and
redacts private paths; twelve long-path rows remain bounded to five at 360px;
terminal removal cannot redirect an open confirmation to a replacement owner. No
real account, production endpoint or native Plugin is accessed. Firefox at 360px
is not physical iOS/WebKit acceptance. Preliminary fixture assertion/type
failures are retained separately and are not counted as passing evidence.

## Production activation

The installed machine-owned activator committed transaction
`1789535298999432546-7d871abfe28a` with `published=true`, without a recovery
override. Its predecessor was
`/nix/store/m5k1cab1cz9frpw8xv8a23vf1ndxhg4d-cowboy-web-release`, source
`768877f2`.

The bounded **13:08:14–13:08:39 +08:00** observation window (not an outage
measurement) retained all **14 worker** PID/start-time pairs and Controller,
resident Machine and Victoria processes. Controller/Machine profiles and
receipts, host closure/unit hashes, cold roots and failed-unit sets were
unchanged. Machine remained online with the same active worker generation,
workspace revision and identity hash. No installed Plugin or host policy was
changed by this task.

Local/public `/healthz` and `/version` passed. Index, admin, service worker and
both entry assets match the immutable Web output exactly, with no-store HTML/SW
and immutable hashed assets. An already-open PWA needs the updated bundle;
activation alone is not proof of a device reload or production native cleanup.

Private scratch evidence: `/tmp/cowboy-code-cleanup-pgjl3F2h`. It is not a
permanent public artifact. Selected SHA-256 values:

- Complete gate:
  `09320c27799a4f3268d6971ae7a71a50dc38da8d9644016f2a62b401d6fe1f44`.
- Four committed-source browser suites:
  `fd44253288654148943706148d818b80f15cc29147f5a6b483d6869cf7b6dfe9`.
- Immutable Web build:
  `9649ea8641e1ee67910cbfbde603a8adbfcccfcb64c730e737ed3464726f3d9c`.
- Component/process/HTTP audit:
  `d3cd757a39030eade33d546a9950fab4efd7769b9d8b670270f9335229e054ce`.

Remaining: ordinary Review integration, explicit disk/native synchronization
with its own effect authority, owned navigation destinations, independently
accepted/activated Machine and Code Plugin, abandoned-browser/restart recovery,
post-effect restoration and supported-device acceptance. The panel explicitly
describes this page only; an empty new page is not evidence of earlier native
release. See the [completion ledger](../plugin-refactor-completion.md).
