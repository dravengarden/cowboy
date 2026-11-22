# Antigravity parity, round 2 — gap analysis and design

Status: proposed design (2026-11-22). Nothing here is implemented. Baseline:
`origin/main` at `c8db89326`, Antigravity Plugin 0.2.27, `cowboy-antigravity-acp`
1.0.22. The design it extends is
[`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md);
the account-risk rules are in [`requirements.md`](requirements.md) (owner-paced
use).

## Scope and the three lessons it starts from

Round 1 (live in 0.2.27) gave Antigravity owner-paced dispatch, login-shell
`RunCommand`, full guidance, reads outside the root, home skills, image reads,
glob search, the continuation hint, the proxy-hosted `Subagent`, and Cowboy
MCP, Matrix memory and folder bindings through the worker extension broker.
This round covers what is still missing compared with Claude Code and Codex.
The closest reference is not native Claude: it is **Claude's remote runtime**
(`plugins/claude-code/runtime/`). It also runs the agent on one Machine and the
tools on another, so most gaps already have a tested design there.

Three review rejections set the rules for this round:

1. **Session-lifetime effects belong to the worker, not the proxy.** The proxy
   relay for user MCP servers (`claude/agy-user-mcp`, 3 review rounds) never
   settled uncertain effects, acks, shutdown or fencing. The broker
   (`src/worker_execution/extension_broker.rs:1-11`) already does all of these
   for folder bindings. User servers go there.
2. **An exemption needs a proof from the component that does the work.**
   `cowboy_read` and `cowboy_receipt` are not pure. `accounts.login_status`
   resolves to `operation_receipt`
   (`components/server/src/server/agent_api.rs:346-367`), and so does the
   `logins` receipt domain (`.../agent_api/actions.rs:857-866`).
   `operation_receipt` runs `candidate::finalize_committed` and
   `native_recovery::observe`
   (`.../provider_accounts/login/operations.rs:163-183`), and those can write.
   The worker cannot treat these calls as reads until the Controller makes
   them pure and says so on each call.
3. **The Controller owns observation and delivery.** Proxy-side post-edit
   diagnostics (`claude/agy-post-edit-wip`) failed for three reasons. The proxy
   cannot produce the Controller's write hint. Its baselines and suppression
   were coupled. It did not own the turn lifecycle. Codex's push
   (`components/server/src/server/agent_api/code_reads/codex_push.rs`) watches
   the session stream and steers into the turn. agy needs the steering channel
   first.

General invariants that every item keeps:

- **Uncertain effects.** A lost reply after an admitted effect makes the effect
  uncertain. The session records it and the owner must acknowledge it. Nothing
  is replayed under a new identity.
- **Turn fencing.** The ledger (`src/worker_execution/ledger.rs:6-17`) admits
  effects only inside the active Cowboy turn. Every new exception is a named
  owner class, recorded durably and ended at teardown.
- **Policy fails closed.** If a capability is not proven by the running peer,
  the feature is off. The PreToolUse gate keeps its single `mcp_cowboy_*`
  namespace.
- **Secrets.** Credentials and target environment values never enter the
  account unit. They also do not leave the target unless an existing runtime
  already moves them.

## Gap inventory

| # | Gap | Decision | Priority | Layers |
|---|-----|----------|----------|--------|
| 1 | Current-turn steering | implement | P0 | proxy |
| 2 | User MCP servers | implement (stdio, Claude config sources) | P0 | worker, target CLI, proxy (presentation only) |
| 3 | Pure `cowboy_read`/`cowboy_receipt`, then a read-only exemption | implement; memory tools deferred | P1 | Controller, worker |
| 4 | Post-edit diagnostics | implement, Controller-observed | P1 | Controller (needs 1) |
| 5 | Repository hooks | implement (PreToolUse, PostToolUse, UserPromptSubmit, SessionStart at first turn) | P1 | proxy |
| 6 | Background commands that outlive a turn | implement; idle wake deferred to an owner decision | P1 | worker, proxy (uses 1) |
| 7 | Plan-mode behaviour | implement read auto-approval and acceptance | P2 | proxy |
| 8 | Per-subagent model and usage labels | implement | P2 | proxy |
| 9 | Built-in `swarm`/`run-workflow` skills advertised to the model | implement guidance; verify live | P2 | proxy |
| 10 | Web treats agy differently without a reason | fix labels | P2 | Web (ships with the Controller commit) |
| 11 | Native fork | won't do now: agy has no `session/fork` | — | — |
| 12 | Web search/fetch results | won't do: vendor limit | — | — |

## 1. Current-turn steering (P0, proxy only)

**Current behaviour.**

- The worker turns current-turn delivery on only when the agent's
  `initialize` result has `_meta.steering.supported: true` and
  `idleBehavior: "promptRequired"` (`src/acp.rs:3836-3860`). The proxy
  forwards agy's result unchanged. agy declares no steering, so
  `supports_current_turn` stays false.
- The Controller then refuses with "current-turn delivery is not supported by
  this running worker" (`components/server/src/server/send_delivery.rs:101-103`).
- Even with the flag set, the proxy would refuse `_session/steering`: only
  `RUNTIME_REQUESTS` pass (`components/provider-runtime/packages/antigravity-acp/proxy.mjs:40-50`,
  refusal at `:586-587`).

**Reference.**

- Codex: `turn/steer` adds input to the active turn. Cowboy's delta makes it
  advertise `promptRequired` and refuse ambiguous replays
  (`third_party/codex-acp/patches/0003-steer-prompt-required.patch`).
- Claude: priority `now` or `next` through `plugins/claude-code/runtime/steering.mjs`.
- The worker sends `_session/steering` with a 25 s timeout. It maps
  `injected` to injected, `promptRequired` to `not_delivered`, and anything
  else to `unknown` (`src/acp.rs:4290-4330`).

**Design.** agy cannot take input in the middle of a turn. However, every tool
agy runs is a facade tool (`hook.mjs` allow-list), so each tool result is a
boundary the proxy controls. The proxy answers steering itself and never
forwards it to agy.

- **Advertising.** In the `initialize` reply the proxy adds
  `_meta.steering = {supported: true, idleBehavior: "promptRequired",
  priorities: ["next"]}`. It does not do this for managed units: the worker
  already masks those (`session_is_managed_read_only`).
- **Admission.** The proxy handles `_session/steering` locally and admits it
  only for a root session that has an open turn (`this.turns`). It never
  admits it for a subagent, a managed unit, or during `/compact`. With no
  turn, it answers `{outcome: "promptRequired"}`. It treats priority `now` as
  `next`; the Send-delivery contract already says this is not a
  cross-provider promise (`docs/agent-api.md`, "Send delivery").
- **Delivery.** An admitted steer is queued on the turn, and the RPC stays
  pending until one of these happens first:
  - (a) The next facade tool result for that session is written to agy. The
    proxy appends one text block
    `<cowboy_owner_message nonce=N>…</cowboy_owner_message>` to that result
    and answers `injected`.
  - (b) agy's `session/prompt` result arrives with steers still queued. The
    proxy holds the reply to the worker and sends a continuation prompt that
    carries the steers. This is the same pattern as the managed-output
    correction (`proxy.mjs` `correct()`, `:1340`). The proxy answers
    `injected` when that prompt is written.
  - (c) A deadline of 20 s, below the worker's 25 s, passes. The proxy removes
    the steer from the queue and answers `promptRequired`.

  The proxy is single-threaded, so removing a steer and delivering it are
  mutually exclusive. A steer is either delivered or reported not delivered,
  never both. Cancellation or turn end before delivery also answers
  `promptRequired`.
- **Injection safety.** The nonce is random per session. The routing
  projection (`projection.mjs`) announces it, the same way
  `managed-output.mjs` uses `afterNonce`. Command output that forges the tag
  without the nonce is not an owner message.
- **Evidence and memory.** A continuation prompt is part of the same worker
  turn. The turn's evidence cursor (`record.cursor`) already covers it.
  Memory recall is not run again.

**Compatibility.** Only the proxy changes. A worker before protocol 34 never
sends steering, and an old proxy never advertises it. Both sides use the
existing wire.

**Tests.** In `proxy.test.mjs` with `fake-agy.mjs`:

- Delivery at a tool result.
- Delivery by continuation when the turn ends.
- Deadline expiry answers `promptRequired`, and nothing arrives at agy later.
- Cancel while queued.
- Subagent and managed refusal.
- A forged tag in `RunCommand` output.

Add a worker conformance case in `src/acp_session_conformance_tests.rs` that
uses the `priorities: ["next"]` form.

**Live acceptance.**

- Columbus agy session. During a long `RunCommand` chain, send
  "current turn: also print the date". The next tool result carries it, the
  Web shows the steered bubble once, and the receipt says `injected`.
- Suger agy session. Steer while agy is only thinking (no tool call). It is
  delivered either by continuation or with `not_delivered` within 20 s, and
  nothing arrives later.

## 2. User MCP servers (P0, worker broker)

**Current behaviour.**

- The proxy replaces `mcpServers` with the single facade server
  (`proxy.mjs:639`).
- The broker runs only Cowboy MCP and folder bindings
  (`src/worker/sandbox_extensions.rs:74-108`).
- The user's `chrome-devtools-*` servers in `~/.claude.json` and
  `~/.codex/config.toml` never reach agy.

**Reference.** Native Claude and Codex read their own config on the target.
Claude's remote runtime reads `~/.claude.json`, each ancestor's `.mcp.json`
and `projects[...]` (`plugins/claude-code/runtime/tools.mjs:2670-2700`). It
then relays each server with `mcp-proxy.mjs`.

**Design.**

1. **Resolution stays on the target.** A new target CLI mode
   `cowboy mcp --user-servers --cwd <root>` (list) and
   `cowboy mcp --user-server <name> --cwd <root>` (run) resolves servers the
   same way Claude's remote runtime does:
   - user `mcpServers`;
   - then `.mcp.json` from `/` down, minus `disabledMcpjsonServers`;
   - then `projects[<repo root or cwd>].mcpServers`;
   - minus `disabledMcpServers`;
   - `${VAR}` and `${VAR:-default}` are expanded from the target
     environment.

   Run mode execs the stdio server. Its output gives only names. Commands and
   environment values never leave the target, which is stricter than Claude's
   remote runtime. `type: "http"`/`sse` servers are listed as unavailable
   (deferred). A JSON test-vector fixture is shared with Claude's
   `mcpInputs` tests so that the two parsers cannot drift.
2. **The broker hosts them.** `sandbox_extensions::derive` adds
   `ServerPlan{name: "user-<name>", argv: [cowboy, "mcp", "--user-server",
   name, "--cwd", root], required: false}` for each listed name, up to 8. The
   list comes from one probe, started by the worker under `SERVER_PREFIX`, in
   the existing discovery window (40 s, `extension_broker.rs:30`). The ledger
   (`ledger.rs:45`, `SERVER_TURN`) already gives these processes session
   lifetime, refuses any unit request that names them, and ends them at
   teardown.

   Calls take the existing path (`extension_broker.rs:294-370`):
   - admitted only inside the turn, which also covers subagents because they
     run inside the parent's turn;
   - the outcome is decided before the turn may complete;
   - a lost call means `mark_uncertain`, with no replay.

   This removes every high-severity finding of the rejected relay by
   construction.
3. **Fail closed.**
   - A server that misses discovery, or exceeds `MAX_TOOLS` (128) in total,
     is left out. A projection line names it as unavailable.
   - An optional server never refuses the session.
   - A target CLI without the subcommand fails the probe, and the session has
     no user servers.
   - Managed units get none: `derive` must skip user servers for a managed
     descriptor, and a test must cover it.
4. **Proxy.** No protocol change: tools arrive through `cowboy/extensions` and
   `extensions.mjs` lists them under the gate's namespace. Presentation shows
   `mcp__<server>__<tool>`. MCP image results (chrome-devtools screenshots)
   pass through, within the 1 MiB result cap.

**Compatibility.**

- New worker, old proxy: tools are listed as ordinary extension tools and
  work.
- Old worker: no user tools.
- Old target CLI: the probe fails closed.

**Tests.**

- `src/worker_execution/tests.rs`: a fake user server is discovered; its
  failure is optional; a lost call is uncertain; a call outside a turn is
  refused; teardown ends it; a unit request naming its id is refused.
- CLI resolver vectors.

**Live acceptance.**

- Columbus agy session. `mcp__chrome-devtools-draven__list_pages` works. A
  screenshot renders. Killing the server process mid-call gives an uncertain
  effect, not a retry.
- Suger agy session. `chrome-devtools-suger` is listed, and the Draven profile
  is not used for Suger.

## 3. Pure reads and the read-only exemption (P1, Controller then worker)

**Current behaviour.** Every lost extension call becomes an uncertain effect
(`extension_broker.rs:354-361`), and so does a lost `cowboy_status` call. The
rejected `claude/agy-worker-readonly-min` exempted `cowboy_read` and
`cowboy_receipt`, but both can run login recovery (see lesson 2).

**Reference.** Claude and Codex have no effect ledger. A lost read is simply
retried by the model.

**Design.**

- **Controller.**
  - Split `operation_receipt` into `observe_receipt`, which only reads the
    store, and the recovery steps. `finalize_committed` and
    `native_recovery::observe` move to a Controller-owned login
    reconciliation task and to explicit `accounts.*` actions. Reuse an
    existing login supervisor loop if there is one; otherwise add one. Login
    convergence must not depend on a client polling.
  - `accounts.login_status` and the `logins`/`accounts` receipt domains call
    only `observe_receipt`.
  - `cowboy_read` gains a request field `_cowboy.pure: 1`. When it is set, the
    Controller runs the program in a read context whose operations are typed
    pure. Any non-pure op raises `invalid_argument`. `cowboy_receipt` accepts
    the same field.
  - Strict argument checking (`only_keys`) makes an old Controller refuse the
    unknown field before dispatch. That refusal is fail-closed and has no
    effect.
- **Worker.**
  - The broker marks a call exempt only when all of these hold: the server is
    the worker-launched `cowboy` server; the tool is `cowboy_status`,
    `cowboy_read` or `cowboy_receipt`; and it sends `_cowboy.pure: 1`.
  - A refused flagged call is re-sent once without the flag, as an ordinary
    non-exempt call. It is safe because the refusal proves nothing ran.
  - An exempt lost reply answers `outcome unknown` to the model without
    `mark_uncertain`.
  - Exempt calls are still admitted only inside a turn. They do not count
    toward `MAX_CALLS` drain, so a slow read does not hold the turn end.
- **Deferred.** Memory tools stay non-exempt until Matrix declares read-only
  status per call. Their discovery is cached across backend changes.

**Tests.**

- Controller: a test that the pure read context refuses every registered
  write op.
- Login status after a committed candidate changes no rows.
- Worker: flagged lost read gives no uncertainty; old-Controller refusal is
  resent unflagged once; a tool other than the three is never exempt.

**Live acceptance.** Restart the Controller during an agy `cowboy_read`. The
turn continues, no "uncertain effect" banner appears, and a following
`cowboy_execute` is unaffected.

## 4. Post-edit diagnostics (P1, Controller; needs 1)

**Current behaviour.** The Codex push skips every provider except Codex
(`codex_push.rs:26`, `:160`).

**Reference.**

- Codex: the Controller observes a completed `edit`/`execute` tool call, runs
  `code.post_edit`, and steers the note into the same turn at the next
  checkpoint. The target is captured at observation, so a later turn refuses
  it.
- Claude: delivers the same read from its context Mod
  (`plugins/claude-code/runtime/context-mod.js:1476-1560`).

**Design.**

- **Gate.** Replace `codex(provider)` with a gate on the declared capability:
  the session's `delivery_controls.next_checkpoint` is true, and the provider
  is in a small allow-list `{codex*, antigravity}`. A Claude session keeps its
  own Mod path, so the push never runs twice.
- **Path hints.** agy's facade edits are presented with `kind: edit` and an
  absolute target path (`presentation.mjs:17-18`, `:80-83`). `RunCommand` is
  `kind: execute`, so `shell_changes` covers files a command changed.
- **Turn binding.** No proxy change is needed. The steer arrives as a
  next-priority `_session/steering` and is delivered at agy's next tool result
  or by continuation (item 1). The note text from `note()` is already escaped
  repository data.

**Compatibility.**

- New Controller with an old proxy: no `next_checkpoint`, so the push is
  inert.
- An old Controller never pushes.

**Tests.**

- Unit: the gate accepts antigravity only with `next_checkpoint`.
- An agy-shaped edit update's path resolves through `relative()` against the
  session cwd.

**Live acceptance.** In a Columbus agy session, introduce a Rust type error
with `EditFile`. The next tool result, or the continuation, carries
"[Cowboy code intelligence after your edit]". No note arrives after the turn
ends.

## 5. Repository hooks (P1, proxy)

**Current behaviour.** agy runs only Cowboy's own `hooks.json` gate
(`components/provider-runtime/packages/antigravity-acp/hooks.json`), bound
read-only. Columbus's `PreToolUse ^Bash$` and `UserPromptSubmit` hooks
(`.claude/settings.json`, `.codex/hooks.json`) do not run.

**Reference.**

- Codex runs its hooks natively.
- Claude's remote runtime reads `.claude/settings{,.local}.json`
  (`tools.mjs:1157-1182`). It runs PreToolUse and PostToolUse for facade tools
  on the target and folds them as native does (`context-mod.js:441`). It
  refuses a matching call whose hook shape it cannot run (`launch.mjs:428-457`).

**Design.**

- **Source.** One source per repository, chosen in this order:
  1. `.codex/hooks.json` (Codex first, as skills are);
  2. otherwise Claude's project settings;
  3. `disableAllHooks` honoured.

  The proxy reads them at projection time through the execution connection,
  using the guidance read rules: regular files only, no symlinks.
- **Events.**
  - PreToolUse and PostToolUse run around facade tools. Names are mapped to
    Claude's: ReadFile→Read, WriteFile→Write, EditFile→Edit,
    RunCommand→Bash, SearchFiles→Grep, ListDirectory→LS, extension calls →
    `mcp__<server>__<tool>`.
  - UserPromptSubmit runs before each owner prompt is forwarded.
  - SessionStart runs at the first turn of a session open or load, inside
    that turn. The ledger admits nothing before a turn except the readiness
    probe, and it stays that way.
- **Execution.** Every hook command is an in-turn `process/start` on the
  target (`cowboy-agy-hook-` ids) with Claude's hook JSON on stdin.
- **Results.**
  - Block or deny output turns the tool result into the hook's reason.
  - `additionalContext` is prepended as a fenced block, as the memory block
    is.
  - A hook shape the proxy cannot run (non-command, `if`, `asyncRewake`)
    refuses the matching call rather than skipping review.
  - Managed units run no hooks.

**Compatibility.** Proxy only. The ledger already admits these in-turn
starts.

**Tests.**

- Matcher mapping.
- Deny, allow and additionalContext folding.
- Unsupported shape refuses.
- SessionStart runs exactly once per open.

**Live acceptance.** In a Columbus agy session, a `git push --force`-style
command that Columbus's PreToolUse hook denies is blocked with the hook's
message. The UserPromptSubmit hook's context appears.

## 6. Background commands (P1, worker and proxy; uses 1)

**Current behaviour.**

- `RunCommand` waits at most 600 s per call (`facade-tools.mjs:18-19`).
- Every turn end stops the session's commands (`facade.mjs:201-221`).
- The ledger fences and terminates a turn's processes (`ledger.rs:757-781`).

**Reference.**

- Claude's remote runtime: `run_in_background` runs up to 2 h
  (`tools.mjs:3182-3191`). Output is written to a task file, and a native
  background task waits for the target command (`task-wait.mjs`). The worker
  counts it as background activity, not a busy turn (`src/acp.rs:3590-3608`).
- Codex: unified exec sessions survive turns.

**Design.**

- **Ledger.** A new owner class `cowboy-session-background`:
  - A `process/start` whose id has the prefix `cowboy-agy-bg-` is admitted
    only inside the active turn, like any start.
  - It is recorded under the background owner, durably in the ledger file.
  - At most 8 such processes; lifetime capped at 2 h by the worker's own
    timer.
  - `fence(turn)` skips the class. `cancel_turn` skips it too (Claude
    behaves the same).
  - Teardown (`settle_all`) ends them.
  - A dead worker's background processes are not inherited. The keeper reap
    remains the backstop, and the projection says so.
  - The worker announces the class in the execution descriptor
    (`ledger.background: 1`). Without that announcement the proxy offers no
    background.
- **Facade.**
  - `RunCommand` gains `background: true`, which returns a `command_id` at
    once.
  - `RunCommand {command_id, wait_seconds}` and `StopCommand` work in any
    later turn.
  - `endTurn` stops only foreground commands.
  - Output keeps the 256 KiB head and tail window.
- **Completion.** The proxy watches the exit through `process/exited` and
  `process/read`. Reads are not mutations, so the ledger lets them pass
  outside turns.
  - While a turn is open, the completion is queued as a steer-like system
    block at the next boundary (item 1 machinery, proxy-internal).
  - While idle, it is held and prepended to the next owner turn as
    `<cowboy_background_update>`.
  - Waking an idle agy on completion is a new agent-started turn category.
    `requirements.md` does not list it, so it is deferred to an explicit owner
    decision.
- **Presentation.** A new ACP extension update `cowboy_background_tasks
  {count}` is folded by the worker into `native_background_tasks`. The Web
  shows the existing background row, and a long-lived command never holds the
  queue.

**Compatibility.**

- Old worker: no announcement, so no background.
- Old proxy: never sends the prefix.

**Tests.**

- Ledger: a background start is refused outside a turn; it survives
  `finish_turn` and `cancel_turn`; it ends at teardown; the cap; the durable
  record after reopen.
- Proxy: completion while running; completion while idle; Stop in a later
  turn.

**Live acceptance.** A Suger agy session starts `bun run dev` in the
background and ends the turn. The next turn reads its output and stops it.
Session teardown leaves no process on the target.

## 7. Plan mode (P2, proxy)

**Current behaviour.** Outside plan mode, facade calls are approved by the
proxy. In `plan` mode every facade call goes to the owner, reads included
(`proxy.mjs:1043`). agy's native plan artifacts (`implementation_plan.md` via
`write_file`) are denied.

**Reference.**

- Claude plan mode runs read tools freely, blocks edits, and asks for plan
  approval through ExitPlanMode.
- Codex runs read-only and publishes a plan.

**Design.**

- In plan mode, the proxy auto-approves ReadFile, ListDirectory, SearchFiles,
  UpdatePlan and exempt reads (item 3).
- WriteFile and EditFile are refused with "plan mode: present the plan with
  UpdatePlan and wait".
- RunCommand still asks the owner. agy's RunCommand is a shell, so it cannot
  be proven read-only.
- Leaving plan mode stays the owner's mode switch.

**Tests.** A mode matrix in `proxy.test.mjs`.

**Live acceptance.**

- A Columbus session in plan mode reads files without prompts.
- An edit is refused with the message.
- Switching to default lets it implement.

## 8. Subagent model and usage labels (P2, proxy)

**Current behaviour.** "Neither a model label nor usage is reported"
(`docs/subagents.md:36`). The proxy knows the child's model, because it
replays the parent's `set_model` (`subagents.mjs:167-178`). It also receives
the child's prompt-result `usage`.

**Design.**

- Before the delegation, send the child a `config_option_update` with one
  model choice, as Claude's `childModelOption` does.
- When the child is idle, put its `usage` on the idle `subagent_update`.
- On the parent's turn result, add
  `_meta.quota.model_usage` rows: the parent's model, plus one row per child
  model with the children's summed `totalUsage`. The worker
  (`src/acp_model_usage.rs`) then totals per model.
- Check that the parent's own `usage` stays parent-only, so the Controller's
  turn usage does not double-count.

**Tests.** Subagent tests assert the label, the idle usage, and rows that sum
consistently.

**Live acceptance.** In a Columbus session, run two subagents. Each row shows
"Gemini 3.8 Flash" or the parent's model and its tokens. Session info lists
one row per model.

## 9. Built-in `swarm` / `run-workflow` skills (P2, proxy)

**Current behaviour.**

- The slash commands are already dropped (`presentation.mjs:334-349`).
- The hook denies their tools and names alternatives (`hook.mjs:57-67`).
- agy's own system prompt can still offer the built-in skills, even with
  `--include-default-customizations=false`. Whether it does is unverified.

**Design.**

- Verify first: run a live probe that asks agy to list its skills. If they
  are listed, the routing projection adds one sentence naming `swarm`,
  `run-workflow`, `schedule`, `browser` and `learn` as unavailable in Cowboy,
  with the alternatives (`Subagent`, managed calls).
- If agy settles a skills-disable setting in `settings.json`, prefer it, and
  add a policy-shape check to readiness.

**Acceptance.** Ask "use swarm to …". agy uses `Subagent` without a denied
call.

## 10. Web differences without a reason (P2)

| Location | Behaviour | Decision |
|---|---|---|
| `web/src/managedCalls.ts:273-279` `providerLabel` | prints the raw id "antigravity" | use the Plugin display name, falling back to title case |
| `web/src/agentTools.ts:162-168` `agentLabel` | prints "Antigravity" only by title-casing | same helper |
| `web/src/sessionFork.ts:4` | hides native Fork | justified: mirrors the Controller's `FORKABLE_PROVIDERS` |
| account, usage and browser specifics (`accountControls.ts`, `UsageGroupedSegment.tsx`, `machineBrowser/shareRules.ts`) | named-only accounts, per-group quotas, IPv4 browser | justified: provider facts |

The current-turn picker and the background row are already driven by
capabilities, so items 1 and 6 need no Web change.

## 11. Native fork (won't do now)

The Controller allows only `claude-code` and `codex`
(`components/server/src/server/session_context/fork.rs:8`, `:136`). The worker
requires `sessionCapabilities.fork` (`src/acp.rs:3868`, `:3891`). agy's
`initialize` offers `load`, `resume`, `list` and `close`, but no fork
(`docs/antigravity-provider-plan.md:110`). The real-accounts design's feature
inventory records the same. Copying agy's conversation store under
`~/.gemini/antigravity-cli/` would depend on an undocumented vendor format
inside the account sandbox. Linked copy stays.

**Revisit trigger.** The agy pin's acceptance probe (`just agy-barrier-probe`)
reports `sessionCapabilities.fork`. Then:

- the proxy admits `session/fork` with the `session/new` rewrites;
- `antigravity` joins both lists.

## 12. Web search/fetch rendering (won't do)

`search_web` and `read_url_content` are presented as WebSearch and WebFetch
with the query or URL as title (`presentation.mjs:34-36`). agy reports no
result over ACP. The model's reply is the only summary, and the proxy cannot
recover the result without running its own fetch, which would be a new
network effect. This stays until agy reports results.

## Dependencies and order

```text
C1 pure reads ─────────────► W2 read-only exemption
P1 steering ──► C2 post-edit gate (inert until P1 ships)
            └─► P4 background completion delivery
W1 user MCP broker + target CLI (independent)
W3 background ledger class ──► P4 background facade
P2 hooks, P3 plan mode, P5 subagent labels, P6 skills note, Web labels (independent)
```

Turn-on rule: the capability comes from the newest component, and every older
peer leaves the feature off. Deployment order is therefore Controller, then
Machine, then component. Each step is inert until the next one arrives.

## Release plan

1. **Controller release** (`.#cowboy-controller-release`, plus the Web
   release from the same commit for the labels):
   - C1: pure `observe_receipt`, recovery moved to the login supervisor, the
     `_cowboy.pure` read context;
   - C2: the post-edit push gate on `next_checkpoint` and the allow-list;
   - Web labels.

   Gate: `just check`, plus a pure-context write-op refusal test.
   Acceptance: login flows still converge from the Web, and Codex push is
   unchanged.
