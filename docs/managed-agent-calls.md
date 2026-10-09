# Managed agent calls

Status: implemented in source; see "Implementation" for the shipped mechanism
and "Acceptance" for what has been proven in production.

## Implementation

- **Grant.** The Controller derives `Authority` from the parent's live worker
  (connected owner, worker epoch, exact launch, owner, Provider generation and
  execution binding) and issues an opaque grant for that incarnation. It installs
  the grant on the parent's execution Machine (`InstallCallGateway`, Machine
  protocol 28) and revokes it when the authority changes. Every forwarded action
  is accepted only from that Machine's current connection, for the grant
  installed there, after re-deriving authority and comparing it with the issued
  one. Native session-id materialization does not revoke; worker replacement,
  owner or generation change and a closing parent do.
- **Ingress.** `cowboy-machine` hosts one private Unix-socket gateway per
  grant. A stable per-session context file
  (`<state>/calls/sessions/<session>.json`) is atomically replaced when a grant
  changes. Local parent workers receive `COWBOY_CALL_CONTEXT` from the broker;
  Remote parents receive it from their target execution keeper, which adds it to
  `process/start` only on the outbound native frame, so ledger replay still
  compares the original parameters. Operator target environment policy is
  unchanged. Machine→Controller requests are `ManagedCall` events answered by
  `ManagedCallReply`; neither read loop awaits the other side.
- **Snapshot.** The target Machine captures `HEAD`, the index and the working
  tree (tracked plus non-ignored untracked files and explicitly named context
  files) as two synthetic commits in a child-owned repository that borrows the
  source objects through `alternates`. It captures twice and refuses a mixed
  state (`input_changed`); filters/LFS, submodules, escaping symlinks and
  subdirectory roots are refused. `context.root` may name another work tree of a
  repository registered on that Machine. The child workspace path is stable; a
  continued conversation is hibernated and refreshed in place, so its binding
  never changes. Repeating a call id returns the original receipt.
- **Child.** A managed child is an ordinary Cowboy session with a
  `managed_child` execution binding on the target Machine. Client prompt entry
  points refuse it; the runner submits its single prompt with a reserved
  message id. The worker forwards the Machine-written round as ACP prompt
  `_meta["cowboy.dev/managedCall"]`; a turn without it is refused.
- **Native profile.** Only Providers whose signed package declares
  `provider.managed-profiles.v1` with `read_only_v1` arguments can run a child.
  Codex 3.4.0's adapter forces `approval never` and a read-only, network-less
  sandbox on every managed turn, maps `outputSchema` to the native
  `turn/start` constraint, refuses mode changes and slash commands, never trusts
  the snapshot project (no project config, rules, hooks or MCP), disables every
  configured MCP server by exact name, disables hooks, plugins, apps, notify and
  native memories, and refuses to start while any ExecPolicy `allow` rule exists
  (those run commands outside the sandbox).
- **Split placement.** Execution never moves off the parent's execution
  Machine. When host policy places the Provider's runtime elsewhere (Claude is
  pinned to OVH), a Machine allowed to run it — the parent's runtime first —
  runs the child against that Machine's snapshot. Both Machines must speak
  protocol 29 and the exact Provider generation must accept the target executor.
  The runtime Machine prepares a private entry (`PrepareRuntime`); the target
  starts a keeper over the snapshot (`PrepareManagedEnvironment`) and returns an
  ordinary remote binding whose `managed` field names the parent and profile.
  Older readers refuse that field. The keeper forces the pinned executor's own
  read-only, network-free sandbox onto every `process/start` (whatever sandbox
  the caller asked for), answers `fs/writeFile`, `fs/createDirectory`,
  `fs/remove`, `fs/copy`, `http/request` and unknown methods with an ordinary
  error, and announces `cowboyManaged {profile, roundPath}` in its native
  initialization. The runtime worker reads the round through the keeper before
  the first prompt. Claude 3.19.8 accepts a managed launch only when the binding,
  its signed profile flag and the keeper announcement agree. It then reads the
  round's schema through the keeper and runs native Claude in `dontAsk` mode
  with Bash/Read/Glob/Grep and its task list, without hooks, skills, MCP
  servers, memory, nested agents or web tools, using `--json-schema`. Native
  validates the result through its `StructuredOutput` tool, and the launcher
  delivers `structured_output` as the turn's final message. A managed Claude
  launch without an execution environment fails closed.
  `just execution-managed-conformance <claude input> <new receipt>` runs this
  path offline end to end. It uses a real Machine `Manager` snapshot of a dirty
  work tree, `PrepareManagedEnvironment` (idempotent; an ordinary `Prepare` of
  the same session is refused), the pinned executor behind the keeper, the
  worker transport, and the worker's round read. It then runs the packaged
  launcher with native Claude against a scripted API, and requires:
  - the permission-mode change is refused;
  - the advertised tool surface is read-only;
  - workspace and `/tmp` writes fail inside the sandbox while snapshot reads
    see the working tree;
  - native `StructuredOutput` is delivered as the final message;
  - closing the child removes the snapshot.
  It uses no credentials or model.
