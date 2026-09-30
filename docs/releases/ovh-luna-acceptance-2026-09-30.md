# OVH Codex / Luna acceptance

Status: in progress; this document is a frozen acceptance contract, not a pass.

The permanent Machine is `ovh`, Service
`svc-4e4d5154f3df9aa109d7d841dd925fd7`. Use the signed Codex 3.1.30
release `sha256:10e7c0bfd658ff9cb612f913e431a7d01fb66d0225c31e3147b0405045c44cd1`.
Select `gpt-6-luna` before sending any prompt; complex cases use `max` effort.
Do not substitute another model. Existing sessions and Machine identity remain.

## Cases and fixed gates

1. Official installation receipt, current credential replica/materialization,
   real Cowboy session creation, advertised model selection and streamed reply.
2. Reattach immediately after initialize, replay, multi-turn memory, tool calls,
   model/effort persistence, usage reporting and independent second observer.
3. Bounded file creation, tests, deliberate failing test followed by repair,
   Unicode paths, Git diff and save persistence in a disposable repository inside
   the session-owned workspace. No business-file edits.
4. Read-only `ssh hawk` / `ssh falcon`, IPv4 and IPv6, pinned host keys and
   constrained remote identities. Give the agent no physical address, proxy
   configuration, provider topology or alternate-destination repair instruction.
5. Cancel, queued follow-up, detach/reload and network reconnect. Record request
   sequence and execution count; missing acknowledgements are not a successful
   exactly-once test. Keep failures and partial results.
6. Stormbird-owned, separately gated bidirectional JMS transport acceptance:
   exact approved leaves, real outer sockets and counters, ACL deny, fail-closed
   IPv4/IPv6, refusal, blackhole, repeated failover, sticky healthy backup,
   all-exit outage/recovery, relay/control failure and long-lived application
   recovery. No fault before an independent timed cleanup/recovery path exists.
7. Machine/host restart, short network outage, identity/plugin/credential/workspace
   persistence, resource peaks and OOM checks require their own maintenance
   boundary and recovery evidence. Existing device regressions require actual
   device checks; server heartbeats cannot replace them.

Before faults, require at least 300 seconds of healthy baseline. After faults,
require at least 1800 seconds of stable observation with zero failed probes.
Exit selection budget is 30 seconds; complete application recovery is 60 seconds.
Measure these separately. Do not relax these thresholds after observing results.
Record unsupported UI cases and physical-device checks as pending.

## Installation evidence

The original operation `ovh-codex-3-1-30-luna-20260930` expired in staging.
Two official reconciliation calls recorded the exact Machine receipt and
resolved the activation-free staging failure. The historical failure remains.

Public signed runtime inputs (162475310 and 46464252 bytes) were transferred
through the authenticated operator SSH path and imported using the Machine's
digest-verifying artifact cache command. No credential files or activation
pointers were copied. The next authorized installation operation,
`ovh-codex-3-1-30-luna-cached-20260930`, completed with revision
`installation-5b46d7bf2e3046c9a003f5a27d39f6366e6f0cece06b5b8f268b9113aca937d4`.
Inventory reports active, authentication generation 7, replica current and
materialization current. No Machine restart was required.

Protected raw evidence is held in the operator's
`~/.local/state/ovh-luna-acceptance-20260930/`; only sanitized results belong here.

## Actual session results

Session `sess-1790767522145` is a real Cowboy-managed Codex worker on OVH.
The client selected `gpt-6-luna` before the first prompt, received the streamed
`OVH_LUNA_READY_20260930` response, then selected `max` effort. Repeated fresh
CLI clients loaded the same session immediately after initialize. The advertised
model and effort persisted. These are ACP/CLI results, not browser UI acceptance.

The agent received only logical SSH aliases. Its exact IPv4 `ssh hawk hostname`
and IPv6 `ssh hawk id` tool calls returned exit 0, hostname `hawk`, and the
constrained `matrix-agent` identity. `ssh falcon hostname` returned exit 255
with a name-resolution failure; it did not try another address or bypass SSH
policy. Logical-address transparency passed for Hawk only. This is a workflow
abstraction, not proof that a full-access agent cannot inspect host configuration.

