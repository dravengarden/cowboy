# Providers

A **provider** is the recipe for launching one agent CLI's ACP adapter. All
providers share the single `src/acp.rs` client backend; a provider only declares
**how to spawn its adapter binary**. Adding one is a registry entry plus, where
needed, hand-coded confirm-detection rules — the core never changes.

## Registry

`provider::builtin()` returns the in-tree registry:

| Provider | Launch | ACP on-ramp |
|---|---|---|
| `claude-code` | `npx -y @agentclientprotocol/claude-agent-acp` | Claude Code ACP adapter |
| `claude-deepseek` | the same Claude ACP adapter with an isolated provider-owned `CLAUDE_CONFIG_DIR` | Claude Code over the independent local DeepSeek Anthropic Messages gateway |
| `codex` | `npx -y @agentclientprotocol/codex-acp` plus full-access defaults | adapter over Codex App Server |
| `codex-deepseek` | the same Codex ACP adapter with an isolated provider-owned `CODEX_HOME` | Codex over the independent local DeepSeek Responses gateway |
| `gemini` | `npx -y @google/gemini-cli --acp` | Gemini's native ACP mode |

Each entry is a **`LaunchSpec`**: `id` + `command` + `args` + scoped environment.
That's the whole contract a provider must satisfy to start; everything downstream
(initialize/handshake/command loop) is provider-agnostic and lives in `acp.rs`.

Managed Machine releases also set Codex ACP's `CODEX_PATH` to
`cowboy-codex-app-server`. This transparent NDJSON proxy forwards the selected
Codex CLI unchanged and adds `excludeTurns: true` only to App Server
`thread/resume`. It prevents no-history ACP `session/resume` from hydrating and
serializing the complete rollout while leaving `session/load` intact: the
adapter still performs its explicit `thread/read(includeTurns=true)` afterward.
The shim does not patch or take ownership of the npm-managed adapter and is
idempotent with an upstream adapter that starts sending the option itself.

`codex-deepseek` is a runtime variant, not a second agent implementation. Codex
still owns planning, tools, approvals, goals, memory, and execution; its model
provider points at `codex-deepseek.service` on loopback. The shim owns only wire
compatibility and credential injection and has an independent release profile,
lifecycle, port, credential, and health check. V4 Flash is forwarded byte-for-byte
to DeepSeek's native Responses API; only V4 Pro uses the temporary Chat
compatibility path until DeepSeek enables native Codex support for that model.
Restarting or upgrading the gateway never restarts Cowboy, Cowboy Machine, or
resident ACP workers.

Its `CODEX_HOME` lives under
`~/.local/state/cowboy/providers/codex-deepseek/codex-home` and is generated from
a closed template. It does not inherit the user's normal Codex config and does
not link `auth.json`, history, memories, rules, plugins, or skills. The worker
also removes inherited OpenAI, Codex, and DeepSeek credential/config variables;
the DeepSeek secret exists only in the gateway's systemd credential mount.
Standard `codex` sessions therefore keep their ordinary account and state, and
the two runtimes can execute concurrently without influencing one another.
The generated config prefers the independently moved component-profile catalog.
During the one-time rollout only, it may use the DeepSeek-only legacy `/etc`
catalog if the profile has not yet been initialized; this fallback contains no
OpenAI config or credentials and can be deleted after older Machine generations
have drained.

`claude-deepseek` is the corresponding Claude Code runtime variant, not another
agent implementation. It uses `claude-deepseek.service` on loopback and a
provider-owned config directory under
`~/.local/state/cowboy/providers/claude-deepseek/claude-config`. The provider
clears inherited Anthropic, Claude, and DeepSeek configuration before applying
its closed base URL, placeholder client token, V4 Pro 1M main-model, V4 Flash
fast/subagent models, and streaming settings. Claude Code is explicitly marked
as provider-managed-by-host, so project or user settings cannot replace the
endpoint or authentication selected by Cowboy. The real API key exists only in
the gateway's distinct systemd credential. Cowboy also supplies a provider-only
POSIX `SHELL` (portable `/bin/sh` by default, with a host override) because a
detached systemd Machine worker does not inherit an interactive login shell;
the override control variable is removed before Claude Code starts.

The isolated directory neither reads nor links ordinary `~/.claude`, top-level
Claude instance metadata, settings, credentials, history, projects, plugins, or
cache. Normal Claude and DeepSeek Claude may run concurrently; only the npm
adapter executable and provider-agnostic ACP implementation are shared. The
gateway in Columbus has its own process, profile, port, receipt, credential,
tests, and release transaction. It preserves native Anthropic Messages/SSE and
contains only current fixture-backed DeepSeek compatibility repairs; it is not
a shared provider router.