- **Lifecycle.** One Controller runner per call drives Queued → Starting →
  Running → terminal through CAS transitions, records the child event cursor
  before submitting, resubmits only a prompt with no trace under the same id,
  keeps a completion that wins a stop race, and stops only the exact child
  worker after a bounded grace. Controller start recovers every non-terminal
  call. A deleted or closing parent cancels its calls; its children are deleted
  once those are terminal. Deleting a child with an active call is refused.
- **UI.** The Prompt stack shows a Calls dock (Desktop `␣G`, J/K/Enter; Mobile
  full-height page with drill-in). Each call shows Provider, runtime and
  execution Machines, state, labels, snapshot, result, verdict and findings,
  stop and open-conversation actions. Children are hidden from the session list.

## Product contract

A parent agent can delegate bounded work to another installed Provider through
`cowboy codex` or `cowboy claude`. Cowboy owns dispatch, placement evidence,
observation, cancellation and the parent/child relationship. The Provider owns
the native conversation and model/tool execution. The originating project's
workflow owns the task, prompts, rounds, findings, acceptance and durable project
artifacts. Cowboy does not implement a second Spec Kit or a generic agent planner.

The default placement is the parent's execution target. For a Claude session
whose runtime is OVH and execution target is Hawk, the child Codex runtime and
tools are on Hawk. This is intentional target-local execution, not another OVH
Remote session. No prompt needs to know where its parent model process runs.
The symmetric Codex-to-Claude case has the same rule. A missing target Provider
or authentication returns an actionable refusal; it never silently runs elsewhere.

### Ownership and native coverage

Native Codex supplies conversations, review/model execution, sandboxing and
native subagents; Claude supplies its corresponding conversation/tool surfaces.
Cowboy already supplies signed Providers, Machine placement, ACP workers,
session event streams, authorization and lifecycle supervision. Extend these
existing surfaces instead of starting an untracked CLI or a second supervisor.
The residual gap is a parent-scoped cross-Provider launch and observation contract,
including the target-shell ingress and UI relationship. Remove the adapter when
native cross-Provider delegation exposes equivalent placement, authority,
idempotency, cancellation and observation across Cowboy's enrolled Machines.
Claude is adapter-backed; common request/lifecycle/UI semantics are shared.
Provider-specific unsupported capabilities remain explicit refusals.

## Authority and discovery

Ordinary product login and private Operator Plugin-install delegation do not
implicitly grant a running agent permission to impersonate an arbitrary parent.
Do not reuse browser cookies, Operator credentials, Provider credentials, an
execution-keeper capability, or a caller-authored `parent_session_id`.

The session supervisor issues a separate, revocable call grant scoped to the
Service, parent session, parent execution binding revision, target Machine and
allowed operation set. The grant delegates no more than the parent's authority.
The target receives a private local gateway binding through the existing
authenticated Machine channel. A CLI invocation reaches that gateway using
session-scoped runtime wiring installed by Cowboy, not `cwd` or transcript
inspection. The Machine authenticates the call context and forwards the request;
the Controller independently validates its scope and current revocation state.
Neither the grant nor Provider authentication appears in argv, reports or logs.

The parent's native conversation ID is not its authorization revision: normal
startup assigns that ID after admission. The grant must separately fence the
active worker incarnation so that a retired worker cannot retain call authority
after Reload, while native conversation materialization alone does not revoke a
newly issued grant. A stable parent ID or Unix uid cannot replace that check.

The Controller now derives an authority observation from its current connected
runtime and exact worker launch. Broker placeholders, resetting/draining workers,
closed parents and changed launch metadata cannot produce it. A pending drain
request alone does not revoke it: the busy worker remains the exact owner until
its safe boundary, and its replacement epoch then revokes the grant. Comparing this
observation includes the worker epoch and parent ownership/placement revision;
turn changes, titles and native conversation materialization do not change it.
The grant issuer and every forwarded action use this check. The observation
API's `parent_runtime_ready` is advisory and never substitutes for revalidation
at dispatch.

