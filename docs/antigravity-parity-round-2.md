# Antigravity parity, round 2 — gap analysis and design

Status: proposed design, revision 2 (2026-11-22). Nothing here is implemented.

- Baseline: `origin/main` at `c8db89326`, Antigravity Plugin 0.2.27,
  `cowboy-antigravity-acp` 1.0.22.
- Revision 2 applies the owner's scope decisions on design review
  `call-251d5a849c979dddb1053e03`.
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
| 1 | Current-turn steering | in, P0 | proxy, worker, wire |
| 2 | User MCP servers from owner configuration | in, P0 | worker, keeper, wire |
| 3 | Post-edit diagnostics | in, P1 (needs 1) | Controller, proxy |
| 4 | Plan mode enforced at dispatch | in, P2 | proxy |
| 5 | Subagent model and usage labels | in, P2 (confirmed data only) | proxy |
| 6 | Built-in `swarm`/`run-workflow` skills note | in, P2 | proxy |
| 7 | Web labels | in, P2 | Web |
| D1 | Repository `.mcp.json` servers | deferred | — |
| D2 | Repository hooks | deferred | — |
| D3 | Background commands that outlive a turn | deferred | — |
| D4 | Pure `cowboy_read`/`cowboy_receipt` and a read-only exemption | deferred | — |
| W1 | Native fork | won't do: agy has no `session/fork` | — |
| W2 | Web search/fetch results | won't do: vendor limit | — |

D1 and D2 wait for the same thing: an owner-approval UX scoped to target,
workspace and exact configuration.

### Invariants

Every in-scope item keeps these:

- **Uncertain effects.** A lost reply after an admitted effect is uncertain. It
  is recorded durably until the owner acknowledges it, and never replayed.
- **Turn fencing.** The ledger (`src/worker_execution/ledger.rs:6-17`) admits
  effects only inside the active Cowboy turn. Every exception is a named,
  worker-owned class.
- **Fail closed.** A feature is on only when the running peer proves the
  capability. Unknown means off.
- **Authority.** Tool output, repository text and agent text never carry owner
  authority. Origin travels outside content.
- **Secrets.** Credentials and gateway context never reach the account unit or
  a process that does not need them.

## 1. Current-turn steering (P0)

### Current behaviour

- The worker enables current-turn delivery only when the agent's
  `initialize` reply declares `_meta.steering.supported` and
  `idleBehavior: "promptRequired"` (`src/acp.rs:3836-3860`). agy declares
  neither, and the proxy forwards agy's reply unchanged.
- The Controller therefore refuses with "current-turn delivery is not
  supported by this running worker"
  (`components/server/src/server/send_delivery.rs:101-103`).
- The proxy also refuses `_session/steering`, because it is not in
  `RUNTIME_REQUESTS`
  (`components/provider-runtime/packages/antigravity-acp/proxy.mjs:40-50`,
  `:586-587`).

### Reference

- Codex `turn/steer` adds input to the active turn
  (`third_party/codex-acp/patches/0003-steer-prompt-required.patch`).
- Claude steers with priority `now` or `next`
  (`plugins/claude-code/runtime/steering.mjs`).
- `docs/agent-api.md` ("Send delivery") defines the outcomes:
  - `injected` means native receipt;
  - `not_delivered` permits a new choice;
  - `unknown` requires observing the original receipt.

### What agy can and cannot do

- agy accepts input only as a `session/prompt`, which is a user-role turn.
- A facade tool result is tool output. It cannot carry owner authority.

Owner text is therefore delivered only as a user-role continuation prompt. A
tool boundary carries only a request to finish soon, which grants nothing. A
forged copy of that request can only make agy stop early.

### Origin

- The wire `Steer` command (`components/wire/src/runtime_wire.rs:645-654`)
  gains `origin: owner | agent | cowboy_diagnostics`.
- The Controller fills it from the admission path:
  - Web and owner CLI sends are `owner`;
  - MCP `queue.submit` from another session is `agent`;
  - the post-edit push is `cowboy_diagnostics`.
