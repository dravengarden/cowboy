# Managed agent calls

Status: implemented in source; see "Implementation" for the shipped mechanism
and "Acceptance" for what has been proven in production.

## Implementation

Calls record the actual child's `provider_account_id` when execution first
starts, independently of the parent account and the current creation default.
This immutable observation survives child removal and is exposed in summaries,
details and CLI/MCP receipts. Older records without it display `Not recorded`;
they are not backfilled from today's defaults. Session rows and Calls previews
use the account's short display label; settings and execution details use the
full label. Renaming changes presentation only.

The optional field is backward-readable by this Controller, but older
Controllers with strict call-record decoding cannot read newly populated
records. Rollback must retain this reader or use a descendant corrective release;
do not erase call history to downgrade.

Image generation/editing extends this lifecycle with the separate signed
`image_v1` profile and verified artifacts; see
[managed native capabilities](managed-native-capabilities.md). Review and
analysis retain their existing snapshot and read-only contracts.

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
  state (`input_changed`); filters/LFS, escaping symlinks and subdirectory
  roots are refused. A submodule with a clean or uninitialized working tree is
  captured as its gitlink pointer (an empty directory for the reviewer); one
  with uncommitted or untracked changes is refused. `context.root` may name another work tree of a
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
  A child whose newest call finished more than three days ago, or whose parent
  no longer exists, is retired hourly, one at a time and at most 32 per pass:
  its worker stops and its target snapshot, including delivered image files
  under it, is closed; the session is removed only after the Machine confirms
  that close, so a failed close is retried on a later pass instead of leaving
  an orphaned directory. The parent's call ledger and results remain.
  Continuation admission and retirement share one fence: a continuation either
  becomes the child's active call first, or is refused with
  `conversation_unavailable` once retirement has begun.
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

Agent calls cost tokens only when an agent decides a change needs a second
opinion, and its prompt keeps that rare (see below), so they are on by default.
A person can still turn them off. Cowboy owns one two-layer policy per agent
kind (`codex`, `claude-code`):

- **Global defaults** (`agent_tools:<agent>` in Hub settings; Settings → Agent
  tools; `GET /api/agent-tools`, `PUT /api/agent-tools/{agent}` for Owners).
  A stored global default wins over the built-in one, so a Service that saved
  "off" keeps it until an Owner changes it.
  Built-in defaults: calls on, both targets allowed with the other agent kind
  first, default `auto`, at most 4 concurrent and 64 total calls per parent
  session; Matrix memory tools and automatic recall on.
- **Session override** (`session_tools:<session>`; the session's **Tools**
  section in the mobile session sheet and the desktop Run configuration modal;
  `GET`/`PUT /api/sessions/{id}/tools` for users who can mutate the session).
  It stores only the fields that differ; an empty override is removed, so the
  session follows later changes to its agent kind's defaults.

An agent never calls its own family (`claude-code`/`claude-deepseek`;
`codex`/`codex-deepseek`): built-in defaults list only other families, stored
settings are cleaned of the caller's family, and such a request is refused with
`same_agent` (not a policy question, so no approval prompt). The Tools editor
shows one card per callable agent with its switch and its presets as choices;
the default-agent chooser appears only when more than one agent is callable. Each
card starts folded to its name, current choice and switch. Unfolded it lists
the recommended presets, then **Custom** selects for each option the presets
set (model, reasoning). Their choices come from the newest live session of
that agent's own configuration options, with any preset-only values added.
A custom choice is stored on the target as `options` (never with `preset`);
values equal to a preset are normalized back to that preset. Admission
records it in the call's `preset` field as `custom:model=…;reasoning_effort=…`
so durable records keep their schema, and the runner applies only option ids
the exact generation's presets set (the managed worker refuses others).

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

### Agents know to use it

An ordinary Claude or Codex session is told about managed calls once, in its
system prompt (Claude `--append-system-prompt`; Codex `developer_instructions`
through the launcher's `-c` options, never replacing a user's or Cowboy's own
`developer_instructions`). The paragraph rides every turn, so it is a pointer
rather than a manual (about 820 characters): it names `cowboy review`, when to
use it, and `cowboy codex|claude --help` for any other delegation, whose help
carries the request contract. It says not to start `codex exec`, `codex review`
or `claude -p` directly: those run hidden from the user, outside the session's
policy and model choice. Managed children do not get it.

