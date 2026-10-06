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
| Shell environment | Claude 3.8.0 runs native's command shape on the target: user bash/zsh, a login-shell snapshot (rc, functions, options, aliases, PATH), `cd` persistence with native's reset, native's environment variables; 36 Bash cases match native-local results in packaged acceptance | Error results keep Mods' `<tool_use_error>` wrapper; `CLAUDE_EFFORT` starts after the first tool batch; no embedded find/grep/rg shadows or `CLAUDE_PID` |
| Project hooks (Claude) | 3.7.0 runs target project hooks: native lifecycle/native-tool hooks through the shell prefix, facade tool hooks (PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest) through the adapter | Settings are a session-start snapshot; non-command facade tool hooks refuse matching calls |
| Project hooks | Claude launch suppresses settings sources; Codex remote hook placement not established by current receipt | Execute target-owned hooks at target, preserve native lifecycle/decisions and trusted configuration; separate runtime-owned hooks |
| Hook types | Command, HTTP, prompt/agent and MCP forms have different ownership and provider support | Inventory exact installed schemas; keep native model evaluators and approval semantics, bridge only external execution/IO |
| Permission modes | Claude 3.6.0 drops the forced bypass: native `$.tool.check` decides each target call under the session's mode and rules, asks reach the SDK host in native `can_use_tool` shape, denial precedes any target effect, amended input runs, dontAsk denies, and abandoned asks are withdrawn (packaged acceptance) | No "always allow" rule persistence, auto-mode classifier or command-string path mapping; plan mode stays refused |
| Native agents | Codex fresh and fully forked children inherit target guidance and route direct/CodeAct commands through the keeper in pinned native acceptance. Claude 3.5.0 admits native background subagents: child calls carry `agentId`, launch/notification/client locators use `cowboy-agent://`, outcomes are durable across resume, TaskStop/interrupt cancel the child's target commands (packaged worker acceptance) | Partial output stream, permission modes, grandchildren, teammates, worktree/remote isolation, custom agents and native-runtime crash with a live agent remain unaccepted; those inputs are refused |
| Skills and project plugins | Claude Skill restricted; implicit local discovery is not target-aware | Target-authoritative discovery with versioned metadata, trust and native expansion; route script execution separately |
| MCP and web/browser tools | Claude bound allowlist chiefly admits Matrix plus owned tools | Classify runtime/service/target placement per server/tool; preserve native discovery/auth/elicitation, without moving all MCP servers to target |
| Plans and task artifacts | Plan tools restricted in Claude lane | Separate runtime transcript from target plan/artifact storage; preserve native approval and resume semantics |
| Images, notebooks, PDFs | Images/notebooks have evidence; Claude describes PDF via target utility | Add native-parity PDF/pages, binary limits, image dimensions, output artifacts and user-upload placement tests |
| File semantics | Stale edits, CRLF, Unicode, quoted paths, ranges tested | Add symlink races, rename/unlink races, case sensitivity, modes, hard links, nonregular files, encoding and concurrent writers |
| Atomic mutations | Stale stamp refusal has evidence; it does not alone establish compare-and-write atomicity | Inspect target implementation and inject mutation between check and write; use target-side atomic primitives where promised |
| Output limits | Large streams/backpressure and terminal events recorded | Test split UTF-8, binary/NUL, truncation markers, slow/absent reader, disk full and retained output expiration |
| Lost replies | Lost start and outages recorded without replay | Distinguish rejected, accepted, unknown and completed effects; never resend an unknown mutation under a new identity |
| Cancel and timeout | Foreground cancellation and retained job cancellation recorded; 3.5.0 adds per-call and per-agent cancellation and forwards interrupt to native before target cancellation | Test cancel/start races, whole process trees, detached children, late completion, keeper death and provider timeout semantics |
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
  handles and give no completion notification. The text follows native's,
  without its promise of a notification. Notifications belong to matrix E.

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
