# Retained Cargo descendant handles — Hawk, October 5

Linux deleted-session cleanup now recursively opens directories with restricted
`openat2`, checks their original target-relative identity around entry operations,
and unlinks nondirectory entries through held parent handles. It retains the
entire directory structure instead of recursively removing directory names.
A child rename after a file check cannot redirect that unlink into replacement
contents. Observed descendant replacement or mount crossing retires cleanup.
The [contract](../plugin-session-cleanup-targets.md) records the precise boundary.

Implementation `aa02a0f1d88600fe05653bc3f552ebafa750bb2e` is published and active
through integrated source `d3f78f690e0f17f47cf117451514b22a3afbaa13`. The immutable
artifact is
`/nix/store/3alpswdhkz3lpmp0lgh32qiw2vbjrcs2-cowboy-machine-writer-host-release`.
Root transaction `1791168118749388655-d3f78f690e0f` started at
`2026-10-05T02:41:58.749388655Z` and committed at
`2026-10-05T02:42:07.790786031Z`; it reports succeeded, committed, published,
maintenance and no recovery. Startup at `02:41:58.828973Z` confirms the writer
remains enabled with zero recorded deletions.

Runtime Rust passed 1874 all-features library tests and 548 standalone Machine
tests; 57 and 17 environment-dependent tests were ignored. Both Clippy gates and
Rustfmt passed. Three added ordinary filesystem fixtures cover replacement after
the leaf check, retained descendant inodes and file-link referent preservation,
and the 64/65-level recursion boundary. Linux admits at most 64 descendant levels
and one million content entries across all candidate targets per pass. The
one-million limit is implemented; no million-entry boundary fixture is claimed.
Native read-only review found no actionable regressions.

Both ignored cleanup mount fixtures were separately executed successfully in a
private user/mount namespace. They cover same-device ancestor/target scan mounts
and descendant bind mounts introduced before and after the original child handle
opens. The descendant cases preserve both foreign artifacts and the original
underlying artifact accessed through its held handle. The process-exit broker
fixture confirms cleanup bookkeeping while directories remain.

The exact final writer, previous active writer, accepted reader-only fallback
and final default reader passed all 32 production conformance groups in private
root mount/PID/network namespaces. An earlier run exposed a conformance-tool
cold-start race: the old socket pathname existed before the new broker accepted
connections. Commit `78e64614` waits up to five seconds for connection
establishment, retrying only missing/refused sockets before sending any frame.
It never retries a sent request. Candidate and final native pairs passed with
this correction. Later integration changed only that Python tool and another
task's Web toolbar; runtime Rust stayed unchanged. No wire, SDK, dependency or
worker pin changes were introduced. No synthetic live journal records were used.

During `02:41:45.359Z`–`02:42:54.716Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`2837499` to `3232214`; Controller PID `2648009` remained unchanged. Accepted
generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, installed component owner
and sudoers digest remained unchanged. `sudo -n true` succeeded. Health, version,
SPA, service worker and Machine deployment-health endpoints returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-descendants-2026-10-05.json)
contains exact artifacts, native hashes, conformance observations, mount-fixture
command and before/after receipts. Nondirectory name observation/unlink remains
non-atomic: a replacement nondirectory in the held original parent can be
unlinked. A rename after verification can permit effects on original contents
before refusal. There is no atomic snapshot or mount/rename freeze; partial
effects are not rolled back. General I/O deadlines, continuous ownership,
durable Session incarnation and portable writer admission remain open.
Non-Linux Unix retains its previous recursive pathname fallback.
