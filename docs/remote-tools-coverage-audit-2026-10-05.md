# Remote tools coverage audit

Research scope: complete Claude/Codex remote execution semantics, not only
hooks. Inspected source: `aad251abecd26f2cffc1b05d2382b93d31cb084b`.
The locked native versions are Claude Code 2.1.287 and Codex 0.159.3.
This is a gap analysis, candidate fixes and proposed acceptance contract, not
a production readiness declaration. No production capability was disabled by this audit.
Try native extension surfaces and reliable adapters before declaring a feature
unavailable. Missing tests do not establish impossibility.

## Evidence and limits

- [Existing native acceptance receipt](acceptance-results/matrix-codeact-2026-10-05.json)
  records 25 Codex and 36 Claude checks. These used scripted model endpoints;
  cross-host network and real subscription inference are explicitly untested.
- Newly rerun: `nix develop -c node --import
  ./tools/register-memory-client.mjs --test
  plugins/claude-code/runtime/tools.test.mjs
  plugins/claude-code/runtime/launch.test.mjs`: 25 passed, zero failed.
  This validates those unit cases, not arbitrary native tool parity.
- A native Claude experiment used a fresh private home, loopback scripted API,
  no production credentials and a network namespace with only loopback.
  Binary SHA-256:
  `3920489a5109cff5786a1a392c25277408ff22bc796d5edb9c16a60e5a1718f0`.
  Native Bash wrote a marker but triggered zero `process.run/spawn` Mod events.
  Positive control called both Mod APIs: each counter became one. These events
  cannot by themselves redirect native Bash on this tested version/path.
- A separate native experiment confirmed `CLAUDE_CODE_SHELL_PREFIX` intercepts
  Bash. Its input includes the runtime shell snapshot and a runtime cwd-result
  file. A passthrough wrapper executed successfully. This establishes an entry
  point, not remote correctness or permission/cancellation parity. A second run
  used `run_in_background:true`: after the initial prompt returned, native
  `system/task_notification` reported the same command completed with exit code
  zero and a native task output file. The scripted endpoint received only two
  requests. This proves task registration/completion delivery with the local
  passthrough wrapper, not autonomous model continuation or remote recovery.
  [Compact experiment receipt](experiments/remote-tools-audit-2026-10-05.json).

## Coverage matrix

### Candidate output fixes

Follow-up implementation on `cae2d52f` prepares Claude Plugin 3.4.8 without
changing the pinned native CLI, ACP or Node versions. `tools.mjs` previously
concatenated stdout and stderr bytes and decoded each tool result independently.
An unfinished UTF-8 character could be corrupted by another stream or by a
result boundary/cold resume. The candidate decodes streams separately and
persists at most three unfinished bytes per stream with the output cursor.
Final incomplete bytes retain replacement-character behavior; BOMs and invalid
complete byte sequences are covered by regression tests.

Output cursor/decoder state is now committed together through `save(update)`.
The collection works on a private job snapshot; failed transport reads leave
the previous cursor untouched, and a failed atomic state replacement rolls back
the in-memory update. A regression test causes a real rename failure, restores
storage and verifies the same retained bytes are still returned on retry.
This fixes pre-delivery state-storage failure, not the separate crash window
after a successful durable commit but before native tool-result delivery.

The two original UTF-8 regressions failed before the fix and passed afterward.
The owning `just claude-remote-check` gate passed 58 tests plus formatting and
type checks. Native Codex review completed with no concrete new findings after
using the host's established review authentication directory; the initial
default-auth attempt returned 401 and produced no review.

An additional deterministic regression caught cancellation during a pending
start acknowledgement: the original implementation issued no terminate request,
because it registered foreground ownership only after `process/start` returned.
The candidate registers the pending start and cancels its acknowledged identity.
Already-running jobs are terminated without waiting for pending starts; explicit
background jobs retain their separate lifetime. Failed/unknown starts are not
resubmitted. This does not establish cancellation after keeper loss or every
native queued-tool race; those remain separate acceptance cases.

