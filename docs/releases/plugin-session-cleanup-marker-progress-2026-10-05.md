# Cargo marker finalization progress — Hawk, October 5

Linux deleted-session cleanup now resumes its own partial marker finalization.
Previously, an I/O error after deleting one root Cargo marker left the retained
retry plan unable to pass the ordinary two-marker eligibility check. The broker
could then retire unfinished marker cleanup as a target change.

Content traversal now skips the two root marker names. After its successful
walk, the plan captures both regular marker objects with restricted
`openat2`/`O_PATH` opens, validates the bounded cache tag and retains at most two
extra handles for the current target. It records each successful unlink before
any later fallible check. Retry finishes only the original pending markers:
previously removed names must remain absent, pending marker identities must
match, and the remaining tag must retain eligibility. It does not repeat the
content walk or remove later inserted files. See the
[contract](../plugin-session-cleanup-targets.md).

Published and active source is
`e51c79bc3a90f3f9bc1f856cc54039b857df4907`. Artifact
`/nix/store/kfkbi1i0l5nxb38lsdbcj93hk4fkyig0-cowboy-machine-writer-host-release`
was activated by the installed owner through root transaction
`1791173415688470865-e51c79bc3a90`. It started at
`2026-10-05T04:10:15.688470865Z` and committed at
`2026-10-05T04:10:21.577408234Z`, reporting succeeded, committed, published,
maintenance and no recovery. Startup at `04:10:15.753096Z` confirms writer
enabled with zero recorded deletions.

The source passed 1894 all-features library tests, 563 standalone Machine tests,
29 targeted workspace tests, both Clippy gates and Rustfmt. Ordinary gates
ignored 58 and 18 environment-dependent tests. Three ignored mount fixtures
were separately executed successfully in an isolated private mount/PID/network
namespace, including a real same-device file mount over the remaining marker
after the first unlink. It refuses crossing and preserves both the foreign
file and original underlying marker.

Production synchronous-entrypoint fixtures and workspace clones inject repeated
errors at the second marker. They confirm the first successful unlink is
retained, retry removes only the remaining original marker, and a later file is
preserved. Marker replacement, disappearance, symlink/FIFO substitution, tag
withdrawal, recreation of the already removed name, and target/parent/Session
replacement all refuse before additional removal callbacks. A later explicit
marked Cargo rebuild remains cleanable. Callbacks are private test seams, not
production CLI/environment/IPC checkpoint controls.

The exact candidate writer, preceding active writer, accepted reader-only
fallback and candidate default reader passed 32 native production conformance
groups. These are finite journal/IPC/admission checks in private root namespaces,
not proof against every filesystem race. Live journal entries remain only the
existing lock. The artifact retains separately accepted worker source
`b97c2724bea23834944ded8af98e2de6729f4256` and generation
`worker-748825b42b4302fe26ca`.

During `04:09:47.577Z`–`04:10:50.417Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`4183275` to `200864`; Controller PID `117233` stayed unchanged. Controller/Web
receipts, resolved SPA, installed owner, sudoers digest, host source
`a0419eeb2bbc7dc535457685169f0adeb4032fa2`, reader floor and journal entries
stayed unchanged. `sudo -n true` succeeded; health, version, SPA, service worker
and Machine deployment-health returned HTTP 200. Independent host and Controller
maintenance completed before this new observation window. This task activated
only the resident Machine component.

The [machine-readable evidence](../experiments/plugin-session-cleanup-marker-progress-2026-10-05.json)
contains exact artifacts, native hashes, test results and preservation receipts.
Marker progress remains process-local and is lost on resident restart. The two
unlinks are not transactional, the final comparison/name unlink remains
non-atomic, and concurrent contents are not frozen. Completed paths report the
original observed walk rather than current emptiness. General I/O deadlines,
continuous ownership, durable Session incarnation and portable writer admission
remain open. Non-Linux marker ordering and pathname fallback are unchanged; no
general-device or power-loss acceptance is claimed.
