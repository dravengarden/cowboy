# Cowboy and OVH efficiency analysis

The initial production research on 2026-10-04 was read-only. No deployment,
policy change, Provider inference request, installation or worker restart occurred
in that research. The scoped implementation below has separate release acceptance.
Temporary, authenticated SSH benchmark connections were used; only their own
control sockets were closed. Existing production masters and sessions were not
closed. See the [sanitized measurements](experiments/cowboy-ovh-efficiency-2026-10-04.json).

## Main finding

The current healthy link is substantially faster than the older DERP-backed
observations. The largest repeatedly measured local delay is still Claude
account-usage preparation, not the Provider HTTP request. Task round trips,
message confirmation and network tail latency require separate optimization.
Making a timeout longer is recovery accommodation, not a speed improvement.

### Current SSH measurements

Timers run inside OVH under its real agent execution account. Each target has
three alternating cold/warm `true` samples, three normal managed-alias samples,
four established echo samples after channel startup, and four concurrent
commands on a separately owned master. These small samples do not establish a
daily p95, fault recovery, maximum bandwidth or causal attribution to one fix.

| Target alias | Cold SSH median | Owned reused SSH median | Existing managed alias median | Established echo median |
| --- | ---: | ---: | ---: | ---: |
| hawk | 1.8874 s | 0.3514 s | 0.3519 s | 0.1628 s |
| falcon | 1.9446 s | 0.3489 s | 0.3515 s | 0.1677 s |
| macbook-air | 1.9827 s | 0.4203 s | 0.3687 s | 0.1701 s |

