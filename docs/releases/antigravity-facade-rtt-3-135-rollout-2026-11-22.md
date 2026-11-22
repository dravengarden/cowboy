# Component release 3.135.0 rollout (Antigravity facade round trips) — 2026-11-22

Status: the seven 3.135.0 Agent Provider releases are published and
`antigravity` 0.2.20 is active on OVH (rollback 0.2.19), accepted with real
traffic. No Machine, worker, Controller or Web release was needed. Release
3.136.0 (Antigravity 0.2.21) builds on it.

## Why

OVH's Local OTel store (three days to 2026-11-22) showed the Antigravity facade
spending most of its execution calls on polling: 2,428 `process/read` for 109
commands in one session (22 per command, against Claude's two), each a ~0.2 s
round trip to the workspace Machine. A `ReadFile` asked for metadata twice, and
a projection read every guidance and skill file one after another.

## Content

`cowboy-antigravity-acp` 1.0.17 (provider-runtime 1.3.19):

- A command quiet for three one-second reads waits for the executor's
  notification naming it (output, exit or close), bounded by 15 s and ending
  one second before its deadline, as the Claude runtime does.
- Reads of one process run one at a time and a command's chunks land in its
  retained output whichever reader took them; a confirmed close wakes a waiting
  reader; a started process that cannot be read again is an unknown outcome,
  fenced on its turn, parent and siblings before cleanup.
- `ReadFile` reuses its metadata; projections read guidance and skills three at
  a time each and read nothing once the guidance budget is spent.

## Gates

| Gate | Result |
| --- | --- |
| antigravity-acp tests | 227 passed; every new test failed on the code before it |
| `check-compact` at the release commit | Passed |
| Code review (Codex Sol Max, routine) | Three rounds (the maximum); all findings fixed except one shared with the former polling: a read that stalls in transit still holds a command's deadline until the connection's 185 s request timeout |

## Activation and acceptance

`cowboy operator install --machine ovh --plugin antigravity --version 0.2.20`
(operation `ovh-antigravity-0-2-20-facade-rtt-install`) completed; active
generation `3b57e302…`, rollback `cd8cfdaf…` (0.2.19).

Session `sess-1795312987846` (runtime OVH, workspace Hawk, preset
`gemini-hydrogen-high`), then deleted: `sleep 12` took five reads (three
one-second reads, one zero-wait read, one after the exit notification, seen at
once) instead of about thirteen; a command printing every two seconds kept
one-second reads; `RunCommand` with `wait_seconds: 3` then `StopCommand` took
one terminate and one drain read. All four steps reported their results.
