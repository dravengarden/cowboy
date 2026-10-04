# Session worktree containment — Hawk, October 4

The Machine now refuses reused checkout links and selected directories that
escape the isolated checkout. Reuse checks precede detached-branch mutation.
Internal links remain compatible. The [contract](../plugin-session-worktree-containment.md)
describes the preparation-time scope and remaining continuous ownership gap.

Source `99b1664d2ab324fc5d63f557cbe1dcac78c3270b` was published to `main` and
activated as `/nix/store/xgg100vqcy9j9d8ywqh1img37p18cp46-cowboy-machine-writer-host-release`.
Transaction `1791126383070476844-99b1664d2ab3` started at
`2026-10-04T15:06:23.070476844Z` and committed successfully at
`2026-10-04T15:06:33.684090087Z`; the root receipt reports published, maintenance
and no recovery. Startup at `15:06:23.152039Z` confirms the deletion writer remains
enabled with zero recorded deletions. No synthetic live journal records were
introduced.

Rustfmt, standalone Machine Clippy and all 535 standalone Machine tests passed;
15 environment-dependent tests remain ignored. Real Git regression cases cover
new and reused escaping selections, a link to the stable checkout, preservation
of dirty files and detached HEAD, and successful internal-link reuse. Native
read-only review found no actionable regressions. The exact new writer and
previous active writer passed 32 production conformance groups in private root
mount/PID/network namespaces, including reader fallback and startup refusal.

The activation window `15:06:06.864Z`–`15:06:44.264Z` retains every one of 12
workers and six execution keepers with identical IDs, PIDs and active states.
The resident Machine changed PID `403650` to `733209`; Controller PID `541829`
remained unchanged. Worker generation `worker-748825b42b4302fe26ca`, reader-floor
digest, live journal entries, Controller/Web receipts, resolved SPA, host source,
component owner and sudoers digest remained unchanged. `sudo -n true` succeeded;
health, version, SPA, service worker and Machine deployment-health endpoints
all returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-worktree-containment-2026-10-04.json)
contains exact artifacts, native hashes, conformance cases and before/after
receipts. This release does not establish continuous worktree ownership,
durable Session incarnation, portable writer admission or supported-device
native acceptance.