Call list responses contain summaries with `has_result`, not full review bodies.
Opening a call uses its parent-scoped detail endpoint to read the complete result.
The store still validates the full durable records before projecting summaries;
the bounded response does not imply a separate lightweight storage index.

Runtime-only environment injection is separate from operator-configurable target
environment variables. The closed target environment still rejects `COWBOY_*`,
`CODEX_*` and `CLAUDE_*` from operators; the keeper adds only the call context
from its own launch contract. An old worker/keeper without this capability
fails with `context_unavailable`.
It must not infer the parent from a shared Unix uid, cwd, process name or last
active session. Local sessions and Remote targets need the same scoped contract;
a target-only implementation is not accepted as coverage of both.

The CLI capability query works without submitting a task:

```sh
cowboy call capabilities
```

It returns supported Providers, purposes, access modes, conversation modes,
limits and resolved default placement for this authenticated context. Outside a
managed context it explains that a Cowboy session is required. Ordinary human
client login is a separate explicit ingress, not an automatic fallback.

## Agent tools policy

Agent calls cost tokens, so they are off until a person turns them on. Cowboy
owns one two-layer policy per agent kind (`codex`, `claude-code`):

- **Global defaults** (`agent_tools:<agent>` in Hub settings; Settings → Agent
  tools; `GET /api/agent-tools`, `PUT /api/agent-tools/{agent}` for Owners).
  Built-in defaults: calls off, both targets allowed with the other agent kind
  first, default `auto`, at most 4 concurrent and 64 total calls per parent
  session; Matrix memory tools and automatic recall on.
- **Session override** (`session_tools:<session>`; the session's **Tools**
  section in the mobile session sheet and the desktop Run configuration modal;
  `GET`/`PUT /api/sessions/{id}/tools` for users who can mutate the session).
  It stores only the fields that differ; an empty override is removed, so the
  session follows later changes to its agent kind's defaults.

The effective policy is the override applied to the defaults. A target names an
agent kind and optionally a preset from that Provider's signed configuration
presets (model and reasoning). `cowboy codex`/`cowboy claude` request one kind;
`cowboy call start --provider auto` lets the policy pick: the configured default
when it is a kind, otherwise the allowed targets with a kind different from the
caller first, falling back to the next ready target. `continue` keeps the
child's original kind. The selection (`explicit`/`auto`) and preset are part of
the durable record, so a replay with the same `request_id` must match them.

Refusals happen before anything is recorded (`admission: not_submitted`):
`calls_disabled` when calls are off, `policy_denied` when the requested kind is
not an allowed target, `concurrency_limit` when the parent already has its
maximum running calls (safe to resubmit the same request later) and
`call_limit` when its total budget is spent. The limits are a soft admission
budget; concurrent submissions may pass them together. `cowboy call
capabilities` reports `enabled`, `default`, the limits and a per-Provider reason.

The runner applies the selected preset's exact value map as configuration
preferences for the child before its first prompt. The managed worker accepts
`session/set_config_option` only for option ids that the signed package's
presets declare and never persists them as the user's preferences; mode and
every other option remain refused. A preset missing from the exact admitted
Provider generation fails the call with `preset_unavailable`.

The Matrix switches are stored and displayed now; Provider adapters enforce
them in a later phase. Managed children have no Tools section; their policy is
fixed by the managed profile.

## CLI contract

Both Provider aliases use one parser and one wire schema. `claude` resolves to
the exact installed `claude-code` Provider; it never resolves a random executable
on PATH. Future Providers can use `cowboy call start --provider <id>` without new
protocol fields or mandatory root subcommands.

```sh
cowboy codex --request-file review.json
cowboy claude --request-file review.json
cowboy codex --request-file - < review.json
cowboy call inspect call_123
cowboy call wait call_123 --timeout-ms 30000
cowboy call result call_123
cowboy call cancel call_123
```

There is no interactive question on stdin and no shell prompt interpolation.
Stdout contains exactly one JSON envelope, stderr contains bounded diagnostics,
and an optional events command emits explicitly selected NDJSON. JSON errors use
the same envelope as successful responses. Unknown fields, unsupported schema,
unknown Provider options and oversized input fail before dispatch. Instructions
are text, never a shell command. Paths are interpreted on the execution target
and relative to the validated caller worktree; there is no implicit local/remote
file transfer. Input files are captured before dispatch and hashed in the receipt.

