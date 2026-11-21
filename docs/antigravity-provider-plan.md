# Antigravity Provider plan

Status: in progress. Phase 3's package and custody fence are implemented
(component release 3.113.0, 2026-11-18) but not yet deployed; no real account
is admitted. The normative
account-risk boundary is
[`requirements.md` — Antigravity consumer-subscription boundary](requirements.md#antigravity-consumer-subscription-boundary).
The plan was design-reviewed by Codex Astra Max (call
`call-3e026b2652b50ebd96eecda1`); its eight findings are folded in below.

## Goal

Add Google's official Antigravity CLI (`agy`) as a first-party Provider
(`antigravity`; the retired `gemini` identity can never return) with parity to
Claude Code and Codex for:

1. integrated login from UI and Agent (Cowboy CLI/MCP), mobile and desktop;
2. OVH remote mode: native runtime on OVH, workspace tools on the session's
   development Machine (Hawk/Falcon), as for current Claude/Codex split
   execution;
3. ordinary sessions: resume, model/effort configuration, account cards.

Authentication is standard Google consumer subscriptions only
(`oauth-personal`: Free, Pro, Ultra). No Gemini API key or Agent Platform.

## Facts

Verified 2026-11-17:

- `agy` replaced Gemini CLI on 2026-05-19. `install.sh` reads
  `https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/linux_x64.json`,
  which returned version `1.5.9`, an immutable versioned
  `storage.googleapis.com/antigravity-public/antigravity-cli/...` tarball and a
  SHA-512. The binary is closed source.
- `agy acp` serves ACP over stdio; Google's ACP page says "Any ACP client can
  launch Antigravity CLI" and reuses credentials cached by an interactive `agy`
  session (or `agy acp --login` for clients with terminal authentication).
- Over SSH, login prints an authorization URL and accepts a pasted code.
  Credentials are stored in the OS keyring (Secret Service on Linux).
- Terms and FAQ: see the requirements section. Enforcement waves occurred in
  2026-02 and were still reported on 2026-11-16.

Unverified (Phase 1): headless keyring behaviour and file-storage fallback,
updater control, tool allow/deny lists, per-session MCP injection, hooks,
ACP `load_session`, session modes, usage/quota surfaces, telemetry, footprint.

Cowboy facts:

- Providers are signed packages under `plugins/<id>/`; all speak ACP via
  `src/acp.rs`. `plugins/grok` (native ACP binary, no adapter) is the template.
- Lock recipe kinds are `npm_archive`, `npm_brotli`, `node_npm` and
  `git_go_static`; publication copies runtime artifacts into the Catalog
  (`tools/publish-plugin-release.ts`). Neither fits a vendor-hosted native
  binary that Cowboy may not redistribute.
- Native-owned custody is fenced to `claude-code`, and
  `components/wire/src/provider_account.rs` exempts `default` accounts.
- Split execution requires `provider.execution-jsonrpc.v1` with a binding on
  every native turn (`components/provider-sdk/src/runtime_capability.rs`);
  Claude needed a tool facade, construction hooks and context projection
  ([execution environments](execution-environments.md#provider-integration)).
- `plugins.disabled` takes effect on Controller restart and stops neither
  workers nor credentials ([usage execution](usage-execution.md)).
- OVH is runtime-only and hosts no projects (Columbus
  `machines/ovh/docs/runtime-only.md`).

## Phases

### Phase 0 — Decision record

The requirements section and this plan.

### Phase 1 — Feasibility spike

Dedicated free-tier test account only; disposable HOMEs; receipts with exact
versions. Hawk covers runtime and ACP behaviour; OVH only receives the verified
binary for login, storage, isolation and footprint probes. Pass/fail questions:

1. Pinned install from the versioned URL; updater disabled or provably inert.
2. Headless login under PTY/pipes; challenge format parseable by
   `login_challenge_tokens`.
3. Credential storage under `$HOME` (file fallback), surviving restarts; two
   HOMEs under one UID independent. Token-free stable identity evidence
   (authority and subject), concurrent renewal on two Machines, logout revoking
   an active worker, crash recovery.
4. ACP: `initialize` capabilities, prompt round trip, `session/load` after
   restart, cancellation, permission requests, model/effort configuration,
   context usage, any documented quota method, suspension and 429 error shapes.
5. Split-execution levers: disabling built-in file/shell tools, per-session MCP
   servers, hooks before tool execution and prompt construction, suppression or
   replacement of runtime cwd/OS/Git context (hook injection alone is not
   suppression), disabling native guidance discovery and subagents.
6. Background daemons, telemetry, writes outside `$HOME`, memory/CPU per session.

Failing 3 blocks multi-account; failing 5 blocks OVH remote mode (there is no
colocated OVH session fallback).

#### Phase 1 progress — Hawk, no login (2026-11-17)

Binary `agy` 1.5.9 from the versioned URL; SHA-512 matched the manifest; the
tarball (99 MB) contains one static executable `antigravity` (266 MB). Run with
a disposable `HOME`/`XDG_*` and no D-Bus session:

- `antigravity acp --help` exposes `--disable-telemetry`, `--login`,
  `--process-per-session`, `--sandbox`, `--app-data-dir`, `--gemini-dir`,
  `--ide-mcp-port` and `--override_user_agent_name` (never to be used: client
  identity spoofing). The binary contains `AGY_CLI_DISABLE_AUTO_UPDATE`; its
  effect is unverified. `GEMINI_FORCE_FILE_STORAGE` does not occur in the
  binary; "keyringAuth" and "no OS keyring" strings exist, so the file fallback
  must be tested with a real login.
- ACP `initialize` (protocol 1) returns `loadSession`, session
  `resume`/`close`/`load`/`list`/`additionalDirectories`, `auth.logout`,
  `_meta.antigravity.systemPrompt.append`, image and embedded-context prompts,
  and auth methods `oauth-personal`, `oauth-business`, `gemini-api-key`,
  `agent-platform`. Only `oauth-personal` may be offered. Unauthenticated
  `session/new` fails with `-32000 Authentication required`.
- All state is written below `$HOME/.gemini/antigravity-cli/` (conversations,
  logs, `installation_id`, `conversation_summaries.db`, built-in skills). No
  other directory was written, so per-generation `HOME` isolation is plausible
  apart from the keyring.
- Idle unauthenticated ACP server: one process, 13 threads, about 170 MB RSS,
  no listening TCP socket.
- Hooks (`hooks.json`): `PreToolUse` can `allow`/`deny`/`ask` and rewrite tool
  arguments; `PreInvocation`/`SessionStart` inject steps; `PostInvocation` and
  `Stop` can terminate. Hooks cannot return a substituted tool result, so split
  execution needs denial of built-in workspace tools plus an MCP facade, or the
  ACP client `fs`/`terminal` capabilities if `agy` delegates to them (to test
  after login).

Remaining Phase 1 items need the dedicated test account.

#### Phase 1 progress — Hawk, dedicated free-tier test account (2026-11-17)

- Login: `antigravity acp --login` under a PTY printed a Google OAuth URL
  (client `1071006060591-…`, PKCE, redirect `https://antigravity.google/oauth-callback`)
  and accepted the pasted code: "Logged in as <email>." The flow fits the
  existing `browser_code` challenge shape.
- A new account first answered every prompt with an agent message
  "⚠ Verification Required" and `stopReason: end_turn`; the log showed
  `PERMISSION_DENIED (403) VALIDATION_REQUIRED` with a Google account
  verification URL. Backend refusals are therefore not JSON-RPC errors: the
  Provider must classify such turns (verification, suspension, quota) into
  account state from a structured source, not from model text alone.
- Credential custody: with `HOME` replaced but no explicit
  `XDG_RUNTIME_DIR`, `agy` still reached the user's session bus and stored
  "Password for 'antigravity' on 'gemini'" in the real GNOME login keyring
  (one fixed item per UID, so a second account would overwrite it). That item
  was deleted after the test. With `XDG_RUNTIME_DIR` set to a private
  directory and no session bus, the log reports "Failed keyring SaveToken,
  falling back to file" and the credential is written to
  `$HOME/.gemini/antigravity-cli/antigravity-oauth-token` (mode 0600). A
  prompt from that HOME succeeded while the real keyring stayed untouched.
  Cowboy must always launch `agy` (login and runtime) with a per-generation
  `HOME`, a private `XDG_RUNTIME_DIR` and no inherited D-Bus address. Cowboy
  never opens the token file.
- Free-tier models: Gemini 3.8 Flash (High/Medium/Low, default High), Claude
  Sonnet 4.6, Claude Opus 4.6, GPT-OSS 120B. `session/new` returns
  `models.availableModels`/`currentModelId`; ACP `usage_update` reports a
  1,048,576-token window; each `session/prompt` result carries input, output,
  cached-read, cached-write and thought token counts.
- Permissions: `run_command` and `write_file` raise
  `session/request_permission`; `read_file` does not.
- Resume: a new process restored a session with `session/load`, replayed its
  history and answered from it.
- Tool delegation: with client `fs` and `terminal` capabilities declared,
  `agy` still executed `read_file`, `run_command` and `write_file` locally; it
  never called `fs/*` or `terminal/*`.
- Facade: a global `~/.gemini/config/hooks.json` `PreToolUse` hook denied
  built-in tools; an MCP server passed in ACP `session/new.mcpServers` exposed
  `ReadFile` and `RunCommand` (seen as `mcp_cowboy_*`); the model fell back to
  them and returned target-only content. The model first tries a built-in
  tool each time; steering via `systemPrompt.append` is untested.
- Context: asked without tools, the model reported the ACP session `cwd`, OS
  Linux and no other path. A same-path mirror directory on OVH may therefore
  present the target path; Git status and guidance discovery remain untested.

- Steering: `session/new._meta.antigravity.systemPrompt.append` (string)
  telling the model to use only the facade removed the built-in attempt; the
  only tool call was `mcp_cowboy_ReadFile`.
- Guidance: `agy` loaded `AGENTS.md` and `GEMINI.md` from the runtime `cwd`
  and obeyed them. On OVH the same-path mirror directory stays empty, so target
  guidance must be injected as a bounded snapshot through `systemPrompt.append`.
  No Git branch or status was injected into model context.
- Built-in tools reported by the model (free tier): `read_file`,
  `run_command`, `write_file`, `edit_file`, `manage_task`, `send_message`,
  `schedule`, `invoke_subagent`, `define_subagent`, `manage_subagents`,
  `search_web`, `run_workflow`, `swarm`, `ask_question`, `list_resources`,
  `read_resource`. `schedule`, `swarm` and `run_workflow` are unattended or
  fan-out surfaces and must be denied by the hook.
- Subagents: a subagent started with `invoke_subagent` ran asynchronously after
  the parent turn ended, and its `read_file` call passed through the same
  global `PreToolUse` hook (denied). Hooks therefore also gate subagent tools,
  but background work outliving `end_turn` needs its own lifecycle handling.

#### Phase 1 progress — OVH (2026-11-17)

- The verified tarball was copied from Hawk (SHA-512 rechecked on OVH, nothing
  built there). Login with a private `XDG_RUNTIME_DIR` stored the credential
  as `$HOME/.gemini/antigravity-cli/antigravity-oauth-token`; over SSH the log
  states "Using file-based token storage because SSH session detected".
- Idle authenticated ACP server on OVH: about 166 MB RSS.
- Location: with default address selection every OVH prompt ended with the
  agent message "Your current account is not eligible for Gemini Code Assist
  for individuals because it is not currently available in your location"
  (`PERMISSION_DENIED`, `UNSUPPORTED_LOCATION`). OVH reaches Google over IPv6
  (`2604:2dc0:202:300::1cec`), for which `www.google.com` answers with the
  `/sorry` abuse interstitial, while its IPv4 address (`40.160.141.233`,
  Hillsboro, Oregon) gets 200 and YouTube reports `GL=US`. Running `agy` (login
  and runtime) in a unit with
  `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK` forces IPv4; the same
  account then completed prompts on OVH and the full facade path returned
  `TARGET_SENTINEL` through `mcp_cowboy_ReadFile` and `mcp_cowboy_RunCommand`
  with no built-in tool attempt. The restriction is per Provider process only;
  no other OVH traffic changes. (Hawk's earlier success used a Stormbird exit
  that Google sees as `GL=FR`, not the OVH address.)
- Logout: ACP `logout` returned `{}`, removed the token file, and the next
  `session/new` failed with `-32000 Authentication required`. A separate grant
  on Hawk kept working afterwards, so per-Machine logins are independent
  grants. Whether logout revokes the grant at Google was not tested.

- Refresh: at 06:09 UTC the Hawk grant (login 05:07) and the OVH grant (login
  05:34) were prompted concurrently. Hawk refreshed and rewrote its token file
  at 06:09:14; OVH was still within its access-token lifetime. Both turns
  succeeded, so a refresh on one Machine did not invalidate the other grant.

Remaining Phase 1: suspension and 429 shapes
(observe only, never provoke).

#### Phase 1 notes — refusal evidence and hooks (2026-11-18)

- Turn attribution: one `agy acp` process writes
  `~/.gemini/antigravity-cli/log/acp-server-<time>-<pid>.log`; the ACP
  `sessionId` is the conversation ID in its lines. Each backend step of a
  turn logs `http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0x… ResponseID: …`;
  the turn ends with `server.go:1323] Stream goroutine exited for <sessionId>, sending completion signal`.
- A refused turn (new account, `VALIDATION_REQUIRED`) logs no stream line,
  then `errorreport.go:307] agent executor error: generating and executing: PERMISSION_DENIED (code 403): Verify your account to continue.`
  (twice) before the barrier. The reason enum appears only once per process,
  in the structured JSON of a background request:
  `http_helpers.go:876] Failed to make code assist backend request (https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary): {`
  … `"status": "PERMISSION_DENIED"` … `"reason": "VALIDATION_REQUIRED"` … `}`.
- A transient capacity failure is retried and logged as
  `run.go:316] Run: attempt 1 failed (UNAVAILABLE (code 503): No capacity available for model gemini-3.8-flash-high on the server), retrying in 4s`;
  the agent message also shows `⚠ API error (attempt 1)`.
- Not observed: `UNSUPPORTED_LOCATION` lines on Hawk (seen on OVH only as
  agent text), 429 and suspension. The classifier in
  [`cowboy-antigravity-acp`](../components/provider-runtime/packages/antigravity-acp/README.md)
  matches the lines above and settles anything else `unknown_refusal`.
- Hooks: the `PreToolUse` input carries `toolCall.name`; MCP facade tools are
  named `mcp_cowboy_<Tool>`. A missing, timed-out, malformed-output or
  non-zero hook denies the tool; an unparseable `hooks.json` disables every
  hook. `settings.json` `permissions.deny` (either the tool-name or the
  documented `read_file(*)` syntax) did not deny `read_file` in `agy acp`.
  Details in the design's "Step 4 acceptance".

### Phase 2 — Native custody as a Provider declaration

Replace the `claude-code` literals of the native-owned fence with a signed
declaration (credential custody, native identity and logout recipes), applying
to every account including `default`, with the auth method persisted per
account. Claude behaviour must not change. Tests prove neither default nor named
native credentials reach candidate, vault or replica storage. SDK bump, worker
pin advance and OVH maintenance.

### Phase 3 — Provider on Hawk

- Vendor-hosted install: a lock/publisher capability that records URL and
  SHA-512, lets each Machine download and verify, and keeps the binary out of
  the Catalog.
- `plugins/antigravity`: `agy acp`, isolated environment, reused behaviour
  profiles where Phase 1 allows, OAuth method only.
- Risk controls from the requirements section: per-account placement and
  concurrency caps, backoff, suspension error rule, exclusion from unattended
  work and from every managed call except read-only context asks (see
  [Managed read-only context asks](#managed-read-only-context-asks)).
- Kill switch: admission fence, confirmed worker termination, native logout,
  offline-Machine handling, conversations and worktrees preserved.
- UI: picker, account cards and Session Settings on mobile and desktop.
- Reader-first rollout across active, automatic-rollback and cold-bootstrap
  Catalog readers.

Acceptance on Hawk with the test account: sessions, resume after worker
restart, logout, kill switch, and two-account isolation if Phase 1.3 passed.

#### Phase 3 progress — package (2026-11-18)

- Phase 2 shipped only the `owner_declared_v1` identity recipe and the
  `ipv4_only_v1` network policy (SDK 3.4.0); the native-owned fence was still
  the `claude-code` literal, so a named Antigravity login would have been
  exported to the account vault and the default account used the vault too.
  `antigravity` is now in `NATIVE_ONLY_PROVIDERS` and the new
  `NAMED_ONLY_PROVIDERS`: `portable_credentials_allowed` refuses its default
  account as well, the Machine refuses portable login candidates and replica
  launches for it, and the Controller refuses its default sign-in. Replacing
  the literals with a signed declaration stays future work.
- Install: `vendor_archive` (Google URL, SHA-512, SHA-256, never copied into
  the Catalog) for `agy` 1.5.9 Linux x86_64. The live `linux_x64` manifest
  returned 404 on 2026-11-18 while 1.5.10 was published for arm64 and macOS
  only, so the pin stays on the tested 1.5.9; macOS waits for one version with
  every target published.
- Launcher: `owned_archive` `agy-launch` (see
  [Provider architecture](architecture/04-providers.md)). Verified on Hawk with
  the test account: the hooks denied `schedule` and `swarm` while
  `run_command` ran; ACP `logout` piped to `agy acp` removed the token file.
- Auto-update: `agy --print` with `AGY_CLI_DISABLE_AUTO_UPDATE=1` spawned the
  updater, which replaced the 1.5.9 executable in place with 1.5.10. Only
  `=true` disables it ("Auto-update disabled via environment variable").
  `agy acp` logs never showed the updater; the launcher sets `=true` anyway.
- `agy` has no non-interactive logout command (`/logout` is refused in print
  mode); the launcher sends ACP `initialize` and `logout` and succeeds only if
  the token file is gone afterwards.
- Configuration: ACP `configOptions` ids `model` and `thought_level`; default
  preset Gemini 3.8 Flash · High. Not offered to managed calls at the time
  (no `managed_profiles`, not a `CALL_TARGETS` member); since 2026-11-21 it
  is a target of read-only context asks only (see
  [Managed read-only context asks](#managed-read-only-context-asks)).
- Codex Astra Max review (call `call-0c43947387086ddb8138f08d`) findings and
  fixes: older SDK 3.4.0 readers would have treated the logins as portable, so
  the manifest now signs `provider.native-custody.v1` (provider-sdk 3.5.0);
  the install probe ran `agy-launch --version` with the Machine user's HOME, so
  only `acp` sessions install hooks; legacy `PATH`/occupancy probes and `npx`
  launches skip Providers without an npm runtime; `invoke_subagent`,
  `define_subagent` and `manage_subagents` are denied; Cowboy draft schedules
  never arm or fire for Antigravity; `accounts.antigravity_admission` (default
  off) gates sign-ins and sessions until acceptance and soak. Deferred: account
  removal deletes the whole native home including `agy` conversations (as for
  Claude); the kill switch must not use that cleanup. Native login/logout
  still lacks the IPv4 restriction; required before OVH login (Phase 4).
  A follow-up review (call `call-cb3524e2003b057b472a43bf`) found that
  managed-call completion notices still dispatched turns; call notices and
  scheduled wakeups now park as drafts for an explicit human send.
- Live check: ACP `logout` from a copied home removed only that local token
  file; the original home's grant kept working, so logout does not revoke the
  Google grant.
- Remaining for Phase 3: deploy with the 3.113.0 OVH maintenance, publish the
  release, Hawk acceptance with the test account (named login through Cowboy,
  keyring untouched, sessions, resume, logout), per-account concurrency caps,
  suspension rule and the dedicated kill switch.

### Phase 4 — OVH split execution

Designed in [Antigravity real accounts on OVH](antigravity-real-accounts-design.md)
(owner decision 2026-11-18: four real accounts on OVH, test account first).


Disable built-in workspace tools, inject a Cowboy MCP facade bound to the
execution JSON-RPC endpoint, project target context, and refuse initialization
unless a zero-inference probe proves facade and projection. Declare
`provider.execution-jsonrpc.v1` only after exact-generation native tests pass for
fresh, resumed and compacted tool dispatch, implicit filesystem access,
cancellation, target loss and lost mutation replies. Auth and history stay on
OVH. Ordinary OVH sessions stay unavailable until this passes. Antigravity
0.2.2 declares it after the owner accepted the hook as the only layer for
read-only built-ins (2026-11-20; see the design's "Split execution
declaration").

### Phase 5 — Extras

Usage card only from a documented surface; fork; Matrix memory bridge.
Managed calls other than read-only context asks stay out of scope.

### Managed read-only context asks

Owner decision 2026-11-21: Antigravity (Gemini 4 Hydrogen) may answer
managed `analysis` and `sessions` context asks, accepting the account risk;
reviews, design reviews, image calls and every call that could write stay
unavailable. The Controller admits only those asks and advertises text-only
output for Antigravity.

- The manifest declares `provider.managed-profiles.v1` with one
  `read_only_v1` profile (`acp`, `--disable-telemetry`,
  `--cowboy-managed-profile=read-only-v1`); there is no `image_v1`. An
  account-sandbox runtime unit starts `bin/agy-proxy` with the command line
  the daemon fixes, so these arguments are a declaration there: the proxy
  learns managed mode from its execution binding (`binding.managed`) and the
  target keeper's `cowboyManaged` announcement, which must name the same
  profile; one without the other refuses `initialize`. `agy-launch` refuses
  every `--cowboy-managed-profile` value because it would start `agy` without
  the facade and gate; `agy-proxy` refuses any value but `read-only-v1`.
- The keeper enforces the profile on every native request: process starts
  are read-only and network-free, and file writes and HTTP are refused. That
  is not a history-only or workspace-only boundary: `ReadFile`, `SearchFiles`
  and `RunCommand` can read any file the execution user can read on the
  target (host-wide reads, no writes, no network), as for Claude's managed
  `Bash`. The facade lists only `ReadFile`, `ListDirectory`, `SearchFiles`,
  `RunCommand` and `UpdatePlan`, and refuses every other name; `StopCommand`
  is not offered (the turn's end stops its commands). The surface a managed
  child must never reach is the Cowboy Agent API: no Cowboy MCP, folder MCP
  or Matrix memory is attached, so it cannot start calls, read sessions or
  change Cowboy. The system prompt carries the managed routing instead of the
  session one.
  Repository skills are neither discovered nor listed, and no slash command
  (agy's or a skill's) is offered or expanded.
- The proxy starts `agy` with `COWBOY_HOOK_PROFILE` in its closed
  environment: `ordinary` for an ordinary session, `read-only-v1` for a
  managed child. The daemon binds `policy/hooks.json` read-only over the
  home; its gate runs the command `agy-hook` from the `PATH` the proxy gives
  `agy` (the runtime's `bin`), whose entry script execs the bundled Node on
  `proxy/app/hook.mjs`, which reads the marker from the environment `agy`
  passes on. Only the exact `ordinary` marker gets the ordinary policy; a
  missing, empty or unknown marker gets the managed allow-list, so a hook
  that does not see the marker can only narrow a session. Under the managed
  allow-list `ask_question`, `search_web`, `read_url_content`, every Cowboy
  MCP or extension tool, subagents, schedules and swarms are denied.
- The proxy validates everything it can check locally before it starts
  `agy`, and chooses the marker from that validated binding only: the
  descriptor itself, a managed record naming exactly `read_only_v1` and a
  nonempty parent session, and agreement with the launch flag (an
  unsupported profile flag, or the managed flag with an ordinary binding, is
  refused). Any failure starts no `agy` at all, and every request of the
  worker is refused with that error. The target's announcement is checked
  once the execution connection is open.
- Until its runtime settles the proxy allows nothing `agy` asks for: every
  permission request is rejected and no command list or session update is
  forwarded. A managed
  child then rejects permission requests for any tool outside its list
  without asking anyone, and a tool outside the list that still runs ends
  the turn as `antigravity_policy_violation`.
- Each turn must carry the Machine-written round as prompt
  `_meta["cowboy.dev/managedCall"]`; the proxy strips it before `agy`. A
  prompt without it, a prompt that is not a nonempty list of text blocks
  (images, resources or mixed content), a round with a non-null
  `outputSchema` (Antigravity cannot apply structured output), a slash
  command, `session/set_mode` and any `session/set_config_option` other than
  `model` and `thought_level` are refused. A managed child has exactly one
  native session, the one its own `session/new` created: a second
  `session/new`, and a prompt, load, resume or configuration request naming
  any other session, are refused; a `session/cancel` for another session is
  dropped, and `agy`'s permission requests and session updates for another
  session (or before the child's session exists) are rejected or dropped
  before anything is granted, recorded or forwarded. Nothing trusted names
  the child's native session id to a replacement proxy (the binding, the
  keeper's announcement and the round carry none), so a managed context ask
  is single-round: a proxy replaced by a worker reload refuses every load or
  resume, the call fails, and its caller retries. An ordinary session
  refuses a prompt that carries a round.

## Rollout

Each phase is its own component release. Real accounts are admitted only after
the test-account acceptance and soak. A blocked Antigravity lane must not
affect other Providers. `gemini` stays in `plugins.disabled`.