2. **Machine release** (`.#cowboy-machine-release`, which advances the
   `cowboy-workers` pin; this is a separate maintenance boundary):
   - W1: user-server plans in the broker, plus the target `cowboy mcp
     --user-server(s)` modes;
   - W2: the exemption;
   - W3: the background owner class and descriptor announcement, plus the
     `cowboy_background_tasks` fold in `src/acp.rs`.

   Existing sessions keep their pinned generation. Acceptance runs on a fresh
   agy session.
3. **Component/Plugin release** (provider-runtime with
   `cowboy-antigravity-acp` 1.1.0, Antigravity Plugin 0.3.0, the other
   Providers getting a patch bump for the shared lock):
   - P1: steering;
   - P2: hooks;
   - P3: plan mode;
   - P4: background facade;
   - P5: subagent labels;
   - P6: skills note;
   - projection lines for user servers and the steering nonce.

   Release it through `.agents/skills/release-cowboy-plugin/`, and do the
   agy-pin probe first.

Live acceptance runs after all three are in, on one Columbus and one Suger agy
session each, using the per-item steps above. In the release record, also
record the cross-version checks:

- new proxy on the old worker: no background, steering works;
- new worker with the old proxy: user tools are listed;
- old Controller with the new worker: flagged reads are refused, then resent
  unflagged.
