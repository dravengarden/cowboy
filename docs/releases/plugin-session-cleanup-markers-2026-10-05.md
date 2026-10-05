# Bounded cleanup markers — Hawk, October 5

Deleted-session cleanup now probes both Cargo marker files relative to the same
opened target directory, rejects symbolic links and nonregular files, and bounds
UTF-8 cache tags to 8192 bytes. A FIFO without a writer no longer waits while
cleanup retains the lifecycle gate. Invalid markers preserve the target. The
[contract](../plugin-session-cleanup-markers.md) records the finite marker scope.

Implementation `2f54346e` is included in the published and activated source
`ab3f06f3c9a74dc83d0ea170f890d8e4c9782bca`. The exact active artifact is
`/nix/store/14d2dvz1jxf30g5j5r7440chyxldidas-cowboy-machine-writer-host-release`.
Root transaction `1791161591985649733-ab3f06f3c9a7` started at
`2026-10-05T00:53:11.985649733Z` and committed at
`2026-10-05T00:53:18.930681097Z`; the receipt reports succeeded, published,
maintenance and no recovery. Startup at `00:53:12.066541Z` confirms the writer
remains enabled with zero recorded deletions. No synthetic live records were
introduced.

The runtime Rust passed 1865 all-features library tests and 540 standalone
Machine tests; 54 and 15 environment-dependent tests remain ignored. Both Clippy
gates and Rustfmt passed. Regression evidence includes complete cleanup with
FIFO markers and no writer, artifact preservation for symbolic links, oversized
tags containing a valid signature, invalid UTF-8 and a directory marker, plus
acceptance at the exact byte limit and ordinary cleanup. Native read-only review
found no actionable regressions. Later integration changed only Web/app-shell and
documentation. No native wire, SDK or dependency changes were made.

The exact final writer and previous active writer, accepted reader fallback and
final default reader passed 32 production conformance groups in private root
mount/PID/network namespaces. Marker-specific evidence comes from the real
filesystem library fixtures; these native groups verify journal/startup/IPC
compatibility. A prior candidate from `d38c567d` was refused before dispatch after
remote main advanced. The accepted artifact was rebuilt and independently
reaccepted; no ancestry check was bypassed.

The activation window `00:52:58.862Z`–`00:53:56.088Z` retained all 12 workers and
six execution keepers with identical IDs, PIDs and active states. Machine PID
changed from `2069302` to `2296482`; Controller PID `803041` stayed unchanged.
Generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, component owner and sudoers
digest remained unchanged. `sudo -n true` succeeded. Health, version, SPA, service
worker and Machine deployment-health endpoints all returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-markers-2026-10-05.json)
contains exact artifacts, native hashes, conformance cases and before/after
receipts. General filesystem deadlines, complete nested-directory removal
fencing, continuous worktree ownership, durable Session incarnation and portable
writer admission remain open.