- An old worker's frame decoder would reject an unknown field. The Controller
  therefore sends `origin` only to a worker whose snapshot reports
  `supports_steering_origin`, as it does today for `supports_subagent_cancel`.
- The worker passes the origin to the proxy as
  `_meta.steering.origin`.

### Negotiation

- A new worker declares
  `clientCapabilities._meta.cowboy.steering = {origin: 1, windowMs: 1}` in
  `initialize`.
- The proxy advertises
  `_meta.steering = {supported: true, idleBehavior: "promptRequired",
  priorities: ["next"], windowMs: 300000}` only when that declaration is
  present, and only for non-managed units.
- An old worker therefore never gets agy steering.
- A new worker uses `min(windowMs, 300 s)` instead of its fixed 25 s for this
  agent. While the window is open the Controller receipt stays `pending`.

### Worker change

- Today an in-flight steer holds `steering_gate` for its whole RPC
  (`src/acp.rs:4301`), and the targeted Interrupt and cancel paths wait on the
  same gate (`:4458-4475`, `:4734`).
- Change it so the gate orders only the admission of a steer against a new
  native prompt. The gate is released before the steering RPC is awaited.
- Cancel and Interrupt must reach the proxy while a steer waits. A conformance
  test holds a steer open and cancels the turn.

### Proxy admission

- The proxy handles `_session/steering` itself and never forwards it.
- It admits a steer only for a root session with an open turn, the current
  native prompt, and no `/compact` running.
- It never admits a steer for a subagent or a managed unit.
- Every block is validated against agy's `promptCapabilities` (text, image,
  embedded resource). If any block is unsupported, the proxy refuses the whole
  request before handoff with `promptRequired`. It never drops part of a
  message.
- Priority `now` is treated as `next`.

### Delivery of an owner or agent steer

1. **Queue.** The steer is queued on the turn with a proxy delivery id. On
   agy's next facade tool result, the proxy appends one fixed data block:

   > "Cowboy has a message for this turn; finish the current step and end
   > your turn to receive it."

   The block contains no owner text and grants nothing.
2. **Continuation.** Only after agy's prompt result for the current attempt
   meets all of these:
   - its `stopReason` is `end_turn` (not `cancelled`, refusal or error);
   - that attempt's evidence settled `ok` with its own barrier, with no policy
     violation and no uncertain effect;
   - after every await: the turn is still the current one, no cancel was seen,
     the window is still open, and the steer is still queued.

   The proxy then holds the worker's reply and sends a continuation
   `session/prompt` whose prompt is the owner's blocks unchanged. This is the
   same hold-and-continue pattern as the managed-output correction
   (`proxy.mjs` `correct()`, `:1340`). The continuation gets its own evidence
   cursor and settlement. The worker's original prompt is answered with the
   continuation's settled result.
3. **Acknowledgement.**
   - `injected` is sent once agy echoes the continuation's first
     `user_message_chunk`, or once it answers the continuation prompt at all.
     Either proves native receipt.
   - Those echoes are suppressed by delivery id and attempt. The worker has
     already pushed the steered bubble, and the blocks are not wrapped, so
     matching is exact.
   - A transport loss after the write and before an acknowledgement answers
     `unknown`.
4. **Not delivered.** The proxy answers `promptRequired` only when it removed
   the steer from the queue before writing the continuation. That happens on
   the window deadline, a cancel, a failed recheck, an end that was not
   `end_turn`, or a closed turn. A steer is never both removed and handed off;
   the proxy is single-threaded and the removal is the decision point.
5. **Several steers.** Queued steers share one continuation. The proxy
   acknowledges each RPC separately.

### Diagnostics delivery (`origin: cowboy_diagnostics`)

- The steer is delivered into the next facade tool result as untrusted data.
  The text is the Controller's already escaped `note()`
  (`components/server/src/server/agent_api/code_reads/codex_push.rs`), inside
  a fixed presentation that says it is data.
