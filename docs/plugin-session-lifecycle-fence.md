# Machine Session declaration/deletion fence

A deleted Session cannot be redeclared by a delayed Controller `EnsureSession`.
Both adopt-only and ordinary launch declarations return the existing negative
CommandAck and leave the launch registry, session state, reconnect admission
and launching set untouched. The worker-launch cancellation check alone was
insufficient: it prevented a process start but still allowed registry resurrection.

Machine-local launch declarations now acquire the same per-session lifecycle
gate as permanent deletion, explicit reset and asynchronous artifact cleanup.
The deletion marker is checked inside this gate before any declaration writes.
FIFO gate admission prevents a later declaration from overtaking deletion;
cleanup cannot concurrently remove generated artifacts beneath a newly admitted
replacement. Different session IDs use independent gates. The existing core
command queue still serializes lifecycle commands; its separate frame reader
continues handling heartbeats while a command waits.

Reset already owns this gate and calls the private declaration path with its
held guard rather than acquiring it recursively. Its existing old-worker exit,
launch-settlement and deliberate cancellation-marker clearing remain intact.
Ordinary adoption never clears a deletion marker. Source worktrees and branches
remain retained; this change adds no new filesystem deletion.

Four regressions cover both late declaration modes, gate blocking with another
session proceeding, FIFO deletion before declaration, and real Unix core IPC
refusal after an acknowledged delete. Existing reset, worker replay, process
exit and artifact-cleanup tests retain their original contracts.

This fence is process-local and adds no protocol field or durable record.
A Machine restart still loses the deletion markers and reconstructs declarations
from Controller and worker observations. Durable Session incarnation, physical
worktree ownership, restart reconciliation and general state leases remain open.
The [worker snapshot fence](plugin-session-snapshot-placement.md) independently
protects declarations against worker observations, and the
[Session filesystem observation](plugin-session-root-observations.md) supplies
physical identities for buffered reads, not general worktree authority.

The [published Machine release](releases/plugin-session-lifecycle-fence-2026-10-03.md)
records source validation and bounded production continuity separately.
