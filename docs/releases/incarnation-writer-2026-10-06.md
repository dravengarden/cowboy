# Session incarnation writer — Hawk, October 6

Resident-only Machine release, active source `9a2d59138e8ba677e92316a3fb17dbdda4ae6ed7`,
artifact `/nix/store/dcz3jlrb0lwsagcqng8p7ajrfnxdryc7-cowboy-machine-writer-host-release`.
Root transaction receipt `9a2d59138e8b`: started `2026-10-06T02:55:32.402Z`,
committed `02:55:41.491Z`, succeeded, maintenance, no recovery. The preceding
artifact was `4b45aznk…` (`331ed2d1`). The retained worker pin and generation
`worker-135348a7…` are unchanged, so no worker was drained. The
[writer contract](../plugin-session-incarnation-writer.md) defines the behaviour:
mint before a declaration is registered or adopted, rotate before a reset's first
effect, end after the terminal deletion commit, keep the lineage on replay,
adoption, reconnect and wake.

## Validation

Rustfmt, both Clippy configurations, 608 standalone and 1946 all-features tests.
Store and broker tests plus a SIGKILL matrix (mint, rotate, end at all four commit
stages, real child processes); changing the commit stage order or removing any of
the three broker wiring points fails them. Native production conformance on exact
artifacts of one worker generation — the active `331ed2d1` writer against the new
writer, with reader-only releases — **45 groups accepted**
([receipt](../experiments/incarnation-writer-activation-conformance-2026-10-06.json)):
the previous 37, plus mint on a real declaration with replay and reopen by the new
writer, the reader-only release and the previous writer, deletion ending the
lineage, storage-failure refusal of a launch, and six incarnation-floor refusal
vectors (absent, corrupt, foreign dataset, foreign Machine, mutable permissions,
wrong owner) that leave no namespace behind. An earlier run of the same matrix
passed against the preceding artifact on a prior commit
([receipt](../experiments/incarnation-writer-native-conformance-2026-10-06.json)).
These are finite journal/IPC/admission checks in private namespaces.

## Production observation

Startup at `02:55:32Z` logged the deletion journal with `deleted_sessions=5
writer_enabled=true`, `Session incarnation namespace ready incarnations=0
writer_enabled=true` and `cleanup continuations ready pending=0`. By `02:56:33Z`
the Controller's adopt-only reconnect declarations had created exactly 8 lineages
for the 8 live workers: all `adopted`, epoch 1, 8 distinct values, no worker
without one and none for a non-live Session. No launch was refused. Samples at
`02:55:29Z` and `02:56:33Z` bracket the activation: all 28 worker/keeper units kept
their IDs, PIDs and states; Machine PID `3542735` → `36709`; Controller PID
`204822`, the deletion floor, installed owner, sudoers digest and host source
unchanged; health, version, SPA, service worker and Machine deployment-health
returned 200. The receipt's `published=false` reflects that the commit was pushed
afterwards; main contains it.

## Limits

Nothing reads the lineage: no Controller carriage, stale-observation refusal or
diagnostics exists, so this fences nothing yet. No reset, deletion or Session
revival under the writer has been observed in production, and nothing yet shows a
lineage persisting across a later Machine activation; that observation is still
open. Existing Sessions' lineages are `adopted`, not proof of continuity before
this activation. An unconfirmed write now refuses the affected launch or reset
(intended for a safety dataset, and an availability cost). Falcon's owner has not
learned the dataset. Not a power-loss proof.
