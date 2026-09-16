# Core process cleanup has no PATH helper

Plugin command/runtime cleanup and the direct Machine-worker watchdog now call
the existing `rustix` process-group primitives. They never resolve or execute
an ambient `kill`. This is a core implementation fix; no signed Plugin byte,
dependency or durable journal format changes.

Only positive group IDs greater than one are eligible. Zero selects the caller's
own group, and the negative form of one would select all signalable processes;
neither is an owned child group. Overflowed IDs are refused as well. The direct
worker mapping stays locked from its original-owner check through the syscall,
without a subprocess spawn or asynchronous gap.

Only `ESRCH` proves absence to the direct-worker watchdog. `EPERM`, interruption,
invalid inputs and other errors retain its fence. The former command exit status
could conflate permission denial with absence. Signal errors remain visible;
success is not by itself proof of termination or restoration.

Existing dedicated cgroup cleanup remains best-effort. A process group cannot
contain a descendant that deliberately creates another session, and this change
does not add restart adoption, durable native recovery, PID-reuse immunity,
cgroup admission guarantees or rollback of child filesystem/network effects.

Tests launch separate child test processes with an unavailable PATH and a PATH
containing a hostile fixture `kill`; no global test environment is mutated. Both
must stop the owned leader and ordinary descendant while preserving an unrelated
child, without executing the fixture helper. Independent tests cover invalid
selectors, failed probes, stale owner mappings and existing direct-worker
graceful/TERM/KILL escalation. An orphan zombie is terminated, not evidence of
verified reaping; the connected acceptance gate separately adopts and reaps its
fixture descendants behind private PID/network/cgroup isolation.

Controller activation covers its Plugin commands only. Applying the broker and
resident Code runtime changes requires a separate Machine maintenance release;
publishing or building that candidate does not activate it. The worker-generation
source closure is unchanged, so no Agent generation drain is requested by this fix.

The subsequent [Zed 1.7.0 rollout](releases/zed-native-sync-2026-09-16.md) includes
that separately authorized Hawk Machine activation. It records the old Machine's
isolated installation timeout, the accepted syscall-based replacement, retained
workers and one later native identity-preserving generation cutover. That
integrated release includes other previously pending Machine changes too.
