# Atomic browser outbox Web release — 2026-09-14

The [atomic outbox delta implementation](../atomic-idb-outboxes.md) is published
on Cowboy main and active on Hawk's Web lane. This accepts updated-peer
concurrency and the bounded deployment below, not all P3 or Plugin refactoring.

## Source and immutable activation

- Cowboy source: `2246b288f1d44d70b3f7859127573a0c99d0ac31`, clean and published.
- Web release:
  `/nix/store/03jy4djqjwgvy7697szjs5av5dyirgx5-cowboy-web-release`.
- Web bytes: `/nix/store/7hm2c41zl543f8av8ma416bv52ycym13-cowboy-web-0.1.0`.
- Transaction: `1789388079585385466-2246b288f1d4`,
  `outcome=succeeded`, `phase=committed`, `published=true`.
- Service worker: `cowboy-v1686`.
- SPA version: `5d47e59a36f004c239df8182f4f6bbfa`, the index content hash,
  not the Git revision.

The machine-owned `cowboy-web-activate` recipe moved only Web resources.
Controller and Machine remain at
`4817bc7f6ca095aa8de279ec6c9287261be865e6`, with their installation writers
unchanged. The host closure/cold recovery pins, Worker generation
`worker-240c2080a8bf9eb8968f`, Plugin installations, Catalog and Victoria
configuration were not changed. No production login or Plugin operation was
used as a smoke test. This receipt documentation is a later source-only commit;
it does not imply a new component activation.

The component registry appends 3.5.0 for state-sync 1.5.0 and state-sync-idb
1.6.0. Historical matrix entries, all seven Plugin sources/versions and their
exact 2.9.0 pins remain unchanged. These are core implementation changes, not
new installable Plugins or a new Catalog publication.

## Acceptance

The complete pinned `just check-compact` passed:

- 1,170 Rust library tests, 285 standalone Machine tests and 1,459 Web tests;
- 17 PostgreSQL contract checks, including the unchanged installation readers;
- format, Clippy, dependency/license/advisory policy, source/feature boundaries,
  component closure/package gates, native-shell checks, website and the
  86-vector composition gate, followed by Web/Rust/adapter builds.

The Web count includes 17 new outbox tests: exact mutation identities, seeded
four-owner delta/reference comparison, strict transaction outcomes and
baseline preservation, load handoff before reentrant observers, late hydration,
no-send after a failed durability barrier, and retained original obligations.

Pinned Firefox 151.0.1 executed the existing eight owned-IDB cases plus ten new
outbox cases in a fresh profile/private loopback namespace. The new cases include
two independent Workers making 20 concurrent write rounds, stale acknowledgement
handling, real replicated clients and cold retry ids, abrupt Worker termination
before and after commit, actual put-success abort, old-record/v1 readability,
incompatible version-change and independent record keys. The final clean source
repeated both suites successfully; earlier new-suite and paired runs also passed.
The fixture bundle hashes were:

- IDB: `ad3ff4fb9af1c34d6e3fddb7bf8ccd9b3cb9801a7698f53299061e450efe2f53`.
- Outbox: `4be89ec0855c0b0d2c6f9377a326cf887b4f83975586fe11e3ca0a968affa69d`.

The test browser is an optional Nix test artifact, not a product dependency.
Worker termination is not physical power loss or browser-process crash evidence.
Unchanged non-fatal Web spread/chunk-size and yanked transitive `spin` warnings
remain. An initial development package check ran before appending the matrix
and correctly failed its version check; the complete final gate passed after
the append. No requirement was relaxed. A non-fatal Nix evaluation-cache busy
message did not prevent the immutable Web build.

## Production verification and limits

The measured activation window was **20:14:36–20:15:17 +08:00**, 41 seconds:

- Controller PID `3820978` and Machine PID `3819042`, both monotonic start
  identities, all 13 worker PID/start pairs and all three Victoria PID/start
  pairs were unchanged.
- Controller/Machine profiles and receipts, the host closure, and failed-unit
  sets were unchanged. Machine remained connected with the same worker generation.
- Local and public HTTPS `index.html`, admin, service worker and the two entry
  assets exactly matched the immutable build. Index/admin/SW used `no-store`,
  hashed assets `immutable`. Both `/healthz` and `/version` passed.

No currently open PWA or physical device was remotely reloaded as verification.
Existing clients must refresh to load this fix. Database name, store, schema
version 1, record keys and `{base,pending}` encoding are unchanged and older
readers remain compatible. **Old blind writers are not fenced**: a pre-upgrade
page or a reverted old bundle retains the original overwrite risk. No historical
outbox data was removed or migrated.

General principal/dataset identity, cross-version exclusive ownership,
browser/host power loss, Safari/native acceptance, native runtime generation
recovery and independent post-effect Plugin restoration remain explicit exits
in the [completion ledger](../plugin-refactor-completion.md).

Private local evidence root: `/tmp/cowboy-outbox-delta.j5AOkY`. SHA-256:

| Evidence | Digest |
| --- | --- |
| Complete source gate | `2df50710a82728db578fc3b6808bbfd7281f57c0fc917ed3feb21e1722292600` |
| Clean-source paired browser gate | `d9d118726d6f0ce0cf6f13731483b39d49f21fee4cb22b282fb0e3b86c8f518c` |
| Bounded activation audit | `0d96194cf5e84cb18b2a3d82804db987f11969fb6c67715c587e084a7bd70695` |
