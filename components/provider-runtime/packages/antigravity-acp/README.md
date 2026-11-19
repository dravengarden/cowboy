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

| File                          | Role                                                                   |
| ----------------------------- | ---------------------------------------------------------------------- |
| `launch.mjs`                  | Proxy entry: readiness, closed `agy` environment, wiring               |
| `proxy.mjs`                   | ACP relay: runtime method allow-list, session rewrite, turn settlement |
| `facade.mjs`                  | MCP tools over the one execution `Connection`                          |
| `facade-relay.mjs`            | stdio MCP server `agy` launches; relays to the proxy's socket          |
| `projection.mjs`              | Target context appended to the system prompt                           |
| `evidence.mjs`                | Backend refusal classification from the unit's `agy` log               |
| `hook.mjs`                    | `PreToolUse` allow-list (`agy-hook`)                                   |
| `hooks.json`, `settings.json` | Policy the daemon binds read-only over the home                        |

## Behavior

- `agy acp --disable-telemetry --include-default-customizations=false` runs
  with only `HOME`, `PATH` (`<runtime>/bin:/usr/bin:/bin`), `LANG`, `TZ`,
  `TMPDIR`, `AGY_CLI_DISABLE_AUTO_UPDATE=true` and
  `DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent`, in an empty private
  directory. The descriptor in `$CREDENTIALS_DIRECTORY` is never passed on.
- Client requests: `initialize`, `session/new|load|resume|prompt|cancel|
  set_mode|set_model|set_config_option`; notification `session/cancel`.
  Everything else (`authenticate`, `logout`, unknown) is refused with
  `-32601`. `initialize` never offers client `fs` or `terminal`.
- Agent requests: only `session/request_permission`, and only for the facade
  tools and `ask_question`; any other tool's request is rejected by the proxy
  without reaching the worker. Agent notifications: only `session/update`.
- Account plan: the `_meta.antigravity.tier` of a `session/new|load|resume`
  result (`agy` 1.5.9: `{name: "Antigravity Starter Quota", id: "free-tier",
  upgradeUri}`) becomes the proxy's plan `{id, name}` when `id` is a slug of
  at most 64 characters and `name` at most 80 printable characters; a tier it
  cannot read clears the plan, a result without one keeps it. Every
  `usage_update` then carries `_meta.cowboy.account_plan` (whatever `agy` put
  under `_meta.cowboy` is dropped first), so the session usage the worker
  already relays shows the Controller the account's plan. `upgradeUri` is
  never copied. Nothing is requested for it: the plan is only what `agy`
  reports while the owner runs a session.
- `session/new|load|resume`: `mcpServers` becomes exactly
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
  the process closed). Paths are canonicalized on the target and confined to
  the workspace root as a guardrail against mistakes, not an isolation
  boundary: `RunCommand` is a shell on the same Machine (design "Revision 5
  simplification"). Tools work
  only inside a turn; on every turn end the turn is fenced, its admitted
  mutations are awaited and its commands stopped.
- A lost reply after a mutation (`fs/writeFile`, `fs/createDirectory`,
  `process/start`, a command read or terminate), or the worker's typed
  `error.data.outcome: "unknown"`, may have been applied: the turn is fenced
  at once (a concurrent tool call admits nothing more), the tool answers
  `outcome unknown`, the proxy sends `session/cancel` and adds
  `cowboy.uncertain_effect` to the turn's result `_meta`, or to its error's
  `data` when the turn ends with an error. Nothing is retried.

## Backend evidence

Every `session/prompt` ending, a vendor error too, is classified: a result
carries `_meta.cowboy.antigravity_backend` (`class`, `successes`,
`transient`, `barrier`, `refusals`), and an error ending without a refusal
record settles `unknown_refusal`, so the Controller never sees an
unattributed vendor error. Any class but `ok` turns the response into
JSON-RPC error `-32000` whose message is
`antigravity_backend_refusal:<class>`, matched by Antigravity's signed
`error_rules`.

Detection behind the gate: an ACP `tool_call`/`tool_call_update` whose tool
(its `name`, remembered per `toolCallId` within the turn) is outside the
allow-list and reaches `in_progress` or `completed` means a built-in tool ran.
The proxy sends `session/cancel` and ends the turn with error `-32000`
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
`agy` 1.5.9 and the dedicated test account (`testdata/`):

| Record                         | Line (glog prefix `I1117 05:11:38.187128     363 ` omitted)                                                                                           |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| success (one per backend step) | `http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0x… ResponseID: …`               |
| barrier (turn end)             | `server.go:1323] Stream goroutine exited for <sessionId>, sending completion signal`                                                                  |
| step failure                   | `errorreport.go:307] agent executor error: generating and executing: PERMISSION_DENIED (code 403): …` (and the same without `agent executor error: `) |
| retried attempt                | `run.go:316] Run: attempt 1 failed (UNAVAILABLE (code 503): …), retrying in 4s`                                                                       |
| request failure                | `http_helpers.go:876] Failed to make code assist backend request (<url>): {` followed by the `google.rpc` JSON error ending in a line `}`             |

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
and `testdata/fake-agy.mjs` a stand-in for `agy acp`; neither is shipped.
