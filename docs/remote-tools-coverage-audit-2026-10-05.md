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

### Disposition of every matrix row (Claude lane, 2026-10-08)

Status after Plugin 3.19.0. "Matches native" means a native-local
measurement and the remote result agree; "intentional" names a gap kept on
purpose with its fallback; "open" names remaining work and its owner.

| Surface | Disposition | Evidence or reason |
| --- | --- | --- |
| Basic commands and edits | Matches native | 3.8.0 (36 Bash cases), 3.11.0 (24 file cases) |
| Native CodeAct | Codex lane; not re-audited here | Pinned Codex receipts above |
| Claude tool dispatch | Intentional: target tools are answered through Mods `tool.call`, the narrowest native interface 2.1.287 offers; native owns everything else, descriptions included since 3.18.0 | No lower boundary exists in the pinned CLI |
| Native Bash lifecycle, background completion | Matches native | 3.9.0, 3.12.0, 3.18.0, 3.19.0 |
| PTY and stdin | Matches native | Measured 2026-10-08: native and remote Bash both run with stdin `/dev/null`, no terminal on stdin or stdout, `read` fails at once |
| Shell environment | Matches native within the executor's environment; open: the Machine's closed executor environment (7 variables on Hawk against about 99 for a local session) | Machine design; aligning it is a Machine release and configuration change awaiting the user's decision |
| Project hooks, hook types | Matches native for command hooks (3.7.0); intentional: non-command facade tool hooks refuse the matching call rather than skip the project's review | 3.7.0 receipt |
| Permission modes | Matches native for mode decisions and asks (3.6.0); intentional: plan mode refused, no persisted "always allow" rules | Plan files and rule storage live in the runtime home |
| Native agents | Matches native for background agents (3.5.0, 3.19.0); intentional: foreground, nested, teammate and isolated agents refused with a message | 3.5.0 receipt |
| Skills and project plugins | Matches native for target skills and commands (3.16.0); intentional: marketplace plugin skills, hook-declaring skills and six bundled skills refused with their reason | 3.16.0 receipt |
| MCP and web tools | Matches native for scopes, precedence and placement (3.17.0); intentional: target-loopback, headers-helper and unresolved-variable remote servers omitted | 3.17.0 receipt |
| Plans and task artifacts | Intentional: plan mode refused; native task-list tools run where the session runs (3.15.0) | Plan files would be runtime files |
| Images, notebooks, PDFs | Matches native (3.13.0, 3.14.0); intentional size bounds stated there | 32 cases |
| File semantics, atomic mutations | Matches native's check-then-write (native also compares the read state and then writes); intentional: files rewritten in place | 3.11.0; aliased-edit follow-up |
| Output limits | Matches native for persisted output and split UTF-8; not tested: target disk full, retained-output expiry | Executor-owned behavior |
| Lost replies | Fail closed without replay | Packaged lost start and lost write-reply checks |
| Cancel and timeout | Matches native for foreground, background, deadline and tree stops; pending-start cancellation tested (3.4.8). Keeper or executor loss fails closed (Machine policy: expose loss, retain the worktree, no replay); the user accepts that the session reports the loss without matching native, which has no such failure (2026-10-08) | Machine policy and user decision |
| Resume and compaction | Covered: agent outcomes across resume, runtime file attachments dropped after compaction, abandoned asks withdrawn; idle sessions move to a new Plugin version on the Controller's schedule | 3.5.0, 3.6.0, 3.10.0 |
| Reconnect and restarts | Covered at the Plugin boundary: lost start receipt, 35-second outage, Machine restart reattach, restarted launcher reconnect (3.17.0); Controller and worker fault injection is Machine/Controller scope | Packaged acceptance |
| Deletion and shutdown | Covered: idempotent close, no recreation; target processes and MCP servers end with the session | Packaged acceptance, 3.17.0 |
| Authorization and paths | Covered: every packaged turn checks that runtime paths and runtime guidance never reach the model | `context_checked` in acceptance |
| Upgrade and platform | Covered per release: coexistence with the previous generation, Linux and actual macOS runtime probes; targets measured are Linux with bash | Release receipts |
| Observability and cost | Open (owner: Cowboy telemetry): causal correlation of session, binding, operation, native tool and agent ids is not established | Not part of the Plugin releases |

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
| Background completion | Claude 3.9.0: a native background task (runtime waiter) stands for each target command left running, so native delivers its completion into a running turn or as an idle turn of its own; TaskStop sends nothing; notifications are rewritten to the target handle. 3.18.0: native's background deadline (requested timeout, else 30 minutes; 30 minutes for a moved command) stops the target command too. 3.19.0: a background agent's background commands notify that agent, resuming it if it ended (packaged acceptance) | Notifications do not survive a native process restart, as natively |
| PTY and stdin | Claude facade explicitly uses `tty:false`, `pipeStdin:false` | Native-parity baseline first: do not invent PTY support where provider lacks it; test Codex PTY, resize, EOF and incremental input |
| Shell environment | Claude 3.8.0 runs native's command shape on the target: user bash/zsh, a login-shell snapshot (rc, functions, options, aliases, PATH), `cd` persistence with native's reset, native's environment variables; 36 Bash cases match native-local results in packaged acceptance | Error results keep Mods' `<tool_use_error>` wrapper; `CLAUDE_EFFORT` starts after the first tool batch; no embedded find/grep/rg shadows or `CLAUDE_PID` |
| Project hooks (Claude) | 3.7.0 runs target project hooks: native lifecycle/native-tool hooks through the shell prefix, facade tool hooks (PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest) through the adapter | Settings are a session-start snapshot; non-command facade tool hooks refuse matching calls |
| Project hooks | Claude launch suppresses settings sources; Codex remote hook placement not established by current receipt | Execute target-owned hooks at target, preserve native lifecycle/decisions and trusted configuration; separate runtime-owned hooks |
| Hook types | Command, HTTP, prompt/agent and MCP forms have different ownership and provider support | Inventory exact installed schemas; keep native model evaluators and approval semantics, bridge only external execution/IO |
| Permission modes | Claude 3.6.0 drops the forced bypass: native `$.tool.check` decides each target call under the session's mode and rules, asks reach the SDK host in native `can_use_tool` shape, denial precedes any target effect, amended input runs, dontAsk denies, and abandoned asks are withdrawn (packaged acceptance) | No "always allow" rule persistence, auto-mode classifier or command-string path mapping; plan mode stays refused |
| Native agents | Codex fresh and fully forked children inherit target guidance and route direct/CodeAct commands through the keeper in pinned native acceptance. Claude 3.5.0 admits native background subagents: child calls carry `agentId`, launch/notification/client locators use `cowboy-agent://`, outcomes are durable across resume, TaskStop/interrupt cancel the child's target commands (packaged worker acceptance) | Partial output stream, permission modes, grandchildren, teammates, worktree/remote isolation, custom agents and native-runtime crash with a live agent remain unaccepted; those inputs are refused |
| Skills and project plugins | Claude 3.16.0: the target's user and project skills and commands load from the target at session start under their native names, listing, precedence, arguments, directories and `!` commands (run on the target); seven bundled skills that work through target tools are offered (29 cases match native-local results) | Target plugin (marketplace) skills, skills that may declare hooks or a non-bash shell, nested-directory skill discovery and six bundled skills with runtime files are not offered |
| MCP and web/browser tools | Claude 3.17.0: the target's user, project and local MCP servers load in native's scopes and precedence; stdio servers run on the target behind a stdio relay, remote (http/sse) servers are reached from the runtime as WebFetch is; native owns discovery, tools, instructions and permissions | Servers at the target's own loopback addresses, with a headers helper or ws transport are not offered; OAuth for remote servers runs on the runtime; servers are a session-start snapshot |
| Plans and task artifacts | Plan tools restricted in Claude lane | Separate runtime transcript from target plan/artifact storage; preserve native approval and resume semantics |
| Images, notebooks, PDFs | Claude 3.13.0/3.14.0: PDF Reads (whole document, `pages` rendered with the target's poppler), images (native's own Read of a private local copy: same checks, resizing, recompression and size note), binary-extension refusal and missing-file messages match native-local results (32 cases) | Whole PDFs above 10 MB and page images above 10 MB in all are refused; images above 64 MB; output artifacts and user-upload placement untested |
| File semantics | Claude 3.11.0: 24 Read/Write/Edit cases match native-local results and on-disk bytes and modes (encodings, BOM, CRLF, empty old_string, unread edits, directories) | Files are rewritten in place (native replaces them: inode, hard links, read-only files differ) |
| Atomic mutations | Stale stamp refusal has evidence; it does not alone establish compare-and-write atomicity | Inspect target implementation and inject mutation between check and write; use target-side atomic primitives where promised |
| Output limits | Large streams/backpressure and terminal events recorded | Test split UTF-8, binary/NUL, truncation markers, slow/absent reader, disk full and retained output expiration |
| Lost replies | Lost start and outages recorded without replay | Distinguish rejected, accepted, unknown and completed effects; never resend an unknown mutation under a new identity |
| Cancel and timeout | Foreground cancellation and retained job cancellation recorded; 3.5.0 adds per-call and per-agent cancellation and forwards interrupt to native before target cancellation; Claude 3.12.0: commands end with their shell, stops kill the whole tree (job-control groups included), detached and left-running children match native-local (14 lifecycle cases) | Leftover processes end with the keeper; test cancel/start races, keeper death and provider timeout semantics |
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
The 3.4.9 fix requires an explicit pending ID and a persisted cancellation
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

The 3.4.9 fix registers foreground identity before submission, persists
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

The [3.4.9 release receipt](experiments/claude-cancellation-release-2026-10-06.json)
records 63 source tests, the full deterministic gate, native review, 28 exact
packaged Claude checks plus six execution transport checks, 3.4.8/3.4.9 worker coexistence, macOS version probes,
three Catalog reader roles and five public artifact digest checks. OVH's
`ovh-claude-code-3-4-9-converge` operation completed with HTTP 204 and an applied
Machine receipt; the re-read inventory reports 3.4.9 active and 3.4.8 retained
for rollback. The production acceptance is installation/inventory evidence;
the loss-of-start-reply regression remains a facade fault injection, not a
production incident or a claim of complete local/remote parity.