The agent decides whether a change needs a review, and skipping is the
default. It reviews at most once per task, before reporting, and only when a
missed bug would be costly and its own checks cannot rule it out: security or
permissions, concurrency, persistence or migrations, data deletion, money, or a
large cross-cutting refactor. Small, mechanical, docs-only, test-only and
exploratory edits never get one. `cowboy review` answers one `skipped:` line
at once when the session's policy refuses
it, and the agent then carries on without mentioning it. An explicit user
request uses `cowboy review --ask`, which goes through the approval prompt.

Before a review, the calling model reads `cowboy call capabilities`. The same
projection is available through MCP's calls capabilities API. It includes
signed presets, session choices, the Provider's quota account, cached account
and model limit windows, reset times and observation freshness. Missing or stale
quota is unknown. Oversized quota data is explicitly omitted and can be read
through `cowboy.usage.read`; capabilities never triggers a collector.

Settings → Tools configures ordered Review preferences for each calling agent
kind. Session card/Session Settings → Tools can override them; restoring defaults
resumes inheritance. Each family inherits independently: overriding GPT does
not freeze Claude defaults. The built-in orders are **Opus → Fable** and **Astra → SOL**.
Each list can contain both models in either order, or just one. Empty lists,
duplicates, cross-family entries and weaker models are rejected. These DB-backed
preferences apply to future Review/Design Review calls, never running calls or
ordinary analysis/image calls. Claude and GPT share the same Controller contract.

Capabilities exposes effective `review_preferences` and each Provider's ordered
signed preset IDs in `review_candidates` (metadata stays in `presets`). The calling model chooses the first preferred family
whose fresh account and model quota permits review, preferring Max within that
family. Greater strength alone is not a reason to skip the preferred family.
The Controller excludes models outside the configured list; without an explicit
preset it selects the first preferred signed preset. Generic Tools target pins
remain inputs to ordinary calls; they do not override Review preferences.
Quota assessment remains the calling agent's responsibility, with no fixed
percentage router or automatic retry. Explicit preset choices must
also remain in the allowed model list. It passes `--agent`,
`--preset`, a short `--title` and `--reason` to `cowboy review`. The controller
still enforces session authorization and signed presets. Generic managed-call
clients may put the same display metadata in `labels.title` and
`labels.selection_reason`; MCP `review.start` and `calls.start` accept `title`
and `reason` directly.

AI configuration uses the existing on-demand `control.describe` contracts for
`tools.defaults_set` and `tools.session_set`. The field is
`calls.review: {claude:["opus","fable"],gpt:["astra","sol"]}`. Read the current
Tools document, preserve unrelated fields, and omit the session override to
inherit. Existing authorization, DB persistence and operation receipts apply.

Calls displays the persisted dispatch configuration (model and effort), chosen
preset and reason, created/started/finished timestamps, elapsed time, time zone,
placement, input snapshot and failure cause. Historical records lacking these
observations say so; current catalog defaults never rewrite history. Start is
the recorded transition to Running before submitting the idempotent prompt,
not the first model token; finish is the controller's terminal observation.

### Bounded review and exact-input reuse

The workflow should perform one primary review, accept fixes through relevant
tests and evidence, and request a targeted follow-up only for a concrete
unresolved risk. Neither minimum rounds nor a model pair per aspect belongs in
Cowboy. A repeated defect family calls for invariant/root-cause investigation,
not a fresh full review. Reaching a budget never approves unresolved work.

`calls.capabilities().review_input_reuse` advertises opt-in automatic reuse.
Supply `task`, `base` (an immutable commit SHA) and the actual repository `root`
to MCP `review.start`. CLI:
`cowboy review --task <stable-id> --base <ref>
--root <worktree>` resolves the
merge-base with HEAD before submission. `fresh:true` / `--fresh` deliberately
requests a new independent opinion. Use this contract only when the instruction
and captured files contain all required evidence; mutable external paths and
branch refs are not review input. Calls without this declaration retain their
existing behavior.

The compatible request encoding uses reserved labels `review_task`,
`review_base` and `review_reuse` (`"true"` or `"false"`). They define review
meaning, never permission or account selection. This encoding requires the
standard completeness JSON schema, including when reuse is disabled.
The exact identity includes
parent placement/authority, task, root, base, captured HEAD/index/worktree,
instruction, output schema, Provider generation, runtime, account, authentication
generation and preset.
Changing any of these invalidates reuse. Cosmetic titles/round labels do not.
Continued conversations are excluded because their native history also matters.
This avoids confusing an outer workspace snapshot with an external project.

