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

Cleanup clears eligible contents and retains the target directory itself. Linux
does not perform a final unlink through the mutable target pathname. The returned
paths identify cleared targets, not removed directories. Both Cargo markers are
cleared with the other contents, so a second pass skips the empty directory; a
later Cargo build can recreate markers and artifacts in the same directory.
Real filesystem tests retain an open handle and check its device/inode against
the empty directory after cleanup, repeat cleanup without markers, then recreate
ordinary Cargo contents and clean again.

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
an atomic tree snapshot. Retaining the empty target avoids the former final
empty-directory name-unlink race. Independent replacement of descendants during recursive child removal,
filesystem/mount boundaries and general I/O deadlines remain separate gaps.
Non-Linux Unix targets retain pathname content access with identity checks and
do not claim Linux descriptor anchoring. A refusal may follow removal of some
original contents; prior effects are not rolled back.

Hawk activation, native conformance and process-preservation receipts are in the
[October 5 release](releases/plugin-session-cleanup-targets-2026-10-05.md).
The subsequent [retained-directory release](releases/plugin-session-cleanup-retained-2026-10-05.md)
records adoption of content-only cleanup and removal of the final name unlink.
