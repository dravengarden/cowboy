# cowboy-antigravity-acp

`agy-proxy`, the ACP entry of an Antigravity account unit, with the Cowboy
tool facade, its stdio relay and the `PreToolUse` hook. Design:
[`docs/antigravity-real-accounts-design.md`](../../../../docs/antigravity-real-accounts-design.md)
("Phase 4: split execution" and the Revision 4 amendments).

Released as the `node_npm` component `cowboy-antigravity-acp` (Antigravity's
`provider_adapter` slot `antigravity-proxy`, command `agy-proxy`). The Machine
places its tree below `proxy/` in the account-sandbox runtime archive and adds
`bin/agy-proxy` and `bin/agy-hook` entry scripts; `policy/hooks.json` and
`policy/settings.json` come from this package.

| File                          | Role                                                                      |
| ----------------------------- | ------------------------------------------------------------------------- |
| `launch.mjs`                  | Proxy entry: readiness, closed `agy` environment, wiring                  |
| `proxy.mjs`                   | ACP relay: runtime method allow-list, session rewrite, turn settlement    |
| `extensions.mjs`              | The worker's extension tools (Cowboy MCP, bindings, memory) in `cowboy`   |
| `plan-gate.mjs`               | Call correlation and plan-mode dispatch enforcement (R3)                  |
| `diagnostics-client.mjs`      | Separate `/diagnostics` WS client for post-edit diagnostic pulls (R4)     |
| `post-edit-notes.mjs`         | LF-SHA256 hashing, 16 KiB note formatting, and tokenized dedupe (R4)      |
| `memory.mjs`                  | Matrix memory over the worker's `cowboy/memory` bridge                    |
| `presentation.mjs`            | Facade calls as native tool calls (kind, title, path, diff, result)       |
| `facade.mjs`                  | MCP tools over the one execution `Connection`                             |
| `facade-relay.mjs`            | stdio MCP server `agy` launches; relays to the proxy's socket             |
| `projection.mjs`              | Target context appended to the system prompt                              |
| `compaction.mjs`              | `/compact` as a typed compaction; context restated after it               |
| `subagents.mjs`               | The `Subagent` tool's host: child `agy` sessions inside the parent's turn |
| `evidence.mjs`                | Backend refusal classification from the unit's `agy` log                  |
| `hook.mjs`                    | `PreToolUse` allow-list (`agy-hook`)                                      |
| `hooks.json`, `settings.json` | Policy the daemon binds read-only over the home                           |

## Behavior

- `agy acp --disable-telemetry --include-default-customizations=false` runs
  with only `HOME`, `PATH` (`<runtime>/bin:/usr/bin:/bin`), `LANG`, `TZ`,
  `TMPDIR`, `AGY_CLI_DISABLE_AUTO_UPDATE=true` and
  `DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent`, in an empty private
  directory. The descriptor in `$CREDENTIALS_DIRECTORY` is never passed on.
- Client requests: `initialize`, `session/new|load|resume|prompt|cancel|
  set_mode|set_model|set_config_option`; notification `session/cancel`.
  Everything else (`authenticate`, `logout`, unknown) is refused with
  `-32601`. `initialize` never offers client `fs`, `terminal`, or URL
  elicitation; in ordinary sessions where the worker advertised
  `clientCapabilities.elicitation.form`, it forwards `elicitation: { form: {} }`
  so `agy` presents `ask_question` as a live ACP form inside the turn rather
  than falling back to chat mode.