Example request (proposed schema 1):

```json
{
  "schema": 1,
  "request_id": "review-r2-security-01",
  "purpose": "review",
  "instruction": "Apply the supplied review prompt to this round's snapshot.",
  "context": {
    "scope": "current-worktree",
    "files": [".review-input/round-2-security.md"]
  },
  "access": "read-only",
  "conversation": {"mode": "fresh"},
  "labels": {
    "task": "CS-FWK-001",
    "group": "review-2026-10-08",
    "round": "2",
    "aspect": "security"
  }
}
```

`request_id` is an idempotency key within the authenticated parent, not an
authority or a native conversation id. Repeating identical immutable inputs
observes the original call; reusing the key with different inputs returns
`request_conflict`. Concurrent admissions use a durable uniqueness constraint.
Labels are bounded display metadata, never authorization, project identity or
workflow state. Purpose selects a supported launch profile, not a reviewer prompt.
Suger supplies the exact prompt and output requirements unchanged.

The initial call waits for a bounded interval (default 30 seconds), then returns
the accepted call id and `running`/`queued` state. This is a successful observation,
not a timeout error. `wait` observes the same call, never submits a new prompt.
Cancellation is explicit and idempotent. A failed transport with uncertain
admission tells the caller to inspect the same request id. It must never return
an instruction to mint a replacement id and retry blindly.

Requests may additionally carry
`output: {"format":"json_schema","schema":{...}}`. This is a native turn
constraint, included in the immutable request digest; it is not appended to the
prompt. Omission means text output. The schema is bounded to 64 KiB and nesting
depth 32, with remote references and schema resource rebasing refused. These
transport checks do not prove native dialect support. Admission must require
the exact Provider adapter to enforce the schema; otherwise return
`unsupported_capability` before launching a child. Never silently downgrade to
text and parse it afterwards as if native structured output were enforced.

```json
{
  "schema": 1,
  "call_id": "call_123",
  "request_id": "review-r2-security-01",
  "state": "running",
  "resolved": {
    "provider": "codex",
    "machine_id": "hawk",
    "workspace_id": "marketplace-service",
    "input_revision": "sha256:..."
  },
  "next": {"argv": ["cowboy", "call", "wait", "call_123"]}
}
```

Use an argv array, not an executable shell string, for machine-readable next
actions. Error envelopes carry a stable code, admission certainty and allowed
next action. Initial codes include `context_unavailable`, `permission_denied`,
`provider_unavailable`, `authentication_required`, `unsupported_capability`,
`input_changed`, `request_conflict`, `capacity`, and `outcome_unknown`.

### Conversation continuity

Calls and conversations have different identities. A call is one bounded
invocation; a managed child session retains a native conversation across calls.
`conversation.mode=fresh` always creates a fresh child native conversation.
`conversation.mode=continue` requires an exact child session id owned by the
parent and the same Provider, Machine, permission profile and workspace identity.
There is no `resume-last`, Provider substitution or automatic conversation reset.
Only one turn may run in a persistent child conversation at a time. A concurrent
request returns `conversation_busy`, rather than interleaving reviewer prompts.

## Permission and input integrity

Read-only is an enforced launch capability, not a sentence in the prompt and not
the fact that a tool is named GET. Resolve and validate the exact Provider's
native sandbox/tool/MCP permissions before the first prompt. Unknown permission
options fail closed. A Provider unable to enforce the profile is unavailable for
that request. Existing normal-session full-access defaults must never apply to
managed read-only children. User approval may not silently widen that profile;
an explicitly different request is required.

Native processes retain their normal Provider-owned credential materialization.
Do not point them at another generation's home or copy authentication from the
parent. Private homes, native memory policy and Plugin generation leases follow
the owning Provider contract. Conversation continuity does not imply persistent
long-term reviewer memory. Pin native/runtime/package versions in the receipt.

Before each review round, capture base/head, index and working-tree changes,
including explicitly scoped untracked files, requested input documents and
necessary spec context. Ignore secrets and excluded paths according to the
owning repository. Detect changes during capture and refuse a mixed snapshot.
A content digest of only the diff does not prove that unchanged context files
stayed fixed. Reviewers need a coherent read-only snapshot of their allowed
repository context and pinned referenced artifacts, not a live checkout paired
with an old diff. Snapshot preparation belongs to the target Machine and existing
workspace machinery, not the Controller's filesystem or a second Git clone tool.
Reject unsupported symlinks/submodules/LFS conditions explicitly rather than
claiming a complete snapshot. Do not mutate or lock the parent's worktree for an
unbounded review. A write-enabled implementation child needs an isolated worktree
and an explicit reconciliation contract; it is outside the first release.

