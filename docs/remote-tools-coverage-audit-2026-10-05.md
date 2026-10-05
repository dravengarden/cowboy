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
| Native CodeAct | Codex has native orchestration; Matrix execution tests are not proof of arbitrary native nested-tool parity | Force nested shell, patch, image, parallel and failed calls through scripted native API; inspect actual target effects |
| Claude tool dispatch | `context-mod.js` replaces native tool bodies with a facade | Restore native ownership where a lower-level boundary exists; retain independently tested file adapters |
| Native Bash lifecycle | Facade retains processes but does not establish native background task registration | Explore shell prefix bridge preserving original Bash tool and native task registry |
| Background completion | Native activity UI support exists; facade handles are a separate system | Validate completion after prompt return, native autonomous continuation, output retrieval, task count and cancellation |
| PTY and stdin | Claude facade explicitly uses `tty:false`, `pipeStdin:false` | Native-parity baseline first: do not invent PTY support where provider lacks it; test Codex PTY, resize, EOF and incremental input |
| Shell environment | Claude starts a new shell with fixed binding cwd and target environment | Test `cd`, exports, login startup, shell snapshots, quoting, signals, pipe status and shell availability; preserve documented native persistence semantics |
| Project hooks | Claude launch suppresses settings sources; Codex remote hook placement not established by current receipt | Execute target-owned hooks at target, preserve native lifecycle/decisions and trusted configuration; separate runtime-owned hooks |
| Hook types | Command, HTTP, prompt/agent and MCP forms have different ownership and provider support | Inventory exact installed schemas; keep native model evaluators and approval semantics, bridge only external execution/IO |
| Permission modes | Claude bound launch selects `bypassPermissions`; remote tools must not be presented as supporting every native permission mode | Design explicit mapping for approval, deny, modified input and concurrent approval cancellation; test denial before any target effect |
| Native agents | Claude Agent/Task paths restricted; Codex nested agents explicitly not checked | Inherit binding and policy into every child; separately test foreground, background, fork, teammate and worktree paths |
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