- Agent requests: only `session/request_permission` and form-mode
  `elicitation/create`. Permissions are allowed only for the facade tools,
  `ask_question` and the web built-ins (`search_web`, `read_url_content` for
  http(s) URLs, `view_content_chunk`; see the design's "Web search and URL
  reading"); any other tool's request is rejected by the proxy without
  reaching the worker. A facade tool's request is approved by the proxy itself
  (Cowboy's default, like Claude's `bypassPermissions`) unless the worker
  selected `agy`'s `plan` mode; only then, and for `ask_question`, does it
  reach the owner. Form `elicitation/create` is forwarded only in ordinary
  sessions where the worker advertised form elicitation, inside a known live
  turn, under a unique worker request ID with tier redaction and explicit
  field allow-listing; non-form modes (`url`), managed calls, inactive turns,
  and malformed replies cancel cleanly (`{ action: "cancel" }`). Agent
  notifications: only `session/update`, whose facade tool calls (and a
  permission request's) are shown as native ones: kind, title, target path and
  an edit's diff.
- Account plan: the `_meta.antigravity.tier` of a `session/new|load|resume`
  result (`agy` 1.5.9: `{name: "Antigravity Starter Quota", id: "free-tier",
  upgradeUri}`) becomes the proxy's plan `{id, name}` when `id` is a slug of
  at most 64 characters and `name` has no character of Unicode category C or
  a line or paragraph separator anywhere (checked before trimming) and, once
  its surrounding spaces are trimmed, 1 to 80 code points (the Controller and
  the Web apply the same rule); a tier it cannot read clears the plan, a
  result without one keeps it. A session result is forwarded without awaiting
  anything, so it reaches the worker before a `usage_update` that follows in
  the same chunk, and that reading already carries the plan. A
  `session/prompt` result is not: its turn stays observed until its evidence
  settles, so a built-in tool reported in the same chunk still makes the
  reply a policy violation. Every
  `usage_update` then carries `_meta.cowboy = {sanitizer: 1, account_plan}`
  (whatever `agy` put under `_meta.cowboy` is dropped first; the Controller
  accepts a plan only beside `sanitizer: 1`), so the session usage the
  worker already relays shows the Controller the account's plan. The raw tier (with its `upgradeUri`) is
  removed from every message forwarded to the worker: each `_meta` at any
  depth (results, error data, notifications, session updates, agent
  requests, tool inputs and outputs) loses only its `antigravity.tier` and
  the objects that leaves empty; nothing else is changed or dropped, however
  deeply nested. One iterative post-order walk copies each changed object
  once. Nothing is requested for it: the plan is only what `agy`
  reports while the owner runs a session.
- `session/new|load|resume`: the worker's runtime directory
  (`binding.runtime.cwd`) becomes the binding's canonical workspace root, the
  `cwd` `agy` works in; `mcpServers` becomes exactly
  `cowboy` = bundled Node + `facade-relay.mjs <socket> <channel>`;
  `_meta` becomes `{antigravity: {systemPrompt: {append: <projection>}}}`;
  `additionalDirectories` is dropped. A session path that exists and is not
  empty on the runtime side is refused.
- Readiness before answering `initialize`: policy files present, identical to
  the runtime's, not writable and with Cowboy's gate shape (`agy` silently
  runs without hooks when `hooks.json` does not parse); descriptor valid; one
  execution connection with a finished handshake; the facade's exact tool
  set; a projection built. The projection's Git probe is a readiness process
  the worker's ledger admits outside any turn, one at a time, only while the
  worker is starting its session (worker state, never the process id);
  guidance files are read only when they are regular files whose canonical
  path is themselves (symlinks refused outright) inside the repository.
  Failure answers
  `cowboy_antigravity_unavailable: <reason>`.
- Tools: `ReadFile` (≤ 256 KiB per call, offset paging), `ListDirectory`
  (≤ 2,000 entries), `SearchFiles` (target `rg`, ≤ 500 lines, 512 KiB, 30 s),
  `WriteFile`/`EditFile` (only after `ReadFile` in the session; `EditFile`
  refuses text that is not valid UTF-8 and a result over 3 MiB before
  building it, and keeps a UTF-8 BOM; publication is the execution helper's
  conditional `apply` (`$COWBOY_EXECUTION_FILE_HELPER`, the Claude runtime's
  primitive): it writes only while the file still hashes to what was read,
  keeps the original until the replacement is in place and leaves the file
  unchanged or exactly at the result; requests over 48 KiB travel as a
  one-shot file in `~/.cache/cowboy/file-buffers`), `RunCommand` (target
  shell, default 120 s, maximum 600 s, enforced by an independent timer even
  when the command is not read again or a reader is busy; 256 KiB head and
  tail; `wait_seconds`, validated before anything starts, returns a
  `command_id`), `StopCommand` (the handle is kept until the target confirms
  the process closed), `UpdatePlan` (validated here, never sent to the
  target; the proxy shows it as the session's ACP `plan`). Paths are canonicalized on the target and confined to
  the workspace root as a guardrail against mistakes, not an isolation
  boundary: `RunCommand` is a shell on the same Machine (design "Revision 5
  simplification"). Tools work
  only inside a turn; on every turn end the turn is fenced, its admitted
  mutations are awaited and its commands stopped.
- Subagents (design "Subagents"): `Subagent` starts another `agy` session in
  the same process inside the calling session's turn (its working directory,
  projection, mode, model and effort, its own facade channel and turn) and
  returns its report; `StopSubagent` stops one. It waits at most 600 s, then
  returns the id to wait for again. They are reported to a worker that
  declared `clientCapabilities.subagents` as ACP subagents (`subagent_update`,
  a linked delegation `session_message`, the child's own updates), at most 2
  at once and 8 per turn, one level deep, none in plan mode or a managed
  call. A parent's turn end or cancel stops them; a child's gate violation,
  unknown effect or backend refusal ends the parent's turn.
- Presentation (design "Feature inventory and Cowboy adaptation"): `agy`
  reports no result for an MCP call, so the facade reports each answer
  before replying and the proxy puts it on `agy`'s terminal update of the
  call it binds to (the oldest unanswered announcement of that tool with the
  same arguments, else of that tool); a running `RunCommand` shows its last
  16 KiB at most every 2 s. Extension calls show as
  `mcp__<server>__<tool>` without `toolSummary`. `UpdatePlan` becomes an ACP
  `plan` and its tool call is not forwarded. A whole `agent_message_chunk`
  "\n⚠ <text>\n" is `agy`'s own status line: for a worker that advertised
  `session.notices` it becomes a `warning` `notice` and stays out of the
  reply and memory. `/schedule`, `/browser`, `/swarm`, `/run-workflow` and
  `/learn` are dropped from `available_commands_update`. The hook's denial
  of a subagent, swarm, workflow, timer or browser tool names the Cowboy
  alternative.
- Compaction: `agy` advertises `/compact` and runs it as an ordinary
  `session/prompt` that reports nothing over ACP (1.5.9 summarizes the
  history in that turn: the next turn's input drops by the summarized
  part). The Web's compact action sends exactly that command. A prompt whose
  text blocks read `/compact` (optionally with instructions) reaches `agy`
  unchanged and, when the worker's `initialize` advertised
  `clientCapabilities.session.compaction`, is shown as ACP's typed
  `compaction_update` (`in_progress` before the prompt, then `completed`,
  `cancelled` or `failed` with the turn error, sent before the turn's
  reply). A `usage_update` `agy` sends during that turn measures the history
  being summarized, so the proxy drops it: with no report during the
  compaction, the Controller shows the context as unknown until the next
  turn's reading. After a completed compaction the first owner prompt that is not a
  slash command is prefixed with `<cowboy_context_restated>`, the projection
  the session was opened with (routing, environment, repository and
  extension guidance), so it stays in effect even if a summary drops it;
  `agy` keeps the appended system prompt as its own section, so this is a
  redundancy, not a replacement. A failed restating turn restates again; a
  `session/load|resume` appends a fresh projection and needs none.
- A lost reply after a mutation (`fs/writeFile`, `fs/createDirectory`,
  `process/start`, a command read or terminate), or the worker's typed
  `error.data.outcome: "unknown"`, may have been applied: the turn is fenced
  at once (a concurrent tool call admits nothing more), the tool answers
  `outcome unknown`, the proxy sends `session/cancel` and adds
  `cowboy.uncertain_effect` to the turn's result `_meta`, or to its error's
  `data` when the turn ends with an error. Nothing is retried.
- A process the executor reports as `unknown process id` after its start was
  answered has ended: the executor forgets a process 30 s after it closed, as
  during a long Controller restart. It is never an unconfirmed termination. A
  forgotten write helper whose report is lost is settled by the requested
  bytes on the target, otherwise it stays `outcome unknown` and the file must
  be read again; a forgotten command reports `Exit code: unknown`; a
  forgotten search asks to run it again
  ([design](../../../../docs/uncertain-effect-recovery.md)).

## Backend evidence

Every `session/prompt` ending, a vendor error too, is classified: a result
carries `_meta.cowboy.antigravity_backend` (`class`, `successes`,
`transient`, `barrier`, `refusals`), and an error ending without a refusal
record settles `unknown_refusal`, so the Controller never sees an
unattributed vendor error. Any class but `ok` turns the response into
JSON-RPC error `-32603` whose message is
`antigravity_backend_refusal:<class>`, matched by Antigravity's signed
`error_rules`, and whose `data` carries the owner-facing sentence as
`message` beside the evidence in `cowboy`. Never `-32000`: that is ACP's
`auth_required`, which the worker presents as rejected credentials (keeping
only the data), so the Controller could not read the closed code and settled
`unknown_refusal` (sess-1795107755038).

One error ending is not a backend outcome: a prompt `agy` refused before its
agent loop saw it. It is recognised only when all of these hold: the error
states a text `agy` answers locally (`LOCAL_REJECTIONS` in `proxy.mjs`) exactly,
as its `message`, a string `data` or `data.message`/`error`/`details`, with
every other present field that text or a generic JSON-RPC label, and never
with ACP's `-32000`; the prompt is the kind the rejection is about (an image
answer needs an image block); and the turn's window holds, through its
barrier, exactly one prompt receipt and no dispatch, with no success, retry or
refusal record anywhere in it. A stopped turn, a forced exit, a violation, a
possible effect, a subagent started in the turn or an earlier structured
attempt never qualifies. The proxy then answers `-32603`
`antigravity_prompt_rejected:<code>` with the owner-facing sentence as
`data.message` and `data.cowboy` `{prompt_rejected: <code>,
antigravity_backend: {class: "not_dispatched", ...}}`; the Controller accepts
exactly that structure for a code it knows, with evidence that itself proves
it (barrier, one prompt, zero dispatches, successes, retries and refusals), and
leaves the account's state unchanged. The only code so far is `image_answer`:
`agy` 1.5.14 refuses a prompt with images while an `ask_question` is pending
("Images can't answer the agent's pending question; reply in text"), which had
parked the account as `unknown_refusal` (sess-1795490077099).

Detection behind the gate: an ACP `tool_call`/`tool_call_update` whose tool
(its `name`, remembered per `toolCallId` within the turn) is outside the
allow-list and is reported running (an `in_progress` `tool_call_update`) or
`completed` means a built-in tool ran. `agy` 1.5.9 announces every call as an
`in_progress` `tool_call` before its `PreToolUse` hook decides and reports a
denied one `failed`; that announcement alone is not a violation.
The proxy sends `session/cancel` and ends the turn with error `-32603`
`antigravity_policy_violation:<tool>` (a name outside `[A-Za-z0-9_.-]{1,128}`
becomes `unknown`), with `data.cowboy.policy_violation` beside the backend
evidence, even when `agy` answers the cancelled prompt with an error. The
Controller then marks the account `needs_attention` and stops its units.
Which field `agy` 1.5.9 uses for the tool name in `tool_call` updates is an
acceptance item; a call that runs without an attributable name (none, an
unknown `toolCallId`, or past the 1,024 names a turn remembers) is a
violation `unknown`.

The proxy reads only its own `agy` process's log
(`$HOME/.gemini/antigravity-cli/log/acp-server-<time>-<pid>.log`) from the
byte offset at the turn's start (window ≤ 4 MiB). Lines matched, observed with
`agy` 1.5.9 and the dedicated test account (`testdata/`); the barrier with
1.5.9 and 1.5.14 (`just agy-barrier-probe`):

| Record                         | Line (glog prefix `I1117 05:11:38.187128     363 ` omitted)                                                                                              |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| success (one per backend step) | `http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0x… ResponseID: …`                  |
| barrier (turn end)             | `server.go:213] ACP Request received: _cowboy/evidence_barrier/<nonce>` (1.5.9: `server.go:988]`)                                                        |
| step failure                   | `errorreport.go:307] agent executor error: generating and executing: PERMISSION_DENIED (code 403): …` (and the same without `agent executor error: `)    |
| retried attempt                | `run.go:316] Run: attempt 1 failed (UNAVAILABLE (code 503): …), retrying in 4s`                                                                          |
| request failure                | `http_helpers.go:876] Failed to make code assist backend request (<url>): {` followed by the `google.rpc` JSON error ending in a line `}`                |
| prompt receipt                 | `server.go:213] ACP Request received: session/prompt`                                                                                                    |
| dispatch (prompt reached loop) | `server.go:1883] Sending user message to conversation <id> (items=…, media=…)` or `prompt.go:358] Responding to pending ask_question for session <id> …` |

The barrier is a request only the proxy sends, once the turn's reply has
arrived: `_cowboy/evidence_barrier/<nonce>` with a fresh random nonce, which
`agy` answers `-32601` (its reply answers no pending request and is dropped).
`agy` logs every request it reads before handling it, so that line follows
every line the turn logged before its reply, and another turn's barrier (a
subagent settling alongside) never settles this one. The worker cannot send
it: only the runtime requests above reach `agy`. `agy` 1.5.9 also ended each
turn's conversation stream (`server.go:1323] Stream goroutine exited for
<sessionId>, sending completion signal`), the barrier until Antigravity
0.2.23; from 1.5.10 the stream outlives the turn (it carries a woken agent's
replies between turns), so the line no longer marks a turn's end and is not
read. A new `agy` pin passes `just agy-barrier-probe <agy> <receipt>` first.

Only `code`, `status` and `ErrorInfo.reason` are read; message text never is.
Rules: a turn is `ok` only with its barrier and at least one success record
and no refusal record. Refusal records are step failures, request failures
and retried attempts with any code but 500, 502, 503 or 504; only those
retried attempts count as `transient` (an unknown retried code is never
assumed transient). A step failure without a reason takes the reason of the
latest request failure with the same status and code earlier in the same log
(`agy` logs `VALIDATION_REQUIRED` once, from its background quota request,
and each refused turn repeats only `PERMISSION_DENIED (code 403)`).
`VALIDATION_REQUIRED` → `verification_required`, `UNSUPPORTED_LOCATION` →
`location_unsupported`, `RESOURCE_EXHAUSTED` or 429 → `rate_limited`, any
other refusal → `unknown_refusal`; several refusals settle by precedence
`suspended` > `verification_required` > `location_unsupported` >
`unknown_refusal` > `rate_limited`. No barrier within 10 s, no log or an
oversized window → `unknown_refusal`. A cancelled turn with its barrier and without a
refusal record is `ok`; a cancelled turn without its barrier is ambiguous and
settles `unknown_refusal`. `suspended` has no matcher: no suspension line has been
observed, so a suspension settles `unknown_refusal`. A request whose failure
`agy` never logs cannot be seen; the success count is not matched against
issued requests.

## Tests

```bash
nix develop -c node --import ./tools/register-memory-client.mjs --test components/provider-runtime/packages/antigravity-acp/*.test.mjs
```

`test-endpoint.mjs` is a fake execution endpoint (Unix socket, WebSocket)
and `testdata/fake-agy.mjs` a stand-in for `agy acp` (it logs every request it
reads, and from 1.5.10 no stream barrier); neither is shipped.
`tools/agy-barrier-probe.mjs` runs a real `agy` executable, unauthenticated and
without network, through this package's `AgyLog`.
