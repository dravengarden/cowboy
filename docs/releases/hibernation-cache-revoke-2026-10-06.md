# Hibernation cache revocation and incarnation floor — Hawk, October 6

Resident-only Machine release, active source `331ed2d1ae73b80a752578c405d82e76842047d3`,
artifact `/nix/store/4b45aznkhzy6y44by9kxq0wrc0396d5i-cowboy-machine-writer-host-release`,
root transaction `1791253112…` (receipt `331ed2d1ae73`, started
`2026-10-06T02:18:43.559Z`, committed `02:18:52.808Z`, succeeded, maintenance, no
recovery). The retained worker pin and generation `worker-135348a7…` are unchanged,
so no worker was drained. The preceding artifact was `nmr8a6kr…` (`48d3054d`).

## Change

`hibernate_session` now revokes the provider gateway's cache-protection snapshot,
as delete, reset, provider roll and configuration change already did. The gateway
replays an unrevoked snapshot with real `cache_keepalive` model requests, so a
hibernated DeepSeek session could have kept spending tokens. See the
[token audit](../hibernation-token-audit-2026-10-06.md) for what was and was not
observed: the gateway lives in the signed Plugin runtime outside this repository,
so this is a conservative fix from the call graph, not a measured saving. A test
fails without the call and shows non-gateway providers are not affected.

Side effect, intended: because the previous active release (`48d3054d`) declares
the incarnation reader, the installed owner anchored the **incarnation reader
floor** at this activation (`session-incarnation-reader-floor.json` and anchor
`nmr8a6kr…`, revision `48d3054db95c…`). A writer declaration is now admissible by
the owner, but none exists.

## Validation

Rustfmt, both Clippy configurations, 596 standalone and 1934 all-features tests.
Native production conformance on exact artifacts of one worker generation: the
active `48d3054d` writer against the `331ed2d1` writer, with reader-only releases
of both: **37 groups accepted**
([receipt](../experiments/hibernation-cache-revoke-native-conformance-2026-10-06.json)),
including the deletion journal after the `namespace.rs` refactor and the five
incarnation vectors. These are finite journal/IPC/admission checks in private
namespaces, not production hibernation of a real DeepSeek session.

## Production observation

Samples at `02:18:39Z` and `02:19:39Z`: all 28 worker/keeper units kept their IDs,
PIDs and states; Machine PID `2557144` → `3542735`; Controller PID `204822`,
reader floor for deletions, installed owner, sudoers digest, host source and the
Controller/Web receipts unchanged; health, version, SPA, service worker and Machine
deployment-health returned 200. Startup logged the deletion journal reader with
`deleted_sessions=5 writer_enabled=true`, `Session incarnation reader ready
incarnations=0` and `cleanup continuations ready pending=0`. The receipt's
`published=false` reflects that the commit was pushed afterwards; main contains it.

## Limits

No real DeepSeek session was hibernated, so the revocation reaching a live gateway
is untested in production. The activation was independent of the worker-pool
maintenance performed earlier by another task. No incarnation writer, state lease
or Controller carriage exists; the floor only reserves the right to admit one.
