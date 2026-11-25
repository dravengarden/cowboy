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
  no longer exists, is retired ten minutes after Controller start and then
  hourly, one at a time and at most 32 per pass:
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
  first, default `auto`, at most 4 concurrent calls and 16 calls per owner
  turn (see [Call budgets](#call-budgets)); Matrix memory tools and automatic
  recall on.
- **Session override** (`session_tools:<session>`; the session's **Tools**
  section in the mobile session sheet and the desktop Run configuration modal;
  `GET`/`PUT /api/sessions/{id}/tools` for users who can mutate the session).
  It stores only the fields that differ; an empty override is removed, so the
  session follows later changes to its agent kind's defaults.

Since 2026-11-21 an agent may call its own family (`claude-code`/`claude-deepseek`;
`codex`/`codex-deepseek`), but only after other vendors: settings still list
only other families, and an Auto call tries them first, because another
vendor's model shares fewer blind spots. A review never runs the caller's own
model: a same-vendor review admits only another reviewer family (an Opus
caller may be reviewed by Fable, never Opus), and is refused `same_model` when
none is left or the caller's current model is unknown. Image
calls are the exception, under the `images` and `self_images` switches of
[call capabilities](call-capabilities.md). The Tools editor
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
`call_limit` when the budget of the current owner turn is spent (also
`not_submitted`: resubmit after the owner's next prompt). The limits are a soft
admission budget; concurrent submissions may pass them together. `cowboy call
capabilities` reports `enabled`, `default`, the limits (`max_concurrent`,
`max_per_turn`) and a per-Provider reason.

### Call budgets

Two limits bound a parent's calls. `max_concurrent` (default 4, 1–16) is the
number of calls running at once. `max_per_turn` (default 16, valid 1–256;
Settings offers 4, 8, 16, 32 and 64) is the number of calls one **owner turn**
may admit. There is no lifetime cap: a long-lived session that talks with its
owner for many turns gets a fresh budget with each of the owner's prompts.
It replaced `max_per_session` (default 64), which ran out in long sessions and
forced new sessions just to get review budget.

A turn starts at the latest prompt a person sent to the session: a user-role
event whose client message id and stamped origin are the owner's
(`prompt_origin::is_owner_prompt_event`, the predicate the superagent causal
budget also uses). The Web composer, an API token, an ACP client and the
Retry button start one. These never do, because otherwise a notice that
wakes the agent into one more call would hand it a new budget each time
(notice → new turn → new call): Cowboy call completion notices and call
results (`call-notice:`, `managed-call:`), peer-session notices
(`session-notice:`), a `queue.submit` from another session (`agent-<digest>`),
scheduled wakeups and scheduled drafts (`__wake__`, `__sched__`), legacy
auto-continuations, superagent and code-intelligence prompts, and runtime
markup injected by the agent. A session no person ever prompted (only peers or
automation drive it) is one turn for its whole life, so its budget is a
lifetime cap of `max_per_turn`, unless its creator grants it turns.

**Turns a creator grants.** A session another session's agent created with the
Agent API `sessions.create` (its creator, recorded as
`session_created_by:<session>` wherever it was filed; one created before that
record existed has it only as the `session_spawned_from:` of a spawn filed
beside its creator) usually never gets a person's prompt: its creator drives it
with `queue.submit`. A message the creator sends it starts a turn there, but
only the first one in each turn of the creator. Each of the owner's prompts to
the creator therefore gives every child at most one fresh budget, and a creator
and child messaging each other cannot mint more. A creator grants under its own
current turn: its owner's prompt, or the turn its own creator granted it. A
grandchild's budget therefore also traces back to an owner's prompt, and a
creator with neither grants nothing. The sender is the Agent API caller session
the Controller authenticated, never the message text; a message from any other
session grants nothing.

The grant is taken once the message was accepted (queued or injected, or for
an interrupt, submitted); a refused, pending or unknown send, and one with
nothing dispatch would prompt (blank text, or no content block that decodes),
grants nothing, and the creator's next message in that turn
may. The child keeps one record, `session_turn_grants:<session>`:
the creator's turn it was granted in and the granted turn, named by the child's
newest event sequence then. Only a creator turn newer than the recorded one
grants again, so a delayed send from an older turn never does. The record, and the
creator link it relies on (written to the store when the session is created,
before the creation's receipt), are written to the store before it counts, so a
restart never forgets a grant a call was admitted under; a failed write grants
nothing. The turn takes effect
at once and does not depend on finding the message in the transcript again: it
survives trimming, a cleared transcript and a restart. A child that acts on the
message before the grant is written (it is written right after the message is
queued or injected) counts that call in its previous turn. The child's turn is
the newer of the granted turn and its own owner turn; a later owner prompt has
a larger sequence and wins, and a grant that shares a sequence with an earlier
turn joins that turn. Durable history is read for an owner prompt only down to
the granted turn. Anything that cannot be established (no creator link, the
creator's turn unreadable, a record this Controller cannot read, which is then
left as it is) grants nothing, and the budget counts as it would without
grants.

Each admitted call is stamped with its turn, the event sequence of the owner
prompt or of the granted turn (`managed_agent_calls.owner_turn`, `-1` for a
session with neither), and
the budget counts the calls that share the current turn, in every state: a
failed or cancelled call, a dismissed call and a continuation round all spent
budget. A replayed `request_id` returns the original call and keeps its turn.
A call admitted before the column existed carries `NULL` and counts in no turn.

A turn begins at the first chunk of the owner's send
(`prompt_origin::newest_owner_turn`): the first of the newest run of consecutive
user-role chunks the Hub stamped with a human origin and whose message id, if it
kept one, is not an automation's. A send's later blocks (a context preamble
ahead of the owner's words, images) extend the run and are one anchor; a direct
WebSocket prompt without an id is stamped and counts; a chunk nobody stamped (an
ACP client's unmatched text, a steered block of an older worker) and rows from
before the stamp never do, whatever their text. The Hub remembers the latest
such sequence per session (`owner_turn_seq`), restored from the retained log
at start, so the turn survives the transcript trimming it out of memory and a
cleared transcript; a send whose first blocks were trimmed, fell before the
tail a restart restored, or lie on an older history page is still anchored at
its first block, and an owner turn found in durable history is remembered
like one the Hub saw begin. Admission reads that, else the Hub's retained log (which
also holds events the store has not written yet, whatever the client bootstrap
window shows), else durable history (200 events per page, three seconds). When
it cannot, it refuses with the retryable `capacity` code and a `reason`
(`session_unavailable`, `owner_turn_scan_timeout` or
`owner_turn_history_unavailable`) rather than guess; nothing is recorded. A
racing duplicate of the same `request_id` that was admitted meanwhile is
answered with its receipt, not with the refusal; a continuation replays under
`auto` or its child's own agent.

**What the budget cannot promise.** The owner's prompt is recognized by its
client message id and the origin the Hub stamps on its echo, not by
authenticated provenance at each entry point. It is a guard against runaway
automation (the notice → call loop), not a security boundary. These still start
a turn because their message ids look like the owner's: an agent's
`sessions.context_send` (`context-<operation>`), `sessions.create` with
`initial_prompt` (`initial:`), a delegated `sessions.retry` (`cowboy-retry:`,
which can resend a wake or a notice), and activating a draft an agent edited
(it keeps the draft's own id). An owner prompt sent with an operation id that
begins `__wake__` or another automation prefix is missed; an id longer than the
128 bytes the store keeps is dropped on restore, which the stamp survives. The remembered turn is lost
when the Controller restarts after the owner's prompt left the retained log
before the store wrote it, or after the transcript was cleared (the cleared
history is gone), and then reads as the previous turn or none; so does an
owner prompt older than the stamp. A call's first
request can reach the Controller before the prompt echo is projected, in which
case it counts against the previous turn. Undecodable durable rows can make a
history page look empty. Closing these needs a trusted sender stamped at each
ingress and the turn anchored when the Controller dispatches the prompt.

A creator's grant follows its own agent's choices: the owner's prompt
authorizes the creator, not each message it then sends, so the creator's agent
can send its child a message that starts the child's turn without the owner
asking for it, once per owner turn and child. Creating another session gives
the creator a session with a fresh budget too, as it did before grants. The
budget bounds the automation loops; it does not limit how much work an agent
the owner prompted delegates.

Further limits of grants:

- An interrupt whose delivery is still pending when its 500 ms reply is due
  grants nothing, even if it is submitted later (fewer budgets).
- A grant names the child's event sequence before the Hub's write-behind
  queue has stored the sequence counter. After a crash the child may reuse
  lower sequences: until they pass the granted turn, a new owner prompt there
  does not start a turn, and a grant taken then names a lower turn than the
  one before the crash, which moves the child's turn back to it (counting the
  calls stamped with it, if any). The child's own grants to its children then
  carry that lower turn, which is not newer than the one they recorded before
  the crash, so its children get no grant from it until its turn passes that.
- The creator link is written to the store when the session is created, with
  a few retries; if every write fails the creation still succeeds (a retry
  would create a duplicate), the link waits in the write-behind queue, and the
  next grant writes it again first. A restart before either write leaves the
  session without a creator: it gets no grants (the budget as before grants).
- A grant or creator link written while its target is being deleted is
  removed again from the Hub, but a direct store write that lands after the
  deletion's tombstone can leave a stale row for the deleted session; nothing
  reads it.

Clearing a transcript whose owner turn the Hub had not established (a restored
tail that began inside an owner's send, or a long history no admission has
read to its start) records the clear (`session_owner_turn_lost:<session>`, the
newest event sequence then). Until a new owner prompt or a grant at least that
new names a turn, calls are refused with `capacity` and the non-retryable
reason `owner_turn_history_cleared` rather than counted against an older turn.
A user Clear decides the record under the lock that holds its pending latch
and writes it in the Clear's own transaction (`StoreWrite::ClearEvents`, with
the transcript's deletion and the native-id fence), so a failed write keeps
the transcript and records nothing. The Hub sets the record in memory before it
clears the transcript, so no admission sees the cleared log without it; the
Supervisor's Clear (`Hub::clear_transcript`) does the same with its own
`ClearEvents`. A session whose durable history an admission already read to
its start without finding an owner turn is not marked: automation that drives
it alone keeps going after a Clear. A durable read that a Clear overtook is
discarded rather than anchored at a fragment of a cleared send.

The remaining limits: the confirmation that a session has no owner turn is
kept in memory only, so after a restart a Clear before the next full read marks
the session again. A creator-link write retried after the created session's
deletion can land after the deletion's tombstone and leave a stale, unread row.

**Migration and compatibility window.** Stored Tools defaults that carry
`max_per_session` keep loading: the lifetime cap becomes `max_per_turn` 16
(the new default) unless a person had lowered it below 16, which is kept
(`legacy_turn_limit`; 64 → 16, 128 → 16, 8 → 8). Session overrides never
carried the field. The same mapping reads a `PUT /api/agent-tools/{agent}` from
a Web bundle that predates the change (a save carrying both fields, which that
bundle sends when it echoes the response it loaded, is refused with 400 so the
page reloads instead of losing an edit); every response carries only
`max_per_turn`. A Controller that predates the change parses the stored
document with `deny_unknown_fields` and falls back to the built-in policy on any
failure, which would re-enable calls a person had turned off. So the stored
`agent_tools:<agent>` document keeps a `max_per_session` for as long as
rollback to such a Controller is supported (the one it already had, else 64)
and the limit is saved under `agent_turn_budget:<agent>`, the way review effort
is, bound to the digest of the document it was saved with. Saves and reads of
the two take turns, so a reader never pairs one save's document with another's
limit. A rollback keeps every other setting and returns to its own lifetime
cap; re-upgrading restores the saved per-turn limit only while the predecessor
has not replaced the document: a reset or an edit made meanwhile orphans the
limit, which then migrates from the document's own cap (a byte-identical
rewrite cannot be told from the original). Like every Hub setting, a save is
acknowledged when committed in memory and persisted by the store queue, one key
at a time: a crash between the two keys reads as the previous document with the
default limit, the same loss a crash before either write has. The ledger column is additive
(PostgreSQL 0087, SQLite 0061) and the call `Record` is unchanged, so a
predecessor reads every row. A predecessor does not load the
`session_created_by:` and `session_turn_grants:` settings and counts only calls
stamped with its own owner turn, so after a rollback a created session that
spent a granted turn's budget returns to whatever its turn before the grant
had left. The window ends when no predecessor remains a
rollback target; then the legacy field, its fixed write and the input mapping
can be removed together.

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

Each kind also has a routine effort, `claude_effort` / `gpt_effort`: `max`
(built-in, the historical strongest-first order: max, xhigh, high, default,
medium) or `standard` (medium, default, high, xhigh, max). It reorders presets
only inside a family; the family order above still wins. A review declares
its risk with MCP `review.start` `risk` or `cowboy review --risk`, carried as
the reserved label `review_risk` (`routine` or `high`) so older gateways
transport it unchanged. `high` always uses the strongest-first order;
`routine` or an omitted risk follows the configured effort. Risk orders the
Controller's automatic choice and never authorizes a model: an explicit
preset is still limited to the allowed families. The stored request keeps
the declared risk beside the resolved preset. The defaults keep the previous
behavior until a person opts a kind into `standard`.

Since 2026-11-21 the routing table in [`review-routing.md`](review-routing.md)
supersedes the effort ordering above for Providers and generations it covers:
`high` gets the strongest reviewer, routine gets the routine tier and refuses a
stronger explicit preset (`preset_outside_tier`), `high` needs a caller-written
reason (`risk_reason_required`), and `standard` selects the economy tier. The
family lists still filter eligibility. Capabilities add `review_routing` with
each tier's automatic preset and measured cost.

Capabilities exposes effective `review_preferences` and each Provider's ordered
signed preset IDs in `review_candidates` in routine order (metadata stays in
`presets`); a high-risk review reorders effort Max first within each family. The calling model chooses the first preferred family
whose fresh account and model quota permits review, following the candidate
order for the review's declared risk within that family. Greater strength alone is not a reason to skip the preferred family.
The Controller excludes models outside the configured list; without an explicit
preset it selects the first preferred signed preset. Admission reads the latest
Catalog, so the runner checks again when it applies the preset from the exact
generation it executes: a Review/Design Review must use a signed preset (not
custom values) whose resolved model is a reviewer family of that Provider
(Astra/SOL for Codex, Opus/Fable for Claude); otherwise, or without a preset,
it fails with `review_model_not_allowed` before the worker starts, and it
confirms the native configuration against exactly those values. Preference
order stays an admission rule, so narrowing preferences never fails a review
that is already running. Every round of a managed child stays on one side of
review versus analysis: analysis shares the read-only runtime profile but never
passed the reviewer allowlist.

Review selection, claim, worker start, prompt delivery, recovery and reuse
are bound to one Provider generation: admission and capabilities rank the
generation readiness would launch; claim refuses a review whose generation
changed since admission or whose preset is no reviewer on the generation it
pins; managed children are never re-pinned or configured
by anyone but their call; a still-queued prompt of a cancelled call or of a
preset-less legacy review is withdrawn before it can start; and only schema-2
review results from a signed reviewer run are reused. See
[`review-generation-binding.md`](review-generation-binding.md). Generic Tools target pins
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
`calls.review: {claude:["opus","fable"],gpt:["astra","sol"],claude_effort:"max",gpt_effort:"max"}`
(the effort keys are optional and default to `max`). The Controller persists
effort under its own settings keys (`agent_review_effort:<agent>`,
`session_review_effort:<session>`), never inside the schema-one Tools document:
an older Controller would reject the unknown field and fall back to built-in
Tools. A rollback therefore only forgets effort (Max first), and Web shows the
effort controls only when the Controller returns the fields. Read the current
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

### Context asks over recorded history

A context ask lets an expensive agent hand one specific question about other
sessions' recorded history to a cheap model, instead of reloading that history
into its own (often cache-cold) context. MCP `calls.start` takes
`sessions: [{session_id, after?, before?}]` (exclusive seq bounds, at most 8
distinct sessions) with an `instruction` that is the question. It is always a
fresh `analysis` call: `files`, `root`, `child_session_id` and other purposes
are refused, and Tools approval and admission apply unchanged. Because it
reads history for the caller rather than giving a second opinion, the caller's
own agent family is admitted (for example a Codex parent asking
`codex`/`luna-6-max`).

`provider:"antigravity"` (for example preset `gemini-hydrogen-high`) is an
explicit-only target: it is admitted only when named, for one fresh read-only
round: a text-output context ask, or an analysis, review or design review of
the workspace (owner extension, 2026-11-21). It is never used for images,
continuations, Auto, Tools targets or approvals, and never reviews for an
Antigravity parent (`same_agent`). It runs on the owner's admitted named
default Antigravity account (`authentication_required` otherwise), guarded by
that account's controls (rate-limit back-off, suspension) and the parent's call
budget; a managed prompt never counts as an owner send, so it cannot clear an
ended rate limit. It needs a signed Antigravity package declaring the
read-only managed profile. `calls.capabilities` lists it under `context_ask`
(with the Codex and Claude agents a context ask may use) and `read_only`, with
`selection: "explicit"`; its entry carries `context_ask_reason:
"machine_upgrade_required"` when the execution Machine cannot receive history
(workspace rounds still run), and an Antigravity parent sees it for analysis
only. Its review candidates are its Claude
Opus preset only (`claude-opus-high`, the opus family; Gemini presets have no
reviewer family, so `review_model_not_allowed`). `agy` has no native schema
constraint: a generation declaring `--cowboy-output-contract=v1` advertises
`json_schema` output (an older one is text-only and a structured round is
`unsupported_capability`). Admission accepts structured output for Antigravity
only for a review or design review carrying Cowboy's own review schema, and
requires it there, since a text review would be an unchecked verdict (reviews
pin their generation); any other schema, and a structured analysis,
is `unsupported_capability` before a child starts. For such a round the proxy
also refuses any keyword its exact check does not cover, appends the schema
as an output contract, holds the reply back as thought, validates it, asks for a
corrected reply at most twice inside the same turn, and sends only a validated
reply as the final message; otherwise the round ends with
`structured_output_invalid`. Its child always runs split, on a Machine allowed
by Provider and project placement policy whose keeper supplies the execution
descriptor the adapter requires; a parent executing on that Machine itself
gets `runtime_policy`. Placement policy is checked when the call is admitted; a
later policy change applies to new calls. Enabling calls for a session
(including "Allow for this session" on another agent's request) is the owner's
consent to these calls; there is no separate Antigravity approval. Rollback
floor: a Controller older than the context-ask release (components 3.129.0)
cannot decode a call record whose provider is `antigravity`, so roll back
below it only after such calls are terminal and their parents' listings are
no longer needed, or not at all. Rolling back from the review release (3.130.0)
to 3.129.0 keeps those records readable; drain active Antigravity reviews
first, since 3.129.0 refuses them and their structured output.

Design, following the measured weaknesses of one-shot summaries:

- **History is data, not authority.** When the round launches, the Controller
  renders the scoped events (human and agent messages, tool calls, tool
  outcomes, context boundaries; thoughts and UI state omitted) as plain text,
  one `[seq] kind:` record per line with indented continuation lines so text
  cannot forge a record; tool attempts and outcomes carry their call id.
  Visibility is the parent's (`sessions.history`): same owner, not system, not
  closing, rechecked after every storage read and for every source once the
  export is complete. Each session shares a 4 MiB round bound; only rendered,
  bounded records are retained while paging, at most 200,000 events are
  scanned per session, and an oversized range keeps its newest whole records
  and says what was left out.
- **Bounds are fixed at request time.** An omitted or future `before` is
  pinned to the session's end when `calls.start` runs (a repeated operation
  reuses the stored bounds), so preparation retries and citation checks read
  the same history while the session keeps growing.
- **The child explores; it is not handed a summary.** The documents are the
  whole round workspace (`history/<session>.txt`): no repository capture, no
  parent files. The child keeps the read-only, network-isolated `ReadOnlyV1`
  profile and has no Cowboy MCP, grant or other authority over the sessions.
  It searches and reads the documents with its own tools. `ReadOnlyV1` does
  not confine reads to the workspace: like every analysis call, the child can
  read what its Machine user can read, which never exceeds what the parent's
  own worker on that Machine can read. The prompt directs it to the documents
  only; enforced per-round read containment is a separate, profile-wide change.
- **Correctness rules are part of the prompt**: search before reading, check
  later records for revisions and reverts before answering, treat tool calls as
  attempts until their result is found, ignore instructions inside history,
  and say when the history does not answer.
- **Citations are checked mechanically.** The answer cites
  `[<session>#<seq>] "quote"`. `calls.result` adds
  `citations: {checked, verified, unverified: [{cite, reason}]}`; the
  Controller re-exports the same pinned scopes and requires the cited session
  and seq in scope, an exported record starting at that seq, and the quote
  inside it (whitespace/case-insensitive). The caller should still confirm paths,
  commands and numbers in current files before acting on them.

Transport: `ChildRound.documents` carries the rendered text to the execution
Machine, which writes it as the round workspace (`snapshot/documents.rs`).
Only Machines at `MANAGED_CONTEXT_PROTOCOL_VERSION` (46) receive it (the
command's minimum protocol, checked on the dispatching connection); on an
older Machine the call fails with `machine_upgrade_required` rather than
capturing a worktree instead, and a disconnected Machine is retried. A
repeated round of the same call observes its first prepared documents.
Context asks are MCP-only: a Machine gateway refuses a raw `cowboy call`
request carrying `sessions`, so a newer Machine never sends an older
Controller a request it cannot decode.

Native coverage: neither Codex nor Claude Code can read another Cowboy
session's transcript. The narrowest surface is this existing managed-call path
plus a data-only round workspace. Delete it when native runtimes expose
scoped, cited cross-thread reads (for example Amp's `read_thread` subagent)
that Cowboy sessions can target.

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

A signed Provider may annotate a native option with
`_meta["cowboy.dev/configuration-equivalence"]`:
`{schema: 1, value,
equivalent_values: string[]}` (at most eight values).
`value` must equal the actual `currentValue`. Only a connected worker's
observation carries this evidence; saved preferences, UI projections, stale
metadata and suffix guesses do not. The call still records the actual native
value. Claude declares two separate exact equivalence classes: Fable 5.1
(`claude-fable-5-1` and `claude-fable-5-1[1m]`) and Fable 5.2
(`claude-fable-5-2`, `claude-fable-5-2[1m]`, and the `fable` alias that ACP
0.107.3's `aliasResumedModelRow` emits when resuming a Fable 5.2 transcript);
both versions have the native 1M window, as documented in
[Claude model configuration](https://code.claude.com/docs/en/model-config#extended-context).
Other versions, models and effort levels remain distinct. This avoids a
configuration timeout when a continuation restores the transcript's canonical
model spelling. Genuine confirmation timeouts retain bounded expected/observed
values in the failure detail and send no prompt.

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

An `instruction` is a task written as text. One that begins with a native
slash command (`/speckit.review-pr ...`, `/compact`) is refused at admission
with `invalid_instruction` and a fixed hint, before any child starts: Claude's
managed runtime ends its process on a native command, which crashed a
persistent reviewer and surfaced only as `provider_error`
(sess-1795490077099). Name the workflow in text instead, for example
"Follow the /speckit.review-pr workflow: round 3 ...".

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
constraint, included in the immutable request digest; a native adapter does
not append it to the prompt (Antigravity's output contract, below, is the one
adapter-enforced exception). Omission means text output. The schema is bounded to 64 KiB and nesting
depth 32, with remote references and schema resource rebasing refused. These
transport checks do not prove native dialect support. Admission must require
the exact Provider adapter to enforce the schema; otherwise return
`unsupported_capability` before launching a child. An adapter without a
native constraint enforces it only by a signed output contract: Antigravity's
generation must declare `--cowboy-output-contract=v1` (see below). Never silently downgrade to
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
`calls.inspect`, `calls.observe` and `calls.result` report a ledger read
failure as `capacity` with `reason: "ledger_unavailable"` and
`retryable: true`, so it is distinguishable from a read-slot refusal (which
carries `scope`) while existing retry logic keeps working.

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