- It is acknowledged `injected` when agy reports that call's terminal
  `tool_call_update`, which proves agy consumed the result.
- A stream loss after the write gives `unknown`.
- If there is no tool boundary before the clean end of the turn, the steer is
  removed and gets `promptRequired`. Diagnostics are never sent as a user-role
  continuation.

### Tests

In `proxy.test.mjs` with `fake-agy.mjs`:

- a continuation after a clean `end_turn`;
- no continuation after cancel, refusal, violation or uncertain effect;
- the deadline before handoff gives `promptRequired`;
- a loss after handoff gives `unknown`;
- backpressure on the relay and on agy's stdin;
- mixed text and image blocks, and an unsupported block that is refused whole;
- echo suppression;
- a forged finish request in `RunCommand` output, which can only end the turn;
- diagnostics acknowledged by the terminal update.

Worker conformance (`src/acp_session_conformance_tests.rs`):

- origin pass-through;
- the window;
- cancel while a steer waits.

### Live acceptance

- Columbus agy session. During a multi-tool turn, send "current turn: also
  print the date". agy finishes its step, the continuation runs, the bubble
  appears once, and the receipt says `injected`. Stop during the wait still
  stops at once.
- Suger agy session. Steer, then cancel before agy ends its step. The receipt
  says `not_delivered`, and no continuation runs.

## 2. User MCP servers from owner configuration (P0)

### Current behaviour

- The proxy replaces `mcpServers` with the facade (`proxy.mjs:639`).
- The broker runs only Cowboy MCP and folder bindings
  (`src/worker/sandbox_extensions.rs:74-108`).

### Reference

Claude's remote runtime reads `~/.claude.json` and relays each server
(`plugins/claude-code/runtime/tools.mjs:2670-2700`, `mcp-proxy.mjs`).

### Sources

The only sources are owner-authored entries in the target home's
`~/.claude.json`:

1. the user `mcpServers`;
2. then `projects[<repository root>]` and `projects[<cwd>].mcpServers`;
3. minus `disabledMcpServers`.

Rules:

- Repository `.mcp.json` is excluded (deferred, D1).
- Only stdio servers are supported. HTTP and SSE entries are listed as
  unavailable.
- `${VAR}` and `${VAR:-default}` are expanded from the target environment by a
  fixed, worker-owned probe that reads only the named variables. The same
  bounded read is used by Claude's remote runtime.
- Expanded values stay in worker memory. They are never sent to the unit,
  persisted or logged.
- A JSON vector fixture is shared with Claude's `mcpInputs` tests so that the
  two parsers cannot drift.

### Launch environment

- The worker starts each server under `SERVER_PREFIX`. The ledger
  (`ledger.rs:45`) gives these processes session lifetime, refuses any unit
  request that names them, and ends them at teardown.
- Today the broker inherits the whole target environment
  (`extension_broker.rs:443-455`). In addition, the keeper's execution host
  adds `COWBOY_CALL_CONTEXT` to every `process/start`
  (`components/wire/src/execution_protocol.rs:96-115`,
  `components/machine/src/execution_host.rs:305`).