All commands and echo markers passed. Four concurrent channels finished in
0.358/0.354/0.377 seconds respectively. All three aliases already have
`ControlMaster auto`, `ControlPersist 600`, strict host keys, disabled agent
forwarding and 15-second authenticated keepalives. Reinstalling these settings
would add no benefit. A repeated command still pays channel/process startup:
connection reuse does not make every shell invocation equal to an established
stream echo. OpenSSH documents these separate
[connection-sharing mechanisms](https://man.openbsd.org/ssh_config#ControlMaster).

Generated 128 KiB downloads took 0.845/1.936/0.955 seconds and uploads took
1.010/1.172/1.144 seconds. Startup, scheduling and transfer are combined; these
are not sustained throughput figures. No business files were transferred.

Stormbird's October 1 history measured Hawk cold SSH median 5.798 seconds,
warm median 0.831 seconds and established echo median 0.403 seconds. Its October
2 routing record measured warm markers at 0.799–0.803 seconds. Current results
are better, but intervening transport/configuration changes and different
windows prevent assigning the entire improvement to one change.

Historical Stormbird sources:
[October 1 SSH measurements](https://github.com/dravengarden/stormbird/blob/main/docs/ovh-link-optimization-2026-10-01.md),
[October 2 routing observations](https://github.com/dravengarden/stormbird/blob/main/docs/ovh-network-efficiency-2026-10-02.md),
and [current family activation and retained failures](https://github.com/dravengarden/stormbird/blob/main/docs/jms-underlay-family-deployment-2026-10-04.md).

### Account-usage preparation is still expensive

A bounded OVH Machine journal read contains 368 preparation/completion samples
between October 2 00:01 UTC and October 4 00:49 UTC. The 12,000-entry limit was
reached; it is not a complete archive. Recorded Claude usage timings are:

| Stage | Median | Observed p95 | Maximum |
| --- | ---: | ---: | ---: |
| Lifecycle queue | 0 ms | 0 ms | 31,651 ms |
| Preparation, including queue | 27,896.5 ms | 28,850 ms | 60,099 ms |
| Collector command | 1,303 ms | 1,552 ms | 9,269 ms |
| Machine total | 29,213.5 ms | 30,275 ms | 61,332 ms |

Preparation accounts for 95.23% of the summed recorded total. Its aggregate
2.867 hours are invocation wall time, including queue, not measured CPU time
or a claim that every second is redundant work. A maximum above the existing
45-second Controller observation budget is a lead for lost/late receipts; this
Machine log alone does not establish the matching Controller outcome.

The [earlier repair](releases/anthropic-usage-timeout-2026-10-01.md) correctly
removed unrelated inventory enumeration for empty sidecars. Current
`resolve_host_invocation` still calls `verified_plugin_generation`, then
`launch_context` reaches `package_for_generation` / `verified_generation` and
the same full runtime verification again. Uncached inventory can add another
verification. `installed_runtime_matches` hashes artifacts and compares archive
contents. These repeated source paths are proven; their individual contribution
to the current 28-second preparation needs stage-level profiling.

`invoke_executable_host` also retains the global lifecycle mutex across
preparation and the awaited collector command. This is a concrete serialization
boundary; the observed queue maximum is not proof of which competing operation
held it. The existing telemetry invocation contract already demonstrates
checkpointing before a network attempt without retaining this lock across HTTP.

## History and current product capabilities

Three committed real-session tool archives contain 83 records. Substring-based
classification finds 82 SSH-bearing commands/titles, 31 read-bearing records,
39 Git-bearing records and 22 repeated identical command/title entries. The
categories overlap. Repetition includes legitimate baseline/final tests and
follow-ups; it is not automatically wasted work. Tool start metadata does not
provide a complete duration or token-cost distribution.

- [Remote development](releases/ovh-hawk-ssh-development-2026-10-01.md) used
  51 actual SSH tool calls to implement and repair a fixture. Missing utilities
  and intermediate edit/test failures are distinct from network delays.
- [Trusted-peer acceptance](releases/ovh-trusted-ssh-2026-10-01.md) completed
  Codex development. Grok repeated a slow read and then failed Provider quota;
  the old quota event is not evidence of the current quota state.
- [Mobile send history](experiments/slow-mobile-send-release-2026-10-02.json)
  records local persistence at 56/87 ms but confirmation at 30.431/33.064
  seconds. Delivery/retry fixes preserve messages, but do not locate or eliminate
  the entire confirmation delay. Synthetic replay results do not replace
  current physical-device timing.
- [Matrix workspace batching](releases/matrix-workspaces-2026-10-01.md) already
  exists. Do not build another SSH wrapper or copy Stormbird policy into it.
- [Native execution](execution-environments.md) already keeps the Agent and
  Provider connection on OVH while target Machines own tools/files/processes.
  Codex and Claude have published implementations. Model-authored SSH remains
  a useful compatibility path, especially for explicit host administration.
  Production model task parity and native remote macOS acceptance must be
  measured rather than inferred from scripted integration tests.

An authenticated private `cowboy operator` read found all four Machines online
and schedulable, no draining, OVH 4/8 active sessions, Hawk 8/128 and default
runtime `ovh`. Installed OVH releases are Codex 3.3.1, Claude Code 3.4.3 and
Grok 3.1.26. Inventory is not proof that old workers adopted new generations.
The optional product ACP session-list attempts returned errors after about
30 seconds; no session was loaded, resumed, prompted or reauthenticated. The
conversation analysis therefore uses committed tool histories, not a claimed
complete live account history export.

### Memory recall and prompt-input admission

The shared Matrix client `begin` persists evidence, then awaits `observe` and
`context` HTTP calls in sequence. Each request has a two-second timeout; the
context request has an explicit budget. Both native Provider adapters await this
work before forwarding the new turn. Local Matrix placement avoids a required
cross-host recall hop, but does not make disk work or service calls free.

This is another concrete critical path to instrument: recall duration, returned
context size, prompt tokens and input admission delay. No current representative
latency or token-saving measurement was obtained. Slow successful observation
followed by slow context retrieval can consume both request budgets. The input
loop's awaited work also deserves cancellation/steering admission tests.
Evaluate a single ordered begin/recall operation only if measurements justify
it; preserve evidence receipts, project/executor binding, durable outbox and
explicit unavailable-memory behavior. Do not silently re-enable native memory
or parallelize dependent evidence/context operations to make a benchmark fast.

## Optimization order and ownership

| Priority | Work | Owner and verification |
| --- | --- | --- |
| First | Profile and remove repeated verification within one host request; reduce lock occupancy | Cowboy Machine. Retain signature/digest, executable integrity, exact generation/auth/installation fences and mutation detection. Verify once against a safely retained generation rather than trusting a mutable inventory cache. Test replacement/tampering/revocation and profile the same account query before/after. |
| First | Correlate local durable save, send, Controller admission/persistence/receipt, runtime delivery and first model output | Existing Cowboy observability. Use operation identity and process-local monotonic durations. Separate browser↔Controller, Controller↔Machine and Provider waits. Record actual slow sends before selecting a fix; raising the deadline is insufficient. |
| Next | Reduce remote task round trips using existing native execution and Matrix batching | Cowboy Provider/executor and Matrix. Compare equal disposable feature/bug tasks with actual model inference, same model/effort, fixed holdouts, tool count, wall time and tokens. Batch related reads/tests near source; retain per-step exits and bound output. Never replay an uncertain mutation. |
| Next | Measure per-turn Matrix recall/context cost and urgent input admission | Shared Matrix client and signed Provider adapters. Test slow/unavailable recall, cancellation and steering without losing evidence or crossing project bindings; compare context utility and tokens on fixed tasks before changing budgets. |
| Next | Improve proxy-path tail latency and reduce transient DERP fallback | Stormbird. Investigate the recorded Mac IPv4 response timeout and TLS delay with per-peer/leaf failure and boundary evidence. Retain approved leaf subset, sticky healthy backup, fail closed, domestic native paths and OVH public direct traffic. |
| After measurement | Bound contention between interactive operations, background collectors and bulk transfer | Cowboy executor scheduling; namespaced host SSH settings only where needed. Current four-channel SSH passes do not qualify eight-agent saturation or shared-master bulk traffic. Measure mixed traffic before separating lanes or changing limits. |
| Continuous | Reuse exact gates/artifacts and release only the changed component | Owning repository. No binary rebuild for observations or config-only edits; checked Stormbird transactions and independent cohorts remain required. Retain failed evidence and old worker generations. |

Request-local verification reuse must not become an unqualified cross-request
mtime cache. Moving synchronous validation to bounded blocking work and releasing
locks require explicit cancellation and install/uninstall/auth concurrency
contracts. Auto and manual usage refresh already share cooldown/last-good
behavior; preserve it rather than adding a second refresher.

The persistent Cowboy WSS and SSH masters already avoid repeated TLS/key
exchange. Keep them. Parallelize independent bounded reads, serialize dependent
mutations, and retain a remote process/task handle for long commands. A benchmark
initially failed because a Bash loop was sent to the target's default shell;
explicit `bash -c` fixed the harness. Interpreter selection and structured
arguments avoid avoidable retry turns; they do not justify changing users'
login shells or retrying a write whose outcome is unknown.

## Network constraints and diagnostic cost

Proxy-backed peer transport removes routine DERP business forwarding but the
current approved VLESS outer leg remains TCP/TLS. Datagram capability does not
make that leg native UDP. Head-of-line effects remain a hypothesis to measure
under loss and mixed load, not a reason to enable unsupported multiplexing,
change MTU/congestion control globally, add a relay or bypass JMS.

Current cross-border outer IPv4-only policy retains inner IPv4/IPv6. Domestic
traffic remains subject to its native policy; OVH Provider traffic remains
direct. Agents need logical aliases and target workspaces, not physical
addresses, country rules or proxy configuration. Stormbird alone owns routing.

Extend existing local counters and Cowboy bounded telemetry instead of building
another logging pipeline. Proposed detailed diagnostic retention is three days,
aggregates seven days; actual retention activation is a separate checked
operation, and this research does not assert it is already deployed everywhere.
Do not export packets, tokens, authored text, command output or every tool event
to Cloudflare. Use finite metric dimensions; session/operation correlation IDs
belong in bounded local traces, not unbounded metric labels. Keep security and
deployment audit retention independent.

## Next acceptance

Start with the scoped account-usage optimization and timing instrumentation,
then compare the same real cross-host development task through native execution
and existing SSH compatibility. Preserve model/effort and frozen tests; report
wall time, tool calls, input/output tokens, preparation/queue time, first output,
target execution time and reconnect behavior separately. No improvement target
is retroactively selected from these preliminary measurements.

Still open: peak/OOM and saturated concurrency, actual physical iPhone send and
network regression, native remote macOS acceptance, new production faults/cold
starts, and the failed four-host 30-minute stability window. Healthy production
remains retained under the operator's prior release decision.

## Scoped optimization implementation

The implementation removes repeated verification within one executable-host
request. Admission first checks the actual active link and current sealed auth
generation. It then verifies the signed Plugin, runtime artifacts, Provider
projection and host bundle once, reconstructs uncached inventory from those
request-local verified bytes, and prepares the launch from the same package.
There is no cross-request executable, digest, timestamp or inventory trust cache.
Ordinary session launches and every subsequent host request still perform full
verification. Installation journal authority is never supplied by artifact cache
metadata. Resolution logs separate verification, inventory and launch timings.

Journaled read-only collection/activity release the lifecycle lock only after
successful child spawn. Legacy slots without an installation incarnation retain
their original serialization, including same-bytes uninstall/reinstallation.
Result admission reacquires the lock and checks the actual active
link, installation incarnation, operation fence and authentication generation.
A same-artifact reinstall or auth rotation rejects the old result as an already
started observation; it does not rerun the command. Reset remains serialized
through completion. Process-group cleanup and command bounds are unchanged.
Completion-fence timing is separate from collector time.

Regression fixtures check one full verification for an uncached request, legacy
observation serialization, journaled read-only collection concurrent with
lifecycle work, serialized reset, auth
rotation, same-bytes reinstall/removal, failed spawn, timeout and retained
tamper rejection. These are hermetic fixtures, not production fault injection.

Matrix memory-client 0.1.1 reserves the current-user outbox path before atomic
publication, so its own background flush cannot duplicate the foreground
observation. A successful observation receipt retires only that file even if
context retrieval subsequently fails. Missing receipts retain durable retry;
completed-turn learning remains a separate observation. Other processes may
still deliver idempotently, and observe precedes context as before. No evidence,
binding, context budget, unavailable-memory behavior or native-memory ownership
is weakened. Claude 3.4.5 and Codex 3.3.2 carry that shared change; private CLI,
adapter and execution pins stay unchanged. Component release 3.40.0 appends
history and advances only this affected closure.

Fresh OVH context-only reads used the existing private configuration internally,
without exporting tokens or context text: Claude samples 6.1/2.5/2.1 ms and
Codex samples 2.8/2.5/2.2 ms, all HTTP 200 with 282-byte responses. This small
read-only sample does not measure a complete turn or memory usefulness. It
does not justify another API, parallelizing dependent observe/context calls,
changing timeouts or attributing historical 30-second sends to Matrix.

Fresh pre-change collection logs show 30.708/29.154 seconds preparation and
1.454/1.383 seconds collector time, with zero queue time. One authenticated CLI
refresh took 45.758 seconds and returned available Anthropic usage. These
point-in-time samples and the historical 27.8965-second preparation median are
distinct baselines. Production activation and before/after acceptance must be
recorded separately; source and fixture success alone are not deployment.

Message delivery already has Controller dispatch/queue/delivery and Machine/
worker queue/prompt/first-output correlation. This work does not change durable
message IDs, acknowledgement semantics or resend policy. Current physical-client
timing and complete-turn traces remain necessary to isolate confirmation delay;
neither a faster usage query nor a larger confirmation deadline proves it fixed.

## Production acceptance

The [sanitized October 4 activation and measurements](experiments/cowboy-ovh-usage-efficiency-2026-10-04.json)
record the deployed host `6397a9c9`, integrated into published main `22c6c47a`.
The OVH override and finite Ubuntu maintenance belong to Columbus `947a0a2c`.
New main also contains another task's portable-launcher changes; this host
receipt does not claim those later changes were activated on OVH.

OVH previously selected worker `2801f50d44994e96b2b4`, while the narrow host
output retains the independently accepted `406471a2` worker bundle. The release
therefore used explicit worker maintenance and selected its declared
`worker-6ede7a91cc8b8b3402d4`; it did not build workers from this optimization.
The independent four-minute rollback timer was armed before activation and
disarmed after acceptance. All seven original workers retained their exact
PID/start/executable/session/workspace/Provider identities at acceptance and
after both Plugin installations; no worker handoff was needed. Normal
`KillMode=control-group` was restored, the Controller reports the declared
generation, and the host has no automatic restarts.

| Measurement | Fresh pre-change sample | Post-acceptance samples |
| --- | ---: | ---: |
| Preparation | 29.327 s, n=1 | median 10.1445 s, n=6 |
| Collector | 1.303 s, n=1 | median 1.460 s, n=6 |
| Logged Machine total | 30.630 s, n=1 | median 11.587 s, n=6 |
| Fresh authenticated CLI refresh | 30.930 s, n=1 | median 11.839 s, n=3 |

Preparation decreased about 65% against the fresh comparison; CLI wall time
decreased about 62%. The older 368-sample preparation median remains 27.8965
seconds, a different window. Two 14–15 ms cached CLI reads are retained but
excluded from refresh latency. The post-acceptance stage selection includes
all successful recorded Claude collections on the new PID, including the new
Plugin; it is not a p95, a zero-failure window or model-response timing.
The old total is its command-completion log; the new total includes the result
fence. Verification accounts for almost all remaining preparation; inventory and
launch take 0–2 ms. Completion fences were 0 ms in this small sample.

Claude 3.4.5 (`acebb015…`) and Codex 3.3.2 (`a833d5b6…`) were independently
verified, published and installed only on OVH through official durable IDs.
Both ended completed with exact Applied receipts and current credential
materialization. Both initial observers returned HTTP 409 after roughly
100–103 seconds; their same-ID reconciliation completed without resending an
installation. Codex's first reconciliation returned HTTP 503 during a transient
control reconnect, whose root cause this task does not establish. These failed
observations remain evidence rather than successful timing samples.

Exact Linux/macOS probes, old/new Provider coexistence and packaged native
execution passed (Claude 30 checks, Codex 15). Actual active, next-transaction
recovery, additional historical recovery and cold Catalog readers were covered;
real first-party signature verification is separate from the temporary signing
key used by reader fixtures. The full integrated gate passed, including 1,791
all-feature and 483 standalone Machine tests, with `RUST_TEST_THREADS=1`.
Earlier parallel fixture EAGAIN/ETXTBSY failures are retained; their production
cause is not inferred. Serial test scheduling does not disable the fixtures'
internal concurrency checks.

Post-install cgroup observation showed about 188 MB anonymous memory and
8.75 GB reclaimable file cache, with zero OOM or OOM kills. Cgroup current/peak
counts are not host-only RSS. Signed artifacts, installed runtime state,
permanent identities and previous recovery roots are retained. Disposable
probe homes, transfer archives and raw private API observations are cleanup
targets; they are not recovery sources.

Before acceptance the independent timer could restore the exact previous
override. Accepted maintenance is immutable; a later rollback requires a fresh
reviewed transaction, not deletion of `committed.json`. Prefer a descendant
host revert retaining the accepted worker floor, and exact signed previous
Plugin releases through Cowboy's installer. Never edit live installation
pointers, credentials, or a permanent Machine identity to undo this change.

To reproduce the usage comparison, use the authenticated
`cowboy operator usage --refresh anthropic` command with stdout captured into
a protected temporary file; retain only HTTP status, Provider status, observation
timestamp and elapsed wall time. A cached observation older than invocation
start is not a fresh-refresh sample. Read the active OVH Machine PID from its
user unit and select only `CollectUsage` resolution/preparation/command/
observation records from that PID's journal. Separate queue time and result
fence from verification, inventory, launch and collector time. Retain unsuccessful
requests and the exact source/generation alongside successful timing samples;
never export account fields, credentials, authored prompts or raw logs.
