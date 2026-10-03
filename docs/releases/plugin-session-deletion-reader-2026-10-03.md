# Session terminal-deletion journal reader — 2026-10-03

The [reader-first contract](../plugin-session-deletion-journal.md) binds a
bounded terminal-ID dataset to configured Machine/Service identity. Loaded IDs
fence worker Welcome, delayed launch declarations and reset. The private source
writer candidate flushes an atomic snapshot before Stop or positive ACK and
refuses unconfirmed storage outcomes without worker/registry deletion effects.

**Production is reader-only. Newly deleted production Sessions still use the
existing process-local marker; this release does not make their deletion durable.**
The writer has no request, CLI or environment admission. Compatible component
activation, fallback and recovery readers must be owned and verified before it
can be enabled. Older Machine artifacts ignore the namespace.

## Verification and packaging

The pinned shell passes **455 standalone Machine tests** (four ignored),
**1,757 main Rust tests** (41 ignored), integration binaries, all-target/all-feature
clippy with warnings denied, Rust formatting, the immutable Machine build and
`checks.x86_64-linux.cowboy-source-boundary`.

Eleven added regressions cover close/read-only reopen, configured owner mismatch,
corrupt/unknown/oversized/duplicate records, one namespace owner, root replacement,
storage poisoning, special files/unknown fields, staging without replay, record
budget without eviction, Stop/registry absence after storage failure, volatile
read-only compatibility, broker restart over actual Unix IPC and permanent-ID
reset refusal. The restart fixture releases the old broker owner and monitor;
it is in-process broker restart, not an OS-process crash or power-loss test.
These source fixtures are not a supplied immutable old/new executable matrix.

The initial full-feature build identifies an obsolete production entry that now
serves only legacy test fixtures; it is made test-only. The initial immutable
Machine build then refuses the newly added module because the narrow source
fileset omits `src/machine_broker/`. The source directory and its boundary check
are added, and the exact final build passes. No failed build is activated.

Source implementation is `3751ff08`; packaging fix is `4223fc92`.
Final source `dd79237c` integrates the separately published native Remote CLI
change and Mac release documentation. Integrated Rust gates pass; this task
claims neither that other task's native acceptance nor a Mac deployment.

## Published Hawk reader activation

- Published clean source: `dd79237c28b1a4873b8d9b4b3271d4976938aaea`.
- Immutable release: `/nix/store/f24jdxk9c6lcr15b40ws9iixhbflpmvf-cowboy-machine-release`.
- [Receipt](../experiments/plugin-session-deletion-reader-activation-2026-10-03.json):
  `1791012750399875325-dd79237c28b1`, succeeded/committed/published,
  maintenance true, recorded `2026-10-03T07:32:39.528214267Z`.
- Worker generation remains `worker-eed1d8105af00846771d`.

The [bounded observation](../experiments/plugin-session-deletion-reader-continuity-2026-10-03.json)
retains all **14 ACP worker and two execution keeper** PIDs and running states.
Machine PID changes from 2827260 to 3409731; Controller PID 1998362 is unchanged.
At `2026-10-03T07:32:30.497141Z`, the new Machine reports the reader ready with
zero records and writer disabled. Its namespace contains only `.lock`;
`deletions.json` is absent. It authenticates as Hawk with protocol 25 at
`2026-10-03T07:32:39.331121Z`. Health/version return 200; index/SW return 200
with no-store; SPA version remains `f6d1eafd740201ea760bb43fac3ed40d`.
This task activates no Controller or Web release.

Writer admission, independently supplied old/new and process-crash acceptance,
durable same-ID incarnation, worktree ownership and general state leases remain
open. The earlier Session filesystem-read matrix is not rerun or attributed to
this reader-only deployment.
