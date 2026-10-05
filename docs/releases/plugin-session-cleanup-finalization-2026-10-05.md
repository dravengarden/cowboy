# Cargo cleanup marker finalization — Hawk, October 5

Linux deleted-session cleanup now defers both root Cargo eligibility markers
until its content walk succeeds. Previously, early marker deletion followed by
a content I/O error could hide remaining artifacts from retry. Enumeration
streams entries while retaining at most two marker entries; all existing node
identity checks still apply when markers are finally removed. The
[contract](../plugin-session-cleanup-targets.md) records this ordered finalization.

Implementation `6c204d84a53dba2de3e4cb4efe10b26969dcc3e9` is published and active
through integrated source `7c6551ffbb3e595e81042041ea47460616c02679`. The immutable
artifact is
`/nix/store/n4j74mr80v1a3vfnkn5pkjvnxlnhhz23-cowboy-machine-writer-host-release`.
Root transaction `1791170446907916891-7c6551ffbb3e` started at
`2026-10-05T03:20:46.907916891Z` and committed at
`2026-10-05T03:20:54.891448865Z`. It reports succeeded, committed, published,
maintenance and no recovery. Startup at `03:20:46.984336Z` confirms the writer
remains enabled with zero recorded deletions.

Runtime source `6c204d84` passed 1877 all-features library tests and the final
551 standalone Machine tests; 57 and 17 environment-dependent tests were ignored.
Both Clippy gates and Rustfmt passed. A real-filesystem fixture injects an I/O
failure after one payload has been deleted, confirms two payloads and both valid
markers remain, then retries successfully. It also fails at the start of marker
finalization, verifies all observed payloads are already gone while both markers
remain, and retries successfully. A third pass skips the unmarked retained tree.
Both ignored mount fixtures were separately executed successfully. Native
read-only review found no new defects.

The first four-thread standalone run reported one failure in the existing
journal close/read-only-reopen test: its exclusive lock was still busy. The
isolated nine-test journal suite and full four-thread standalone rerun passed
without source changes. No journal code or lock semantics were changed; the
cause of that contention was not conclusively established. These are finite
verification results, not proof against every concurrent fork/lock scenario.

The exact final writer, previous active writer, accepted reader-only fallback
and final default reader passed 32 production conformance groups in private root
mount/PID/network namespaces. An earlier candidate was accepted too; the final
writer was rebuilt and reaccepted after integrating a concurrent Web usage-widget
change. Runtime Rust, wire, SDK, dependencies and worker pin remained unchanged
by that merge. This task activated only the Machine component. No synthetic
records entered the live journal and no native production checkpoint hooks were
introduced.

During `03:20:35.209Z`–`03:21:30.560Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`3625760` to `3869872`; Controller PID `2648009` remained unchanged. Accepted
generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, installed component owner
and sudoers digest remained unchanged. `sudo -n true` succeeded. Health, version,
SPA, service worker and Machine deployment-health endpoints returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-finalization-2026-10-05.json)
contains exact artifacts, native hashes, conformance observations, initial test
failure and final outcomes, and before/after receipts. The two marker removals
are not transactional: failure after the first can leave unmarked marker residue
after the observed content walk succeeded. Concurrently inserted contents or
marker mutations are not frozen. The final comparison/name unlink race, general
I/O deadlines, continuous ownership, durable Session incarnation and portable
writer admission remain open. Non-Linux ordering and pathname fallback are
unchanged.