The receipt links the snapshot identity to the exact prompt and results. If the
parent subsequently edits its worktree, keep the old review valid for its old
snapshot and mark it stale for the current code. Do not auto-approve or auto-rerun.
Concurrent reviewers for the same round can lease the same immutable snapshot.

## Durable lifecycle

Persist the parent relationship, request digest, child session id, exact placement,
launch profile, input lease and lifecycle transition before dispatch. Reuse
Cowboy's session store, supervisor, worker ownership and event subscriptions;
do not add a second process daemon, mutable transcript database or task planner.
The relationship requires an additive migration; deployed SQL baselines remain
unchanged. Both SQLite and PostgreSQL need deterministic conformance.

| State | Meaning and allowed observation |
| --- | --- |
| queued | Accepted durably; no child effect dispatched yet |
| starting | Exact child launch in progress; never admit a second launch |
| running | Worker is live; stream actual native events |
| waiting_input | A supported input request needs the controlling parent/user |
| stopping | Cancellation requested; worker exit/turn termination not yet proven |
| completed | Native turn completed and final result was durably captured |
| failed | A known terminal failure, with classified cause and retained output |
| cancelled | Cancellation and applicable child termination confirmed |

`reconnecting` is connection health, not a replacement for the durable execution
state. An unknown launch/termination outcome is exposed explicitly and fences
redispatch. A zero process exit alone is not a successful review and a P1 finding
does not mean execution failed. There is no fabricated progress percentage.

Controller restart and parent/browser disconnection preserve accepted calls.
On recovery, observe the original child owner and cursor before acting. A user
stop of the parent cascades to its owned active calls; normal parent turn
completion does not. Parent deletion first cancels or explicitly detaches children
under the owning lifecycle before purging relationship/snapshot state. Child
deletion retains the result summary for its parent's history until normal retention.

Cancellation during preparation prevents dispatch. Cancellation racing completion
records the actual terminal outcome without rewriting a completed result as
cancelled. Provider cancellation has a bounded grace period; termination targets
only the exact owned process group/session, never a Machine or unrelated worker.
Preserve original causes and output even when cleanup fails. Wait deadlines,
execution budgets and snapshot/result retention are separate bounded policies.

One controller owns a child conversation at a time. The parent is the default
controller; the UI can explicitly take over, which fences parent submissions.
Returning control is also explicit. The first release can omit takeover rather
than expose concurrent input. Recursive child delegation is disabled initially;
later depth/concurrency budgets require explicit authority attenuation.

## Suger integration boundary

Do not change any files in `marketplace-service` or `artifacts` for this feature.
The canonical `.claude/commands/speckit.*.md`, constitution, native skill wrappers,
design inputs, finding ledger and task status semantics stay project-owned.
The outer Suger workspace owns a shared integration instruction and thin adapter.
It maps transport, not prompts or acceptance policy. Both agent runtimes discover
the same instruction through their existing guidance surfaces.

The inspected `speckit.review-pr` contract requires one persistent Claude reviewer
across rounds and fresh Codex reviewers per round/aspect, with concurrent review
and recomputed round inputs. Preserve that exact asymmetry. Cowboy grouping labels
show round/aspect; they do not determine how many rounds or which findings block.
The handler remains responsible for fixes, verification and task/artifact updates.
Cowboy never marks a Suger task complete because a child command exited zero.

The canonical command currently invokes an explicit cached `codex-companion.mjs`
path. Adding `cowboy codex` to PATH cannot intercept that invocation. The outer
integration must explicitly map that execution step to the managed call adapter
in a Cowboy context while retaining its full prompt, fresh-thread semantics,
concurrency and output contract. Do not shadow `node`, replace cached Plugin code,
rewrite canonical commands, or claim automatic interception. Outside Cowboy,
leave the original companion path intact. An unrecognized workflow revision or
capability mismatch must report a compatibility error rather than guess.

The inspected companion 1.0.6 adversarial-review path constructs its prompt
from the Plugin template and freshly collected Git context, and passes
`schemas/review-output.schema.json` as native `outputSchema`. The managed
adapter must preserve both the completed prompt and this output constraint,
then use the original result parser/renderer. Passing only the aspect's focus
text, or replacing schema enforcement with a prompt instruction, is not a
compatible transport substitution.

