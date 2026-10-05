# Original Session directory during terminal cleanup

The [October 5 Hawk release](releases/plugin-session-cleanup-root-2026-10-05.md)
records the active artifact and bounded continuity evidence.

Terminal StopSession captures the validated managed worktree directory before
dispatching the worker stop. The asynchronous cleanup retains an open directory
handle while waiting for process-exit proof. A failed capture preserves artifacts
without cancelling terminal deletion or weakening the deletion journal.

Cleanup checks the current session pathname against the original handle's
device/inode before traversal and before target removal. The retained handle
prevents inode reuse. Disappearance, a replacement directory, a non-directory or
a final symbolic link returns a distinct refusal. The broker then retires that
cleanup and releases the handle instead of retrying against the replacement.
Ordinary I/O failures retain the existing retry behavior. Reset and command
replacement continue to use the per-session lifecycle gate.

On Linux, traversal and removal paths remain under `/proc/self/fd` for the
captured directory. A root rename after the last pathname check cannot redirect
that access into a replacement root. Returned cleanup paths still use the
logical worktree pathname. Other Unix targets retain pathname access with the
identity checks; they do not claim Linux's descriptor-anchored access.

Tests use real directory rename/replacement, disappearance and a symlink back to
the original directory. They preserve the original and replacement artifacts,
check that the Linux handle still reads the original object, and verify that a
broker cleanup queued behind its lifecycle gate retires after replacement
without retrying or clearing terminal cancellation. Existing reset and
process-exit fixtures continue to exercise successful cleanup and refusal
without process-exit proof.

This observes the directory present at terminal deletion, not a continuously
owned directory established at Session launch. It does not fence independent
writers replacing nested directories, provide an atomic filesystem snapshot,
roll back prior removal, persist cleanup across restart, or establish durable
Session incarnation. One handle is held per pending cleanup until completion,
retirement, reset/replacement or process exit; general lifecycle resource bounds
remain a separate requirement.
