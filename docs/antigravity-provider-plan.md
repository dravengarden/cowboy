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