Gemini CLI stopped accepting Google Login for consumer, Google AI Pro, and AI
Ultra accounts on 2026-06-18. Its ACP mode remains usable with a Gemini API key
or Code Assist Standard/Enterprise credentials. Machine authentication therefore
has two explicit paths:

- **Gemini API key** writes `GEMINI_API_KEY` to the target user's
  `~/.gemini/.env` with user-only permissions and selects `gemini-api-key` in
  Gemini settings.
- **Standard/Enterprise Google Login** is offered only when
  `GOOGLE_CLOUD_PROJECT` is configured on the target Machine. The project is the
  non-secret evidence that the OAuth credentials belong to a still-supported
  Code Assist deployment.

An old `oauth-personal` credential file without that project is never reported
as signed in. Cowboy recognizes the upstream retirement response as a terminal
startup error: selecting the crashed session does not create a restart loop,
while an explicit Retry remains available after the user changes credentials.
Personal, Google AI Pro, and AI Ultra accounts belong to Antigravity;
Antigravity CLI is not a drop-in Cowboy provider until it publishes an ACP
server mode.

## Resume is discovered, not declared

There is **no static resume flag**. A provider gains resume support purely by the
agent advertising `agent_capabilities.load_session` in its `initialize`
response. The supervisor reads that at runtime and decides whether to
`session/load` on revive. This keeps the registry honest: capability follows the
adapter, not a hardcoded table.

## Deferred: ephemeral side conversations (`/side`, `/btw`)

Codex itself supports an ephemeral side conversation, but the published
`@agentclientprotocol/codex-acp` adapter does not currently advertise or expose
that operation over ACP. Cowboy intentionally does **not** approximate it with a
normal prompt, the main-session queue, or a visible forked session: all three
would violate the feature's contract by disturbing the active task, polluting
its transcript, or leaving persistent session state.

TODO: add the Desktop and Mobile BTW affordances when the adapter exposes a
structured, capability-negotiated ACP operation with these semantics:

- the active turn continues without interruption;
- the side exchange is isolated from the main transcript and model context;
- the side exchange is ephemeral unless the user explicitly promotes it;
- unsupported providers omit the affordance rather than simulating it;
- capability detection, not a provider/version string, controls availability.

Until then `/side` and `/btw` must not be added to Cowboy's fallback command
list. A future implementation should consume the adapter-advertised capability
and keep the shared domain operation separate from the Desktop and Mobile UI.

## Per-provider quirks the core absorbs

```mermaid
flowchart LR
    CC["claude-code"] --> CFG1["config via<br/>notification"]
    CX["codex"] --> CFG2["config in<br/>session resp"]
    GM["gemini"] --> MODE["session MODES<br/>→ mode chip"]

    style CC fill:#eef2ff,stroke:#6366f1
    style CX fill:#dcfce7,stroke:#16a34a
    style GM fill:#fef9c3,stroke:#ca8a04
```

- **claude-code** sends its config options (mode / model) *after* the session is
  created, via a `config_option_update` notification.
- **codex** returns config options in the session-creation response.
- **gemini** has no config-option concept for approvals — it uses ACP session
  **modes**. cowboy synthesizes a `"mode"` config chip so the UI presents one
  uniform control, and routes a change to `SetSessionModeRequest` instead of the
  `session/set_config_option` ext method.

## L1 confirm detection

Each provider also owns deterministic **layer-1** rules for the confirm-detect
skill (`src/provider/confirm.rs`). Given a `TurnEndCtx { stop_reason, final_text }`,
a provider's hand-coded rules (stop-reason markers, text patterns) can decide
"awaiting user" / "done" cheaply, short-circuiting the expensive LLM-based
layer-2 judge. Only an ambiguous `EndTurn` falls through to L2. See
[Confirm-detect & inference](07-confirm-inference.md).

## Toward out-of-process providers

The registry is in-tree today, but the launch-spec shape is deliberately thin so
a provider could later be spawned as its **own subprocess** talking to cowboy
over a local socket — letting third parties add providers without recompiling
cowboy. Not built yet; the interface just doesn't preclude it.

## Verifying a provider

`cowboy try-agent --provider <id>` spawns the adapter end-to-end with no Hub and
no persistence, sends one prompt, and streams the result to stdout. It is the
quickest way to confirm an adapter is installed and the handshake works before
wiring it into a live session.