Claude 3.4.8 was subsequently published and activated on OVH; the exact
[release and remaining limits](releases/remote-parity-2026-10-05.md) are recorded
separately from this research. The pre-existing app-shell digest mismatch was
repaired by an app-shell metadata bump to 1.1.20 and a new append-only component
release 3.41.0, without changing app-shell functionality or prior registry
history. Three formatting-only changes repair the existing keyboard acceptance
script's gate failure. The first packaged candidate passed 27 native checks;
adding real split stdout/stderr UTF-8 writes passed 28 checks. The rebuilt
candidate including cancellation passed those 28 native checks again. Native
review of the final fixes found no concrete new defect. These isolated fixtures
and the source gate do not constitute a production activation receipt.
The [candidate evidence](experiments/remote-parity-candidate-2026-10-05.json)
binds source hashes and exact artifact digests to the native execution checks,
Linux/macOS probes and old/new generation coexistence with descendant drain.

“Recorded” means the linked receipt covers the named behavior, not that the
whole row is universally supported. “Candidate” requires implementation and
acceptance. Current restrictions below are observations, not recommendations
to introduce more restrictions.

| Surface | Current evidence / gap | Reliable direction and required proof |
| --- | --- | --- |
| Basic commands and edits | Both recorded against target, runtime files unchanged | Keep native environment binding and exact target identity; test every exported tool schema |
| Native CodeAct | Native nested shell, patch, image, parallel/error results, yield/wait and cold resume now have pinned scripted evidence | Keep these in default native acceptance; separately test child agents and runtime failure while a cell is pending |
| Claude tool dispatch | `context-mod.js` replaces native tool bodies with a facade | Restore native ownership where a lower-level boundary exists; retain independently tested file adapters |
| Native Bash lifecycle | Facade retains processes but does not establish native background task registration | Explore shell prefix bridge preserving original Bash tool and native task registry |
| Background completion | Native activity UI support exists; facade handles are a separate system | Validate completion after prompt return, native autonomous continuation, output retrieval, task count and cancellation |
| PTY and stdin | Claude facade explicitly uses `tty:false`, `pipeStdin:false` | Native-parity baseline first: do not invent PTY support where provider lacks it; test Codex PTY, resize, EOF and incremental input |
| Shell environment | Claude starts a new shell with fixed binding cwd and target environment | Test `cd`, exports, login startup, shell snapshots, quoting, signals, pipe status and shell availability; preserve documented native persistence semantics |
| Project hooks | Claude launch suppresses settings sources; Codex remote hook placement not established by current receipt | Execute target-owned hooks at target, preserve native lifecycle/decisions and trusted configuration; separate runtime-owned hooks |
| Hook types | Command, HTTP, prompt/agent and MCP forms have different ownership and provider support | Inventory exact installed schemas; keep native model evaluators and approval semantics, bridge only external execution/IO |
| Permission modes | Claude bound launch selects `bypassPermissions`; remote tools must not be presented as supporting every native permission mode | Design explicit mapping for approval, deny, modified input and concurrent approval cancellation; test denial before any target effect |
| Native agents | Codex fresh and fully forked children inherit target guidance and route direct/CodeAct commands through the keeper in pinned native acceptance; Claude Agent/Task paths remain restricted | Separately test grandchildren, live-child resume, cancellation, background, teammate and worktree paths |
| Skills and project plugins | Claude Skill restricted; implicit local discovery is not target-aware | Target-authoritative discovery with versioned metadata, trust and native expansion; route script execution separately |
| MCP and web/browser tools | Claude bound allowlist chiefly admits Matrix plus owned tools | Classify runtime/service/target placement per server/tool; preserve native discovery/auth/elicitation, without moving all MCP servers to target |
| Plans and task artifacts | Plan tools restricted in Claude lane | Separate runtime transcript from target plan/artifact storage; preserve native approval and resume semantics |
| Images, notebooks, PDFs | Images/notebooks have evidence; Claude describes PDF via target utility | Add native-parity PDF/pages, binary limits, image dimensions, output artifacts and user-upload placement tests |
| File semantics | Stale edits, CRLF, Unicode, quoted paths, ranges tested | Add symlink races, rename/unlink races, case sensitivity, modes, hard links, nonregular files, encoding and concurrent writers |
| Atomic mutations | Stale stamp refusal has evidence; it does not alone establish compare-and-write atomicity | Inspect target implementation and inject mutation between check and write; use target-side atomic primitives where promised |
| Output limits | Large streams/backpressure and terminal events recorded | Test split UTF-8, binary/NUL, truncation markers, slow/absent reader, disk full and retained output expiration |
| Lost replies | Lost start and outages recorded without replay | Distinguish rejected, accepted, unknown and completed effects; never resend an unknown mutation under a new identity |
| Cancel and timeout | Foreground cancellation and retained job cancellation recorded | Test cancel/start races, whole process trees, detached children, late completion, keeper death and provider timeout semantics |
| Resume/compaction | Rebinding and Claude target context recorded | Test resume with live children, queued completion, changed plugin version, stale approvals and artifact locators |
| Reconnect and restarts | Keeper reattachment recorded | Inject Controller, worker, keeper and native-runtime failures independently; fence old generations and late replies |
| Deletion and shutdown | Idempotent close and no recreation recorded | Verify pending callbacks cannot resurrect sessions, issue new model turns or affect another session |
| Authorization and paths | Binding and foreign task rejection tested | Keep filesystem authority at target, distinguish relative/absolute/runtime artifacts, test credential and socket leakage |
| Upgrade/platform | Evidence tied to pinned builds and Linux paths | Capability/version negotiation; preserve active generation; test different shells, OS and target utilities before claiming portability |
| Observability/cost | Local telemetry exists; full causal coverage not established | Correlate session/binding/operation/native tool/child IDs; distinguish lost telemetry from success; no model polling for status |