- User servers instead use:
  - `envPolicy.inherit: "none"`;
  - `includeOnly` of `PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `TZ` and
    `TMPDIR`;
  - the entry's own `env`;
  - a new start field `callContext: "omit"`, which the execution host honors
    by not injecting the gateway context.
- The keeper advertises `omitCallContext` in its target initialization. The
  worker offers user servers only when the keeper advertises it.

### Discovery

- Required bindings keep their existing deadline and outcome.
- Each optional server (Cowboy MCP and user servers) is discovered under its
  own 20 s deadline, in a separate task.
- A hung optional server is terminated, and its ledger `starting` entry is
  settled. It cannot discard a ready required binding.

### Aggregate limits

Limits are enforced in the broker before anything is forwarded to the unit:

- 128 tools across all servers, admitted in this order: Cowboy, bindings,
  memory, then user servers in configuration order;
- 256 KiB of descriptions and schemas in total;
- 1 MiB per result, refused as a tool error.

The proxy only presents results. A server left out by a limit is terminated
and named as unavailable in the projection.

### Durable pending record

- Before dispatching a user-server `tools/call`, the worker appends to the
  ledger file: sequence, server, tool name, argument digest, worker epoch and
  turn. The record is bounded by `MAX_CALLS` (8).
- The record is settled when the reply arrives or the call is refused before
  sending.
- When a ledger is reopened, any unresolved record marks the session uncertain
  at that sequence, using the existing owner acknowledgement. Nothing is
  replayed.
- This bumps the ledger `SCHEMA` to 2. A worker that only reads schema 1
  refuses a schema-2 ledger, so rollback must stay at or above the new
  generation; see the release plan.

### Calls

Calls take the existing path (`extension_broker.rs:294-370`):

- admitted only in the turn, which also covers subagents in the parent's turn;
- the outcome is decided before the turn may complete;
- a lost reply is `mark_uncertain`.

Managed units get no user servers: `derive` must skip them for a managed
descriptor.

### Compatibility

- Old keeper (a retained environment): no `omitCallContext`, so no user
  servers.
- Old worker: none.
- Old proxy: user tools are listed as ordinary extension tools through
  `cowboy/extensions`.

### Tests

In `src/worker_execution/tests.rs`:

- the minimal environment and no call context at the keeper;
- a hung optional server next to a ready required binding;
- termination of omitted servers;
- the aggregate caps;
- a pending record left by a worker crash becomes uncertain on reopen;
- a lost call is uncertain;
- no starts outside a turn.

Plus the resolver vectors.

### Live acceptance

- Columbus agy session on a fresh binding:
  - `mcp__chrome-devtools-draven__list_pages` works and a screenshot renders;
  - `printenv COWBOY_CALL_CONTEXT` in the server's environment is empty;
  - killing the worker mid-call shows an uncertain effect after resume.
- Suger agy session: `chrome-devtools-suger` is the server used.
- A retained binding shows no user servers.

## 3. Post-edit diagnostics (P1; needs 1)

### Current behaviour

- The push runs for Codex only (`codex_push.rs:26`, `:160`).
- The Controller recognizes an apply-helper publish only from adjacent
  `apply`, `--request` argv
  (`components/server/src/server/execution/writes.rs:52-75`).
- The facade starts
  `["/bin/sh","-c",APPLY,"sh","--request",b64]`, or `--request-file` above
  48 KiB (`facade.mjs:679-703`). Neither form matches.

### Design

- **Publish receipt.**
  - Inline form: the facade passes `apply` as a positional argument, so its
    argv has the `apply --request` shape the Controller already parses. The
    Controller then records the trusted target path and the helper's `applied`
    SHA-256.
  - File form: the execution helper adds the canonical `path` to its
    `applied` outcome, and the Controller accepts `apply --request-file` with
    that path. Until both sides have this, a file-form edit gets no push.
  - The ACP edit path is never used as a fallback.
  - The unit chooses the shell script, so the receipt is a trigger, not a
    proof. A forged receipt can only cause a diagnostics read of current text.
- **Turn identity.**
  - When the Controller ingests a runtime event, before broadcast, it attaches
    the session's `(worker_epoch, turn_id)` delivery target to the in-memory
    frame.
  - `Calls` and in-flight batches are keyed by
    `(session, worker_epoch, turn_id)`. Coalescing never crosses keys.
  - Work for a target that is no longer current is discarded before the
    observation and again before the send. `start_steer` with that target
    still refuses.
  - A test delays a completion across a turn boundary.
- **Gate.** Codex's gate and its fallback to ordinary steering are unchanged.
  `antigravity` additionally requires the running `next_checkpoint` and
  `supports_steering_origin`. The release gate tests both providers.
- **Shell coverage.**
  - For agy sessions with language servers enabled, the Controller establishes
    the code-observer cursor when a turn starts, so `shell_changes` has a
    baseline before the turn's foreground `RunCommand` calls.
  - Writes before the cursor are a gap: silence is not "clean".
  - Background commands are out of scope (D3).
- **Delivery.** `origin: cowboy_diagnostics` uses the data path of item 1.

### Live acceptance

In a Columbus agy session:

- an inline `EditFile` that introduces a Rust type error delivers
  "[Cowboy code intelligence after your edit]" in the next tool result;
- no note arrives after the turn ends;
- an edit over 48 KiB delivers one only after the helper change.

## 4. Plan mode (P2)

### Current behaviour

- The mode is checked only in `session/request_permission` (`proxy.mjs:1043`).
- In `plan` mode every facade call asks the owner.
- A call that reaches the facade without that request is not checked against
  the mode.

### Design

- `Facade.answer` reads the parent session's confirmed mode at dispatch. A
  mode still unknown or switching counts as plan.
- In plan mode:
  - ReadFile, ListDirectory, SearchFiles and UpdatePlan run;
  - WriteFile and EditFile are refused with "plan mode: present the plan with
    UpdatePlan and wait";
  - RunCommand and every extension call run only with an approval bound to the
    exact call: `toolCallId` plus an argument digest, recorded when the owner
    selects allow, consumed once, and checked against the mode again at
    dispatch;
  - nothing effectful is auto-approved.
- Subagents act in the parent's mode, as today.

### Tests and acceptance

- A test matrix covers dispatch without a permission request and a mode switch
  while approval is pending.
- Live: in plan mode, reads run without prompts, edits are refused, and an
  approved command runs only as approved.

## 5. Subagent model and usage labels (P2)

### Current behaviour

- The 3.134 rollout shows a child model label
  (`docs/releases/antigravity-subagents-3-134-rollout-2026-11-22.md:59`).
  `docs/subagents.md:36` still says none is reported.

### Design

- The label comes only from confirmed child configuration: agy's
  `session/new` result `configOptions`, or agy's own `config_option_update`
  for the child. It never comes from a replayed request.
- Probe the pinned agy for usage on a child's prompt result:
  - if it reports usage, put it on the idle `subagent_update`, and aggregate
    the parent's turn as one `_meta.quota.model_usage` `token_count` row per
    model (parent and children of the same model summed), so the worker counts
    one turn per model;
  - if it does not, keep usage absent.
- Correct `docs/subagents.md`.

### Acceptance

The probe receipt is recorded. A Columbus run of two subagents shows confirmed
model labels, and tokens only if the probe found them.

## 6. Built-in skills note (P2)

- The slash commands are already dropped (`presentation.mjs:334-349`), and the
  hook denies their tools (`hook.mjs:57-67`).
- Probe whether agy's own prompt still offers `swarm` or `run-workflow`. If it
  does, the routing projection names `swarm`, `run-workflow`, `schedule`,
  `browser` and `learn` as unavailable, with the alternatives (`Subagent`,
  managed calls).
- Acceptance: "use swarm to …" leads to `Subagent` with no denied call.

## 7. Web labels (P2)

- `web/src/managedCalls.ts:273-279` `providerLabel` shows the raw
  "antigravity".
- `web/src/agentTools.ts:162-168` `agentLabel` title-cases it.
- Both use the Plugin display name instead.
- `web/src/sessionFork.ts:4` is justified: it mirrors W1.
- The account, quota and browser specifics are justified Provider facts.

## Deferred items and their requirements

### D1. Repository `.mcp.json`

Opening a worktree must not run repository code. Requirements:

- owner approval scoped to target, workspace and the exact resolved
  configuration, frozen across list and run;
- the item 2 launch environment.

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

- The wire additions are needed before steering, the post-edit push for agy,
  and user servers:
  - `Steer.origin` and `supports_steering_origin`;
  - the steering window;
  - `callContext: "omit"` and `omitCallContext`.
- The worker `steering_gate` fix is needed before the proxy advertises
  steering.
- The facade argv change and the helper `path` field are needed before the
  post-edit push.
- Plan mode, labels, the skills note and the Web labels are independent.

## Release plan

Everything ships in three releases plus their host steps. Each one stays inert
until the next arrives.

### 1. Controller release

- **Contents:**
  - ingestion-time turn identity and the agy gate in the post-edit push;
  - the `apply --request-file` parser;
  - `Steer.origin`, sent only to capable workers.
- **Build:** `.#cowboy-controller-release` from the gated commit. A Web
  release from the same commit carries the labels.