The official [shell-prefix contract](https://code.claude.com/docs/en/env-vars),
checked 2026-10-06, also includes hook, status-line and stdio MCP shell commands;
the Bash argument contains the full native shell setup. These have different
placement and credential requirements, so a Bash-only success cannot accept
blanket shell forwarding. Native task completion/output, cancellation, runtime
bookkeeping and execution placement need separate cross-host evidence.

### File tools follow-up

The 2026-10-06 follow-up prepares Plugin 3.4.10 with unchanged Claude 2.1.287,
ACP 0.84.0 and executor 0.159.3 pins. The official
[tool reference](https://code.claude.com/docs/en/tools-reference) and
[Mods events](https://code.claude.com/docs/en/plugins/mods/events) were checked
again on that date. No tool inventory, native schema, permission mode or
allowlist changes are introduced; this patch changes the existing file facade.

| Surface | Previous behavior | Candidate behavior and evidence |
| --- | --- | --- |
| Image-to-text Write | Target bytes changed successfully, then decoding the binary original for a text diff threw and reported failure | Prepare optional text diff before mutation; binary originals use no text diff. Regression failed before the fix. |
| Post-write state failure | A failed atomic state replacement left an updated in-memory read stamp, allowing a subsequent edit despite failed persistence | Commit the stamp transactionally and restore the old stamp on failure; real rename-failure test requires a fresh Read before another Edit. The original Write may already have taken effect and is never replayed automatically. |
| Home-relative paths | Every leading tilde was rejected | Resolve `~` and `~/` using executor initialization's home URI, never runtime HOME; normalized absolute paths share the same read stamp. Missing home and named-user expansion remain rejected. |
| Invalid UTF-8 Read | Decoder error code was mislabeled as a state-storage failure | Return a specific invalid UTF-8 error; unsuccessful reads still grant no mutation stamp. |

The source gate passes 66 tests, formatting and types; native Codex review found
no concrete regression. Packaged-native scenarios additionally require image
replacement success, binary-read failure, target home expansion, and writing
through a symlink without replacing the link or losing the target's executable
mode. The candidate passed all 32 packaged Claude scenarios plus six execution
transport checks in 84.58 seconds, including the four new file scenarios.
This is isolated scripted-native evidence, not a production release receipt
or an authenticated cross-host model test.

The [3.4.10 release receipt](experiments/claude-file-semantics-release-2026-10-06.json)
records a second 38-check acceptance of the clean committed final package,
3.4.9/3.4.10 worker coexistence, Linux/macOS probes, three Catalog reader roles,
five public artifact digest checks, and the full quality gate before and after
main integration. OVH's `ovh-claude-code-3-4-10-converge` completed with HTTP 204
and an applied Machine receipt. Independent inventory confirmed 3.4.10 active,
3.4.9 retained for rollback, and current authentication/materialization. This
installation check sent no production inference prompt.

This is not atomic compare-and-write against unrelated target processes. Lexical
path normalization does not unify symlink or hard-link aliases; concurrent
rename/unlink, nonregular files, cross-platform path behavior and arbitrary
external writers remain separate audit cases. No previously supported feature
was disabled, and the full local/remote parity audit remains open.

### Unknown file-write outcome follow-up

The subsequent 2026-10-06 audit separates two failure boundaries. If the worker
loses a target completion while the keeper still retains it, transport recovery
must observe or retry the same operation identity without reapplying the write.
If the facade itself loses its connection and cannot receive a result, it must
report uncertainty and must not grant a new read stamp or replay during cold
load. File-content equality alone cannot prove an earlier operation's outcome.

The source matrix covers Write, Edit and NotebookEdit with both an applied
effect and a request that never applies. It verifies an uncertain result,
unchanged persisted read authority, no cold-load mutation, and rejection of
blind repeat mutations when the original effect changed target bytes. A fresh
Read reports the actual target bytes; the notebook insertion remains one cell.
These six cases use a mock transport and do not establish power-loss durability
or atomicity against another writer.

The packaged native fixture now discards a real successful fs/writeFile
completion, after independently replacing the just-written target contents.
It retains the original operation ID and requires the later native Read to
observe the independent writer's bytes. A replay would overwrite those bytes
and fail the check. A positive assertion also requires the lost-completion
injection to have occurred, so an unexercised fault cannot silently pass.
The fixture changes neither production Provider code nor its private pins.

The retained signed 3.4.12 adapter passes 35 packaged Claude scenarios plus six
transport checks (41 total) in 114.54 seconds. The source gate passes 76 tests,
including the six new fault cases and their parent test. This is additional
acceptance of the existing artifact, not a new Plugin release. Independent OVH
inventory still reports that exact 3.4.12 generation active. The
[write-outcome receipt](experiments/claude-write-outcome-audit-2026-10-06.json)
binds the tested artifact, test-source hashes and observed installation.

Permanent loss of the keeper's operation history and facade reconnection to
an unresolved mutation still require explicit observation rather than invented
success or automatic resubmission. This work does not provide OS-level
compare-and-write, external-writer isolation, native notification parity or
complete permission/subagent coverage.

### Durable explicit cancellation follow-up

The next 2026-10-06 audit reproduced a cancellation recovery gap: foreground
interrupts already persisted stop intent, but TaskStop and private utility
timeouts sent termination directly. Losing that request or its reply left no
durable instruction for a cold runtime to finish stopping the target process.

Plugin 3.4.12 routes those entrypoints through the existing cancellation
reconciler. Intent is saved before transport IO, remains pending until target
closure is observed, and is retried for the original process identity after
cold load. Reconciliation does not advance the output cursor or replay the
command. A closed output observation also retires the intent; an open output
commit preserves concurrent cancellation. Other tasks retain their identities.

The source gate passes 69 tests, including lost TaskStop response/cold resume,
private utility timeout/transport loss, pending cancellation and concurrent
output collection. The unfixed TaskStop regression fails because the persisted
intent is missing. These fault tests use mock transport; packaged native
acceptance and production activation are recorded separately. The CLI, SDK,
tool inventory, schemas, permissions and native pins are unchanged. Official
[tool](https://code.claude.com/docs/en/tools-reference) and
[Mods](https://code.claude.com/docs/en/plugins/mods/events) references were
checked again on 2026-10-06.

The [3.4.12 release receipt](experiments/claude-durable-taskstop-release-2026-10-06.json)
records the full quality gate and native review, 40 packaged execution checks
accepted in 84.30 seconds, old/new worker coexistence, Linux/macOS probes,
three actual Catalog reader roles and five public artifact digest checks.
OVH operation `ovh-claude-code-3-4-12-converge` completed with an applied receipt;
independent inventory confirmed 3.4.12 active, 3.4.11 retained for rollback,
and current authentication/materialization. No production inference was sent.
The exact lost-TaskStop-response fault is covered by source mock transport;
the native suite covers actual cancellation, cold resume and its existing
transport-loss scenarios, not that exact combined fault.

This does not recover an executor that permanently lost its process ledger,
establish native background notification parity, or resolve unknown file-write
results. Full remote/local parity remains under audit.

### Aliased concurrent mutations follow-up

The next 2026-10-06 audit reproduced silent lost edits within one facade:
two previously read paths naming the same target could both check the original
bytes, both return success, and overwrite one another. Lexical per-path queues
do not protect symlink or hard-link aliases. The regression holds the first
target write pending while admitting the second alias edit, and proves an
independent read completes before releasing the write. Before the fix the
second edit incorrectly succeeded; after the fix it reports a stale-read
conflict and only one target write occurs.

Plugin 3.4.11 serializes file mutations across paths within the same facade,
from the initial read/check through write acknowledgement and state commit.
The pinned executor metadata has no stable inode identity for a narrower lock.
Existing per-path ordering still governs reads and edits on the same lexical
path; independent reads, searches, commands and process cancellation retain
their separate queues. Mutation failure does not poison the queue because its
existing ordering primitive schedules subsequent operations after either
completion or rejection. Native pins, tool schemas, permissions and allowed
tools are unchanged.

The source gate passes 67 tests. The native fixture now checks actual symlink
and hard-link pairs, submitting two edits in the same scripted native response:
the first succeeds, the second rejects the stale stamp, and both names retain
the first edit and inode identity. Native scheduling may already serialize
those two edits; the source regression supplies the deterministic overlap.
Record packaged acceptance and deployment separately; source tests alone do
not accept a release.

The [3.4.11 release receipt](experiments/claude-alias-mutation-release-2026-10-06.json)
records 34 packaged Claude scenarios plus six transport checks, accepted in
84.00 seconds. It also includes 3.4.10/3.4.11 worker coexistence, Linux/macOS
version probes, three Catalog reader roles, five public artifact hash checks,
and the full quality gate before and after main integration. OVH's
`ovh-claude-code-3-4-11-converge` completed with HTTP 204 and an applied Machine
receipt. Independent inventory confirmed 3.4.11 active, 3.4.10 available for
rollback and current authentication/materialization. Installation verification
sent no production inference prompt.

This session-local queue does not synchronize other sessions or external
processes. It does not supply OS-level compare-and-write, prevent symlink
retargeting, or resolve an unknown target write after a lost acknowledgement.
Those remain explicit gaps; no additional tool was disabled.

### File tool semantics (Plugin 3.11.0)

A differential check runs 24 Read, Write and Edit calls on prepared files
natively on 2.1.287 (`tools/claude_file_native_probe.py`) and through the
packaged remote lane, then compares results and on-disk effects. The native
baseline is `tools/claude_file_native_baseline.json`. Native behavior:

**Read**
- Shows a file as UTF-8, with invalid bytes replaced; a UTF-16 file is shown
  that way too.
- Sets a UTF-8 byte order mark aside and shows CRLF as LF.

**Edit**
- Detects UTF-16LE by its byte order mark.
- Matches the strings as given against the file's LF form, so a CR in
  `old_string` does not match a CRLF file. It writes back in the file's own
  encoding, BOM and line endings; an inserted `\n` becomes CRLF in a CRLF
  file, and a CRLF in `new_string` stays as given in an LF file.
- Failing edits use native's messages (`String to replace not found in file.`,
  `Found N matches of the string to replace, …`).
- Rewrites a Latin-1 file as UTF-8, with replacement characters (data loss,
  natively).
- With an empty `old_string`, creates a missing file or fills an empty one.
  On a non-empty file it fails: `Cannot create new file - file already
  exists.`

**Read-before-write**
- An unread file may be edited or written. Edit then marks its result
  `contentNotInModelContext`.
- A file read and then changed on disk is refused: `File has been modified
  since read, …`. Only NotebookEdit still requires a Read.

**Write**
- Writes content as given and creates missing directories.
- Writing to a directory fails: `<path> is a directory, not a file. …`

**Results** name the file as the call did, not as an absolute path.

**Replacement** — native replaces the file under its name: a new inode, its
mode kept. A hard link therefore keeps the old content, and a read-only file
in a writable directory can still be edited.

Before 3.11.0 the facade differed in several ways:

- It refused files that are not valid UTF-8, showed byte order marks and
  required a Read before any mutation.
- It inserted LF into CRLF files.
- It could not edit UTF-16 files, or create or fill files with an empty
  `old_string`.
- It named files by absolute path and used its own messages.

3.11.0 follows native on all of these, and the acceptance compares the file
bytes and modes of every case.

Two differences are stated rather than fixed:

- **In-place rewrites.** Files are rewritten in place, so the inode stays, a
  hard link sees the change, and editing a read-only file fails. The executor
  offers no rename. A rename through a target process would have to join the
  mutation journal: unknown outcomes, cancellation, and the no-replay rule.
  That is a separate design.
- **Trailing tab.** A Mods-answered Read keeps the tab after the number of an
  empty last line, where native's own Read trims it.

Large files read through the range helper (128 KiB and up) are shown the same
way, so a range Read's text can be matched by a later Edit.

Allowing unread mutations removed a protection the facade relied on. A file
whose write outcome was lost, or whose state save failed, could have been
changed again by a retry. Before writing an unread or new file, 3.11.0
therefore records what the file held, or that it was absent. A retry then
finds the file changed and is refused, instead of applying the change twice
or overwriting a later change.

Four native review rounds were run, and the last reported none. They found
and fixed:

- a CR doubled by normalizing edit strings; native takes them as given, as
  measured
- an unbounded `originalFile`
- range-read boundaries (a BOM only at the file's start, a CR only before an
  LF)
- the range helper's strict decoding
- lost-write retries, for unread and for new files

The [3.11.0 release receipt](experiments/claude-file-semantics-release-2026-10-07.json)
binds feature commit `fb6e2ff7` and artifact
`sha256:994a0575c7025a1c6b042b33ca7afea20ab39c9a0a3287509e4a83d61834b1a3`.
It records:

- 78 accepted checks, including file parity, on the exact signed package
- 3.10.0/3.11.0 coexistence
- Linux and actual macOS probes
- three Controller reader roles
- five public artifact digests
- Catalog `ready`

A gate run after signing rebuilt the unsigned release envelope in `dist`. The
preserved signed envelope was restored and verified again before publication.
OVH operation `ovh-claude-code-3-11-0-converge` completed. Inventory reports
3.11.0 active, 3.10.0 retained for rollback and no session leases. No live session
was restarted.

### PDF and file-type Reads (Plugin 3.13.0)

Native-local baselines on 2.1.287 (`tools/claude_pdf_native_probe.py`, 21
cases in `tools/claude_pdf_cases.py`, baseline
`tools/claude_pdf_native_baseline.json`) show, for a file named `.pdf` (any
case):

- Without `pages`: `pdfinfo` counts the pages; more than 10 is refused
  ("This PDF has N pages, which is too many to read at once…"). Otherwise the
  file is sent whole as a `document` block after "PDF file read: <path>
  (<size>)". An empty file, one above 20 MB and one without a `%PDF-` header
  have messages of their own.
- With `pages` ("3", "1-5", "10-"; at most 20 pages): `pdftoppm -jpeg -r 100`
  renders the pages as JPEG images after "PDF pages extracted: N page(s)
  from <path> (<size>)". A page past the end, a corrupt or a protected file
  have messages of their own; without `pdftoppm` native says to install
  poppler. `pages` on a file not named `.pdf` is ignored.
- Any file whose extension is on native's binary list (`.bin`, `.zip`,
  `.exe`, …) is refused by name, whether or not it exists; PDF bytes under
  another name read as text.
- A missing file reads "File does not exist. Note: your current working
  directory is <cwd>."

Before 3.13.0 the facade refused every PDF and any `pages`, read binary-named
files as text, and answered a missing file with the generic
"Target operation failed" message.

3.13.0 runs `pdfinfo` and `pdftoppm` on the target with native's arguments
and limits, reads the rendered pages from a private target directory (then
removes it) and returns native's own `pdf` and `parts` results, so native
renders the blocks. The rendering belongs to the tool call: an interrupted or
stopped call kills it. Binary-extension refusal and the missing-file message
follow native's text. As natively, a PDF Read records no read state for
later edits (native's PDF branch never enters its read-file state); a review
finding asking for one was declined for that reason. The dev shell now provides poppler so the native-local
baseline and the target find the same `pdfinfo`/`pdftoppm`; the rendered JPEG
bytes match native's.

Packaged acceptance adds `pdf_and_file_type_reads_match_native_local` (all 21
results). Three native review rounds fixed: the rendering not belonging to
the call (now cancelled with it), a whole PDF or page images too large for one
execution or bridge message (now refused as above), and the missing-file
message skipped when the range helper's metadata call ran first. One existing check read a `.bin` file expecting lossy text; natively
that name is refused, so it now reads a file with an unlisted extension.

Gaps:

- A whole PDF crosses the execution connection and the Mods bridge in one
  message, so above 10 MB (native: 20 MB) it is refused with native's own
  wording for a PDF too large to return from another machine. Rendered pages
  above 10 MB in all are refused with a request for fewer pages; native
  recompresses each page above 500 KB instead.
- Images (also rendered pages) are not resized or recompressed, and carry no
  dimension metadata; native's image-extension mismatch message ("File has an
  image extension but its content is not a valid PNG/JPEG/GIF/WebP…") is not
  reproduced.
- The missing-file message omits native's "Did you mean …?" suggestions.
- Without `pdfinfo` on the target the page count is unknown and the PDF is
  sent whole, as natively when `pdfinfo` is missing.

The [3.13.0 release receipt](experiments/claude-pdf-reads-release-2026-10-07.json)
binds commit `c76059fe` (release commit `3d73db7f`) and artifact
`sha256:9f83a86e1b837f4cb6eddf3b0f22d001b749d653ed4a7c5e3e7b5a3e1993d832`.
It records 80 accepted checks on the exact signed package, 3.12.0/3.13.0
coexistence, Linux and actual macOS probes, three Controller reader roles,
five public artifact digests and Catalog `ready`. OVH operation
`ovh-claude-code-3-13-0-converge` completed: 3.13.0 active, 3.12.0 retained
for rollback, no session leases, no live session restarted.

### Images through native Read (Plugin 3.14.0)

Native-local baselines (11 image cases added to `tools/claude_pdf_cases.py`;
baseline `tools/claude_pdf_native_baseline.json`) show native re-encodes
every image it reads (even a 2x2 PNG). An image larger than 2000 pixels on a
side is resized and followed by a note ("[Image: original 3000x2000,
displayed at 2000x1333. Multiply coordinates by 1.50 to map to original
image.]"); one above its byte budget becomes a JPEG of about 500 KB.
Invalid contents ("File has an image extension but its content is not a
valid PNG/JPEG/GIF/WebP…") and empty files have messages of their own. Only
`.png`, `.jpg`, `.jpeg`, `.gif` and `.webp` names, or a file without an
extension, are read as images; PNG bytes named `.txt` read as text.

The facade used to send the target bytes as they were: no resizing, no note,
and a large image (over 4 MB, or over what the API accepts) failed. Native's
encoder is not available to a plugin, so 3.14.0 lets native do the work:

- The target bytes (in 4 MB parts above the execution connection's message
  limit, up to 64 MB) are written to a private runtime file (0600 in a 0700
  directory beside the plugin state) with the file's extension.
- The context Mod calls native's own Read (`next`) on that file. Native's
  image result carries no path. Its size note comes back with the result and
  is kept. Its refusals name the target file instead of the copy. The copy is
  removed afterwards (and any leftover when the launcher starts).
- The target Read records its read stamp as before; project hooks, nested
  instruction files and permission decisions still see the target path.

Packaged acceptance compares the 11 image cases (byte digests and notes)
inside `pdf_and_file_type_reads_match_native_local`. Every image matches
native byte for byte, including the resized PNG, the 7.7 MB noise PNG
recompressed to JPEG and the size note. Candidate acceptance first showed the
size note dropped (native returns it beside the result; it is now kept) and
native's refusals arriving in an error form; two native review rounds fixed
`$` sequences in a target path being read as replacement patterns, and the
second reported none.

Gaps:

- Images above 64 MB are refused; native has no such limit of its own.
- Native's Read of the copy also records the copy in its read state; the copy
  is gone afterwards and nothing edits it.

The [3.14.0 release receipt](experiments/claude-image-reads-release-2026-10-07.json)
binds commit `1a3bae91` and artifact
`sha256:bd75137c0924e9912033d33a006762a325e3e21d795d1cdc66110c7065cafa85`:
80 accepted checks on the exact signed package, 3.13.0/3.14.0 coexistence,
Linux and actual macOS probes, three Controller reader roles, five public
artifact digests and Catalog `ready`.

The first OVH converge (`ovh-claude-code-3-14-0-converge`) ended with an
unknown outcome: the Machine's connection dropped and reconnected while the
installation was staging (Machine receipt: staging, `authorization_ended`).
3.13.0 stayed active. While the slot was fenced, three existing claude-code
sessions on OVH recorded 11 errors asking to reconcile the installation
(about 70 seconds). Two `reconcile-install` calls recorded the Machine's
receipt and resolved the staging failure without replaying anything; a new
installation `ovh-claude-code-3-14-0-retry-1` then completed. Inventory
reports 3.14.0 active, 3.13.0 retained for rollback and no session leases. No
live session was restarted.

### Native tool set (Plugin 3.15.0)

A local Cowboy Claude session starts native with the ACP adapter's
`--tools default`. On 2.1.287 that set (`AskUserQuestion` aside, which the
adapter removes without form elicitation) is: Agent, Bash, CronCreate,
CronDelete, CronList, DesignSync, Edit, EnterWorktree, ExitWorktree,
ListAgents, NotebookEdit, Read, ReportFindings, ScheduleWakeup, SendMessage,
Skill, TaskCreate, TaskGet, TaskList, TaskStop, TaskUpdate, WebFetch,
WebSearch, Workflow and Write. There is no Glob, Grep or TodoWrite: searches
go through Bash and the task list through the Task tools.

The remote lane advertised Glob, Grep and TodoWrite instead, and lacked the
Task list and web tools. 3.15.0 advertises native's set minus the tools
below, and runs TaskCreate, TaskGet, TaskList, TaskUpdate, WebFetch,
WebSearch and ReportFindings natively where the session runs, as in a local
session (they read or write no target file). WebFetch of the machine's own
names (`localhost`, `*.localhost`, 127/8, `::1`, `0.0.0.0`) is refused with a
pointer to Bash on the target, since natively those name the user's machine,
which here is the target.

Packaged acceptance checks the advertised set
(`native_default_tool_set_without_runtime_only_search_tools`), a native
TaskCreate and the loopback refusal
(`native_task_list_runs_and_target_loopback_fetch_is_refused`); the
concurrent-search check now uses read-only Bash commands. One native review
round reported nothing.

Still not offered, each with its reason:

- Skill, EnterWorktree/ExitWorktree, Workflow: they read or change the
  project from the runtime's filesystem; target-aware versions are separate
  work.
- CronCreate/CronDelete/CronList, ScheduleWakeup: scheduled prompts start
  model turns later; their lifetime across the remote binding is untested.
- ListAgents: it lists Claude sessions on the runtime machine, not the
  user's.
- DesignSync: claude.ai design login, not part of this lane.
- Searches through Bash use the target's own `rg`, `grep` and `find`; native
  shadows them with its embedded ripgrep, which is not on the target.

The [3.15.0 release receipt](experiments/claude-tool-set-release-2026-10-07.json)
binds commit `5f93175e` and artifact
`sha256:8d357aeb685c8e9feda83cdd9bcb22a4970763b40754089d9c92ca7f1aeb7143`:
82 accepted checks on the exact signed package, 3.14.0/3.15.0 coexistence,
Linux and actual macOS probes, three Controller reader roles, five public
artifact digests and Catalog `ready`. OVH operation
`ovh-claude-code-3-15-0-converge` completed: 3.15.0 active, 3.14.0 retained
for rollback, no session leases, no live session restarted.

### Agents' background commands and the target environment (Plugin 3.19.0)

Native 2.1.287 delivers a background agent's background command completion to
that agent (measured): into its running turn, or, if it has ended, by resuming
it, after which the parent is notified of the agent's new result. The remote
lane gave agents' background commands no notification, because the native
task standing for a target command was started by the plugin's own
`$.tool.call`, and a plugin's call belongs to the main session even with an
agent id (measured). 3.19.0 runs the agent's own Bash call natively as that
task (the `tool.call` hook's `next` with the waiter command), so native
registers it as the agent's; the model still sees the target command's
result. Packaged acceptance runs a background agent whose background command
notifies it while it waits
(`agent_background_command_notifies_the_agent_as_natively`). One native
review round reported nothing.

The plugin also stops asking the executor to drop credential-like variables
(user decision); see the MCP section for the Machine's closed environment,
which still decides what reaches the target.

The [3.19.0 release receipt](experiments/claude-agent-background-release-2026-10-08.json)
binds commits `446346f5` and `98828c9b` and artifact
`sha256:d7469ce2bc3a30e19314539393cdb0e02af808c4fe0f896b2ee9cec4ad7579ed`:
90 accepted checks on the exact signed package, 3.18.0/3.19.0 coexistence,
Linux and actual macOS probes, three Controller reader roles, five public
artifact digests and Catalog `ready`. OVH operation
`ovh-claude-code-3-19-0-converge` completed: 3.19.0 active, 3.18.0 retained
for rollback, no session leases, no live session restarted by this task.

### Tool descriptions and background deadlines (Plugin 3.18.0)

Two differences the earlier phases had not recorded:

- **Tool descriptions.** The remote lane replaced native's descriptions of
  Bash, Read, Write, Edit, NotebookEdit and TaskStop with short ones of its
  own (Bash: one paragraph instead of native's ~10,000 characters with its
  background, sleep, git commit and pull request guidance). The model was
  therefore instructed differently from a local session. 3.18.0 keeps
  native's own descriptions; they hold for the target (absolute paths,
  images, PDFs, notebooks, background output read with Read).
- **Background deadlines.** Native 2.1.287 stops a background command at its
  deadline (measured): the requested `timeout` for `run_in_background`
  (default 30 minutes, at most 2 hours), 30 minutes for a command moved to
  the background, whose result says so. It then notifies with status
  `killed` and "was stopped after reaching its background time limit".
  In the remote lane native applied the deadline to the runtime waiter that
  stands for the target command: the waiter ended, the target command kept
  running, and the notification named the waiter's runtime command line.
  3.18.0 gives the waiter the command's own deadline, stops the target
  command before the notification is stored, rewrites the notification to
  the target command, and adds native's 30-minute sentence to a moved
  command's result.

Packaged acceptance compares the six descriptions with the
`tool_descriptions` of `tools/claude_native_behavior_baseline.json`
(`target_tools_carry_native_descriptions`), runs a background command with a
3-second timeout and checks native's notification and that the target command
stopped (`background_deadline_stops_the_target_command_as_natively`); the
lifecycle comparison no longer excludes the 30-minute sentence. Four native
review rounds were run; the last reported none. Fixed findings: the deadline
follows the input as hooks or approvals amended it; a background `timeout`
up to two hours is accepted, as natively (a foreground one stays at ten
minutes); a deadline stop the target did not confirm says so in the
notification instead of reading as stopped.

The [3.18.0 release receipt](experiments/claude-deadline-release-2026-10-08.json)
binds commit `04890852` and artifact
`sha256:07cf132d68ff5f2d43e8658d8b98d7d9966e899b76de1c8b8c1669f2f1df7140`:
89 accepted checks on the exact signed package, 3.17.0/3.18.0 coexistence,
Linux and actual macOS probes, three Controller reader roles, five public
artifact digests and Catalog `ready`. OVH operation
`ovh-claude-code-3-18-0-converge` completed: 3.18.0 active, 3.17.0 retained
for rollback, no session leases, no live session restarted by this task.

### MCP servers (Plugin 3.17.0)

Native 2.1.287 in a local session (measured, SDK mode, settings sources
user, project and local):

- **Scopes.** User servers from `~/.claude.json` `mcpServers`; project
  servers from the `.mcp.json` of every directory from the root down to the
  working directory, nearer files overriding, loaded without an approval
  prompt; local servers from `~/.claude.json`
  `projects[<repository root, else working directory>].mcpServers`.
- **Precedence.** Local over project over user. `disabledMcpjsonServers`
  drops a project server; `disabledMcpServers` turns a server off.
- **Startup.** `${VAR}` and `${VAR:-default}` expand from Claude Code's
  environment; a stdio server starts in the working directory with that
  environment plus `CLAUDE_PROJECT_DIR`, `CLAUDE_CODE_SESSION_ID` and
  `CLAUDECODE`; server instructions reach the model in an
  `# MCP Server Instructions` reminder.

Before 3.17.0 the remote lane loaded only Matrix (`--strict-mcp-config`).

3.17.0 reads the target's configuration at session start (`mcp.mjs`,
`WorkspaceTools.mcpInputs`; only the variables it names are read from the
executor's environment) and hands native an `--mcp-config`:

- **stdio servers run on the target.** Native starts `mcp-proxy.mjs` for each;
  it starts the server through the exec-server with stdin piped
  (`process/write`) and relays its stdin, stdout and stderr. Native still owns
  the MCP session: initialize, tool listing, instructions, permissions,
  cancellation and reconnection. Output wake-ups come from the executor's
  `process/output` notifications, so an idle server makes no executor calls;
  MCP calls queue for four of the connection's slots. Servers end with the
  session, and a launcher ends any an earlier one left.
- **Remote servers are reached from the runtime**, as WebFetch is, with
  their URL and headers expanded from the target's environment.
- **Not offered:** servers at
  the target's own loopback names (the runtime cannot reach them), servers
  with a `headersHelper` (it would run on the runtime), ws or other
  transports and invalid entries.

Packaged acceptance runs user, project and local servers and a shadowed name
on the target (`target_mcp_servers_run_on_target_in_native_scopes_and_precedence`:
server, working directory, expanded arguments and environment,
`CLAUDE_PROJECT_DIR`, `CLAUDECODE`), checks the instructions reminder and that
the loopback server is absent. The first candidate polled each server's output
and slowed every other target call until the first turn timed out; reads now
wait on notifications. A later run found a restarted launcher refused by the
one-connection endpoint while the previous connection was closing; the
launcher now retries the connection briefly.

Twelve native review rounds were run; the last reported none. Fixed findings:

- a closed server's unread output pages, and output the executor no longer
  retains: a gap in its sequence ends the server with a stated reason
  instead of handing native a cut JSON-RPC stream (an exit's own number,
  known from its notification, is not a gap)
- MCP calls exceeding the connection's request limit, now queued on four
  slots; leftover-server cleanup one at a time, keeping a record when the
  stop's outcome is unknown, and waiting for starts in flight at session end
- remote servers whose URL or headers name variables the target does not
  set (native would expand them from the runtime's environment), loopback
  names written with a trailing dot or as IPv4-mapped IPv6 (also for
  WebFetch)
- relay backpressure toward a slow native reader
- credential-like variables (`*TOKEN*`, `*KEY*`, ...) that the executor drops
  by default: MCP servers and the variables their configuration names now
  get the executor's whole environment, as natively

Gaps:

- Target commands do not see the environment a local session's commands
  see. The executor's environment is closed by Machine design
  ([execution environments](execution-environments.md)): only a fixed base
  set (on Hawk observed as `HOME`, `PATH`, `SHELL`, `USER`, `LOGNAME`,
  `LANG` and the executor's own `CODEX_HOME`) plus operator-listed names,
  never credential-shaped ones. A local Cowboy session's commands inherit
  the worker's environment (on Hawk about 99 variables, among them
  `SSH_AUTH_SOCK`, `DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`,
  `EDITOR` and the Columbus and cache locations). The user accepts exposing
  credential-like variables; the plugin therefore no longer applies the
  executor's own default exclusion, but the Machine's closed set still
  decides what reaches the target. Aligning it is a Machine release and
  configuration change, not a plugin one.
- A disabled server is absent rather than listed as disabled.
- Remote servers' OAuth and their network origin are the runtime's.
- Target `.claude/settings.json` MCP permission rules are not read (the
  3.6.0 rule gap).
- Servers are a session-start snapshot; `/mcp` changes are refused as before.

The [3.17.0 release receipt](experiments/claude-mcp-release-2026-10-08.json)
binds feature commit `d3e8203f`, release merge `38905216` and artifact
`sha256:991ac736cd1eff51396a94ab81bbdab0ca1fec71d9e71109f34e0d159f09f5c0`:
87 accepted checks on the exact signed package, 3.16.1/3.17.0 coexistence,
Linux and actual macOS probes, three Controller reader roles, five public
artifact digests and Catalog `ready`. OVH operation
`ovh-claude-code-3-17-0-converge` completed: 3.17.0 active, 3.16.1 retained
for rollback, no session leases, no live session restarted by this task.

### Skills, commands and native reminders (Plugin 3.16.0)

Native 2.1.287 in a local session (measured with a scripted API):

- **Discovery.** User skills (`~/.claude/skills/<name>/SKILL.md`) and
  commands (`~/.claude/commands/**/*.md`), then `.claude/skills` and
  `.claude/commands` of each directory from the working directory up to the
  repository root, never the home directory itself. A user skill shadows a
  project skill of the same name, a skill a command. Nested commands are
  named with colons (`grp:inner`). A frontmatter `name` is an alias.
- **Listing.** A `skill_listing` reminder lists user skills, project skills,
  then commands, then bundled skills. A skill without a description gets its
  first line (heading marks removed, cut at 100 characters).
- **Expansion.** `Base directory for this skill:`, `${CLAUDE_SKILL_DIR}`,
  `${CLAUDE_PROJECT_DIR}`, arguments, re-invocation and "already loaded"
  notes, `disable-model-invocation`, and `!` commands (inline and fenced)
  run by Bash after a permission check. A failing command fails the Skill
  call with `Shell command failed for pattern …`.

Before 3.16.0 the remote lane disallowed Skill, sent an empty skills
allowlist and disabled native attachments, so the model saw no skills and
none of native's per-turn reminders.

3.16.0:

- **Target skills.** The launcher reads the target's skill and command files
  at session start (`skills.mjs`, `WorkspaceTools.skillFiles`) and writes
  them into a private plugin native loads. Their directory placeholders name
  the target; `!` commands carry a per-session marker, so native does not run
  them on the runtime. `context-mod.js` runs them on the target through the
  Bash facade (same shell session, permission check, agent ownership and
  cancellation; a command still running fails the skill and is cancelled).
- **Native names.** Native names plugin skills `cowboy-target:<name>`. The
  Skill tool, typed `/name` commands, the skill listing (order and
  shadowing), the tool result, the expanded text, the typed command's tags,
  the client's command list and `system/init` all use the native-local name
  and target paths. A Mods result that changes a Skill result drops the
  skill's messages, so names are projected as messages are appended.
- **Bundled skills.** code-review, init, keybindings-help, run,
  security-review, simplify and update-config are offered; their
  instructions act through target-bound tools. claude-api, dataviz and
  plugin-authoring (reference files on the runtime), fewer-permission-prompts
  (runtime transcripts), loop (Cron) and workflow-authoring (Workflow) are
  refused with that reason. A target skill shadows a bundled one, even one
  this lane cannot offer.
- **Native reminders.** Native attachments are enabled again. Types built
  from this machine's files, editors or memory (`RUNTIME_ATTACHMENTS`: file
  and directory @-mentions, read and edited-file reminders, nested memory,
  dynamic skills, diagnostics, IDE selections, plan files, memories) are
  dropped. The rest reach the model as natively: the skill listing, token and
  budget reminders, task-list reminders, background-task status, async hook
  responses, date changes and others. An @-mention of a file therefore
  attaches nothing; the model reads the target file itself.

Packaged acceptance compares 29 Skill results and the listing with
`tools/claude_skill_native_baseline.json`
(`target_skill_results_match_native_local`,
`target_skill_listing_matches_native_local_with_stated_bundled_skills`), and
checks a typed command, a runtime @-mention and that the private plugin never
reaches the model (`typed_target_command_and_runtime_mentions_as_native`).
The acceptance target now gets a fresh home directory, so its user files are
fixtures rather than the build account's. Eleven native review rounds were
run; the last reported none. Fixed findings:

- frontmatter keys that could hide `hooks` (quoted, escaped, merged,
  complex), unindented lists and uniformly indented keys
- comments read as allowed-tools rules
- mirror paths of skills whose names share a prefix
- command file names whose colons would leave the plugin directory
- skill commands not owned by their agent or cancelled with their call; two
  calls of one skill now take turns
- a typed command's expansion not cancellable
- name projection limited to skill messages, so file contents keep the text
- a skill command left running treated as complete
- rule placeholders not resolved, a bundled skill used in place of a refused
  target skill of the same name, path-like prompts treated as commands
- an unreadable skill file stopping the session (natively it is skipped)

Gaps:

- Error results keep Mods' `<tool_use_error>` wrapper (stated in the
  comparison).
- Skills that may declare hooks or name a non-bash shell, target
  marketplace plugins' skills and skills discovered under nested
  directories during the session are not offered.
- The target's `disableSkillShellExecution` setting is not read.
- A skill's allowed-tools grant only single plain Bash commands to its `!`
  commands; others follow the session's rules.
- Skills are a session-start snapshot (natively, edits are picked up).
- `update-config` edits the target's settings, which this lane reads only
  for hooks at session start.

The [3.16.0 release receipt](experiments/claude-skills-release-2026-10-07.json)
binds commit `4bbd3f46` and artifact
`sha256:8c81ff9c90b42eddf47905975b1661c8dce01ced11867699213ddf49b5028bae`:
85 accepted checks on the exact signed package, 3.15.0/3.16.0 coexistence,
Linux and actual macOS probes, three Controller reader roles (re-resolved:
the active Controller had changed), five public artifact digests and Catalog
`ready`. OVH operation `ovh-claude-code-3-16-0-converge` completed: 3.16.0
active, 3.15.0 retained for rollback, no session leases, no live session
restarted and no session errors.

Main had meanwhile released claude-code 3.15.1 (component release 3.42.0,
execution recovery) from 3.15.0, so 3.16.0 lacked its package record. 3.16.1
merges both; its runtime archives are byte-identical to 3.16.0's. The
[3.16.1 receipt](experiments/claude-skills-merge-release-2026-10-08.json)
binds merge `35d7835e` and artifact
`sha256:96259ce4ae999e5555fbe0d6a54b52bb86ab14f7e5b1dd1c909b4612682cd175`
with the same 85 checks, 3.16.0/3.16.1 coexistence, macOS, Catalog readers
and public digests. OVH operation `ovh-claude-code-3-16-1-converge` completed:
3.16.1 active, 3.16.0 retained for rollback. The Controller moved idle
claude-code sessions to the new version on its own schedule, with no
claude-code session error.

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

#### Bash results and shell state (Plugin 3.8.0)

A differential check found that the facade's Bash differed from native-local
Bash in most model-visible results. The check runs the same 35 Bash calls in one
turn on native-local 2.1.287 and on the packaged remote lane, then compares the
normalized results. Native baselines also captured native's own command line,
environment and shell snapshot. The differences found were:

- a trailing `Exit code: 0` on every result
- no output trimming and no `(Bash completed with no output)`
- stdout and stderr not interleaved in write order
- `set -e` aborting commands that natively run on
- no `cd` persistence
- a large output silently cut at the first executor read, because a process
  reports closed while output remains to be read
- none of native's command environment or user shell snapshot

3.8.0 reproduces native's command shape on the target:

- **Command wrapper.** It sources a snapshot, turns extglob off, merges the two
  streams, and runs `eval '<command>' < /dev/null && pwd -P >| <cwd file>`.
- **Shell.** Commands use the user's shell when it is bash or zsh, as natively.
- **Snapshot.** At session start, the target user's login shell runs native's
  generator steps: rc file, shopt, functions, set -o options, aliases and
  PATH.
- **Directory.** The final directory persists inside the project. Outside it,
  the directory resets with native's `Shell cwd was reset to …` line.
  Background commands never move it.
- **Environment.** Native's variables are set: `CLAUDECODE`,
  `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ATTENDED`,
  `CLAUDE_CODE_ENTRYPOINT`, `COREPACK_ENABLE_AUTO_PIN`, `GIT_EDITOR`, `SHELL`,
  `AI_AGENT`, plus the session's `CLAUDE_CODE_SESSION_ID` and `CLAUDE_EFFORT`.
- **Results.**
  - Output is trimmed, and an empty result reads as native's placeholder.
  - Failures read `Exit code N` plus output, capped at 30,000 characters and
    kept to the first and last 5,000.
  - An output over 30,000 characters is written whole (up to 4 MiB) to a
    private target file. The model sees native's `<persisted-output>` preview
    with that target path.
- **Draining.** Output collection drains a closed process until a read returns
  nothing.

Packaged acceptance now requires every case to match the committed native
baseline (`tools/claude_shell_native_baseline.json`). One stated exception
remains, described below. A former check expected a character split around a
stderr write to survive. Native interleaves the bytes and decodes them as
replacement characters (measured), so the check now requires native's result.

Remaining differences and their reasons:

- **Error wrapper.** A Mods-answered error is wrapped in `<tool_use_error>`
  tags. Mods offers no error result without the wrapper: its `isError`
  variant is set by core only.
- **Effort timing.** `CLAUDE_EFFORT` and the hooks' `effort` field appear
  only after native first reports them. Native reports effort only in
  tool-context hook input (PostToolBatch, Stop). A `turn.step` hook would
  carry it from the first request, but the pinned build refused to load the
  module with one.
- **`CLAUDE_PID`.** It is not set: it names a runtime process.
- **find, grep, rg and pkill.** Native shadows these with its embedded
  executable, and with a guard that reads `CLAUDE_PID`. The target has no
  such executable, so the system commands run. Native does the same when its
  executable is absent.
- **Other rc-file exports.** Native's snapshot keeps only PATH from the user's
  environment, and so does 3.8.0. Other exported variables come from the
  executor's environment, as native's come from its own process.
- **Time-limited and background commands.** They keep `cowboy-task://`
  handles. 3.8.0 gave no completion notification; 3.9.0 adds them (below).

Four native review rounds found and fixed these defects:

- output consumed before a timeout was lost
- the persisted file could exceed what Read returns
- `shopt` ran under zsh
- a still-running command's directory file was left behind
- `..name` directories were treated as outside the project
- a directory reset was dropped after a persisted preview

The fourth round reported none. Making the hook phase reliable required two
harness fixes:

- the hook checks read the result's own message, because the latest request
  may be a background child's
- the check for leftover hook files waits for in-flight child hooks

The [3.8.0 release receipt](experiments/claude-shell-parity-release-2026-10-06.json)
binds feature commit `dd1a2367`, release merge `7d0e35cd` and artifact
`sha256:bf1da8c889c9900f1883e51fcac63d8ee35700a1513118cea4e0fc395f06c56a`.
It records:

- 69 accepted checks on the exact signed package, including Bash parity
- 3.7.0/3.8.0 coexistence
- Linux and actual macOS probes
- three Controller reader roles
- five public artifact digests
- Catalog `ready` on both platforms

OVH operation `ovh-claude-code-3-8-0-converge` completed. Inventory reports
3.8.0 active, 3.7.0 retained for rollback and no session leases. No live session
was restarted.

#### Background completion notifications (Plugin 3.9.0)

Natively, a background or timed-out command becomes a native background task.
Native-local baselines on 2.1.287 show:

- When it ends during a turn, its `<task-notification>` (completed or failed,
  with the exit code) is delivered into that turn, appended to a tool result.
- When it ends while the session is idle, native starts a turn of its own
  with the notification, framed as "SYSTEM NOTIFICATION - NOT USER INPUT".
- A TaskStop'd command sends nothing.
- A command that hits its timeout is moved to the background and notified
  later, unless it starts with `sleep`. Such a command is killed with
  `Exit code 143` / `Command timed out after <duration>`.

Mods cannot inject such a notification. `$.session.receive` is absent at
runtime on the pinned build. `$.prompt.submit` waits for idle and frames the
text as a plugin prompt "in the user's place", which is the opposite of
native's framing. A Mod can, however, start a native background Bash with
`$.tool.call`, and native then notifies for it exactly as for its own.

3.9.0 therefore pairs each target command left running with a native
background task:

- **Waiter.** The task runs `task-wait.mjs` on the runtime. It long-polls the
  bridge (`/task-wait`), which observes the target process without consuming
  the output its handle reads, and exits with the command's status. A
  stopped command never reads as finished: the waiter stays until native
  stops it.
- **Native ownership.** Native owns the task, so delivery timing, idle turns
  and the framing are native's own.
- **Notification rewrite.** The Mod maps the notification to the target
  command: the job id, the model's tool use id, `cowboy-task://` as the output
  file and the original command line. This applies wherever native renders it
  (prompt row, delivery and queued-command attachment).
- **Stopping.** TaskStop on the command also stops its native task, so
  nothing is sent.
- **Permissions and hooks.** A `tool.check` hook keeps the waiter from asking
  the user a second time in prompting modes. The hook proxy keeps it away
  from project hooks.
- **Results.** Background and moved results now read exactly as native's
  ("You will be notified when it completes."). Without a native task (a
  subagent's command, or a failed start), that sentence is removed instead
  of promising a notification.
- **Timeouts.** A timed-out command starting with `sleep` is killed with
  native's text.

Packaged acceptance adds five checks, all on the target:

- an idle completion starts a native notification turn with native framing,
  the target handle, the model's tool use id, the command line and no runtime
  path
- a completion during a turn is delivered into that turn
- a stopped command sends nothing
- a timed-out command moves to the background and is notified
- in default mode the user is asked once, for the command, not again for its
  notification task

Gaps:

- Native stops an auto-backgrounded command after 30 minutes and says so; the
  waiter does not, and the result omits that sentence.
- Subagents' background commands give no notification.
- Native's other background moves are not reproduced: a message arriving
  during a foreground command, or a user's Ctrl+B.
- Native's duration format for timeouts of whole minutes was not measured
  (`Xm Ys` is assumed).
- Like native background tasks, the notification does not survive the
  session process. The target command does, and its handle stays readable.

Two native review rounds were run. The first found a stop overwritten by a
concurrent output read, which could have produced a notification for a
stopped command. The fix keeps the stop and comes with a regression test that
fails without it. The second round reported none.

The [3.9.0 release receipt](experiments/claude-background-notifications-release-2026-10-07.json)
binds commit `812c8683` and artifact
`sha256:4fdef6086dbb6485cac7946edf85072727e876994b3cb5e302e6cdd560449be6`.
It records:

- 74 accepted checks on the exact signed package
- 3.8.0/3.9.0 coexistence
- Linux and actual macOS probes
- three Controller reader roles
- five public artifact digests
- Catalog `ready`

OVH operation `ovh-claude-code-3-9-0-converge` completed. Inventory reports
3.9.0 active, 3.8.0 retained for rollback and no session leases. No live session
was restarted.

#### Process lifetimes (Plugin 3.12.0)

Native-local baselines on 2.1.287 (`tools/claude_lifecycle_native_probe.py`,
14 cases in `tools/claude_lifecycle_cases.py`, baseline
`tools/claude_lifecycle_native_baseline.json`) show:

- A command ends when its shell exits. `server & echo started` returns at once
  even though `server` still holds the output; what the command started keeps
  running, also after `exit 3`, `exec` or the session's end.
- Stopping a command (TaskStop, a timeout kill) kills its whole process tree.
  Children started into their own session with `setsid` are left running.
- A background command completes, and is notified, when its shell exits.
- TaskStop answers
  `{"message":"Successfully stopped task: <id> (<command>)","task_id","task_type":"local_bash","command"}`;
  for a command that has already ended it fails with
  `No task found with ID: <id>`.

Measured remote differences before 3.12.0:

- A command finished only when its output pipe closed. With a child holding
  the pipe, the call waited for its timeout and then moved the command to the
  background; a command starting with `sleep` was even killed with
  `Exit code 143`.
- The executor ends a stopped command's process group only. Native's shell
  snapshot generator keeps `set -o monitor`: its `set -o | grep "on"` filter
  matches the option name, not its state, so every `&` job runs in a group of
  its own. A stopped or timed-out command's background jobs kept running
  (packaged acceptance showed it for TaskStop of moved and background
  commands and for a timeout kill; 3.11.0 behaves the same under the executor
  with job control on).
- TaskStop answered `Command stopped.` without the command, and "stopped" an
  already finished command.

3.12.0:

- **End of a command.** The command's shell runs under a minimal parent shell
  that writes `\x1e<nonce>:<status>` to both streams after it and exits with
  the same status; its own notices (a killed child) are discarded. The nonce
  is per command and recorded with the job. Each stream's output up to its
  line is the command's, and the command ends once both lines are read; the
  line gives the exit status, so `exit`, `exec` and traps cannot skip it.
  Output that may begin a line is held back until it is decided, also across
  reads. The executor's `closed` remains the end of a command killed before
  its lines.
- **Background completion.** The waiter finds the same line without consuming
  the handle's output, so the notification follows the shell's exit. It
  records the end, so a later TaskStop leaves what the command started.
- **Stopping.** Before the executor terminates a command whose shell sourced
  the snapshot, a target utility finds its process (by the nonce, split so the
  utility never matches itself) and kills every descendant through `ps` and
  `awk`, as native's tree kill does. Without `ps` only the group ends.
- **TaskStop.** Native's answer, with the command; a finished command reads as
  `No task found with ID: <id>`. The command is kept only while a task can be
  stopped, at most 4 KiB each and 256 KiB in all, so the state still loads.

Packaged acceptance adds `bash_process_lifetimes_match_native_local`: all 14
results, which processes still run afterwards (by heartbeat files, since the
target may run commands in another process namespace) and the background
command's completion notification match the native-local baseline. The
remaining difference is stated in the check: the moved-to-background result
omits the 30-minute auto-stop sentence (below).

Gaps:

- Native stops a command moved to the background after 30 minutes; the target
  command keeps running.
- A command left running when the session's keeper ends is ended with it
  (the executor reaps its groups, the Machine service owns the cgroup).
  Natively it outlives the session. This bounds leftover processes to the
  binding's lifetime instead of leaking them on the target.
- A process that left the tree before the stop (its parent exited, so it was
  reparented) is not killed. Natively the same.
- A command whose shell had no snapshot gets only the group kill; a command
  that itself runs `set -m` then keeps its jobs.
- A stop that races the shell's own exit can still kill what it left running.
- A stopped command named by a command longer than 4 KiB, or beyond the
  256 KiB budget, is named in part or not at all.

Five native review rounds were run; each finding was fixed with a regression
test that fails without it:

- A large background output could leave the end line on a later page, and the
  parent shell's own success status was reported instead (now the waiter
  reads to the line, and the parent exits with the command's status).
- A tree kill that ran before the command's process existed was cached as
  done (now it is cached only once the process was found).
- Every task kept its full command forever, which could grow the state past
  what loads (now bounded and dropped when the task ends).
- stdout's end line was taken as the end of stderr too (now each stream has
  its own).

The fifth round reported none. Packaged acceptance found two defects before
these rounds: an output larger than one read lost its rest when the executor
reported the process closed before the end line was read, and the missing
tree kill above.

The [3.12.0 release receipt](experiments/claude-process-lifetimes-release-2026-10-07.json)
binds commit `11c9e5fd` (merge `3a808a13`) and artifact
`sha256:281faae172bd93b43f8f4543bca0dd508f3e31c8dff7d3b5172f8634e2bfd0b9`.
It records:

- 79 accepted checks on the exact signed package
- 3.11.0/3.12.0 coexistence
- Linux and actual macOS probes; the stop's `ps`/`awk` utility was also run
  by hand on macOS
- three Controller reader roles (the active Controller had changed and was
  re-resolved; its rollback predecessor read as next-transaction recovery)
- five public artifact digests
- Catalog `ready`

OVH operation `ovh-claude-code-3-12-0-converge` completed. Inventory reports
3.12.0 active, 3.11.0 retained for rollback and no session leases. No live
session was restarted.

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

#### Target instructions and session context (Plugin 3.10.0)

Native-local baselines on 2.1.287 decide what a session starts with. The
recorded instruction files are:

- the user's `~/.claude/CLAUDE.md`
- for each directory from the root down to the working directory: its
  `CLAUDE.md` (with `@` imports right after it), `.claude/CLAUDE.md`,
  unconditional `.claude/rules/*.md` and `CLAUDE.local.md`

Each file is rendered as `Contents of <path> (<tier>):` under native's
"Codebase and user instructions" framing. Further findings:

- `AGENTS.md` is never read, with or without a `CLAUDE.md`.
- A Read below the working directory attaches the `CLAUDE.md` of each
  directory in between and the rules whose `paths` match, once each.
- A Write, a Read outside the project and a repeated Read attach nothing.
- The environment block and the Git status block (current branch, main
  branch, Git user, `git status --short` cut at 2,000 characters, five recent
  commits) have fixed native forms.

Before 3.10.0, the facade loaded `AGENTS.md`, `CLAUDE.md` and
`.claude/CLAUDE.md` from the ancestors as raw text behind its own preamble. It
had no imports, no `CLAUDE.local.md`, no rules, no user file and no nested
files. Its environment line used its own format, and no Git status reached the
model, because the launcher disabled native's Git context.

3.10.0 reproduces native discovery on the target:

- **Discovery.** `instructions.mjs` finds the files and hands them to
  native's own renderer through `prompt.context`'s `instructionFiles`; runtime
  files never count.
- **Nested files.** The target equivalents follow a facade Read once per
  conversation (main and each agent), with their own imports.
- **Environment.** The environment block has native's form, with target
  facts.
- **Git.** Native's Git context is enabled again: its `gitStatus` section
  reads the target repository, and outside one it is dropped. The git commit
  and PR instructions return to the system prompt as natively.
- **Label.** The `tool.call hook additional context:` label that Mods put
  before this module's context is removed. Nested files and PostToolUse
  feedback therefore read as native's (3.7.0 showed the label).

**Behavior change.** Remote sessions no longer read `AGENTS.md`, as a local
Claude Code session does not. A project that relies on `AGENTS.md` for Claude
should reference it from `CLAUDE.md` (`@AGENTS.md`), as it would locally.

Packaged acceptance checks:

- the order and framing of the target files, including the parent
  directory's, an import, `.claude/CLAUDE.md`, a rule and `CLAUDE.local.md`
- that neither a scoped rule nor `AGENTS.md` loads
- the environment and Git blocks
- a Read bringing the nested `CLAUDE.md` and the matching scoped rule once,
  without the label

Gaps:

- Each nested file natively gets its own `<system-reminder>`. Here they share
  one, separated by a blank line.
- The target's Git section goes into native's session context, and is added
  when native had no Git section of its own. If native emits no session
  context at all, the section is absent; the acceptance runtime directory had
  one.
- HTML comment stripping and import edge cases (depth beyond five,
  non-text files) follow a reading of native behavior rather than
  measurement.
- Files are a session-start snapshot, as natively.

Ten native review rounds were run, and the last reported none. Fixed findings:

- deduplication per conversation, serialized for parallel Reads
- brace and class globs, and quoted, multi-line and commented `paths` lists
- imports of nested files and of rules
- symlinked rule directories and files
- a user's scoped rules matched relative to the project, and collected once
- read and listing failures stopping the session instead of silently
  dropping instructions
- a failed nested load reported to the model instead of looking like no
  instructions
- a Git status that cannot be read reported as unavailable, not clean
- the target's Git section added when native had none

The [3.10.0 release receipt](experiments/claude-target-instructions-release-2026-10-07.json)
binds feature commit `f3ca57a8`, release merge `e2323171` and artifact
`sha256:f102b54e23792271f439d5b44a0201160fed67e5adcc97ed9a2d38041b97c888`.
It records:

- 77 accepted checks on the exact signed package
- 3.9.0/3.10.0 coexistence
- Linux and actual macOS probes
- three Controller reader roles, re-resolved because the active Controller had
  changed
- five public artifact digests
- Catalog `ready`

OVH operation `ovh-claude-code-3-10-0-converge` completed. Inventory reports
3.10.0 active, 3.9.0 retained for rollback and no session leases. No live session
was restarted.

### 3. Native agents and orchestration

Codex already selects the execution environment at thread start and every turn.
Determine whether native child creation inherits this selection; prove it with
distinct runtime/target markers. If it does not, use the narrowest native child
creation extension. Claude's Mod `agent.spawn` is a candidate for binding and
context propagation. The pinned child callback research below establishes an
explicit Read route; implicit IO still needs separate acceptance.
Never assume a parent interceptor automatically covers descendants.

#### Claude native Agent research (2026-10-06)

The offline `execution-claude-agent-research` recipe uses Claude 2.1.287,
executor 0.159.3, a resident keeper and a disposable copy of the shipping Mod.
It enables Agent only in that disposable fixture. Native child `Read` reaches
`tool.call` with an `agentId`, reads target bytes through the keeper, and retains
target instructions and cwd. Runtime workspace bytes and instructions do not
reach the child. Native asynchronous `system/task_notification` delivers the
child result. Scripted API routing identifies the child by its prompt rather
than assuming parent/child request order. Completion matches the native task ID
in both already consumed frames and subsequent frames: a fast child can finish
before the parent result. Readiness uses `/cost` without an API request. These
are native runtime tests with zero real model requests.

The baseline also exposes runtime-home task-output paths in model context and
completion frames. Adding `--project-task-output` projects the native Agent
result's `outputFile` and exact corresponding text to `cowboy-agent://<agentId>`.
This removes that path from model context while preserving target Read and native
completion. It does **not** implement handle reads or project outgoing completion
frames: their `output_file` still names the runtime-home file.

Before production enablement, retain an exact native task-to-handle registration
scoped to session/binding/generation, project client completion frames using that
registration, and route handle reads to the owning task. Preserve child `agentId`
at the production bridge boundary and partition cancellation by child ownership;
the current bridge drops it. Validate permission modes, cancellation races,
resume, descendants, worktrees and implicit IO. Do not replace arbitrary path
substrings in user content or infer ownership from a task ID alone.

Run `just execution-claude-agent-research KEEPER CLAUDE CLAUDE_SHA EXECUTOR
EXECUTOR_SHA NEW_RECEIPT` inside the pinned shell, once without and once with
`--project-task-output`. The recipe isolates network and PID namespaces. Its
Unix socket must use `/tmp/cowboy-claude-mod-*`; inherited Nix temporary roots do
not satisfy the shipping bridge validator. Disabling all nonessential Claude
traffic also blocks Mod Unix HTTP, so this fixture uses individual telemetry
switches instead. Those setup failures are not evidence of native incompatibility.

Evidence: [baseline and projection receipts](experiments/claude-native-agent-research-2026-10-06.json).
This is same-host shared-filesystem research with a fixture Read bridge and
full-access permissions, not acceptance of enrolled worker transport, cross-host
execution, autonomous model continuation, or production Agent support. No
production runtime or capability admission is changed by this experiment.

#### Native Agent output follow-up (2026-10-06)

Extending the same fixture past completion exposes another boundary. A
session-local map can translate an exact owner-registered handle back to native
Read and recover the child's answer, but that file is a JSONL transcript. Its
rows retain runtime `cwd` and original environment attachments even when the
child's actual API requests received target context. Reading it into the parent
reintroduces runtime context. Locator projection alone is therefore insufficient.

Two alternatives were exercised on the pinned binary. `TaskOutput` is absent
from the available native tools in this lane: a scripted invocation reports
`No such tool available: TaskOutput`, even with the name in `--tools`. A public
type declaration is not proof of runtime admission. A native `turn.complete`
observer does supply the child's final answer and `agentId`. The research Mod
can return that answer through an owner-registered Read handle without loading
the raw transcript. This preserves native completion delivery and adds no model
requests for collection. The guard requires an answered, non-aborted turn.

Use `--project-task-output --read-task-output` for the raw-file control, add
`--native-task-output` for the unavailable-tool observation, or add
`--completion-output` for final-answer collection. These are mutually exclusive
output alternatives. The output probe waits for its own observed tool request
and response; an earlier background result can otherwise satisfy the next
`prompt()` wait and produce an empty success.

The successful final-answer read still does not project native notification
paths. A subsequent parent request includes the runtime-home output locator from
the native completion notification; the initial tool-result projection only
covered earlier requests. Output content and notification delivery need separate
projections. The fixture's in-memory registration is not a durable production
task registry. Partial output, error/refusal/cancellation outcomes, reload/resume,
foreign-owner rejection and cross-host operation still require acceptance.
Do not present a final-answer-only handle as a complete native output stream.

Evidence: [five output-path observations](experiments/claude-native-agent-output-research-2026-10-06.json).
Production Agent admission remains unchanged. No raw-transcript parser or new
production restriction was introduced.

#### Native agent admission (Plugin 3.5.0)

Plugin 3.5.0 replaces the research splice with a production path in the shipping
Mod, bridge and facade; CLI 2.1.287, ACP 0.84.0 and executor 0.159.3 are unchanged.
A native-local baseline was measured first with native Bash and an observer-only
Mod: interrupt (active or idle parent) and TaskStop both stop background agents
without a further model request, and `TaskOutput` is absent from the inventory
while `SendMessage` is present. See the
[lifecycle research receipt](experiments/claude-native-agent-lifecycle-2026-10-06.json).

| Surface | Native local | Remote 3.5.0 |
| --- | --- | --- |
| Launch | Async agent; result names a runtime-home JSONL `output_file` and tells the model not to read it | Same native launch; that exact registered locator becomes `cowboy-agent://<agentId>` in the result, in idle (`prompt`) and mid-turn (`delivery`/`queued_command`) notifications, and in client `task_notification` frames |
| Child tools | Run locally | Every child call carries `agentId` through the bridge and runs on target; target guidance and cwd reach the child |
| Output read | Raw transcript file | Read on the handle returns the recorded final answer (or stopped/failed/running status); partial output is explicitly unavailable |
| Continue | `SendMessage` to the agent | Same, limited to this session's registered agents and without cross-session idle subscriptions |
| Stop | TaskStop; child tools aborted | Native TaskStop, then the bridge cancels every target call that agent still holds; an aborted child `turn.complete` does the same |
| Interrupt | Stops turn and background agents | Forwarded to native first; target foreground processes are cancelled on its acknowledgement (or after 5 s) so a held child call cannot hand an exit status to a still-running agent |
| Resume | Earlier agents end with the process; a stale notification may settle with zero turns | Outcomes persist in the session state; a running agent from an earlier process reads as ended; readiness tolerates only a zero-turn, zero-API notification result |

Refused with a reason instead of silently running elsewhere: `isolation`
(`worktree` creates a runtime-side worktree, `remote` another environment),
`run_in_background:false`, agent types outside general-purpose/claude/Explore/
Plan, and `SendMessage` to non-registered recipients or with
`notify_when_idle`. These are untested surfaces, not native impossibilities.

Two runtime defects were found and fixed while accepting this. First, the
pinned runtime processes no other native work while a Mod hook's `$.http.fetch`
is pending: with the previous 20 s idle hold a text-only parent turn waited 25 s
behind a held child call, and TaskStop took 18.5 s in packaged acceptance. The
bridge hold is now 1 s; ready results still return immediately. The same parent
turn then takes 0.1 s and TaskStop 0.42 s. This also affects 3.4.x sessions
with any long facade command, not only agents. Second, cancelling target
processes before forwarding an interrupt let a held child call return exit 137
to a still-running agent, which issued another model request.

Four native Codex review rounds found further cancellation and state defects,
each fixed with a deterministic regression: a continued agent is durably
`running` until its next outcome (and a stale expiry flag no longer hides a new
answer); cancellation is rechecked after start persistence, before a queued
mutation begins and after directory creation, so an abandoned call submits no
command or write; a background start whose result native discarded is still
cancelled; and agent answers yield to the whole state file's cold-load bound.
The fifth round reported no defect. Native writes already in flight complete.

Packaged worker acceptance on the exact candidate passes 51 checks (45 Claude,
6 transport), including the nine `native_agent_*`/interrupt checks and
`parent_turns_and_taskstop_progress_during_child_command`. A negative control
that restores only the 20 s hold fails that check. Earlier candidate runs also
observed the unprojected queued notification, the uncancelled child after
TaskStop and the post-interrupt child request before their fixes.

The [3.5.0 release receipt](experiments/claude-native-agents-release-2026-10-06.json)
binds source `3d38ed74`, artifact
`sha256:50019180ae347da5d40bd1cea739c9871406bf735877805a0be2a48bd9e37dd0`, the
51-check acceptance of that exact package, 3.4.12/3.5.0 worker coexistence,
Linux and actual macOS arm64 probes, three Controller reader roles and five
public artifact digests. OVH operation `ovh-claude-code-3-5-0-converge`
completed with an applied Machine receipt; re-read inventory reports 3.5.0
active, 3.4.12 retained for rollback and current authentication and
materialization. No live session was restarted and no production inference
was sent.

Not established: partial/streaming child output, grandchildren and teammates,
permission modes other than the bound bypass mode, native-runtime or keeper
crash with a live agent, cross-host latency and real-model continuation. A
stopped agent's own background (`run_in_background`) jobs keep their retained
handles; native behavior for that case was not measured.

#### Native permission modes (Plugin 3.6.0)

Before 3.6.0 the bound launch forced `bypassPermissions`, and the facade answered
`tool.call` itself, which skips native permission evaluation. Users who switched a
remote session to another mode still got bypass behavior for target tools. Cowboy
continues to select bypass by default, so default sessions are unchanged.

The pinned 2.1.287 types document `$.tool.check({tool, input})`: the engine's own
rules-and-mode decision, with no tool body or dialog. Offline baselines with
native local tools showed it agrees with native prompts in `default` and
`acceptEdits`: every `ask` matched a real native `can_use_tool` request and every
`allow` matched none. In `dontAsk` it still reports `ask` while native denies
silently, so the launcher converts that case to denial.

Native judges paths on its own filesystem. File-tool paths are therefore
resolved as target paths (target cwd, `..`, target home): inside the target
workspace they are checked at the runtime-workspace equivalent, everything else
under a root native can never treat as its workspace or home. Native review
found that the first, partial mapping let a target path that collided with the
runtime workspace be auto-approved; the packaged collision case and a negative
control now cover it. The arguments as written are also checked, so a rule
naming a target path still decides and any deny wins.

Native-local baselines showed symlinks matter in every mode: a `Write` onto a
symlink is refused ("Write to the link's target path instead"), and edits,
reads and Bash paths that resolve outside the workspace lose workspace-scoped
auto-approval. Remote `Write` reproduces the refusal from target metadata (a
behavior change: 3.5.0 wrote through such links). When an allowance depends on
the path being inside the workspace, the target real path is resolved and
checked; an outside or unresolvable destination asks.

| Mode decision | Native local | Remote 3.6.0 |
| --- | --- | --- |
| allow | Tool runs | Target tool runs; no host request |
| deny (rule) | Refused | Refused before any target effect |
| ask | `can_use_tool` to the SDK host | Same request shape from the launcher (`tool_use_id`, `agent_id`, `decision_reason`), matched by a private request id; deny has no target effect, `updatedInput` is executed |
| ask in dontAsk | Denied without a request | Same |
| abandoned while asking | Request withdrawn | `control_cancel_request` to the host; the call never runs |
| host `setMode` / `interrupt` | Applied by native | Applied to native through its own control requests |

Gaps, by design rather than impossibility: permission suggestions are sent
empty because an approved persistent rule could not reach native's own rule
store, so "always allow" is not offered; auto mode's classifier is not asked
(`check` excludes it), so those calls prompt instead. Bash command strings are
not rewritten: target-absolute paths in commands may prompt where a local run
would not, a command naming the runtime workspace asks unless native would allow
it regardless, and in `acceptEdits` a filesystem command through a target
symlink escaping the workspace is auto-approved where native-local asks. That
last case is a residual safety gap; native exposes no paths for an allowed
command, and a `cd` probe cannot separate it because native asks for the `cd`
itself. In non-bypass modes each in-workspace path call may cost one target
`realpath`. The ACP permission preview may read runtime paths for diffs. Plan
mode remains refused.

Source tests cover decisions, path mapping, approval polling, cancellation and
the request shape. Packaged acceptance switches modes through the host and
checks target bytes for denial, amended approval, read-only allow, acceptEdits
with relative and target-absolute paths, an outside-workspace ask, a runtime
path collision, a write under an escaping directory symlink, dontAsk, bypass and
Write onto a symlink. Disposable candidates without the gate, or with the
earlier partial path mapping, fail. Four native review rounds found and fixed
the collision, rule-matching, mode-blind command and symlink defects; the last
round reported none.

The [3.6.0 release receipt](experiments/claude-permission-modes-release-2026-10-06.json)
binds source `4c10a6f0` and artifact
`sha256:a3a9012fdb45c1f22268718789cbd71567729eca152d924791fee49dab1f2975`: 59
packaged checks on the exact package, 3.5.0/3.6.0 worker coexistence, Linux and
actual macOS probes, three current Controller reader roles and five public
artifact digests. OVH operation `ovh-claude-code-3-6-0-converge` completed;
inventory reports 3.6.0 active with 3.5.0 retained for rollback. Cowboy's
worker still selects bypass at every Claude start, so default sessions keep
their prior behavior; no live session was restarted.

#### Project hooks research (2026-10-06)

Target project hooks are still not loaded in the Claude remote lane (empty
setting sources). Measurements on 2.1.287 decide what a faithful design can
use. Hooks passed with `--settings` do run in this lane, and lifecycle events
(SessionStart, UserPromptSubmit, Stop and others) are independent of tool
bodies. The shell prefix receives each hook command as one argument with stdin
piped, so an exact-command proxy could run registered target hook commands on
the target while leaving runtime-owned commands local. Hook stdin still names
the runtime `cwd` and transcript.

Tool hooks are the obstacle. Settings PreToolUse/PostToolUse run only when the
`tool.call` chain reaches core, and the facade answers target tools before
that. A Mod proxy invoked with `$.tool.call` reaches the settings hooks only if
no Mod answers its body; answering the body suppresses them, and a proxy hidden
from the model is not callable. Native tool hooks therefore cannot wrap the
target file tools on this build. Options: redirect native Bash bodies to the
target through the shell prefix (Bash only, with the snapshot/cwd/cancellation
work recorded above), reproduce the documented hook input/output contract in an
adapter for target tools, or keep hooks unloaded. Loading only lifecycle hooks
would silently skip a project's PreToolUse guards and PostToolUse formatters,
so no partial loading was shipped. Evidence:
[project hooks research](experiments/claude-project-hooks-research-2026-10-06.json).

#### Project hooks (Plugin 3.7.0)

3.7.0 loads the target project's hooks: `.claude/settings.json` and
`settings.local.json`, read from the target when the session starts. Two paths
run them, and both execute every command on the target:

- Native runs lifecycle hooks and hooks on its own tools (Agent, TodoWrite,
  AskUserQuestion). The settings reach native with `--settings`, and
  `CLAUDE_CODE_SHELL_PREFIX` hands each command to `hook-proxy.mjs`, which runs
  only registered commands on the target. Stdin, the streams, the exit code,
  the timeout and cancellation are forwarded. Exec-form hooks bypass the prefix
  natively, so they are given to native as an equivalent quoted shell form and
  run on the target as argv.
- The facade answers target tools before native's tool hooks, so `context-mod.js`
  runs PreToolUse, PostToolUse, PostToolUseFailure and PermissionRequest for
  them. It reproduces native's input fields, matching, parallel execution and
  folding, and the messages the model sees.

Behavior was taken from offline native baselines on 2.1.287 rather than from
documentation; see the
[hook baselines](experiments/claude-project-hooks-baselines-2026-10-06.json):

- Exit 2 and `deny` in PreToolUse refuse the call before any effect. `ask`
  prompts even in bypass mode. `continue:false` still runs the tool, then ends
  the turn; this also applies when the call fails.
- PostToolUse feedback follows the result. PreToolUse context also follows a
  failed call.
- PostToolUseFailure runs for target errors, and its `continue:false` does not
  end the turn.
- A non-zero Bash exit is a tool error (`Exit code N` followed by the output),
  so failure hooks run instead of PostToolUse. This changes the result shape
  for failed commands: 3.6.0 reported them as successes with a trailing
  `Exit code: N`.
- PermissionRequest hooks race a pending host prompt. A hook decision withdraws
  the prompt (`control_cancel_request`) and allows the call (with amended input
  if given), denies it, or denies it and ends the turn. Hooks also race each
  other: the first decision wins, and a deny wins over an allow that arrives at
  the same time. The adapter sees decisions at its next bridge poll, so
  "the same time" is a window of up to about one second rather than native's
  single tick. Exit 2 or no decision leaves the prompt to the host.
- Async hooks run in the background. Their context reaches the next request.
- A subagent's hooks carry `agent_id` and `agent_type`, and, as natively, the
  main session's `transcript_path`. A transcript is copied to the target only
  from native's projects directory and is removed afterwards.
- `CLAUDE_ENV_FILE` is a private target file that facade Bash sources first.
- An unparsable settings file is skipped, as native skips it.

The adapter refuses a call rather than skip a project guard or reviewer. A
matching tool hook it cannot reproduce makes the call fail before any effect:
`prompt`, `http`, `agent` or `mcp` types, an `if` condition, or `asyncRewake`.
A PreToolUse hook that cannot run on the target denies the call, as does a
settings read error other than a missing file. A PermissionRequest hook that
cannot run decides nothing, so the host still asks. Hook temporary files live
under a private `~/.cache/cowboy/hook-input` directory (mode 0700) and are
removed on every path. An abandoned call cancels its hooks.

Native review ran 15 rounds, and the last reported no findings. Fixed findings
include:

- fail-closed handling of unsupported hook types
- hook cancellation
- transcript copies
- async delivery
- base input fields
- timeouts
- private temporary files
- the live permission mode
- PostToolUseFailure
- failed Bash dispatch
- PreToolUse stop and context on failure
- PermissionRequest decision order

Three findings were declined because native baselines contradict them:

- a PreToolUse `continue:false` stopping before execution
- a subagent getting its own transcript path
- aborting the session on an unparsable settings file

The baselines taken for those findings also exposed PermissionRequest hooks and
`agent_type` as missing; both are now implemented.

Gaps:

- Settings are a session-start snapshot. Native watches its settings files;
  this lane does not watch the target's files.
- `PermissionDenied` (fired by the auto-mode classifier, which this lane does
  not use) and `updatedPermissions` (which cannot persist rules, as in 3.6.0)
  have no effect.
- For events native raises itself (for example PostToolBatch or PreCompact),
  the proxy rewrites `cwd` to the target and copies the transcript. Other
  runtime-local paths those inputs may contain are not translated, and each
  event's fields were not measured individually.
- Facade Bash still reports a successful command with a trailing `Exit code: 0`,
  where native prints only the output, or `(Bash completed with no output)`.
  This belongs to matrix A.

Packaged acceptance adds hook checks for:

- lifecycle and native-tool hooks running on the target
- SessionStart `CLAUDE_ENV_FILE` reaching target Bash
- a facade PreToolUse block before the effect
- PostToolUse context and async context
- PostToolUseFailure for Read and Bash
- non-zero Bash as a tool error
- a PermissionRequest hook answering a pending prompt
- a subagent's hook input naming its agent
- private, cleaned hook inputs, with no hook running on the runtime

Two negative controls fail: a candidate without the shell prefix, and one
without the facade adapter.

The [3.7.0 release receipt](experiments/claude-project-hooks-release-2026-10-06.json)
binds feature commit `758aaeb9`, release merge `0d811d54` and artifact
`sha256:beae86098c8e4614db0f87e0b00ae61f9af27bf3eb287d63319907097d3f6426`.
It records:

- 68 accepted checks on the exact signed package
- 3.6.0/3.7.0 coexistence with the current Machine worker
- Linux and actual macOS probes
- three Controller reader roles, re-resolved because the active Controller had
  changed
- five public artifact digests
- Catalog `ready` on both platforms

OVH operation `ovh-claude-code-3-7-0-converge` completed. Inventory reports
3.7.0 active, 3.6.0 retained for rollback and no session leases. The converge
was bounded to claude-code; other pending upgrades (grok, zed) were left alone.
No live session was restarted.

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

### Incremental checks for later upgrades (2026-10-08)

The measured native behaviors this lane relies on are kept as baselines with
probes; the newest group is also pinned by unit tests:

| Behavior | Baseline (probe) | Compared by |
| --- | --- | --- |
| Tool descriptions, background deadlines, Bash stdin, MCP scopes, agent background notifications | `claude_native_behavior_baseline.json` (`claude_native_behavior_probe.py`) | unit tests in `claude-remote-native-baseline.test.mjs`; packaged descriptions check |
| Bash command line, environment, snapshot | `claude_shell_native_baseline.json` | packaged `shell` phase |
| Process lifetimes and stops | `claude_lifecycle_native_baseline.json` | packaged `lifecycle` phase |
| File reads | `claude_file_native_baseline.json` | packaged `files` phase |
| PDF and file-type reads | `claude_pdf_native_baseline.json` | packaged `pdf` phase |
| Skills and commands | `claude_skill_native_baseline.json` | packaged `skills` phase |
| Project hooks: outcomes, matchers, PermissionRequest race, shell prefix, agent hook input | `claude_hooks_native_baseline.json` (`claude_hooks_native_probe.py`) | unit tests in `claude-remote-hooks-baseline.test.mjs`; packaged `hooks` phase |

`tools/remote_impact.py` turns a Git diff into the remote suites, Claude
phases and native probes to run, from `tools/remote_check_map.json`; it covers
the Claude and Codex lanes, the shared execution transport, keeper and Machine
control, and every native executor probe. The Claude conformance harness
accepts a `"phases"` subset. Against main's history it selects nothing for the
3.19.0 and 3.19.1 version-only releases, the full Claude run for the 3.18.0
change to `tools.mjs`, and the Rust tests, both worker suites and the session
gate for the `machine_transport` heartbeat fix. The flow is in
[remote checks by change](../.agents/skills/release-cowboy-plugin/references/remote-checks.md).

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