After capture and before worker activation, a durable reservation selects one
original call. Concurrent duplicates wait for it; controller recovery observes
the same reservation. A reused result identifies `reused_from` and starts no
model request. Only a successful result explicitly reporting
`review_complete:true` and `approve` or `needs-attention` can be reused by later
requests. Completeness does not mean approval. It also does not require
execution: the read-only snapshot may lack project toolchains, dependencies or
browsers, so the contract asks the reviewer to name checks it could not run in
the summary and reserve `incomplete` for missing inputs or unread changed code.
Failed, cancelled, incomplete or
truncated results are not reusable completion evidence; a later explicit retry
submitted after that attempt finishes gets a new reservation. Requests submitted
before it finishes share its outcome, even if they were still queued and first
look up the reservation after failure. This prevents implicit retry storms.
The selected source is persisted per call, so waiting survives controller recovery
and an unavailable execution Machine. Unreserved launches keep their timeout.
API `resolved.child_session_id` and UI navigation point to the original conversation;
continuing the duplicate receipt's unused child is refused.

Final output follows native message IDs, so commentary immediately before the
final message cannot corrupt structured output. ID-less Providers retain the
existing tool-boundary fallback.

Native ACP prompt-response usage is retained on the turn-end event and in the
call result, including cancelled turns when the Provider reports it. Calls and
MCP expose total/new/cached input and output; reasoning is an output subset.
Cache writes are included in new input. Missing usage/cache fields remain
unknown, including historical calls and older workers. Reused receipts point to
the original usage and do not duplicate it. These are reported token counts, not
billing, proof of cache reuse on continuation, or an estimate of waste.

Native Providers own token accounting and review execution. Cowboy only retains
and projects their evidence across Machine, call ledger and UI boundaries. The
optional ACP turn usage field is additive: older readers ignore it and new
readers accept old events without counters. Claude shares this contract when its
adapter reports native usage; absence remains explicit. Remove local
normalization when ACP standardizes one unambiguous input/cache representation.

Clear ended calls supports individual selection, bulk selection and Undo.
`POST /api/sessions/{id}/calls/dismiss` takes
`{"call_ids":["call-id"],"dismissed":true}` (maximum 100); `false` restores
them. This authenticated, parent-scoped operation hides only terminal calls.
It preserves results, child ownership, budget counts and admission receipts,
so cleanup cannot cause a duplicate launch. It is also available to product
CLI clients through `cowboy api` and MCP `control.apply` with
`sessions.calls_dismiss`; it does not cancel active work or delete code.
Suger's Codex reviewer adapter applies its own rule to its review rounds (it
skips without prompting when off).

`cowboy review` is shaped for the agent that runs it, since every byte it
prints is read back into that agent's context. Optional focus text comes on
stdin (otherwise it reviews the worktree's change); it builds the request,
submits it with Auto selection, waits in one blocking command (`--timeout-min`,
default 20), and prints only the reviewer's reply. The request asks for one
line per concrete problem (`path:line - problem - fix`, at most 10) and exactly
`No findings.` when there is nothing to fix, so a clean review costs a few
tokens. `--design` reviews design documents; `--agent` and `--preset` choose
the reviewer. Compared with the earlier flow (a capabilities read after every
substantive change, then start, wait and result, each printing a JSON
envelope), a task that needs no review now costs nothing beyond the paragraph,
and one that does costs one command and its findings.

`--preset ID` on `cowboy codex|claude|call start` selects one of that explicit
Provider's signed presets for this call (refused with Auto, with `continue`,
and as `preset_unavailable` when the exact Provider has no such preset); it is
part of the replay identity. `cowboy call capabilities` lists each Provider's
`presets`. Without it the session's Tools choice applies.

### Asking the person

A policy refusal is a question, not a dead end. The coordinator records each
`calls_disabled`/`policy_denied` request in an in-memory registry per parent
and pushes `call_approval` (gated by session visibility) with one aggregated
prompt: the caller, the requested agents and a short label per request. The
refusal carries `"approval": "pending"`, and `cowboy codex|claude|call start`
resubmits the same request every 3 seconds for `--approval-wait-ms` (default
100 seconds, up to an hour) while printing one waiting notice on stderr. Each
resubmission refreshes the prompt; a request nobody resubmits drops out after
15 seconds, and clients hide a prompt whose `ttl_ms` passed, so the prompt
lives exactly as long as an agent waits and survives reconnects.

