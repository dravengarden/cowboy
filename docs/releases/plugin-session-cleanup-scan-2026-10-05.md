# Bounded cleanup scan opens — Hawk, October 5

Linux deleted-session cleanup now opens every pending directory and candidate
relative to the retained Session-root descriptor. `openat2` uses `BENEATH`,
`NO_SYMLINKS` and `NO_XDEV`; enumeration uses the opened directory descriptor.
Linked ancestors, mount points (including same-device bind mounts), missing
paths and nondirectories are skipped before marker probes. Other errors fail
collection before removal without a Linux pathname fallback. The
[contract](../plugin-session-cleanup-targets.md) records the finite scan boundary
and links the Linux interface specification.

Implementation `30fe44043d8f6b483362a14d079ac10a845332b8` is included in published
and active source `8ca8be38adfe66de57db983ea4a427fa71bd41ef`. The exact artifact is
`/nix/store/2c62zpf3zail41iqimndr43gcw5b86sy-cowboy-machine-writer-host-release`.
Root transaction `1791166661918469653-8ca8be38adfe` started at
`2026-10-05T02:17:41.918469653Z` and committed at
`2026-10-05T02:17:50.760753723Z`. Its receipt reports succeeded, committed,
published, maintenance and no recovery. Startup at `02:17:42.035297Z` confirms
the writer remains enabled with zero recorded deletions.

Runtime source `30fe4404` passed 1871 all-features library tests and 545
standalone Machine tests; 56 and 16 environment-dependent tests were ignored.
One newly ignored fixture was separately executed successfully in an explicitly
private mount namespace. It binds same-device directories over both an ancestor
and a direct Cargo target, confirms bounded opens refuse both, preserves foreign
artifacts and clears a separate local target. An ordinary filesystem regression
replaces an ancestor with a link and refuses linked and escaping paths.

Clippy requested a direct pattern match instead of a redundant guard for the
same four errno alternatives. Final source `8ca8be38` passed all eight focused
cleanup tests, the separately executed actual bind-mount fixture, both Clippy
gates and Rustfmt. Native read-only review of the implementation found no
actionable regressions. No wire, SDK, dependency or worker pin changes were made.

The final immutable writer was rebuilt after that syntax correction. It, the
previous active writer, accepted reader-only fallback and final default reader
passed 32 production conformance groups in private root mount/PID/network
namespaces. These verify journal/startup/IPC compatibility; scan-specific
evidence comes from the filesystem and actual mount fixtures. No synthetic
records entered the live journal and no native production checkpoint hooks were
introduced.

During `02:17:32.184Z`–`02:18:16.717Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`2692488` to `2837499`; Controller PID `2648009` remained unchanged. Accepted
generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, installed component owner
and sudoers digest remained unchanged. `sudo -n true` succeeded. Health, version,
SPA, service worker and Machine deployment-health endpoints returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-scan-2026-10-05.json)
contains exact artifacts, native hashes, conformance observations, the mount
fixture command and before/after receipts. This is scan-time mount admission.
Mounts added after an accepted open or inside a target during recursive removal,
independent descendant mutation, I/O deadlines, continuous ownership, durable
Session incarnation and portable writer admission remain open. Non-Linux Unix
retains its explicitly weaker pathname fallback.
