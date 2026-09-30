# OVH Luna remote repair on Hawk

This acceptance exercises a complete remote repair, beyond the earlier local
OVH coding and read-only SSH probes. The deliberately broken project lives on
Hawk. A real Cowboy-managed Codex worker on the permanent `ovh` Machine must
read, diagnose, edit, test and commit it through SSH. No business project is
used as a fault fixture.

## Execution boundary

Session `sess-1790767522146` uses signed Codex 3.1.30, model `gpt-6-luna`,
effort `max`, through the official authenticated `cowboy serve-acp` client.
Its tool working directory is the session worktree under OVH Service
`svc-4e4d5154f3df9aa109d7d841dd925fd7`. The task explicitly prohibits local
implementation/testing, delegation, alternate hosts, credential inspection,
dependency installation and host-service changes.

The permanent `matrix-agent` identity intentionally permits read-only queries.
This test uses a separate temporary key generated on OVH, never copied off it.
Hawk authorizes that key with `restrict`, an expiry and a root-owned forced
command. The command runs an unprivileged Bubblewrap sandbox with only the
isolated fixture writable at `/workspace`, disposable `/tmp`, read-only tools,
no host home or credentials, and no network inside the sandbox. Commands have
CPU, memory and wall-clock limits. An independent two-hour cleanup timer was
armed before use. Permanent SSH policy was not broadened.

The session-scoped launcher supplies `ssh hawk` with a pinned host key and
Stormbird MagicDNS destination. The prompt contains no physical IP, provider
location, SOCKS configuration or tunnel instruction. This proves a logical-name
workflow; it does not establish that an unrestricted OVH account cannot inspect
network configuration. The existing operator recovery path is separate.

## Frozen problem and independent checks

Initial fixture commit: `24467b9`. All six original tests fail before repair
(four assertions and two errors). The required behavior covers timezone-aware
instant comparison, a half-open time interval, latest in-window event per ID,
last-input tie breaking, nearest-rank P95, empty results, gzip/Unicode paths,
stdin and invalid-record accounting. Existing tests must remain byte-identical.

Before the agent started, an independent holdout was frozen outside the SSH
sandbox. Its eight test methods include eight P95 boundary sizes and 100
deterministically generated cases using a separate reference calculation.
Neither the holdout nor a correct implementation was supplied to the agent.

## Results

The complete remote repair passed. The official ACP prompt returned
`stopReason: end_turn`. All 12 tool calls used the scoped `ssh hawk` launcher
from the OVH session worktree. The only nonzero exit was the intended failing
baseline test run; there were no SSH connection failures in these calls.
The first-to-last tool event window was 330.550 seconds. Individual tool event
durations were 8.625–16.768 seconds, including transport, execution and event
delivery; these are not isolated network-latency measurements.

The agent performed these actual remote operations:

```sh
ssh hawk 'cd /workspace && python3 -m unittest -v'
ssh hawk 'cd /workspace && cat > report.py' <<'PY'
# Full implementation was delivered through SSH stdin; see the evidence JSON.
PY
ssh hawk 'cd /workspace && cat > test_regressions.py' <<'PY'
# Five added regressions; existing test_report.py was not changed.
PY
ssh hawk 'cd /workspace && git add report.py test_regressions.py && git diff --cached --check && git diff --cached && git status --short --branch'
ssh hawk 'cd /workspace && git commit -m "Repair request log summaries"'
```

These excerpts omit the common session-scoped `PATH` prefix; the evidence
contains the exact commands and complete stdin bodies. They are a transcript,
not instructions to overwrite another repository.

The root causes were lexical timestamp comparison, retaining the first request
instead of the latest, an off-by-one percentile index and empty-array access,
and treating every input as a plain file with no record validation. The repair
normalizes aware timestamps, selects the latest in-window event with explicit
tie behavior, calculates the nearest rank using integer arithmetic, handles
empty results, and supports plain/gzip/stdin input with invalid-line accounting.

Remote commit: `12eab32c9b7df5dd3a5b2b8ae2946caf6ff3df93`,
`Repair request log summaries`. It changes only `report.py` and adds
`test_regressions.py` (164 insertions, 8 deletions). The agent inspected the
staged diff, passed `git diff --cached --check`, committed, and read back HEAD
and status through SSH. The only untracked content was Python bytecode.
A harmless `printf` option error during early source inspection is retained in
the trace; it did not prevent reading files or subsequent execution.

Validation:

- Agent's remote suite: all 11 tests passed, including all six original tests.
- Operator independently reran the same 11 tests on Hawk: all passed.
- Frozen independent holdout: all eight methods passed, including eight rank
  boundary sizes and 100 generated reference cases.
- Original test SHA-256 remained
  `689ea3039c2a1b46f58eb89c372ce317bc164d93b301ee46795fb6b335cd1259`.
- The agent's real stdin CLI example combined a timezone-offset retry and a
  newer successful request, returning
  `{"count":1,"errors":0,"invalid_lines":0,"p95_ms":25}`.
- The operator made no repair-code changes. The solution and added regression
  tests were written by the OVH agent over SSH.

## Evidence and cleanup

The sanitized [evidence JSON](../acceptance-results/ovh-hawk-ssh-repair-2026-09-30.json)
contains the initial fixture files, repair patch, exact SSH tool commands and
terminal outputs, frozen holdout source and independent test output. Native
reasoning messages, authentication data and key material are excluded.
The private operator archive also retains a verified Git bundle with both
fixture commits. The permanent Cowboy session and Machine remain available.

The temporary key was removed from Hawk's authorization file, which matched
the pre-test bytes exactly. A fresh OVH SSH attempt with that key returned
exit 255 and `Permission denied`. The OVH private key and scoped launcher,
Hawk forced-command runner, cleanup timer and isolated working directory were
then removed. The permanent Machine remained active with PID 56151 and zero
restarts; no production component deployment was needed.

Recovery does not require the temporary working directory or access key. To
reproduce locally, write the JSON's `baseline_files` into a new temporary Git
repository and run `python3 -m unittest -v` (expect four failures and two
errors). Apply `repair_patch` with `git apply`, then rerun the suite (11 passes).
Write `holdout_source` outside that repository, change only its `ROOT` assignment
to the reconstructed directory, and run it (eight passing methods, including
100 generated cases). Verify `holdout_sha256` before changing the path and
`fixed_tests_sha256` against the reconstructed original test file.
This exact reconstruction was independently executed successfully after the
remote working directory was removed. It validates archive reproducibility;
the actual agent acceptance remains the earlier OVH-to-Hawk SSH execution.

After cleanup, the permanent OVH `ssh hawk hostname` path still returned `hawk`
and the Cowboy Machine retained the same active PID and zero restarts.

`nix develop -c just check-compact` passed, including the isolated PostgreSQL
cases, Web tests and release builds. The complete active documentation/evidence
diff was reviewed against the recorded commands, remote commit, independent
checks and cleanup results. No Cowboy implementation change was necessary for
this remote-repair task.

This task does not promote the earlier failed network baseline, activate an
OVH transport policy, or accept Falcon, browser UI, host reboots or mobile-device
regressions. Those retain their separately recorded status.