The adapter reads the request file on the current target, validates its workspace,
passes task/group/round/aspect references as labels, and writes output only to an
explicit caller-owned output path using atomic replacement. It must not write the
source project or artifacts merely to register a managed call. Tests use disposable
fixtures and verify all production project/artifacts paths are untouched by setup.

## Desktop and mobile experience

### Parent transcript

Render an authoritative managed-call event as a card. Do not parse Bash command
text or provider prose to infer a call. If the launch also appears as a native
shell tool call, use the trusted correlation identity to attach the card to that
row; never show two contradictory status sources. Unknown/unrelated shell CLI
processes remain ordinary tools, not fabricated managed children.

Group by the caller's explicit parent-scoped group id; display task, round and
aspect as optional labels. Use the real Provider identity/icon. A compact group
shows completed/total, active aspect, elapsed time, actual Machine and a visible
details action. Never imply that all aspects passed because execution finished.
Do not add every child to the main session list by default; provide a children
filter and deep links from the parent. Keep the parent link available in all views.

### Desktop

Open a selected child's detail in a resizable side panel while retaining the
parent transcript. Show a compact child list beside activity/results when space
allows; collapse to a single detail pane at narrower desktop widths. Support
keyboard navigation, visible focus, Escape/back restoration and a full-session
link. Results can link to existing Code Review/diff surfaces for the exact
snapshot rather than introducing a second diff editor. Stop the selected child
and stop the group are separate labelled actions. Source/tool detail remains
progressively disclosed, not hidden behind hover-only controls.

### Mobile

Use a vertically stacked summary card with large touch targets. Open child detail
as a full-height page with a stable back-to-parent action and preserved scroll
position. Results, activity and execution information use a compact accessible
switcher; avoid a nested drawer inside the existing Agent/Review swipe surface.
Long code uses the established code viewer, not a horizontally scrolling table.
Keep the appropriate stop/respond action accessible without covering text or the
system safe area. Never autoscroll away from what the user is reading when tokens
arrive. Collapse completed aspects by default; surface active/blocked ones without
reordering a row under the user's finger. Screen-reader live announcements are
limited to meaningful state changes rather than every output token.

### Shared semantics

The overview separates execution status, review findings and handler disposition.
Technical diagnostics include runtime Machine, execution Machine, native version,
input revision, call/session ids and timestamps without credentials. Read-only
appears only after enforced admission. A stale result displays its original input
revision and a warning that current code differs. Network recovery retains the
last observed state and age. Cancellation shows stopping until acknowledged.
Notifications, if added later, follow the owning PWA/Focus/installation contract;
the first release needs no new notification authority.

## Delivery and acceptance gates

1. Contract and authorization: request bounds, scope isolation, revocation,
   idempotency conflict/races, unsupported capability, target-context absence and
   no Provider credential leakage. Parser tests are not proof of managed execution.
2. Execution: native read-only launch verified with attempted writes, exact
   Provider generation, correct target and snapshot, fresh and continued native
   identities, final output, child-only cancellation, capacity and bounded errors.
3. Recovery: disconnect before/after dispatch, Controller restart, lost reply,
   repeated request id, cancellation races and parent deletion. No duplicate
   prompt/process or false terminal state is acceptable.
4. Two real end-to-end paths: OVH Claude to Hawk Codex and OVH Codex to Hawk Claude.
   Prove runtime process placement and tool placement separately. Missing Hawk
   Claude installation is a prerequisite failure, not grounds to run on OVH.
5. Suger: preserve persistent Claude/fresh Codex semantics and exact round inputs;
   compare canonical workflow bytes and verify no edits to either protected repo.
6. UI: actual event-backed cards, desktop keyboard/focus, mobile navigation and
   touch targets, long output, partial failure, light/dark and narrow/wide screens.
   Browser simulation is not physical iOS acceptance; report that boundary.
7. Release: owning deterministic gates, native scoped review, compatible reader/
   Machine/worker/Provider floors and exact signed packages where required. Never
   expose submit controls until all dependent capabilities are active. Keep old
   sessions working; old generations can explicitly lack managed-call ingress.

Completion means real managed calls and both UI surfaces have passed their gates,
with production receipts. This document, a prototype, or a parser alone is not a
shipped solution. Known unresolved implementation work includes scoped target
ingress, enforced Provider launch profiles, coherent snapshot leases, additive
relationship persistence and trusted native-tool/UI correlation.