## Implementation candidates

### Native CodeAct acceptance follow-up

The default Codex worker conformance now exercises native `functions.exec` and
`functions.wait`, independently of Matrix MCP. Nested patch and parallel shell
calls preserve target placement, stderr and exit code 37. Image bytes reach the
scripted model input. A yielded cell completes through `wait` with one target
effect, and a cold native resume reads the same target state without replay.
The relay observes the nested shell and image requests crossing the actual
worker/keeper transport, in addition to checking unchanged runtime files.
Review caught an image assertion shared by both paths: CodeAct success could
mask a failed direct image call. Each call now needs its own image result and
distinct relay pathname. A negative control with a missing direct image was
rejected by the direct-image assertion; the restored fixture passes.
Exact hashes and limitations are in the
[native CodeAct receipt](experiments/codex-native-codeact-2026-10-05.json).

This also repaired two acceptance-fixture defects: its copied native package
omitted `codex-resources/bwrap`, and namespace setup left capabilities that the
real sandbox helper rejects. The fixture now preserves the pinned helper and
drops setup capabilities before executing native tests. The shipped package
already included that helper. The cold-resume check now requires returned
target bytes, rather than only unchanged mutation markers. These are test
corrections, not a new runtime release or a weakened sandbox policy.

The fixture remains same-host with separate runtime/target directories and a
real transport. It does not prove cross-host latency behavior, arbitrary nested
agents, or survival of a pending V8 cell after native-runtime death.

### Native child inheritance follow-up

The default Codex conformance now asks the native runtime to spawn two children:
`fork_turns=none` executes a direct command and `fork_turns=all` executes a
CodeAct command. Both must load target guidance, return the target cwd, produce
exactly one target-side effect and leave runtime files unchanged. Independent
relay observations require both child commands to cross the worker/keeper
transport; shared host paths alone cannot satisfy acceptance. See the
[child inheritance evidence](experiments/codex-native-children-2026-10-05.json).
The exact packaged ACP entry point also creates a child, waits for it, and
executes a delayed parent command. The parent effect must exist when
`session/prompt` returns, so an unrelated child completion cannot satisfy the
parent prompt. If that command returns a running process handle, the fixture
polls the same handle to a successful exit before sending the final answer;
it does not assume that a fixed sleep guarantees process completion.
Its child command independently crosses the target transport. The combined
native receipt has 32 checks and zero real-model requests.

