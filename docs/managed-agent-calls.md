# Managed agent calls

Status: implementation in progress; the commands and UI below are not shipped.

Implemented source includes the closed CLI/request contract, private local
gateway primitive, parent-scoped durable ledger, observation API, signed native
launch-profile selection and worker guards against permission widening. These
pieces do not yet form a working launch path. Production grant issuance and
gateway wiring, snapshot preparation, native Provider profile acceptance,
structured-output forwarding, child lifecycle dispatch, the review UI and the
outer Suger transport adapter remain required before release. No Provider source
currently advertises the managed read-only profile; unsupported launch attempts
must remain refused. Candidate component release 3.43.0 records SDK compatibility
inputs, not successful native acceptance or a published installation.

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
closed parents and changed launch metadata cannot produce it. Comparing this
observation includes the worker epoch and parent ownership/placement revision;
turn changes, titles and native conversation materialization do not change it.
This is the authority check for the future grant issuer, not a shipped grant or
CLI context producer. The observation API's `parent_runtime_ready` is advisory
and must never substitute for revalidation at dispatch.

Call list responses contain summaries with `has_result`, not full review bodies.
Opening a call uses its parent-scoped detail endpoint to read the complete result.
The store still validates the full durable records before projecting summaries;
the bounded response does not imply a separate lightweight storage index.

Runtime-only environment injection is separate from operator-configurable target
environment variables. The current closed target environment intentionally rejects
`COWBOY_*`, `CODEX_*` and `CLAUDE_*`; do not relax its operator allowlist. New
wiring requires an explicit versioned native boundary and compatibility tests.
An old worker/keeper without this capability fails with `context_unavailable`.
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
