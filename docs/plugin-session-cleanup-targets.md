# Observed Cargo targets during Session cleanup

Cleanup retains the directory opened during each successful bounded Cargo marker
probe, rather than retaining only its pathname. Collection admits at most 128
marked targets and finishes before any removal. Exceeding that limit preserves
all candidates and releases their handles; the existing directory traversal
limit remains separate.

Linux collection opens each pending directory and candidate relative to the held
Session-root descriptor using `openat2` with `BENEATH`, `NO_SYMLINKS` and
`NO_XDEV`. It enumerates each opened directory through its own descriptor and
retains only root-relative pending names. Linked ancestors, mount points
(including same-device bind mounts), missing paths and nondirectories are skipped
before marker reads. Other errors, including unsupported kernels, fail collection
before removal without a pathname fallback. These flags follow the
[Linux interface contract](https://man7.org/linux/man-pages/man2/openat2.2.html).
Non-Linux Unix keeps its explicitly weaker pathname fallback.

A filesystem regression replaces a pending ancestor with a link and refuses
linked and escaping paths. An ignored fixture runs explicitly in a private mount
namespace, binds same-device directories over both an ancestor and a direct
target, checks refusal, preserves their original Cargo artifacts and clears a
separate ordinary target. This is scan-time mount admission; the additional
removal-time rules below apply to candidate descendants.

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

Linux recursively opens descendant directories with the same restricted
`openat2` flags. It compares each opened child with the observed device/inode,
then rechecks its original target-relative directory identity around entry
operations. Enumeration uses held directory handles; nondirectory unlink uses
`unlinkat` in the held parent, without following links or requesting directory
removal. A directory rename after a file check cannot redirect that unlink into
the replacement tree. Descendant replacement, missing directories and mount
crossings produce the target-change refusal and retire cleanup. A private mount
fixture covers a same-device descendant bind both before and after opening its
original handle, preserving foreign and original underlying artifacts.

Removal admits at most 64 descendant levels and one million content entries
across all candidate targets in one pass. It retains the target and descendant
directory structure: no final directory-name unlink follows an identity check.
Actual fixtures verify retained descendant inodes, removal of file links without
touching their referents, replacement after the leaf check, and the 64/65-level
boundary. Bounds can refuse after partial effects and do not impose I/O deadlines.

Cleanup clears eligible files and retains directory structure on Linux. Non-Linux
Unix retains only the target itself and its previous recursive pathname fallback.
The returned paths identify cleared targets, not removed directories. Both Cargo
markers are cleared with the other contents, so a second pass skips unmarked
directory structure; a
later Cargo build can recreate markers and artifacts in the same directory.
Real filesystem tests retain an open handle and check its device/inode against
retained directory after cleanup, repeat cleanup without markers, then recreate
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
an atomic tree snapshot. Retaining directory structure avoids directory-name
unlink races. Nondirectory name observation and unlink are still not atomic: a
replacement nondirectory in the original held parent can be unlinked. A rename
after verification may permit effects on the held original object before refusal;
there is no claim of an atomic tree snapshot or a freeze on mounts/renames.
General I/O deadlines and continuous launch-time ownership remain separate gaps.
Non-Linux Unix targets retain pathname content access with identity checks and
do not claim Linux descriptor anchoring. A refusal may follow removal of some
original contents; prior effects are not rolled back.

Hawk activation, native conformance and process-preservation receipts are in the
[October 5 release](releases/plugin-session-cleanup-targets-2026-10-05.md).
The subsequent [retained-directory release](releases/plugin-session-cleanup-retained-2026-10-05.md)
records adoption of content-only cleanup and removal of the final name unlink.
The [bounded-scan release](releases/plugin-session-cleanup-scan-2026-10-05.md)
records root-relative Linux scan admission and actual bind-mount acceptance.
