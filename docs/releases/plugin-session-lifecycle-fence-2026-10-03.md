# Machine Session declaration/deletion fence — 2026-10-03

The [finite lifecycle contract](../plugin-session-lifecycle-fence.md) prevents
late `EnsureSession` from restoring a deleted Session's launch registry.
Both adopt-only and ordinary declarations check the deletion marker while
holding the same per-session gate as deletion, reset and artifact cleanup.
Reset uses its already-held guard, preserving existing reset behavior without
recursive locking. Different session keys have independent gates; the core
command queue retains its existing serialization and separate heartbeat reader.

## Verification

The pinned shell passes all **444 standalone Machine tests** (four ignored),
all **1,746 main Rust tests** (41 ignored), actual integration binaries,
all-target/all-feature clippy with warnings denied, Rust formatting and the
immutable Machine release build. No failed gate or timeout adjustment occurs.

Four regressions cover late adoption and launch after deletion, a held cleanup
fence with another session proceeding, FIFO deletion before a queued declaration,
and actual Unix core IPC refusal after an acknowledged delete. FIFO admission
is established by polling the real futures, without a scheduling delay.
Existing reset, replay, heartbeat, worker-exit and artifact-cleanup tests pass.
These are source-linked regression tests, not an independently supplied
immutable old/new artifact matrix.

## Published Machine activation

- Published clean source: `c53343aa188da31d8ecddd23ebcf481778849999`.
- Immutable release: `/nix/store/zq5akjdrhh94z69n7zwlkab93sfdavb4-cowboy-machine-release`.
- [Receipt](../experiments/plugin-session-lifecycle-activation-2026-10-03.json):
  `1791008047776620083-c53343aa188d`, succeeded/committed/published,
  maintenance true, recorded `2026-10-03T06:14:19.516383517Z`.
- Worker generation remains `worker-eed1d8105af00846771d`.

The [bounded activation observation](../experiments/plugin-session-lifecycle-continuity-2026-10-03.json)
retains all **14 ACP worker and two execution keeper** PIDs and running states
across this activation. Machine PID changes from 2575457 to 2827260;
Controller PID 1998362 is unchanged. The new Machine authenticates as Hawk
with protocol 25 at `2026-10-03T06:14:19.188656Z`. Health/version return 200;
index and service worker return 200 with no-store. SPA version stays
`f6d1eafd740201ea760bb43fac3ed40d`; this task activates no Controller or Web release.

This does not establish durable deletion after Machine restart, same-ID
incarnation, physical worktree ownership, native resume or general state leases.
The marker and lifecycle gates are process-local; restart reconciliation remains
open. The earlier Session filesystem-read connected matrix is not rerun or
attributed to this change.
