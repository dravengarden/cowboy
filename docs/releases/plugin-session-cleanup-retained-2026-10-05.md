# Retained empty Cargo targets — Hawk, October 5

Deleted-session cleanup now clears eligible Cargo target contents while retaining
the target directory itself. Removing an empty target by pathname after checking
its identity could unlink an independently substituted empty directory. Omitting
that final unlink removes this particular race. Returned cleanup paths identify
cleared targets. The empty directory has no Cargo markers and is skipped by a
later pass; a later build can recreate its markers and artifacts. The
[contract](../plugin-session-cleanup-targets.md) records the remaining boundaries.

Implementation `2e7d01860ee2c3d946d8559c28954aa6c817e217` is published in remote
main and active through source `77cde0b52e7f19351e06705a454559525b2ed4cb`.
The immutable artifact is
`/nix/store/gal22mqascj2230mjy3yxj83nhx912h9-cowboy-machine-writer-host-release`.
Root transaction `1791165660843507685-77cde0b52e7f` started at
`2026-10-05T02:01:00.843507685Z` and committed at
`2026-10-05T02:01:08.835617609Z`; its receipt reports succeeded, committed,
published, maintenance and no recovery. Startup at `02:01:00.921759Z` confirms
the writer remains enabled with zero recorded deletions.

Integrated Rust source `f3ad2ed45db277977a7f3ad5a6f4949eb3a3ff58` passed 1870
all-features library tests and 544 standalone Machine tests; 55 and 15 tests
requiring separate environments remain ignored. Both Clippy gates and Rustfmt
passed. The new real-filesystem fixture holds the original target handle and
verifies that cleanup retains its device/inode, leaves no contents, skips the
next unmarked pass and cleans recreated Cargo contents. The broker process-exit
fixture verifies cleanup bookkeeping completion with the retained empty target.
Native read-only review found no actionable regressions.

An initial preintegration test binary encountered newly merged migration files
without its compiled checksum registration and failed two migration tests. The
integrated rebuild passed; no applied migration bytes or stored checksums were
altered. An overbroad standalone `--bins` lint selected the full-only ACP worker;
the corrected gate explicitly selected the library and three resident Machine
executables and passed. The integrated source includes another task's accepted
Controller/Web workspace-document changes; this task activated only the Machine
component. The final merge added only acceptance-result documentation. No worker
pin, native wire, SDK or dependency changes were introduced.

The exact final writer, previous active writer, accepted reader-only fallback
and final default reader passed 32 native production conformance groups in
private root mount/PID/network namespaces. The final writer was rebuilt and
reaccepted after the documentation merge because its exact source revision is
part of writer admission. No synthetic records entered the live journal.

During `02:00:52.600Z`–`02:01:37.291Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`2564297` to `2692488`; Controller PID `2648009` remained unchanged. Accepted
generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, installed component owner
and sudoers digest remained unchanged. `sudo -n true` succeeded. Health, version,
SPA, service worker and Machine deployment-health endpoints returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-retained-2026-10-05.json)
contains exact artifacts, native hashes, conformance observations and before/after
receipts. Independent descendant mutation, mount boundaries, I/O deadlines,
continuous ownership, durable Session incarnation and portable writer admission
remain open. Non-Linux content access retains the documented pathname fallback.
