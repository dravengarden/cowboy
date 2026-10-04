# Session deletion writer declaration admission — October 4

The installed Columbus component owner now accepts schema-1 Machine writer
declarations only when the target already has a valid root-owned Session
deletion reader floor. Empty or missing Machine data cannot substitute for that
floor. A writer candidate cannot originate its own floor from a compatible
fallback; the reader-only transition and independent reader anchoring must
finish first.

Activation, fallback restoration and independently selected recovery targets
share the floor and dataset checks. A schema-1 reader-only fallback remains
eligible. Missing, corrupt or foreign floors refuse before profile mutation.
Interrupted writer recovery also rechecks the floor on replay before retention
or journal effects. Existing maintenance, ancestry and exact worker-generation
constraints remain in force.

This changes the host's release admission policy. The production Machine
constructor still hard-codes `writer_enabled=false`; no production writer
artifact was activated, and new production deletions remain process-local.
Exact production writer artifacts, runtime authority checks and their actual
crash/reopen/failure acceptance remain separate work. The
[immutable private writer matrix](../experiments/plugin-deletion-immutable-writer-2026-10-04.md)
is a prerequisite, not production writer acceptance. The newly installed owner
still refuses both private fixtures as foreign component/lane artifacts before
dispatch; all component receipts remain unchanged.

## Verification and activation

Columbus implementation is `6e5da18389eb046991db19019573ff6f80ef7866`;
integration and activated source is
`68077bae1e234f654b6031268eac7d32f61035fd`, published to remote `main`.
The package tests and complete `just verify` passed before commit, and the
integrated gate passed again. A read-only native Codex review with persistent
memory disabled found no actionable defects. Its sandbox could not rerun Nix
tests; the deterministic gates were independently executed in the task shell.

The clean Hawk build produced:

```text
/nix/store/0wbpzbvs2v35hqnly895iq4nqg3j21im-nixos-system-hawk-26.05.20260731.5b4f72e
```

Activation used the machine-owned transaction. The first dispatch refused
before switching because an unrelated Carrack heartbeat had timed out. Its
scheduled timer naturally succeeded at `2026-10-04T20:31:45+08:00`; no failed
state was cleared and no service was manually restarted. Re-dispatching the
same immutable candidate succeeded with transaction
`1791117133710119640-68077bae1e23`, recorded at
`2026-10-04T20:32:16+08:00`. The root receipt reports `published=true`, no failed
health checks and no new failed units. Only `mandb.service` is listed as changed.

The installed owner is:

```text
/nix/store/j4lm759j2vz9rgll0wn1njxmrfbm0dc7-columbus-machine-activate-1da44eb/bin/cowboy-release-activate
SHA-256: 67bbe31186fa4337a447509282a47e3b19a15cccce73ab1ead6d48d4bd558d0b
```

Snapshots immediately before activation and after acceptance retain all 13
worker and five execution-keeper unit identities, PIDs and states. Machine PID
`3031824` and Controller PID `3461065` remain unchanged. Machine, Controller and
Web component receipts and the resolved SPA bytes are identical. The root
reader floor digest remains
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`;
the live namespace still contains only `.lock`. The last reader startup records
zero terminal IDs and writer false. Public health, version, SPA, service worker
and Machine deployment health all return 200. Passwordless sudo still works,
and the sudoers digest is unchanged.

Exact before/after snapshots, host receipts, fixture refusals and scope limits
are in [the acceptance receipt](../experiments/plugin-deletion-owner-writer-admission-2026-10-04.json).
