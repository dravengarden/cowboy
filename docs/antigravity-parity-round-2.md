# Antigravity parity, round 2 — gap analysis and design

Status: design, revision 4 (2026-11-22), scoped to **round 2a**, which is
implemented on the branch that carries this revision. Round 2a is the model label
for subagents (A2), the built-in skills note (A3) and the Web provider labels
(A4). Post-edit diagnostics (formerly A1), steering, user MCP servers and plan
mode are deferred to round 3 (see
[Deferred to round 3](#deferred-to-round-3-redesign-requirements)).

- Baseline: `origin/main` at `c8db89326`, Antigravity Plugin 0.2.27,
  `cowboy-antigravity-acp` 1.0.22.
- Revision 2 applied the owner's scope decisions on design review
  `call-251d5a849c979dddb1053e03`. Revision 3 applied the first 2a/3 split after
  re-review `call-8dc781e16a7ca99702fb61b0` (6 high and 11 medium findings).
  Revision 4 applies the coordinator's decision on design review
  `call-2061c3702f0dafec842f659e` of revision 3 (3 high findings, all in
  post-edit diagnostics): that item moves to round 3, and A2 and A3 take the
  review's findings.
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
| A2 | Subagent model label from confirmed data | **round 2a** | proxy |
| A3 | Built-in `swarm`/`run-workflow` skills note | **round 2a** | proxy |
| A4 | Web labels | **round 2a** | Web |
| R4 | Post-edit diagnostics from the Controller (formerly A1) | deferred to round 3 | Controller, proxy, facade, wire, worker |
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

Round 2a changes only the Antigravity proxy package
(`components/provider-runtime/packages/antigravity-acp`) and the Web. It needs no
change to the Controller, the wire (`runtime_wire.rs`, `execution_protocol.rs`),
the worker (`src/acp.rs`, `src/worker.rs`), the Machine, the keeper or the
execution helper, so there is no Controller release, no worker pin advance and no
Machine release; see [Release plan](#release-plan).

### Invariants

Every in-scope item keeps these. Round 2a adds no effect, no process, no
credential and no delivery to the model:

- **Uncertain effects.** A lost reply after an admitted effect is uncertain.
- **Turn fencing.** The ledger (`src/worker_execution/ledger.rs:6-17`) admits
  effects only inside the active Cowboy turn.
- **Fail closed.** A label or a note is shown only when it is confirmed. Unknown
  means absent.
- **Authority.** Tool output, repository text and agent text never carry owner
  authority.
- **Secrets.** Credentials and gateway context never reach the account unit.

## A2. Subagent model label (P2)

### Current behaviour (before round 2a)

- The worker labels a child from the child's own `config_option_update` or a
  one-model usage breakdown (`src/acp_subagents.rs:234-279`, `:500-508`,
  `:682-689`). The 3.134 rollout shows the label
  (`docs/releases/antigravity-subagents-3-134-rollout-2026-11-22.md:59`).
- The proxy discarded the child's `session/new` result. After it opens a child it
  replays the parent's `session/set_model` and `session/set_config_option`
  settings (`subagents.mjs`, `Subagents.configured` and `start`). A replay's
  answer may carry the new configuration without a separate update, and agy's
  opening `config_option_update` may predate the replays or arrive after them. The
  label could therefore name the default model while the child runs another.

### Design

The label is the child's **confirmed effective model**. The proxy tracks it and
tells the worker only what is confirmed, from an authoritative final snapshot:

- The snapshot is the full `configOptions` set agy answered with: the
  `session/new` result when nothing was replayed, otherwise the answer to the
  **last** replay. A `session/set_config_option` answer is the full set after the
  change, so the last answer covers every earlier replay. Arrival order of agy's
  own updates is not used, because it cannot prove which configuration an update
  describes (a delayed opening update, or a model update emitted during an earlier
  replay).
- If a replay's answer lists no `model` option with a current value, the model is
  **unknown**: the proxy forwards no label, and an earlier snapshot is not reused.
- agy's own `config_option_update` frames for a session this host opened are
  **fenced by default** (`Subagents.configFence`, filled in `adopt`, never evicted
  while the session may still speak: an id leaves only when agy confirms
  `session/close`, after which it emits nothing for it). Only a child that started
  with nothing replayed onto it is let through,
  and then its updates describe its unchanged default. A child with replayed
  settings stays fenced for good, for the ordering reason above. A session whose
  start was abandoned, or that agy answered after its request timed out, is fenced
  too: the worker has no row for it, and an update for a session the worker does not
  know would be read as the parent's configuration. The fence is separate from
  `children`, which the proxy prunes (64 settled) while the worker keeps the row. A
  configuration update of a session nobody claimed (a refused or unanswered open)
  is dropped when the quarantine is released, since it would be read as the
  parent's; the root session's own configuration also arrives in its `session/new`
  answer.
- When the child is announced the proxy sends exactly one `config_option_update`
  under the child's session id, carrying only the `model` option as a complete ACP
  select option (`id`, `name`, `category`, `type`, `currentValue`, `options`). A choice
  list over 32 KiB (measured in bytes) is sent as `[]`, so the worker still decodes
  the update and labels the child by its current value. The worker's existing
  labelling then applies.
- While the model is unknown the child has no label and no model attribution. A
  one-model usage breakdown still names a child in the worker as it does today,
  but the proxy adds none and the pinned agy is not known to report any.

Usage is **not** added. Aggregating per-model token rows needs a confirmed shape of
what the pinned agy reports on a child's prompt result, which is a probe on a real
account. No live probe on a real account is part of round 2a; it waits for a
dedicated free-tier test account (`docs/requirements.md`, owner-paced use).

`docs/subagents.md` is corrected: a label is reported when agy confirms it, and
tokens are not.

### Tests

`subagents.test.mjs`: the new session names the model; a replayed child is
labelled from the last replay's full configuration, with agy's default-model
opening update and a later update both withheld; answers that list no
configuration, no model option or no model value leave no label while the child is
still announced; a child with no replays still shows agy's own updates; an
oversized choice list is sent empty as a complete select option; an abandoned child's
late update, a replayed child's after the proxy pruned it, and an unclaimed
session's quarantined update after a refused open never reach the worker; the fence
survives thousands of later adoptions and ends at a confirmed close; a refused replay
still fails the start.

### Acceptance

Unit tests only in round 2a. A Columbus run of two subagents (one on a replayed
model) showing each child's confirmed label, and the usage probe, wait for a
dedicated free-tier account and are outside this round.

## A3. Built-in skills note (P2)

- The slash commands are already dropped (`presentation.mjs`
  `UNAVAILABLE_COMMANDS`), and the hook denies their tools (`hook.mjs`).
- The routing projection (`projection.mjs`) names `swarm`, `run-workflow`,
  `schedule`, `browser` and `learn` as unavailable in every session, built from
  `UNAVAILABLE_COMMANDS` so the two cannot drift (`UNAVAILABLE_SKILLS_NOTE` in
  `presentation.mjs`). A managed call's routing replaces the default one, so
  `withUnavailableSkills` adds the same note to it at its one call site
  (`launch.mjs`); `managed.mjs` stays free of imports because the packaged hook
  loads it alone. Whether the pinned agy's own
  prompt still offers them is a live probe; the note is harmless if it does not and
  needed if it does, so it ships unconditionally.
- The alternative is stated **only for a recipient that has the tool**. The owner's
  session is told to use `mcp_cowboy_Subagent` where it would have used swarm or
  run-workflow. A subagent lists `Subagent` but cannot start one, so its
  projection is the parent's without that paragraph (`childProjection`), its
  guidance says swarm and run-workflow are unavailable to it too and that it does
  the work itself, and the hook's denial text says a subagent does the work itself.
  This also removes the earlier contradiction, where a child was told both to use
  `Subagent` and that it could not.
- Tests: the projection carries the note once and the delegation paragraph once;
  a child's session prompt has the note and not the delegation paragraph; a managed
  projection keeps its own routing and carries the note once; the denial text. The live check ("use swarm to …" leads to `Subagent` in an ordinary
  root session, with no denied call) waits for a dedicated free-tier account.

## A4. Web labels (P2)

- `web/src/managedCalls.ts` `providerLabel` showed the raw "antigravity", and
  `web/src/agentTools.ts` `agentLabel` title-cased any other id.
- Both name a Provider by the signed Plugin's `manifest.display.name` when the
  loaded Catalog carries a published (`ready`, artifact-bound) release of it
  (`web/src/providerDisplayName.ts`, newest compatible manifest), and otherwise keep
  today's text. An unbound entry is an unpublished manifest and never renames a
  signed Provider. Claude and Codex keep their short
  names, so `claude-code` stays "Claude" and `codex` "Codex".
- The managed-call data hooks (`useManagedCalls`, `useManagedCall`), the
  background-tasks hook (whose memoized summaries depend on the returned Catalog
  snapshot) and the agent-tools components subscribe to the Catalog cache without
  starting a read (`useProviderLabels`), so a label updates when the Catalog loads
  even inside the memoized dock. The dock and approval overlay files are structure-ratchet
  offenders and are not edited; the approval overlay reads the cache when it
  renders.
- Tests: the display name of a Provider from entries (newest release wins, an
  unbound entry never overrides), labels before and after the Catalog loads, the two
  short names unchanged, an unknown id falling back. The hook subscription is not
  unit-tested: the Web unit tests have no React renderer.
- `web/src/sessionFork.ts:4` is justified: it mirrors W1. The account, quota and
  browser specifics are justified Provider facts.

## Deferred to round 3: redesign requirements

These items stay out of round 2a. Re-review `call-8dc781e16a7ca99702fb61b0`
found high findings in R1 to R3, and design review
`call-2061c3702f0dafec842f659e` found three in R4, that no wording change fixes:
they need a different mechanism. Round 3 starts from these requirements, not from
the earlier designs, which they replace. Nothing here is implemented, and round 2a
builds toward none of it.

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

Round 2a changes nothing here: an Antigravity session keeps refusing current-turn
delivery, because the proxy still declares no steering.

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
R4's call-correlation requirement is the same problem, so R3 and R4 share one
mechanism.

### R4. Post-edit diagnostics from the Controller (formerly A1)

The goal: after an agy edit through the apply helper, the Controller runs the same
`code.post_edit` read it runs for Codex and gives the result to agy's turn as
untrusted data, never as a user message. The revision-3 design proposed:

- a publish receipt from the helper's `applied` outcome (the facade passing `apply`
  as a positional argument so the Controller's existing `apply --request` parser
  matches; inline requests of at most 48 KiB only, since a file-form request needs
  the execution helper to report the canonical path, a Machine change);
- delivery over the worker's existing `_session/steering` path, with the proxy
  advertising steering for Controller notes only and appending the note to the next
  facade tool result, acknowledged by agy's terminal update for that call;
- the Antigravity gate in `codex_push` (Codex unchanged), the Hub and delivery
  options kept from offering owner steering for Antigravity, and key-aware batching.

Design review `call-2061c3702f0dafec842f659e` found that this cannot ship without a
wire or worker change, and a call-correlation proof. Requirements:

High findings to resolve:

- **Execution retries can stamp an old publish with a newer turn.** A
  `RuntimeRequest` carries a worker epoch and no turn id. The worker resends
  pending requests on reconnect (`src/worker.rs:1049-1061`) and the Machine can
  resend them after a bulk-lane failure (`components/machine/src/machine_cli.rs:1665-1672`),
  so delayed copies of a start and its read can return cached keeper outcomes in
  another turn of the same epoch; sampling the delivery target at both points then
  agrees on the newer turn. Requirement: immutable admission-time turn attribution
  across retries (a turn id carried by the request, which is a wire and worker
  change and so a pin advance), one-shot completed receipt ids, and dropping any
  operation whose originating turn cannot be proven.
- **A single waiting announcement cannot prove exact-call receipt.** ACP
  announcements arrive on agy's stdout and MCP requests on a separate relay, with
  no shared call id; `bind` matches heuristically and caches the match, the waiting
  list evicts, and two identical concurrent calls can be confused, so a completion
  of the other call could report `injected` although the response carrying the note
  was lost. Requirement: a shared call identity, or a proven one-to-one ordering
  guarantee for the pinned agy (a live probe on a dedicated free-tier account); skip
  the handoff when that proof is absent. The same correlation is R3's requirement.
- **A Controller rollback reopens owner and peer-agent steering.** A legacy
  Controller exposes the new proxy's advertised capability and forwards
  caller-controlled text over HTTP and `queue.submit`; a `next_checkpoint` send
  carrying the note header would satisfy a format filter and reach agy as tool
  output. Requirement: provenance a legacy caller cannot produce, which is R1's
  authenticated origin channel, or a compatible Controller floor enforced across
  activation, rollback and surviving workers, with header-shaped owner and peer
  sends in the mixed-version tests.

Medium findings to resolve:

- A targeted Interrupt and a pending note share the per-session waiter slot
  (`start_steer_targeted` and `start_cancel_turn`, `src/remote_runtime.rs:923-931`,
  `:976-984`), so an Interrupt is refused while a note waits. Give targeted
  cancellation independent admission and priority over diagnostics.
- A proxy window does not bound end-to-end delivery: the Controller's 30 s, the
  worker's 25 s and the proxy's window start at different times, and reconnect
  resends commands (`src/remote_runtime.rs:1571-1576`, `:1655-1715`). Require one
  expiry checked at handoff, note commands that are never resent, a defined effect
  of a late acknowledgement, and a pending RPC kept off the proxy's serialized
  input queue so Stop reaches it. The worker holds its steering gate for the whole
  RPC, so a targeted interrupt waits behind a note.
- The worker's steering echo is a `user_message_chunk` with a cmid and no
  `promptOrigin` (`src/acp.rs:4329-4335`); the Hub derives an origin only for cmids
  it dispatched, so Web shows the note as a human message. Project the note at the
  Controller by its own admitted id and matching receipt, and verify the persisted
  row on replay; a cmid prefix alone is not provenance.
- `send_delivery::begin` clears the owner's configuration undo window after
  admitting a steer (`config_revert::clear`); a note must not.
- Batching needs freshness: reject a stale incoming key before changing pending
  state, keep the newest receipt per path, and suppress superseded or
  digest-mismatched observations; test reversed order and a second edit during
  analysis.
- The write tracker (`writes.rs`) stops admitting helpers at its 256-entry capacity
  and a helper whose proxy exits before the final read can stay forever, disabling
  tracking for every session and Provider until restart. Bound and expire helper
  and pending-read state by epoch and operation.
- If the Controller build also bumps the shared Provider manifests, deploying it
  before the Plugin is published violates the exact-release coverage gate
  (`.agents/skills/release-cowboy-plugin/SKILL.md`). Plan separate Controller and
  Web revisions with published Provider pins, or publish first and hold installation.
- Live acceptance needs evidence the model used the appended block: a unique value
  only the appended block carries, on a dedicated free-tier account, with the receipt
  verified separately (the fake agy reads only the first content block).

Out of scope for R4 as well: shell-made writes (no baseline for a session's first
foreground command), edits above 48 KiB, and background commands (D3).

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

A2, A3 and A4 are independent of each other and of every deferred item.

## Release plan

Round 2a ships in two releases, in any order. The diff touches no Controller code,
and none of `Cargo.toml`, `Cargo.lock`, `runtime_wire.rs`, `execution_protocol.rs`,
`execution_environment.rs`, `machine_protocol.rs` or the SDK sources that
`retained-worker-interface-compatible` pins
([`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md)),
so there is no Controller release, no worker pin advance, no Machine release and
no keeper change. The implementation records that with a diff check.

### 1. Component/Plugin release

- **Changed artifact:** the private runtime package `cowboy-antigravity-acp`
  (`components/provider-runtime/packages/antigravity-acp`, 1.0.22 in the Antigravity
  Plugin 0.2.27): `proxy.mjs`, `subagents.mjs`, `projection.mjs`, `hook.mjs` and
  their tests. The other private packages and the pinned agy are unchanged.
- **Release:** through `.agents/skills/release-cowboy-plugin/`. By the pattern of the
  last Antigravity releases that is a patch bump of the package and of the
  Antigravity Plugin (expected 1.0.23 and 0.2.28), a components bump (expected
  3.143.0) and, because the components lock is shared, a patch bump of the other
  Plugins' manifests; the release skill fixes the numbers. Then
  `cowboy operator converge` (canary Hawk, then OVH). Antigravity on OVH is installed
  by exact release.
- **agy pin:** unchanged.
- **Existing sessions:** they keep their pinned Plugin generation, so a running agy
  session shows the new label and note only after it is restarted on the new
  generation. New sessions and new subagents use it.

### 2. Web release

`.#cowboy-web-release` from the gated commit carries A4. It restarts nothing and
moves only `/run/cowboy-web`. Reload the PWA hard, since a WS reconnect keeps stale
JS.

### Mixed-version checks for the release record

- New Web with an old Plugin: labels use the Catalog's display name; nothing else
  changes.
- Old Web with a new Plugin: the label text is the old one; A2 and A3 are
  independent of it.
- New proxy with an old worker or Controller: no wire or worker contract changed, so
  the worker folds the one `config_option_update` as it does agy's own.
