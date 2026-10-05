# Original directory during terminal cleanup — Hawk, October 5

Asynchronous deleted-session cleanup now retains the directory observed before
worker stop. Observed root replacement retires cleanup and preserves artifacts.
On Linux the actual filesystem access remains attached to that directory handle.
The [contract](../plugin-session-cleanup-root.md) describes the finite observation,
existing process-exit proof, and remaining nested-directory and launch-time gaps.

Implementation `3b1b9923` is included in the published and activated source
`4e049fd6cbfaa551118fe4c69286cb63c945a649`. The exact active artifact is
`/nix/store/06k1h84gck3y41dl1vcwf32x1x50il9l-cowboy-machine-writer-host-release`.
Root transaction `1791160087069820309-4e049fd6cbfa` started at
`2026-10-05T00:28:07.069820309Z` and committed successfully at
`2026-10-05T00:28:16.104454979Z`; the receipt reports published, maintenance and
no recovery. Startup at `00:28:07.151450Z` confirms `writer_enabled=true` and zero
recorded deletions. No synthetic records were introduced into the live dataset.

The integrated runtime source passed 1862 all-features library tests and 537
standalone Machine tests; 54 and 15 environment-dependent tests remain ignored.
Rustfmt and both Clippy gates passed. The inherited Draft backend contract needed
one test-only `too_many_lines` allowance, matching the repository's other shared
backend fixtures. Test bodies and runtime code did not change in that lint repair;
the later main integration changed only Web and acceptance files. Native read-only
review of the cleanup commit found no actionable regressions. The exact final
writer, previous active writer, accepted reader fallback and final default reader
passed 32 production conformance groups in private mount/PID/network namespaces.
This covers writer startup and IPC compatibility; cleanup's directory replacement
and lifecycle race evidence comes from the real-filesystem library fixtures.

The activation window `00:27:58.295Z`–`00:29:10.762Z` retained every one of 12
workers and six execution keepers with identical IDs, PIDs and active states.
Resident Machine PID changed from `733209` to `2069302`; Controller PID `803041`
remained unchanged. Generation `worker-748825b42b4302fe26ca`, reader floor, live
journal entries, Controller/Web receipts, resolved SPA, host source, component
owner and sudoers digest remained unchanged. `sudo -n true` succeeded. Health,
version, SPA, service worker and Machine deployment-health endpoints returned 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-root-2026-10-05.json)
records exact revisions, native hashes, conformance cases and before/after receipts.
Continuous Session/worktree ownership, durable Session incarnation, portable writer
admission, nested-directory mutation fencing and general lifecycle resource bounds
remain open.