The session created `acceptance-luna-20260930` inside its own worktree and
initialized a separate Git repository. It committed a deliberately incomplete
topological-sort implementation, observed 6 passes and 2 failures, fixed the
graph handling, observed 7 passes and 1 CLI whitespace-expectation failure, then
corrected that expectation and obtained 8 passes. Unicode paths, duplicate
prerequisites, absent dependency keys, cycles, deterministic order, input
immutability and subprocess execution were exercised. Independent operator SSH
reran all 8 tests successfully. The final uncommitted diff changes two files,
with 11 insertions and 6 deletions. No business file was edited.
An independent exhaustive-permutation oracle also passed 100 generated DAG
cases (seed 20260930, zero to six nodes), comparing the result against the
lexicographically smallest valid ordering and checking input immutability.

The test driver's 300-second prompt deadline expired before the complex turn's
final response. It detached without resending the task. A second client loaded
the still-busy session and subsequently received its final response and idle
state. The timeout is retained; it is not a lossless-reconnect or latency pass.

Cancellation needs two separate conclusions:

- The first single readiness marker was absent from the received terminal
  deltas; no timely cancellation was triggered. A later idle cancellation does
  not validate that run.
- A second bounded command repeated its marker. Cancellation returned
  `stopReason: cancelled` after 1.312 seconds, but an independent process check
  still found the test Python process. The follow-up observed that the original
  command subsequently completed its 120 ticks. Turn cancellation passed;
  termination of an already-running tool process did not.
- The second run's execution counter contained exactly one line. Follow-up
  reran all 8 tests successfully and returned `LUNA_FOLLOWUP_OK`. This proves
  this command was not restarted, not general exactly-once delivery.

The CLI follow-up queue also passed a bounded two-turn case: after receiving
live output from A's 60-second tool command, the client submitted B without
waiting for A's prompt response. A completed first, then B completed. Independent
operator readback found exactly `A`, `B` in the execution-order file. This
validates this ACP queue case, not the browser queue UI or reconnect deduplication.
At final cleanup the bounded cancellation process was gone, the Machine retained
PID 56151 and `NRestarts=0`, and the two temporary uploaded public runtime archives
were removed. The normal digest cache remains part of the installed Machine state.

No standard `usage_update` appeared in the captured ACP events. Context usage,
browser reload, Code surface, machine/host restart, resource
peaks and physical-device regressions remain unaccepted. The temporary test
repository remains available for those remaining checks; the permanent Machine
and existing sessions remain registered.

## Network gate failure

Before any production profile change or injected fault, a 21-round baseline
over more than 300 seconds ran four probes per round. OVH-to-Hawk SSH failed
in both private address families in rounds 16 and 17: first the 25-second outer
driver deadline, then explicit inner SSH connect timeouts. All 21 Controller
health and all 21 management HTTPS requests succeeded. Thus 80 of 84 probes
passed; the zero-failure baseline failed.
The management probe's `/health` path served the SPA, so its successes establish
HTTPS-origin reachability only, not health-API correctness. Future probes use
`/api/v1/health`; the old samples remain limited evidence.

The OVH-only JMS profile candidate passed strict preview, and Stormbird
`just verify-config` passed. It was **not activated**. No rollout promotion,
fault injection, host restart, threshold relaxation or claim of a 30-minute
stable window followed the failed baseline. Hawk remained intent 395 with
proxy-required transport; OVH remained intent 1 with direct-relay transport;
Falcon remained outside the new production policy. Therefore bidirectional JMS
acceptance and network stability are still blocked. The failure location is not
yet proven; healthy relay-selection labels after recovery do not establish it.

## Record validation

The complete `nix develop -c just check-compact` gate passed, including the
19 isolated PostgreSQL tests and final release builds. Its initial attempt found
the fresh worktree's missing Web TypeScript dependency; the repository's
`just install` command installed the locked dependencies, after which the
complete gate passed. The active documentation diff was reviewed against the
receipts and raw observations. No Controller, Web or Machine binary deployment
was needed for these documentation changes. Stormbird's network record is
published through commit `6851608`.
