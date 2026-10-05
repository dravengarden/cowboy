# Cargo cleanup retry observations — Hawk, October 5

Deleted-session cleanup now retains its first complete target scan across ordinary
I/O retries. Previously, an error dropped the observed target handles, and a
later scan could admit a same-path replacement carrying valid Cargo markers.
Cloned cleanup workspaces now share a serialized, process-local plan containing
at most 128 target handles and successful-target progress. Retries skip completed
targets and do not discover replacement or newly added targets. Failed scans
admit no plan or removal effects; overall success releases the plan so a later
explicit cleanup can observe a new Cargo build. See the
[contract](../plugin-session-cleanup-targets.md).

Implementation and active source
`75442a8afcf28f1a66ff2d04a8de580ceada7554` is published on remote main. Artifact
`/nix/store/x8b2gfhphbnlafplm7rj7blhrsv3l99d-cowboy-machine-writer-host-release`
was activated by the installed component owner through root transaction
`1791172362737356005-75442a8afcf2`. It started at
`2026-10-05T03:52:42.737356005Z` and committed at
`2026-10-05T03:52:50.72524786Z`, with succeeded, committed, published,
maintenance and no recovery. Startup at `03:52:42.819110Z` confirms writer
enabled and zero recorded deletions.

The source passed 1892 all-features library tests, 561 standalone Machine tests,
27 targeted workspace tests, both Clippy gates and Rustfmt. The ordinary runs
ignored 57 and 17 environment-dependent tests. Both mount fixtures were
separately executed successfully in an isolated private mount/PID/network
namespace. Linux regression fixtures use the production synchronous entrypoint
and workspace clones: an injected I/O error precedes marked/unmarked target
replacement, symlink substitution, disappearance or parent replacement. Retry
refuses each change and preserves the observed original and replacement files.
Another fixture completes one target, fails the next, replaces the completed
target and adds a late target; retry clears only the original pending target.
Two repeated I/O failures against an unchanged original also recover successfully.
No callback is exposed through production CLI, environment or IPC.

The first all-features run had 1891 passes and one failure in the existing
telemetry deadline test: its `Destination::started` two-second wait timed out.
The isolated 12-test telemetry suite and full four-thread all-features rerun
passed without source changes. No telemetry code, deadlines or tests were
changed; the cause was not conclusively established. The evidence retains the
initial failure alongside the successful final results.

The exact candidate writer, previous active writer, accepted reader-only
fallback and candidate default reader passed 32 native production conformance
groups in private root namespaces. No synthetic records entered the live
deletion journal. The build retains separately accepted worker source
`b97c2724bea23834944ded8af98e2de6729f4256` and generation
`worker-748825b42b4302fe26ca`.

During `03:52:31.002Z`–`03:53:20.486Z`, all 13 workers and six execution keepers
kept identical IDs, PIDs and active states. Machine PID changed from `3869872`
to `4183275`; Controller PID `4180769` stayed unchanged. Controller/Web
receipts, resolved SPA, installed owner, sudoers digest, host source, reader
floor and journal entries stayed unchanged. `sudo -n true` succeeded. Health,
version, SPA, service worker and Machine deployment-health returned HTTP 200.
An independent Controller update had completed before this observation window;
this task activated only the resident Machine component.

The [machine-readable evidence](../experiments/plugin-session-cleanup-retry-2026-10-05.json)
contains exact artifacts, native hashes, acceptance observations, tests and
before/after receipts. The retry plan is not durable across resident restart,
and returned completed paths describe historical cleanup rather than replacement
contents. Marker finalization is not transactional. The final comparison/name
unlink race, concurrent content changes, general I/O deadlines, continuous
ownership and durable Session incarnation remain open. Non-Linux content access
retains its weaker fallback; no general device or power-loss acceptance is claimed.
