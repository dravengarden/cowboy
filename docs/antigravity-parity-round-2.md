# Antigravity parity, round 2 — gap analysis and design

Status: design, revision 3 (2026-11-22), scoped to **round 2a**. Round 2a is
the only part to implement now. Steering, user MCP servers and plan mode are
deferred to round 3 (see
[Deferred to round 3](#deferred-to-round-3-redesign-requirements)).

- Baseline: `origin/main` at `c8db89326`, Antigravity Plugin 0.2.27,
  `cowboy-antigravity-acp` 1.0.22.
- Revision 2 applied the owner's scope decisions on design review
  `call-251d5a849c979dddb1053e03`. Revision 3 applies the 2a/3 split after
  re-review `call-8dc781e16a7ca99702fb61b0` (6 high and 11 medium findings;
  every high finding sits in a deferred item).
- The design it extends is
  [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md).
  The account-risk rules are in [`requirements.md`](requirements.md)
  (owner-paced use).

## Scope

Round 1 (live in 0.2.27) gave Antigravity:

- owner-paced dispatch;
- login-shell `RunCommand`;
- full guidance;
- reads outside the root;
- home skills;
- image reads;
- glob search;
- the continuation hint;
- the proxy-hosted `Subagent`;
- Cowboy MCP, Matrix memory and folder bindings through the worker extension
  broker.

The closest reference for the rest is Claude's remote runtime
(`plugins/claude-code/runtime/`). It also runs the agent on one Machine and its
tools on another.

| # | Gap | Decision | Layers |
|---|-----|----------|--------|
| A1 | Post-edit diagnostics from the Controller | **round 2a** (edits through the apply helper) | Controller, proxy, facade |
| A2 | Subagent model label from confirmed data | **round 2a** | proxy |
| A3 | Built-in `swarm`/`run-workflow` skills note | **round 2a** | proxy |
| A4 | Web labels | **round 2a** | Web |
| R1 | Current-turn steering | deferred to round 3 | proxy, worker, wire, Controller |
| R2 | User MCP servers from owner configuration | deferred to round 3 | worker, keeper, wire |
| R3 | Plan mode enforced at dispatch | deferred to round 3 | proxy |
| D1 | Repository `.mcp.json` servers | deferred | — |
| D2 | Repository hooks | deferred | — |
| D3 | Background commands that outlive a turn | deferred | — |
| D4 | Pure `cowboy_read`/`cowboy_receipt` and a read-only exemption | deferred | — |
| W1 | Native fork | won't do: agy has no `session/fork` | — |
| W2 | Web search/fetch results | won't do: vendor limit | — |

D1 and D2 wait for the same thing: an owner-approval UX scoped to target,
workspace and exact configuration.

Round 2a needs **no change** to the wire (`runtime_wire.rs`,
`execution_protocol.rs`), the worker (`src/acp.rs`, `src/worker.rs`), the
Machine, the keeper or the execution helper. It therefore needs no worker pin
advance and no Machine release; see [Release plan](#release-plan).

### Invariants

Every in-scope item keeps these:

- **Uncertain effects.** A lost reply after an admitted effect is uncertain. It
  is recorded durably until the owner acknowledges it, and never replayed.
  Round 2a adds no effect: its only new delivery is a Controller note that is
  data, and a lost note outcome is `unknown`, never retried.
- **Turn fencing.** The ledger (`src/worker_execution/ledger.rs:6-17`) admits
  effects only inside the active Cowboy turn. Every exception is a named,
  worker-owned class. Round 2a adds none.
- **Fail closed.** A feature is on only when the running peer proves the
  capability. Unknown means off.
- **Authority.** Tool output, repository text and agent text never carry owner
  authority. Round 2a delivers nothing in the user role: a note is data inside
  a tool result.
- **Secrets.** Credentials and gateway context never reach the account unit or
  a process that does not need them. Round 2a adds no process and no
  credential.

## A1. Post-edit diagnostics (P1)

### Current behaviour

- The push runs for Codex only. Its gate is `codex(provider)`
  (`components/server/src/server/agent_api/code_reads/codex_push.rs:26-28`,
  checked at `:160`), and it observes completed `edit` tool calls in the
  session stream.
- The Controller recognizes an apply-helper publish only from adjacent
  `apply`, `--request` argv
  (`components/server/src/server/execution/writes.rs:52-75`). The facade starts
  `["/bin/sh","-c",APPLY,"sh","--request",b64]` with
  `APPLY = 'exec "$COWBOY_EXECUTION_FILE_HELPER" apply "$@"'`
  (`components/provider-runtime/packages/antigravity-acp/facade.mjs:679-703`,
  `facade-tools.mjs:83`), so no `apply` token is in argv and nothing matches.
- Today agy accepts no current-turn delivery. The worker enables it only when
  the agent's `initialize` reply declares `_meta.steering.supported` and
  `idleBehavior: "promptRequired"` (`src/acp.rs:3836-3860`), and the proxy
  forwards agy's reply unchanged.

### Design

The Controller observes an apply-helper publish, runs the same `code.post_edit`
read as for Codex and delivers the result as **untrusted data inside agy's next
facade tool result**. It is never a user-role message.

#### Publish receipt

- The facade passes `apply` as a positional argument: argv becomes
  `["/bin/sh","-c",'exec "$COWBOY_EXECUTION_FILE_HELPER" "$@"',"sh","apply","--request",b64]`.
  The helper still receives `apply --request b64`, and the Controller's parser
  already matches that shape.
- Only the inline form is covered (a request of at most `INLINE_APPLY`, 48 KiB).
  A request above that travels in a file (`--request-file`); recognizing it
  needs the execution helper to report the canonical path, which is a Machine
  change. **A larger edit gets no push in round 2a.** Round 3 or a later Machine
  release can add it.
- On the helper's `applied` outcome for exactly the expected digest, the
  Controller emits one **receipt**: session, worker epoch, turn, the
  `process/start` operation ID, the trusted target path and the helper's
  SHA-256. The existing write log also records it, so `code.post_edit`'s write
  identity (`post_edit::attach_writes`) now covers agy edits.
- The ACP edit update is never used as a trigger or fallback for agy.
- The unit chooses the shell script, so the receipt is a trigger, not a proof. A
  forged receipt can only cause a diagnostics read of current text.

#### Turn identity (event time)

- The worker epoch is the one on the execution request
  (`RuntimeRequest.worker_epoch`). The turn is the session's delivery target
  (`supervisor.delivery_target`), captured **when the `process/start` is
  forwarded** and compared again when the `applied` outcome is read. The two
  must be equal, and the epoch must equal the request's, or the receipt is
  dropped.
- Why this is event time and not ingestion time: an apply helper runs inside the
  turn that started it, because the worker's ledger refuses any effect outside
  the active turn, and the facade awaits the outcome before the turn can end.
  The execution forward is a live request, not a replayed event, so a reconnect
  that installs a newer snapshot and replays older events cannot restamp it.
  The Controller's view can lag the worker (an older turn or none) but cannot be
  ahead, so a lag drops or mis-targets toward an *expired* target, which
  `start_steer` refuses; a receipt is never assigned to a later turn.
- A receipt is one-shot and bounded in Controller memory (256, oldest dropped).
  Nothing survives a Controller restart, and nothing is replayed or retried.
- Calls and in-flight batches are keyed by `(session, worker_epoch, turn)`.
  Coalescing never crosses keys: a receipt for a different key discards the
  pending paths of the earlier one instead of retargeting them.
- A stale receipt is discarded before the observation and again before the
  send. `start_steer` with the captured target still refuses an expired one.

#### Gate

- Codex's gate and its fallback to ordinary steering are unchanged.
- Antigravity requires all of: provider `antigravity`, not a managed child,
  language servers allowed for the session, a busy session with a current turn,
  and the running worker reporting `next_checkpoint`. There is no fallback to
  ordinary steering.
- The release gate tests both providers.

#### The delivery channel (no wire change)

The worker already speaks `_session/steering` to the agent. Today the Controller
refuses steering for agy because agy's `initialize` declares none. Round 2a
uses that existing path for Controller notes only:

- **Proxy advertisement.** For a non-managed unit the proxy adds
  `_meta.steering = {supported: true, idleBehavior: "promptRequired",
  priorities: ["next"]}` to agy's `initialize` result. The worker therefore
  reports `supports_current_turn` and `next_checkpoint` exactly as it does for
  Codex, with no worker change.
- **Owner steering stays off.** The advertisement must not offer owner or agent
  steering, which is round 3. The Controller therefore keeps `antigravity`
  sessions out of the owner surface:
  - the Hub records `supports_current_turn = false` for such a session even when
    the worker reports `true` (`src/core.rs:3262-3276`, fed from
    `src/remote_runtime/snapshot.rs:17`), so the Web never offers steering;
  - the delivery options mask `controls.next_checkpoint` for it
    (`components/server/src/server/send_delivery.rs:419-436`);
  - `send_delivery::begin` takes an origin. An owner or agent origin keeps
    today's refusal ("current-turn delivery is not supported by this running
    worker") for such a session; only the Controller's note origin skips the Hub
    flag and relies on the runtime's own worker check
    (`start_steer_targeted`: `supports_current_turn`, `next_checkpoint`,
    target equality).
  - The origin is a Controller-internal argument, decided by which Controller
    code path calls `begin`. It is not a wire field, and the wire is unchanged.
- **The proxy accepts only a note.** The proxy handles `_session/steering` itself
  and never forwards it to agy. It admits a request only when all hold:
  - a root session with an open, uncancelled turn, not a subagent, no
    compaction running, and no earlier steer queued;
  - `priority` is `next`, `idleBehavior` is `promptRequired` and no
    `nativeCycle` is present;
  - the prompt is exactly one text block of at most 24 KiB whose first line is
    the fixed note header `[Cowboy code intelligence after your edit]`.
  Anything else is answered `promptRequired` at once, before any handoff.
- This is a format restriction, not authentication. It is sufficient because the
  proxy never delivers a steer in the user role: the most any steer can do is put
  data in a tool result. A new proxy behind an older Controller therefore offers
  owner steering that the proxy refuses (`not_delivered`), never delivers as a
  user turn. Deploy order below avoids even that.

#### Delivery and acknowledgement

1. **Queue.** An admitted note is queued on the session's turn. One note at a
   time.
2. **Handoff.** The proxy appends the note as an extra text block to the result
   of the next facade call that satisfies all of:
   - a root-session call, a built-in tool other than `UpdatePlan`, `Subagent`,
     `StopSubagent` and `StopCommand`, with a result that is not an error;
   - the turn is still open, uncancelled and has no uncertain effect;
   - the call binds **unambiguously** to one ACP tool call: exactly one waiting
     `cowboy` announcement exists for that tool name, with the same arguments
     (`AcpProxy.bind`, `proxy.mjs:491-504`, otherwise matches heuristically).
     An ambiguous call carries no note; the note waits for the next one.
   Removing the note from the queue and writing the block is one synchronous
   step: a note is never both removed and handed off. The presentation of the
   tool call (`resultContent`) uses the result without the note.
3. **Acknowledgement.** The worker's `injected` is native receipt
   (`docs/agent-api.md`, "Send delivery"). It is answered only when agy reports
   the **terminal `completed` `tool_call_update` for that exact ACP call**. A
   `failed` update, a lost stream, a turn that ends without that update, or the
   acknowledgement deadline after handoff answer `unknown`. Writing to the relay
   is never an acknowledgement.
4. **Not delivered.** The proxy answers `promptRequired` only for a note it
   removed from the queue before the handoff: the window expired, the turn
   ended cleanly with no tool boundary, a cancel arrived or the turn was
   fenced. Diagnostics are never sent as a user-role continuation.
5. **Timing.** The worker holds its steering gate for the RPC and gives up after
   25 s (`src/acp.rs:4301`, `:4313-4318`); the Controller waits at most 30 s
   (`SteeringDelivery::outcome`). The proxy therefore decides within 20 s of
   receiving the note: 15 s to reach a tool boundary, then 5 s for the
   acknowledgement. A plain Stop does not take that gate
   (`AgentCommand::Cancel`, `src/acp.rs:4769`); a targeted interrupt waits for
   it, at most 20 s. The worker gate fix and a longer window belong to round 3
   with steering.
6. **Transcript.** On `injected`, the worker pushes the delivered text as it does
   for Codex: a row tagged with the Controller's `cowboy-code-intelligence:`
   message id, shown by the Web as "Code intelligence after an edit"
   (`components/store/src/prompt_origin.rs`, `web/src/promptOriginPresentation.ts`).
   It is a Cowboy-origin note carrying exactly what agy received, never a
   human-composer bubble. No row exists for `unknown` or `not_delivered`.
7. **No other effects.** A note changes no file, starts no process and holds no
   Controller capacity: it does not use the owner send semaphore, so Interrupt
   capacity is untouched.

#### Out of scope in round 2a

- Shell-made writes (`RunCommand`). `shell_changes` has no baseline for the
  first foreground command of a session and cannot attribute a background
  command's later writes. An agy shell edit gets no push; the agent can call
  `cowboy.code.diagnostics`.
- Edits above 48 KiB (see Publish receipt).
- Background commands (D3).

### Tests

Controller (`writes` and `code_reads::publish_push`):

- the new facade argv produces a receipt on the `applied` outcome and the old
  shape still does not;
- the receipt carries the request's worker epoch and the turn captured at start;
  a turn change between start and outcome drops it;
- a delayed receipt across a turn boundary is discarded before observation and
  before the send;
- coalescing never crosses keys;
- the gate: Codex unchanged (steering fallback kept), Antigravity requires
  `next_checkpoint` and has no fallback;
- `begin`: an owner origin is refused for an Antigravity session whose worker
  reports support, the note origin is not; the Hub flag and the options mask.

Proxy (`proxy.test.mjs` with `fake-agy.mjs`):

- `initialize` advertisement for a non-managed unit only;
- admission: a forged or malformed note, an owner message, a second note, a
  subagent session, a cancelled turn and a compaction are `promptRequired`;
- handoff on the next unambiguous facade result; an ambiguous bind waits;
- `injected` only on the terminal `completed` update of that exact call;
  `failed`, a stream loss and a turn end after handoff are `unknown`;
- window expiry, clean turn end and cancel before handoff are `promptRequired`;
- a note never rides on an error result, a subagent call or an unsupported tool;
- the presentation of the tool call does not contain the note.

Facade: the argv shape of an inline publish.

### Live acceptance

In a Columbus agy session:

- an inline `EditFile` that introduces a Rust type error delivers
  "[Cowboy code intelligence after your edit]" in the next tool result and the
  receipt says `injected`;
- no note arrives after the turn ends;
- an edit above 48 KiB delivers none (round 2a limit).

## A2. Subagent model label (P2)

### Current behaviour

- The worker labels a child from the child's own `config_option_update` or a
  one-model usage breakdown (`src/acp_subagents.rs:234-279`, `:500-508`,
  `:682-689`). The 3.134 rollout shows the label
  (`docs/releases/antigravity-subagents-3-134-rollout-2026-11-22.md:59`);
  `docs/subagents.md:36` still says none is reported.
- The proxy discards the child's `session/new` result. After it opens a child it
  replays the parent's `session/set_model` and `session/set_config_option`
  settings (`subagents.mjs:171-193`, `:431-438`). A replay reply may carry the
  new configuration without a separate update, and the child's first
  `config_option_update` may predate the replay. The label can then name the
  default model while the child runs another.

### Design

The label is the child's **confirmed effective model**. The proxy tracks it per
child and tells the worker only what is confirmed:

- Confirmed sources, in stream order: the `session/new` result's
  `configOptions`; a successful replay reply's `configOptions`; and the child's
  own `config_option_update` received **after** the last replay reply. Never a
  replayed request.
- While the child's session is held during opening, the proxy withholds its
  `config_option_update` frames. When the replay settings are applied it
  invalidates the model: a frame received before the last replay reply is stale
  and dropped.
- If a replay changed the model and neither its reply nor a later update names
  the effective model, the model is **unknown**: no label is forwarded.
- When the child is released to the worker the proxy sends exactly one
  `config_option_update` under the child's session id, built from the last
  confirmed configuration, or none when the model is unknown.
- A model that changes later is confirmed only by agy's own update for the child.
- While the model is unknown, no label and no usage attribution is produced for
  the child. A usage breakdown that names exactly one model still names the child
  in the worker, as it does today; that is the vendor's own per-model data.

Usage is **not** added in round 2a. The pinned agy reports no per-child token
usage today. Whether it reports usage on a child's prompt result is a probe, not
an assumption: aggregating the parent's turn as one `_meta.quota.model_usage`
`token_count` row per model needs a confirmed shape and its own acceptance, and
waits for the probe receipt. Usage stays absent when unreported.

`docs/subagents.md` is corrected: a model label is reported when confirmed,
tokens are not.

### Tests

`subagents.test.mjs`: the new-session result names the model; a replayed
`set_model` whose reply carries the new configuration labels the child with it;
a pre-replay update is dropped; a replay with no confirmation forwards no label;
a later agy update changes it; a refused replay still fails the start.

### Acceptance

A Columbus run of two subagents (one on a replayed model) shows each child's
confirmed model label. Tokens appear only if a recorded probe found them.

## A3. Built-in skills note (P2)

- The slash commands are already dropped (`presentation.mjs:334-349`), and the
  hook denies their tools (`hook.mjs:57-67`).
- The routing projection names `swarm`, `run-workflow`, `schedule`, `browser`
  and `learn` as unavailable in a Cowboy session, with the alternatives
  (`Subagent`, managed calls). Whether the pinned agy's own prompt still offers
  them is a live probe; the note is harmless if it does not and is needed if it
  does, so it ships unconditionally and the probe result is recorded at
  acceptance.
- Acceptance: "use swarm to …" leads to `Subagent` with no denied call.

## A4. Web labels (P2)

- `web/src/managedCalls.ts:273-279` `providerLabel` shows the raw
  "antigravity".
- `web/src/agentTools.ts:162-168` `agentLabel` title-cases it.
- Both use the Plugin display name (`manifest.display.name`) when the Plugin
  registry names the provider, and otherwise keep today's text.
- `web/src/sessionFork.ts:4` is justified: it mirrors W1.
- The account, quota and browser specifics are justified Provider facts.

## Deferred to round 3: redesign requirements

These three items stay out of round 2a. Re-review `call-8dc781e16a7ca99702fb61b0`
found high findings in each that no wording change fixes: they need a different
mechanism. Round 3 starts from these requirements, not from the revision-2
designs, which are replaced by them. Nothing here is implemented, and round 2a
must not build toward any of it beyond the note channel described in A1.

### R1. Current-turn steering (owner and agent input)

What agy can and cannot do:

- agy accepts input only as a `session/prompt`, which is a user-role turn.
- A facade tool result is tool output and cannot carry owner authority. Owner
  text can therefore reach the model with owner authority only as a user-role
  continuation prompt; a tool boundary can carry only a request to finish soon.

High finding to resolve:

- **Agent steers acquire owner authority.** An MCP `queue.submit` from another
  session takes the same continuation path as an owner steer and can share its
  prompt, so agy receives the blocks in the user role without their trusted
  origins. Requirements:
  - an authenticated origin channel from the Controller admission path to the
    proxy, covering the Controller-to-Machine and Machine-to-worker relays, with
    absent or unknown origin rejected before handoff, and mixed-version tests for
    both relays;
  - raw user-role continuations only for owner steers;
  - peer-agent messages through a separate, explicitly untrusted channel or
    refused before handoff;
  - origin preserved in the transcript.

Medium findings that belong to the same redesign:

- The Controller drops the receipt waiter after 30 s
  (`SteeringDelivery::outcome`) while a steer can wait minutes. Carry one bounded
  deadline through Controller, worker and proxy, allow time for the terminal
  acknowledgement, and reserve Interrupt capacity outside the shared send
  semaphore.
- A `user_message_chunk` carries a native session id and no correlation to the
  continuation attempt, so a delayed echo of identical text can settle the wrong
  steer. Use an echo only with proven attempt correlation; otherwise wait for the
  exact prompt result and classify a JSON-RPC rejection or loss explicitly.
- Releasing the worker `steering_gate` is not enough: after Stop reaches the
  proxy, `AcpProxy.cancel` waits for root facade cleanup and child cancellation
  before forwarding `session/cancel`, and an in-flight extension call can hold
  that for up to 170 s. Latch the cancel, fence the queued continuation
  synchronously and forward native cancel before awaiting cleanup. Test with an
  extension call outstanding.
- A steer that contains `/compact` bypasses compaction bookkeeping. Refuse native
  slash commands before steering handoff.

Round 2a keeps the existing refusal for owner and agent steering of an Antigravity
session (A1, "Owner steering stays off").

### R2. User MCP servers from owner configuration

Source: owner-authored entries in the target home's `~/.claude.json` (user and
per-project `mcpServers`, minus `disabledMcpServers`), stdio only, as Claude's
remote runtime reads them (`plugins/claude-code/runtime/tools.mjs:2670-2700`).
Repository `.mcp.json` stays excluded (D1).

High findings to resolve:

- **Opening a worktree can execute repository code.** The broker starts servers
  before a turn with the workspace as cwd, so an owner entry such as
  `node ./server.js` runs a branch-controlled file on open, and an earlier
  session's `RunCommand` can rewrite the same user's `.claude.json`. Requirement:
  a frozen, owner-admitted launch plan; pre-turn executables and startup inputs
  kept outside mutable worktree state.
- **Omitting `COWBOY_CALL_CONTEXT` leaves the gateway credential accessible.** The
  keeper starts ordinary processes under the target user, whose private gateway
  files hold the socket and capability. Requirement: a target-side filesystem and
  socket boundary for user servers and their resolver, tested by a server that
  cannot read the context file or connect to the gateway. A credential-isolated,
  frozen launch plan covers both findings.
- **The pending record can disappear before the result reaches the proxy.** A
  record cleared on the server reply loses the effect if the worker dies before
  delivering it. Requirement: keep the record through a correlated proxy
  acknowledgement, or atomically convert it to an uncertain effect on recovery;
  gate older proxies that cannot acknowledge.
- **Schema-2 writes can precede a safe rollback floor.** The ledger `SCHEMA` bump
  makes older workers refuse the ledger, and OVH's timer and the broker fallback
  can select one. Requirement: ship a reader for both schemas first, preserve
  existing uncertainty sequences, and fence schema-2 writes until automatic,
  manual and per-session fallback all use compatible readers. The Hawk
  production-conformance step then needs the full artifact set (two distinct
  writer revisions and ELFs, a reader-only fallback on the same worker
  generation, a default-reader artifact and a fresh receipt path).

Medium findings that belong to the same redesign:

- A user-server process on a retained keeper can outlive a worker replacement.
  Persist server identity and keeper scope before launch; fence it on an
  uncertain dispatch or reopen and confirm termination before discovery or reuse.
- Required bindings must keep their budget: reserve it for them, refuse startup
  if they do not fit, and bound optional launch concurrency.

### R3. Plan mode enforced at dispatch

Today the mode is checked only in `session/request_permission`
(`proxy.mjs:1043`); a facade call that arrives without that request is not
checked against it.

High finding to resolve:

- **Plan approval has no reliable identity at facade dispatch.** The facade MCP
  request carries a tool name and arguments but no ACP `toolCallId`, and the
  proxy matches announcements heuristically, so two identical concurrent calls
  can let an unapproved invocation consume the other's grant. Requirement:
  unambiguous correlation at dispatch, bound to the native session, attempt, tool
  and canonical arguments; ambiguous calls refused; grants expire on cancel, turn
  end or mode change.

The design to refine: `Facade.answer` reads the parent session's confirmed mode at
dispatch (unknown or switching counts as plan); in plan mode reads and
`UpdatePlan` run, `WriteFile` and `EditFile` are refused, and `RunCommand` and
extension calls run only with an approval bound to the exact call, consumed once.
A1's unambiguous bind is a first step toward the required correlation but is not
sufficient on its own.

## Other deferred items and their requirements

### D1. Repository `.mcp.json`

Opening a worktree must not run repository code. Requirements:

- owner approval scoped to target, workspace and the exact resolved
  configuration, frozen across list and run;
- the frozen, credential-isolated launch plan that R2 requires.

### D2. Repository hooks

- The approved hook source and its content digest are pinned to the execution
  workspace. A changed, unreadable or unsupported approved source is a policy
  failure, and replacing or disabling it requires approval. A new
  `.codex/hooks.json` must not shadow an approved Claude guard.
- A fail-closed PreToolUse preflight: timeout, start failure and denial by
  exit status or stderr all deny.
- The turn is rechecked after each hook await.
- A PostToolUse rejection keeps the committed outcome and only blocks further
  work.
- The rules for UserPromptSubmit on steers and automatic notices are defined.

### D3. Background commands

- Worker-allocated process ids through a dedicated admission path, which
  checks the ordinary root turn and the negotiated policy.
- Later control is scoped to the recorded owner.
- Durable records (process, binding, worker epoch, deadline) are written
  before dispatch.
- Expiry is enforced on the target, or cleanup is confirmed before a binding
  is reused. Tests cover worker death with an immediate resume and a lost
  termination reply.
- A negotiated, worker-epoch-scoped completion event, with one stable notice
  id per command and exactly one delivery path:
  - steered into a running turn as data;
  - otherwise one automatic notice, never also prepended.

  It preserves Pause, account gates and dedup across reconnects.
- Owner decision 2026-11-22: when idle, a completion notice may start agy's
  next turn, as call and peer notices do. `requirements.md` names it when D3
  ships.

### D4. Pure reads and the read-only exemption

- A versioned purity contract through the target CLI, gateway
  (`Action::AgentRead`) and Controller. Only a typed, pre-dispatch
  "unsupported" may permit a separately admitted, non-exempt fallback.
  `src/agent_mcp.rs` rejects extra arguments today.
- An audited allowlist of read implementations and receipt domains:
  - `code.diagnostics` starts the observer and moves baselines;
  - login receipts run `finalize_committed` and `native_recovery::observe`
    (`.../login/operations.rs:163-183`).
- Login reconciliation at startup and reconnect under the original fences.
  The `accounts` receipt lookup is kept.
- The classification is carried through endpoint settlement and the proxy, so
  a lost read is an ordinary tool error rather than `UncertainEffect`. Exempt
  reads get a separate concurrency bound.

## Won't do

**W1. Native fork.**

- agy's `initialize` offers `load`, `resume`, `list` and `close`, but no fork
  (`docs/antigravity-provider-plan.md:110`).
- The Controller allows only `claude-code` and `codex`
  (`components/server/src/server/session_context/fork.rs:8`, `:136`).
- Linked copy stays.
- Revisit when the agy-pin probe reports `sessionCapabilities.fork`.

**W2. Web search/fetch.** agy reports no result over ACP
(`presentation.mjs:34-36`). Recovering it would need a new network effect.

## Dependencies

- A1's Controller part (receipt, key-aware batching, gate, `begin` origin, Hub and
  options masks) and its proxy part (advertisement, note handling, facade argv)
  are independent code, but the feature is on only when both are present. The
  Controller must ship first.
- A2, A3 and A4 are independent of each other and of A1.

## Release plan

Round 2a ships in three releases. No Machine release, worker pin advance,
`workerGeneration` change or keeper change is involved: the diff touches none of
`Cargo.toml`, `Cargo.lock`, `runtime_wire.rs`, `execution_protocol.rs`,
`execution_environment.rs`, `machine_protocol.rs` or the SDK sources that
`retained-worker-interface-compatible` pins
([`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md)),
and the retained worker source is unchanged. The implementation records that with
a diff check before it is handed off.

### 1. Controller release (first)

- **Contents:** the publish receipt and its turn capture, key-aware batching, the
  Antigravity gate in the post-edit push, the Antigravity note origin in
  `send_delivery::begin`, and the Hub and options masks for owner steering of an
  Antigravity session.
- **Build:** `.#cowboy-controller-release` from the commit whose complete gate
  passed.
- **Effect:** it restarts only `cowboy.service`. Codex's push must be unchanged.
  It is inert for agy until the Plugin release: an old proxy advertises no
  steering, and the facade's old argv produces no receipt.
- **Rollback:** a previous Controller restores the old behavior; a new proxy then
  gets owner steering offered and refuses it (`not_delivered`), never as a user
  turn.

### 2. Web release

`.#cowboy-web-release` from the same commit carries A4. It restarts nothing.
Reload the PWA hard, since a WS reconnect keeps stale JS.

### 3. Component/Plugin release (last)

- **Contents:** provider-runtime with `cowboy-antigravity-acp` and the
  Antigravity Plugin carrying A1's proxy part, A2 and A3 (the facade `apply`
  argv, the steering advertisement and note handling, child label tracking and
  the skills note). The other Providers get a patch bump for the shared lock.
- **Release:** through `.agents/skills/release-cowboy-plugin/`, then
  `cowboy operator converge` (canary Hawk, then OVH). Antigravity on OVH is
  installed by exact release.
- **agy pin:** unchanged. A future pin first passes `just agy-barrier-probe` and
  the OVH `agy-sha512.allow` update.
- **Existing sessions:** they keep their pinned Plugin generation. Acceptance uses
  fresh Columbus and Suger agy sessions. A retained session keeps the old argv and
  proxy and gets no note, which is the intended fail-closed state.

### Mixed-version checks for the release record

- New Controller with an old proxy: no steering advertised, no receipt, owner
  surface unchanged.
- New proxy with an old Controller: not deployed in order; if it happens, owner
  steering is offered and refused by the proxy as `promptRequired`.
- New facade argv with an old Controller: the old parser matches too, so the write
  log records the edit, and nothing is pushed.
- Codex sessions: delivery, steering fallback and the cmid origin are unchanged.