- **Effect:** it restarts only `cowboy.service`. Codex's push must be
  unchanged.
- **Safe early:** no worker advertises the new capabilities yet.

### 2. Machine release — a worker pin advance

The wire and worker changes touch `runtime_wire.rs` and
`execution_protocol.rs`, so host-only outputs refuse to build until the
retained `cowboy-workers` input advances
([`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md)).

**Hawk** (execution host for agy, with an active deletion writer):

1. Land the wire changes together with the pin commit. Get the user's explicit
   go.
2. Run independent acceptance of revision `R`: worker, session, agent and logs
   conformance against `.#cowboy-machine-release` of `R`, with fresh receipt
   paths.
3. Make the pin commit, then build both `.#cowboy-machine-writer-host-release`
   and `.#cowboy-machine-host-release`. Check `workerGeneration` and
   `retained-worker-source.json`.
4. Run `just session-deletion-production-conformance` with two builds that
   carry the new pin.
5. Dispatch the writer artifact with Columbus
   `just --justfile machines/justfile cowboy-machine-activate <writer root>`.
6. Verify:
   - the receipt and the `writer_enabled=true` log line;
   - the deletion and incarnation floors;
   - `/healthz`, `/version` and Machine health;
   - worker and keeper ids and PIDs before and after. A pin advance
     intentionally replaces every live Hawk worker; record the list first.
7. Update the Hawk recovery floor in Columbus
   `machines/docs/nixos-deployment.md`. Schema-2 ledgers mean agy sessions
   must not fall back to a pre-advance worker generation.

**OVH** (agy's runtime host, where the workers and broker run):

1. Build `.#cowboy-machine-release` of the same revision on Hawk. OVH is
   runtime-only.
2. Transfer the verified closure.
3. Apply it through OVH's explicit worker maintenance (`activate-workers`,
   then `accept-workers`):
   - arm the independent four-minute rollback timer first;
   - keep the previous override as the rollback baseline;
   - check the Controller's reported generation before disarming
     (Columbus `machines/ovh/docs/usage-host-maintenance.md`).

**Execution-side CLI and keepers:**

- agy's tools and user servers run under Hawk keepers.
- Each keeper pins its `agent_cli` and contract when its environment is
  prepared, and retains them (`docs/mcp-bindings.md:432-441`).
- So:
  - retained bindings keep the old keeper and get no user servers;
  - fresh bindings get the new keeper's `omitCallContext`.
- Verify a retained and a fresh agy binding separately, before and after any
  rollback.

### 3. Component/Plugin release

- **Contents:** provider-runtime with `cowboy-antigravity-acp` 1.1.0 and
  Antigravity Plugin 0.3.0, carrying:
  - steering;
  - plan mode;
  - labels;
  - the skills note;
  - the facade `apply` argv.

  The other Providers get a patch bump for the shared lock.
- **Release:** through `.agents/skills/release-cowboy-plugin/`, then
  `cowboy operator converge` (canary Hawk, then OVH). Antigravity on OVH is
  installed by exact release.
- **agy pin:** unchanged. A future pin first passes `just agy-barrier-probe`
  and the OVH `agy-sha512.allow` update.
- **Existing sessions:** they keep their pinned Plugin generation. Acceptance
  uses fresh Columbus and Suger agy sessions.

### Mixed-version checks for the release record

- New proxy with an old worker: no steering advertised.
- New worker with the old proxy: user tools are listed; no steering.
- Old keeper: no user servers.
- New Controller with an old worker: no `origin` sent, and no agy push.
