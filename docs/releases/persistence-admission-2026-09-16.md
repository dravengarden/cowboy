# Controller persistence admission repair — 2026-09-16

The admission repair is published on `main` and active on Hawk. Local and public
health return HTTP 200; the new Controller epoch has zero rejected intents and
zero failed database batches. **Recovery of the two historical rejected intents
remains unknown; this repair did not replay them.** Healthy new counters do not
establish their recovery.

## Exact release

- Source: `255fe2b725010ea564d94c7f80e223deb9d0a9a7`, clean committed source.
- Controller: `/nix/store/sd524wjyvjlfllzr70y40rbf489r0sv0-cowboy-controller-release`.
- Actual predecessor/recovery: `/nix/store/n2r5956hr0ac0xiqvg3d04qmgivipgak-cowboy-controller-release` (`2fb32d52`).
- Owned transaction: `1789573076743366809-255fe2b72501`.
- Started `2026-09-16T15:37:56.743Z`; committed `15:38:14.809Z`.
- Receipt: `succeeded`, `committed`, `published: true`, `maintenance: false`.

Only the Controller component was activated. No Machine/Web activation, Plugin
installation, policy cutover, SQL migration, credential change, event replay,
manual health reset or forced worker/native retirement was performed.

## Repair and verification

The [admission contract](../persistence-admission.md) replaces the racing shared
byte check and detached metadata waiters with one bounded FIFO. An independent
oversized lane prevents a large event from excluding small subsequent writes;
finite control reservations preserve accepted order. Attachment/settings size
accounting, receiver ownership, shutdown drain and writer batch bounds are
covered. Genuine overload/failure still degrades health; this is not unlimited
lossless buffering, durable spooling or a new Plugin lifecycle.

The complete `CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 just check-compact` gate
passes: **1,374 main Rust, 314 standalone Machine, 26 core-adapter, 74 private
adapter, 1,613 Web and 18 isolated PostgreSQL tests**, plus strict Clippy,
formatting, dependencies, type/feature checks, composition and release builds.
Main/Machine/private-adapter retain 32/two/two explicitly ignored tests; all 18
PostgreSQL cases ran separately in the owned temporary cluster. Existing unrelated
lint/dependency/chunk warnings were not suppressed. All eight `nix flake check`
checks and published Agent release coverage pass.

Thirteen queue regressions cover concurrency, finite budgets, ordering, close,
drop, cancellation and payload accounting. Three writer tests cover SQLite,
clear/new-event order and shutdown. Both SQLite and PostgreSQL drive the real
writer with a 16 MiB event followed by a 437-byte event, another session's
lifecycle event, and reserved title/settings writes; reopen verifies exact
content/sequence and latest settings. Early fixture attempts failed on an
unseeded Machine alias and an unsupported setting key; corrected fixtures and
the final complete gate pass. These tests never use production storage.

The canonical `release-cowboy-plugin` skill's Controller checks were applied,
without releasing/installing a Plugin: candidate, actual next-transaction
recovery and cold `cc09k6l7…` read the same **84-release Catalog twice**. Their
actual-Service configuration-only host/telemetry reports match, including
`catalog_only`, exact selections, managed policies `unconfigured` and legacy
selection `not_checked`. No private environment or destination policy was
retained. No journal schema or writer authority changed.

## Production observation

The bounded continuity window is **23:37:40–23:39:15 +08:00**, not an outage
duration. All **16 workers and four private Code processes** retain PID/start
identities, including the existing 1.7.0 pair and the later 1.8.0 pair. Machine
PID `1974549` and generation `worker-4208d4d141de95cf9feb` are unchanged. All
installed Plugin identities and the saved Zed installation operation match.
Victoria processes, Machine/Web profiles and receipts, host/cold closure, unit
hashes and failed-unit sets are unchanged. A later final sample remains healthy
and connected with 16 workers; normal draining workers were not force-replaced.

Machine reconnect reread a preexisting owned workspace manifest: advertised
revision changed from `e4000509…` to `5f38ce9d…`, while all 31 workspace IDs and
their digest stayed identical. The first strict audit refused an unchanged-
revision claim. The final audit binds the new projection to the exact manifest
and its **symlink** modification time (`15:09:05.397Z`, before this activation),
not the immutable target's epoch mtime. It does not claim the full workspace
projection stayed unchanged. The retained Machine's own source reloads that
manifest on every Controller connection; no Machine restart is required.

Local/public health, version and five exact SPA/admin/SW/entry files with cache
headers pass against the unchanged Web root, version
`4d8405ee6e987d3fa911a2bb540bdf0b`. This is HTTP acceptance, not physical-device
login, UI gestures, turn-history restoration or complete Plugin refactor acceptance.

An independent earlier Web transaction, `1789566575530411524-8edf513e0307`, was
already `recovery-required` before this repair. Its profile/root had returned to
the preceding `0b256bc0` release, and its journal is preserved unchanged. This
Controller activation does not finish or re-run that Web transaction.

## Evidence and limits

Private evidence root: `/tmp/cowboy-persistence-admission-x5GCkydu`. The final
activation audit supersedes the earlier failed/insufficient workspace checks;
original evidence was retained, not overwritten.

| Receipt | SHA-256 |
| --- | --- |
| Complete serial gate | `a1a691df75c939802cfcce174183797a18919f38d1c52f6320074908df04f796` |
| Full flake gate | `b079fff1416d877707ecfc41b4e6ee2a415e3a5b707564339f80b087e7a914ba` |
| `activation-audit-final.json` | `a036505a4d0c2db968ed9003c21c6054ac53bb8c54ec078960fe65a843aa96ef` |
| HTTP receipt | `8e3abe83c23b438aa3e544e5506117fc55d6fc5c60c0d01739e097bd2b63a576` |

The original queue rejection logs contain no sufficient event identity/content
to reconstruct the two lost intents. Their recovery remains unknown. The
verified but unactivated Machine candidate, independent Web recovery, core
buffer synchronization authority/Review integration and other
[Plugin completion exits](../plugin-refactor-completion.md) remain separate.
