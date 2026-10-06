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

## Later observation: persistence across another activation

Another task activated Machine `551474a3` (worker pin `5b3547a6`, generation
`worker-d62183a8…`) at `2026-10-06T11:12:07Z`. Its startup read the namespace back
as `Session incarnation namespace ready incarnations=6 writer_enabled=true`, with
the deletion journal at `deleted_sessions=7` (it was 5 at this release). Afterwards
the record holds 6 lineages, all `adopted`, epoch 1, 6 distinct values, one for each
of the 6 live workers and none for anything else, and no launch was refused for an
unconfirmed lineage. So the lineages created here survived a Machine restart and a
worker-generation change, and the drop from 8 to 6 is consistent with the two
Sessions deleted since (records ended after the journal commit). The earlier
values were not saved, so equality of each value across the restart is inferred from
the unchanged `adopted`/epoch-1 origin of every survivor, not compared directly. No
reset has been observed in production yet. That activation was another task's, not
this release's.

## Later observation: a minted lineage and four restarts

At the end of 2026-10-06, after three further resident activations by this and other
tasks (`551474a3`, `933acdd1`, `5d4d0c88`), `cowboy operator durable-state` and the
files agree: 7 lineages for 7 live workers, all distinct and epoch 1; 6 are `adopted`, the
set created at this release, so they have outlasted four Machine restarts, and 1 is
`minted`, the first observed in production, for a Session first declared after this
release. No lineage exists for any of the 7 durably deleted IDs, none is orphaned, and no
launch was ever refused for an unconfirmed lineage (zero such journal lines). Still not
observed in production: a reset rotating a lineage, and a deletion ending one on a live
Session. Equality of the six original values across the restarts remains inferred, not
compared, because the first values were not saved.

## Limits

Nothing reads the lineage: no Controller carriage, stale-observation refusal or
diagnostics exists, so this fences nothing yet. No reset, deletion or Session
revival under the writer has been observed in production, and nothing yet shows a
lineage persisting across a later Machine activation; that observation is still
open. Existing Sessions' lineages are `adopted`, not proof of continuity before
this activation. An unconfirmed write now refuses the affected launch or reset
(intended for a safety dataset, and an availability cost). Falcon's owner has not
learned the dataset. Not a power-loss proof.
