# Observed Cargo targets during Session cleanup

Cleanup retains the directory opened during each successful bounded Cargo marker
probe, rather than retaining only its pathname. Collection admits at most 128
marked targets and finishes before any removal. Exceeding that limit preserves
all candidates and releases their handles; the existing directory traversal
limit remains separate.

Before removing contents, cleanup checks the candidate's pathname against its
retained directory handle and revalidates its markers through that handle. A
missing, symbolic or replacement directory, or withdrawn marker, produces a
distinct target-change refusal. The broker retires this cleanup instead of
retrying against the replacement. The original Session-root observation and
worker process-exit proof remain required.

Linux content access uses the candidate's `/proc/self/fd` directory, not its
mutable pathname. Identity checks surround child removals. A target rename after
a child check cannot redirect that child operation into a replacement target.
Previously the scan remembered only eligible pathnames, so a later replacement
could receive the recursive removal even when it carried no Cargo markers.
Returned cleanup paths still use the logical worktree location.

Real-filesystem fixtures cover same-path marked and unmarked replacement,
ancestor replacement, disappearance, a symlink to the original object and marker
withdrawal. A deterministic unit seam replaces the target after the child check
and verifies that Linux leaves replacement contents intact. It is not exposed
through any production CLI, environment variable or IPC frame. A 129-target
fixture preserves all candidates; the exact 128-target limit succeeds. Ordinary
cleanup, root replacement, FIFO markers and reset/process-exit fixtures remain
part of the Machine gate.

This is a scan-time target observation, not continuous Session/worktree ownership
from launch or terminal deletion. It does not establish a reader/writer lease or
an atomic tree snapshot. The final empty-directory name check and unlink are not
atomic: an independently replaced empty directory can still be unlinked in that
window. Independent replacement of descendants during recursive child removal,
filesystem/mount boundaries and general I/O deadlines remain separate gaps.
Non-Linux Unix targets retain pathname content access with identity checks and
do not claim Linux descriptor anchoring. A refusal may follow removal of some
original contents; prior effects are not rolled back.