This uncovered a fixture assumption: app-server interleaves child and parent
`turn/completed` events. Completion now matches the requested thread before
checking its turn identity. The scripted API routes each child independently
and serializes response selection, so concurrent child requests cannot consume
another thread's scripted response. Its request budget remains bounded.
An initial research probe also misclassified native `agent_message` input as
ordinary user input; that run did not exercise a child command and is not
positive execution evidence.

No production binding change or feature restriction was needed for these
tested paths. This establishes two first-level child modes for the pinned
runtime, not arbitrary native delegation, child cancellation, grandchildren,
live-child cold resume, or a cross-host acceptance result. Claude's native
agent/task integration remains a separate gap. The parent uses `never` approval
and full access; restricted-mode approval and denial behavior is not established.

### Native child interruption and scoped stop

The pinned Codex 0.159.3 has a cancellation distinction in both local and remote
execution. After positive admission of a child shell and its descendant,
`collaboration.interrupt_agent` stops a direct `exec_command` process tree.
The same call through native `functions.exec` leaves both processes alive after
12 seconds, despite the child turn reporting `interrupted`. The descendant
writes a delayed marker after the interrupt. This is a reproduced native
lifetime distinction, not evidence of an OVH-only transport defect.

The native `thread/backgroundTerminals/list` and
`thread/backgroundTerminals/terminate` APIs provide a working explicit stop:
use the owning child thread and its original process ID. Both tested process
trees then exit, while an independently admitted parent background process
retains its original PID/start-time identity. No additional model turn is
needed to invoke these management APIs. Using the parent thread with the real
child process ID returns `terminated:false` and leaves the child tree alive;
the subsequent correctly scoped request stops it. The reproducible four-case probe is
`just execution-child-stop-conformance CLI SHA256 RECEIPT`; it requires a
disposable PID/network namespace, exact binary hash and a fresh receipt.
Its CodeAct expectation deliberately records the current pin's behavior; an
upstream cancellation change requires reviewing that expectation.
The [exact receipt](experiments/codex-native-child-stop-2026-10-05.json) records
all four cases and helper hashes. A negative control fabricated a successful
termination response without issuing the stop; acceptance failed on the live
process tree, rather than trusting the response alone.

An earlier immediate-stop exploratory run observed an empty terminal list
while the remote command still lived. Empty enumeration immediately after
interruption is therefore not proof of cleanup. The committed probe observes
12 seconds before enumeration and does not establish recovery of that early
registration race. It also does not exercise the Cowboy worker/keeper relay,
restricted permissions, detached grandchildren or cross-host transport.

Inspection of the accepted Codex 3.3.2 packaged ACP adapter found an existing
`_session/async_task/stop` extension backed by these same native APIs. Its task
publication is opt-in through `jetbrains.air` capability metadata (`asyncTasks`);
Cowboy's current ACP initialization does not negotiate it. Native activity
counts are not equivalent to exposing that task-control extension. Integration
must negotiate an actual supported contract, preserve child/session ownership,
surface late task registration and reconcile state without inference. Do not
advertise an extension without consuming its updates, or make ordinary turn
interrupt silently terminate all background tasks. Packaged ACP behavior for
these cancellation cases remains unverified; no product stop behavior or
production configuration was changed in this follow-up.

### Follow-up native probes

Two research probes now live in `tools/` and require exact binary hashes,
fresh receipt paths and a network namespace containing only loopback. They use
the existing scripted native fixtures and disposable homes, not production
credentials. Both have `--help` for binary/receipt arguments.

- `claude_native_shell_probe.py` forwards native Bash's shell envelope through
  a separate Codex exec-server process. A background command wrote only in the
  target directory, and Claude emitted a native completion notification with
  retrievable output. [Receipt](experiments/claude-native-shell-research-2026-10-05.json).
  This is still a shared-filesystem, same-host experiment. The prototype starts
  a disposable executor per command, buffers bounded output and does not handle
  cancellation/reconnection. Production must instead use the bound persistent
  keeper and native stream/signal semantics. Do not ship this probe as a bridge.
