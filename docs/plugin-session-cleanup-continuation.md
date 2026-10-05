# Durable Session cleanup continuation

A resident Machine that accepts a terminal Session deletion now leaves a small
advisory record so a later resident can finish generated-artifact cleanup. Before
this record, the retained target plan and the pending cleanup lived only in the
resident process: every Machine maintenance restart silently abandoned the
cleanup of Sessions deleted shortly before it, and their Cargo targets were
never reclaimed.

## What a record is

`state_dir/session-cleanups/cleanups.json` is a closed schema-1 document: the
configured Machine/Service owner and a list of `{session_id, root}` entries.
`root` is the storage identity of the original worktree root object: device,
inode and creation time with nanoseconds. Unknown fields, duplicate IDs, another
owner, a different schema, nonregular files and oversized input (2 MiB, 4,096
entries) refuse and are never rewritten.

A record is **a nomination, not an authorization**. It carries no target list,
no path, no marker observation, no retry cursor and no command identity. The
process-local retry plan and marker progress described in
[Observed Cargo targets](plugin-session-cleanup-targets.md) are deliberately
not serialized: restoring them would turn stale observations into execution
authority. After a restart the Cargo targets are always found again by the same
bounded, handle-anchored scan as for a first invocation.

## Ordering

1. Terminal deletion is committed in the existing reader-first
   [deletion journal](plugin-session-deletion-journal.md). That commit, with its
   exclusive writer, floor and worker fencing, is the only deletion decision.
2. The Machine then captures the original root handle and, if the root reports a
   creation time, commits the nomination (staged file, file sync, atomic rename,
   directory sync) before starting the in-process cleanup.
3. A write failure is logged and fences further writes in that owner. It never
   fails or delays the deletion acknowledgement: the consequence is exactly the
   previous behaviour (artifacts preserved, no resume after a restart).
4. The nomination is retired after the cleanup completes (including "no marked
   targets") or after the original root is observed gone or replaced.
   Successful cleanup that crashes before retirement is harmless: the next
   resume finds nothing to remove and retires it.

## Resume

On startup, after the journal and namespace are attached and before peers are
accepted, one background task visits the nominations one at a time:

- A nomination whose Session has no committed terminal deletion is left untouched
  and logged. The journal, not this file, is the authority.
- The root path is derived from the configured worktree root and the Session ID,
  never read from the record. A missing root, a symlink, a nondirectory or an
  object whose device/inode/creation time differs from the nomination is a
  `CleanupRootChanged` refusal: no effects, nomination retired. A dev number that
  changed across a reboot therefore refuses (artifacts are preserved) rather than
  matches by accident.
- Any other observation error keeps the nomination and is retried on the next
  restart.
- An admitted root enters the existing cleanup task unchanged: per-Session
  lifecycle gate, worker owner-exit proof through the transient unit, bounded
  start wait, Linux `openat2` scan, retained target handles, descendant handle
  walk, leaf identity and deferred marker finalization, with its own in-process
  retry/backoff. The task is waited for at most two minutes before the next
  Session starts; a persistently failing Session cannot starve the rest.
- Direct (non-systemd) mode has no owner-exit proof and preserves artifacts as
  before, keeping the nomination pending.

## Admission and compatibility

Only a Machine already admitted to write the deletion journal opens the
namespace. Default and reader-only builds never create, read or write it, so a
reader-only fallback is unaffected. The namespace uses its own exclusive lock and
retained directory/lock handles, and any open failure (lock held, corrupt or
foreign record) only disables continuation: it can never keep the resident
Machine from starting or alter the deletion journal.

No reader floor or owner change is needed because the dataset is monotone and
liveness-only. An older resident ignores the directory and behaves as before. A
newer resident that later finds a nomination still requires the committed
permanent deletion, the exact original root object and a fresh scan, so a
rollback/roll-forward sequence cannot revive an effect that the journal and the
filesystem do not both support. Historical deletions made without a nomination
are not reconstructed.

## Limits

- This is cleanup continuation, not continuous Session/worktree ownership or a
  durable Session incarnation. It adds no launch-time, reset or runtime identity.
- Creation time is required. A filesystem that cannot report it yields no
  nomination.
- The nomination is made after the journal commit and the root capture, so a
  crash between them leaves no record.
- Content, markers and names are not frozen; the final comparison/name unlink is
  still non-atomic as documented for cleanup targets. The two marker unlinks are
  not a transaction. Completed paths describe the original scan, not current
  emptiness.
- A resumed Session whose cleanup keeps failing leaves its in-process retry loop
  running (60-second backoff) with its retained root, target and marker handles,
  after resume stops waiting for it. There is no global cap on such loops or
  handles across Sessions; the 4,096-entry nomination bound is the only limit. A
  later change should give up in-process after bounded retries and rely on the
  durable nomination, releasing those handles.
- No general filesystem I/O deadline is added. Resume merely bounds how long one
  Session delays the start of the next.
- A worktree that was deleted and recreated while a resident was down is
  preserved, not adopted. A worktree that is never deleted keeps its artifacts.
- There is no cross-Machine or portable writer admission, power-loss proof or
  supported-device acceptance. Non-Linux Unix keeps its weaker pathname fallback.

## Evidence

Source fixtures cover the real `StopSession` path recording a nomination, a
second resident resuming it, completion being durable across a third resident,
root replacement while down, missing and symlinked roots, an unjournaled
nomination, and a Machine without the namespace. A mutation that disables the
identity comparison fails the replacement fixture. The store tests cover owner
and schema mismatch, unknown/duplicate/oversized input, links and special files,
exclusive ownership, replacement of the directory or lock, storage failure
fencing and staging that is never replayed. The native production conformance
additionally checks that the default reader never creates the namespace and the
admitted writer owns an empty one.
