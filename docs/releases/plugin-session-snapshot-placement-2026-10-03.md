# Session worker snapshot placement fence — 2026-10-03

The [finite contract](../plugin-session-snapshot-placement.md) prevents a
launch-bearing worker snapshot from changing the declared session ID,
Provider ID, runtime cwd, system flag or execution binding. Validation and
registry writeback share one critical section. Old snapshots cannot undo a
staged workspace reset. Rejected snapshots stop before Controller projection,
rollout rehabilitation or cutover; retired peers cannot reject a current peer.

## Verification

The pinned shell passes all **440 standalone Machine tests** (four ignored),
all **1,742 main Rust tests** (41 ignored), actual integration binaries,
all-target/all-feature clippy with warnings denied, and Rust formatting.
The six added regressions cover altered placement fields, stale connection and
epoch, first-observation session-ID mismatch, staged reset, compatible release
and native-thread updates, and actual Unix IPC rejection without Controller or
rollout effects. The IPC test uses a subsequent frame as its progress barrier.

One complete test attempt overlaps release compilation and times out in the
telemetry fixture waiting for its first HTTP request. Its isolated retry and
complete post-build serial suite both pass without changing test deadlines.
The cause is consistent with scheduling pressure; this is not a proven
telemetry product failure or a broker-test failure.

## Published Machine activation

- Published clean source: `8fdce234f809b230e4c3dd48368bedd1f1342aa0`.
- Immutable release: `/nix/store/1zs40gl2xhizn5zsh3705sabpvz52nkp-cowboy-machine-release`.
- [Receipt](../experiments/plugin-session-snapshot-activation-2026-10-03.json):
  `1791006019638626649-8fdce234f809`, succeeded/committed/published,
  maintenance true, recorded `2026-10-03T05:40:28.190732309Z`.
- Worker generation remains `worker-eed1d8105af00846771d`.

The [bounded activation observation](../experiments/plugin-session-snapshot-continuity-2026-10-03.json)
retains all **13 ACP worker and two execution keeper** PIDs and running states
across this activation. Machine PID changes from 2377011 to 2575457;
Controller PID 1998362 is unchanged. The new Machine authenticates as Hawk
with protocol 25 at `2026-10-03T05:40:27.783797Z`. Health/version return 200;
index and service worker return 200 with no-store. SPA version stays
`f6d1eafd740201ea760bb43fac3ed40d`; no Web or Controller activation is performed.

These are source-linked IPC regression tests and a bounded production release
observation, not an independently supplied immutable negative/positive matrix.
The earlier Session-root connected gate is not rerun or attributed to this
change. Launch-less legacy compatibility, durable Session incarnation,
continuous worktree ownership and general state leases remain outside this
finite fence.