- `remote_native_hooks_probe.py` discovers the four fixture-created user hooks
  through native `hooks/list` and trusts only their exact reported hashes in the
  disposable home. Its native command/patch/resume checks pass. SessionStart,
  PreToolUse, PostToolUse and Stop run with target cwd but runtime environment.
  The nearest native ancestor is app-server, not the separately launched target
  exec-server. This observation is about user command hooks in 0.159.3; it does
  not establish project-hook discovery, MCP hooks or all hook types.
  [Receipt](experiments/codex-native-hooks-research-2026-10-05.json).

The initial Codex attempts did not establish hook trust, so their absent events
were invalid negative evidence. Native hook inventory/trust must be a positive
control before evaluating remote hook placement. The final probe uses exact
hash trust rather than a broad trust bypass. The state shape is defined by
[the upstream hook configuration](https://raw.githubusercontent.com/openai/codex/main/codex-rs/config/src/hook_config.rs)
and checked against the tested binary's `hooks/list` response.

For target-dependent Codex hooks, investigate a narrow trusted command proxy
that forwards hook stdin, streams, exit code, timeout and cancellation to the
existing bound keeper. Keep runtime-owned hooks in place. A target pathname
on the runtime is not sufficient: tests must make that pathname unavailable on
the runtime to catch wrong-machine execution. Changes to wrapper definitions
must still require native hook trust; do not silently trust project content.

Example isolation wrapper (run from this checkout in its dev shell):

```sh
unshare --user --map-current-user --keep-caps --net bash -euc '
  ip link set lo up
  exec python3 tools/claude_native_shell_probe.py "$@"
' probe --claude /absolute/pinned/claude --executor /absolute/pinned/codex \
  --claude-sha256 EXACT_CLAUDE_SHA256 --executor-sha256 EXACT_CODEX_SHA256 \
  --receipt /absolute/new-receipt.json
```

Use the same isolation wrapper with `remote_native_hooks_probe.py`, whose
arguments are `--native-cli`, `--sha256` and `--receipt`.

### Claude native TaskStop with a resident keeper

The 2026-10-06 follow-up uses the same pinned Claude 2.1.287 and executor
0.159.3, but compares four process topologies. A scripted native Bash starts
a shell and descendant; the next scripted call uses the actual native task ID
with `TaskStop`. Admission and successful stop acknowledgement are observed
before checking PID/start-time identities and a delayed filesystem effect.

| Topology | TaskStop acknowledgement | Observed target processes and effect |
| --- | --- | --- |
| Native local Bash | Success | Both tracked processes stopped; no late effect |
| Prefix with a transient executor in its process group | Success | Both stopped; no late effect |
| Prefix with an independent persistent keeper, no cancellation forwarding | Success | Both still alive after six seconds; late effect occurred |
| Same persistent keeper, explicit signal-to-process/terminate forwarding | Success | Both stopped; no late effect; keeper and independent peer job survived |

This explains why the earlier transient-executor probe cannot establish
production cancellation parity: killing a local wrapper does not by itself
cancel a separately owned target job. The prototype explicitly forwards the
wrapper's signal to the original admitted process ID. Its paired unforwarded
case is a negative control, not an accepted product cancellation policy.
The forwarded case also exposes an acknowledgement gap: both target processes
were still alive when native TaskStop reported success, and stopped about one
second later in this run. Eventual signal delivery is not local-equivalent
completion acknowledgement. A production integration must retain a reliable
native-task/target-process identity and observe target termination before
reporting completed cancellation, or explicitly report that cancellation is
pending. Matching arbitrary command text is not a safe identity mechanism.
The existing shipping `WorkspaceTools` facade already sends explicit target
termination and returns a pending message while its output record is not
closed. A new deterministic regression holds that target open through the
collection deadline, verifies the pending message and retained task handle,
then observes exit through Read without starting or terminating another job.
The shell-prefix research failure is not evidence that this facade uses the
same incorrect local-wrapper cancellation path.
The [exact research receipt](experiments/claude-native-task-stop-2026-10-06.json)
binds the probe and keeper hashes. Re-run with
`just execution-claude-task-stop-conformance KEEPER CLAUDE CLAUDE_SHA EXECUTOR EXECUTOR_SHA RECEIPT`
inside the pinned dev shell; the recipe creates disposable PID/network namespaces.
No real model requests or production sessions participate.

The signal handler is intentionally installed after start acknowledgement.
Pending-start cancellation, forced SIGKILL, foreground interrupt, reconnect,
cold resume, remote paths and permission equivalence remain unproven. A
production implementation must reconcile cancellation with the original start
operation even if its acknowledgement is lost, and distinguish explicit cancel
from a connection failure. Do not enable this experimental prefix globally or
replace the existing shipped remote tool facade on these results alone.

#### Lost start acknowledgement in the shipped facade

The 2026-10-06 follow-up found a separate gap in facade 3.4.8. `start` saves the original
process ID before submitting the command, but `startForeground` only adds it
to the foreground set after a successful reply. `cancelForeground` ignores a
rejected pending start. Consequently an interrupt can return successfully even
when that rejected start was already admitted and its target process remains
live. This is not the shell-prefix signal-forwarding issue above.

The deterministic `lost start acknowledgement retains the original job for
recovery without replay` regression injects admission before reply failure.
The 3.4.9 candidate requires an explicit pending ID and a persisted cancellation
intent; reopening the state with a recovered transport automatically terminates
that same ID. It requires exactly one start, one terminate, a closed output
record and an unaffected independent peer. This is a facade simulation, not
native Claude, a real reconnect or a cross-host acceptance test.
Both this regression and the earlier pending-TaskStop regression live in
`tools/claude-remote-routing.test.mjs`, which `claude-remote-check` runs. The
earlier test was moved out of the immutable Plugin source tree after the
follow-up full gate found its unchanged-version source fingerprint mismatch;
the pinned Plugin source bytes are restored without deleting its test coverage.

The keeper has an existing durable operation observation surface. Its
`execution-keeper-conformance` separately discards a start reply, leaves all
control clients disconnected for 35 seconds, observes the original operation,
and cancels the original live process without running the command again.
The [fresh keeper receipt](experiments/claude-lost-ack-keeper-2026-10-06.json)
records all 11 checks passing with the exact executor and keeper hashes. It
explicitly does not prove enrolled transport or Provider integration.
That lower-level capability does not automatically repair the facade:
`worker_execution::Client::invoke` currently allocates the operation identity
internally, while the facade retains only the process identity. A transport
failure can prevent the facade from observing whether admission has settled.

The 3.4.9 candidate registers foreground identity before submission, persists
cancel intent before target IO, and waits independently for pending starts.
Only an observed closed target clears the intent. Missing/unknown admission,
lost replies and transport errors retain it for bounded-interval observations
or cold runtime recovery; no command is resubmitted. Native model interruption
is still forwarded, but its success response becomes an explicit pending/error
response if target cancellation is unconfirmed or intent could not be saved.
Output collection and failed state-save rollback preserve concurrent cancel
intent without consuming its output cursor. Tests cover missing admission,
delayed exit, failed intent persistence and concurrent output commits/rollback.
This does not establish arbitrary process-tree, SIGKILL, cross-host filesystem
or permission parity. Cancellation after total keeper/incarnation loss cannot
claim that the old process stopped. Release/activation requires the exact
packaged native acceptance in addition to these source tests.

The official [shell-prefix contract](https://code.claude.com/docs/en/env-vars),
checked 2026-10-06, also includes hook, status-line and stdio MCP shell commands;
the Bash argument contains the full native shell setup. These have different
placement and credential requirements, so a Bash-only success cannot accept
blanket shell forwarding. Native task completion/output, cancellation, runtime
bookkeeping and execution placement need separate cross-host evidence.

### 1. Preserve native Bash through an execution bridge

The official [environment-variable reference](https://code.claude.com/docs/en/env-vars)
documents a shell prefix wrapping spawned commands. The native experiment
confirms it is reachable without replacing Bash's tool body. This is a stronger
candidate for background parity than reconstructing task notifications after
the facade has bypassed native task registration.

The wrapper must act as a process proxy over the existing authenticated
execution connection, with a stable operation identity and streamed output.
It must preserve exit/signal behavior and reconnect to the same target process.
Do not start a new remote command when an acknowledgement is lost.

The native shell envelope currently includes runtime-owned snapshot and cwd
files. Do not use string substitutions on arbitrary command text or ship the
runtime environment wholesale. Prototype a supported way to provide target
shell state while keeping native bookkeeping on the runtime. Test prefix
invocations from hooks, status commands and MCP separately: blanket forwarding
would place runtime-owned services and credentials on the wrong machine.
If this boundary cannot reliably separate ownership, investigate a supported
upstream execution backend before falling back to a narrower adapter.

### 2. Project configuration and implicit reads

Build an explicit target project context interface for configuration, guidance,
skills, agents, hooks, plans and artifacts. Native discovery/permission owners
consume target-origin content with identity and version information. A cache
is not a second source of truth: changes, deletion, symlinks and resume must
invalidate it correctly. Do not enable implicit native access merely because
the explicit Read/Bash tools are remote.

Commands used by project hooks execute beside the project. Native hook ordering,
matchers, timeouts and outputs remain provider-owned. Runtime authentication,
history and service clients remain on the runtime. HTTP hooks need an explicit
network/credential placement decision. Prompt/agent hooks retain their native
model evaluation, inheriting target binding for any nested tools.

### 3. Native agents and orchestration

Codex already selects the execution environment at thread start and every turn.
Determine whether native child creation inherits this selection; prove it with
distinct runtime/target markers. If it does not, use the narrowest native child
creation extension. Claude's Mod `agent.spawn` is a candidate for binding and
context propagation, but child tool callbacks and implicit IO must be tested.
Never assume a parent interceptor automatically covers descendants.

Keep native tool orchestration as native. Matrix CodeAct is a separately scoped
MCP capability, not a replacement for a general native tools runtime. A
multi-tool code block is not a transaction: post-tool rejection cannot undo
already completed mutations.

### 4. Completion and recovery

Prefer native task ownership and native completion delivery. A provider adapter
fallback needs a durable completion ledger keyed by session, binding generation
and operation, plus acknowledged consumption. Deduplicate transport delivery;
do not promise exactly-once model consumption across crashes without a native
acknowledgement/idempotency contract. A Mod-origin prompt is an explicit new
model turn, not equivalent to a native background completion event.

Restore UI task state without an inference request. Completion-driven model
continuation may cost tokens; idle polling, reconnects and state reconciliation
must not create model turns. Closed/cancelled/deleted sessions must not wake.

## Acceptance strategy

Use a native local baseline and a remote candidate with the same pinned runtime,
scripted tool sequence and fixtures. Compare observable semantics and actual
filesystem/process effects, not only tool-result prose. Run the same matrix
through direct calls, CodeAct and each enabled child-agent path.

For every mutation, inject failure before admission, after admission, after the
effect and before delivery. Observe both machines and reconcile by operation
identity. Cross product these boundaries with cancellation, concurrent tools,
resume, compaction and generation changes. Include negative controls: disabling
target binding must fail the location assertion; disabling completion dedup
must fail duplicate-delivery checks. Wait for positive evidence of admission or
resource ownership before asserting cleanup.

Use three evidence levels: deterministic adapter tests, isolated real-native
scripted tests, then a small authenticated cross-host canary on exact release
bytes. Existing checks remain valuable but cannot be promoted to full fleet
coverage. Report every unsupported or untested path explicitly. Prototype
success does not authorize enabling a production capability before its safety
and compatibility cases pass.

## Source map

- `plugins/claude-code/runtime/{launch.mjs,context-mod.js,tools.mjs}`:
  settings/tool admission, native interception and file/process facade.
- `components/provider-runtime/packages/codex-acp/launch.mjs`: native binding.
- `src/worker_execution/`, `src/execution_host/`: execution transport and keeper.
- [Native activity projection](claude-autonomous-activity.md): existing UI
  support for native autonomous activity; not registration of facade jobs.
- [Execution contract](execution-environments.md): ownership and version limits.
- [Claude Mod reference](https://code.claude.com/docs/en/plugins/mods/reference):
  candidate native extension contracts; verify against installed generated types.
- [Codex hooks](https://learn.chatgpt.com/docs/hooks): native lifecycle semantics;
  current public documentation is not an acceptance receipt for pinned remote
  binaries.
