# Antigravity Provider plan

Status: proposed, 2026-11-17. Nothing is implemented or admitted. The normative
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

Remaining Phase 1: OVH install, login and footprint; ACP logout; suspension
and 429 shapes; concurrent refresh on two Machines; `systemPrompt.append`
steering; Git/guidance context; disabling native subagents, browser and other
tools that bypass the facade.

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
  concurrency caps, backoff, suspension error rule, exclusion from managed and
  unattended work.
- Kill switch: admission fence, confirmed worker termination, native logout,
  offline-Machine handling, conversations and worktrees preserved.
- UI: picker, account cards and Session Settings on mobile and desktop.
- Reader-first rollout across active, automatic-rollback and cold-bootstrap
  Catalog readers.

Acceptance on Hawk with the test account: sessions, resume after worker
restart, logout, kill switch, and two-account isolation if Phase 1.3 passed.

### Phase 4 — OVH split execution

Disable built-in workspace tools, inject a Cowboy MCP facade bound to the
execution JSON-RPC endpoint, project target context, and refuse initialization
unless a zero-inference probe proves facade and projection. Declare
`provider.execution-jsonrpc.v1` only after exact-generation native tests pass for
fresh, resumed and compacted tool dispatch, implicit filesystem access,
cancellation, target loss and lost mutation replies. Auth and history stay on
OVH. Ordinary OVH sessions stay unavailable until this passes.

### Phase 5 — Extras

Usage card only from a documented surface; fork; Matrix memory bridge. Managed
calls are out of scope.

## Rollout

Each phase is its own component release. Real accounts are admitted only after
the test-account acceptance and soak. A blocked Antigravity lane must not
affect other Providers. `gemini` stays in `plugins.disabled`.