The composer shows it in the tool-permission slot with the same material
(below a pending permission, above the turn status). `POST
/api/sessions/{id}/calls/approval {"decision": "once"|"session"|"decline"}`
(requires `can_mutate`) applies to every request waiting at that moment:

- `once` admits those requests with calls on and their agent added as a target
  for that admission only; the session policy is unchanged.
- `session` also writes the session override: calls on and the requested
  agents added to its targets.
- `decline` makes their resubmissions fail with `calls_declined`.

Desktop `A`/`R` resolve it like a permission (Allow once / Decline). Limits
still apply after approval; decisions expire after 30 minutes and are
forgotten on Controller restart, where the agent simply asks again.

The runner applies the selected preset's exact value map as configuration
preferences for the child before its first prompt. The managed worker accepts
`session/set_config_option` only for option ids that the signed package's
presets declare and never persists them as the user's preferences; mode and
every other option remain refused. A preset missing from the exact admitted
Provider generation fails the call with `preset_unavailable`.

The Controller waits for the connected worker's actual model/effort values
before sending the prompt. The worker exposes only signed preset options;
permission and mode controls remain hidden and refused. The Hub's optimistic
UI selection, a saved preference and a retained disconnected snapshot are not
confirmation. An older worker that hides these options cannot silently run an
explicit preset on its default model: the call ends as
`configuration_unconfirmed` without sending a prompt. Each call records the
observed configuration, and continuation/recovery retains the original preset
even after the parent's Tools defaults change.

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
cowboy call wait call_123 --timeout-ms 900000   # alias: --wait-ms
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
not a timeout error. The Controller times this observation from the request's
receipt, so approval, readiness and admission spend the same budget and the
response never outlives the gateway exchange. `wait` observes the same call,
never submits a new prompt. It accepts up to one hour; the CLI splits longer
waits into 60-second gateway exchanges and returns early once the call is
terminal, needs input, or is refused, so callers need no polling loop.
Cancellation is explicit and idempotent. When a start response is lost, the CLI
itself observes the same request id (observing never submits) and returns the
accepted call; only an outcome it still cannot resolve tells the caller to
`cowboy call observe` that id. It must never return an instruction to mint a
replacement id and retry blindly.

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
Reject unsupported symlinks, dirty submodules and LFS conditions explicitly rather than
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

The canonical `.claude/commands/speckit.*.md`, constitution, native skill wrappers,
design inputs, finding ledger and task status semantics stay project-owned.
The outer Suger workspace owns a shared integration instruction and thin adapter.
It maps transport, not prompts or acceptance policy. Both agent runtimes discover
the same instruction through their existing guidance surfaces.

Review-policy changes must update the canonical command and constitution in
their owning project. The transport must not preserve an obsolete minimum round
count or reviewer pairing as a second policy. Cowboy grouping labels show
round/aspect; they do not determine how many rounds or which findings block.
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

### Calls modal (implemented)

A call is read in a modal with three levels, the same on every surface:
**list → the child's transcript → details**. Selecting a call opens its child
conversation, not a summary: the same Transcript renderer as an opened session,
read-only and live, scrolled to the newest message. The store keeps that child
loaded and receives its events while shown (`peekSession`) without opening it,
reviving its agent or changing the opened session. Its header shows Provider,
version, runtime → execution Machine, elapsed time, state and verdict; Details
(or the verdict) opens execution information, the structured result and
findings, Stop and **Open as session**.

- Mobile: one cover sheet. Nested levels show Back beside Close in the shared
  glass footer island (`BottomSheet` `onBack`); the transcript level fills the
  sheet (`fill`) and passes under that island. Closing keeps the level, so the
  modal reopens where it was left. Edge-swipe back is deliberately absent until
  it is arbitrated with the Agent↔Review pager.
- Desktop: the inline Calls list above the Prompt stays the overview (its focus
  and shortcut unchanged); a call opens a Desktop modal with the transcript and
  a Details panel side by side. `i` toggles Details, `o` opens the child as a
  session, Escape closes.
- An opened child session's notice offers the same Details beside Open parent.

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
