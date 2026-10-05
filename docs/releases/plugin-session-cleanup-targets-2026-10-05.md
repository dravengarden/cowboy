# Observed cleanup targets — Hawk, October 5

Deleted-session cleanup retains up to 128 observed Cargo target directory
handles, verifies identity and markers before removal, and anchors Linux content
removal to those handles. Replaced targets or withdrawn markers retire cleanup
without retrying against the replacement. Collecting 129 targets refuses before
any removal. The [contract](../plugin-session-cleanup-targets.md) describes the
finite observation boundary and remaining races.

Implementation `0542f022594922944fa27146cfa44f9e81500a8c` is published in remote
main and activated through integrated source
`2766b0bf1fa4c2cf2c5acb3e8a632306d7cdcbeb`. The immutable artifact is
`/nix/store/rkdp7ggg7q2vad6s4lqik7d6nk8v3sm2-cowboy-machine-writer-host-release`.
Root transaction `1791164844084773496-2766b0bf1fa4` started at
`2026-10-05T01:47:24.084773496Z` and committed at
`2026-10-05T01:47:35.757789565Z`. The receipt reports succeeded, committed,
published, maintenance and no recovery. Startup at `01:47:24.171487Z` confirms
the writer remains enabled with zero recorded deletions.

The runtime passed 1868 all-features library tests and 543 standalone Machine
tests; 54 and 15 environment-dependent tests remain ignored. Both Clippy gates
and Rustfmt passed. Real filesystem regressions cover marked and unmarked
replacement targets, replaced parents, missing paths, links, withdrawn markers,
the 128/129 target budget and a target rename after the child identity check.
Native read-only review found no actionable regressions. Integration after
testing changed only Web files; no Rust, wire, SDK, dependency or worker pin
changes were introduced.

The exact final writer, previous active writer, accepted reader-only fallback
and final default reader passed all 32 native production conformance groups in
private root mount/PID/network namespaces. These verify journal/startup/IPC
compatibility; cleanup-specific evidence comes from the filesystem tests. No
synthetic records were written to the live journal and no native production
checkpoint hooks were added.

During the observation window `01:47:12.461Z`–`01:47:46.261Z`, all 13 workers and
six execution keepers retained their IDs, PIDs and active states. Resident
Machine PID changed from `2296482` to `2564297`; Controller PID `803041` remained
unchanged. Accepted generation `worker-748825b42b4302fe26ca`, reader floor,
journal entries, Controller/Web receipts, resolved SPA, host source, installed
component owner and sudoers digest remained unchanged. `sudo -n true` passed.
Health, version, SPA, service worker and Machine deployment-health endpoints
returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-targets-2026-10-05.json)
retains exact artifact/native hashes, conformance observations and before/after
receipts. Final empty-directory pathname unlink is still non-atomic; independent
descendant mutations, mount boundaries, I/O deadlines, continuous ownership,
durable Session incarnation and portable writer admission remain open. Earlier
effects on original contents are not rolled back after refusal.
