# Zed ACP integration

Zed starts `cowboy serve-acp` as a stdio External Agent. The bridge connects to
the already-running cowboy daemon (`http://127.0.0.1:3333` by default); it does
not start another Hub or another copy of a provider session.

Add three custom agents alongside Zed's official Registry agents so native
Codex/Claude/Grok remain available and each Cowboy thread stays fixed to its
provider:

```json
{
  "agent_servers": {
    "cowboy-codex": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "codex"]
    },
    "cowboy-claude": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "claude-code"]
    },
    "cowboy-grok": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "grok"]
    }
  }
}
```

`serve-acp` does not require a copied token. On the first connection it creates
an Ed25519 key locally, opens Cowboy's configured login page, and asks the
signed-in user to approve that public key. The browser may use Cardea,
password, or any other server-configured login method. The bridge then keeps a
private rotating credential under `~/.config/cowboy/client-auth/` (mode 0600),
uses 10-minute sender-constrained access tokens, and rotates its refresh token on every refresh; the device stays signed in for
one year from approval. The Service stores only the public key and refresh-token
hash. Account → Authorized devices lists and revokes these clients.

Zed hides the bridge's stderr, so while the approval is pending every
`session/new`, `session/list` and `session/load` fails at once with the
approval link and the client fingerprint instead of a readiness timeout. Open
the link on any signed-in device, approve, and start the thread again. The
link is also always written to stderr: a headless host starts its browser
opener successfully and shows nothing. A denied approval ends the bridge; an
expired one is requested again.

On a headless host the ACP bridge may keep its API connection on loopback. The
Controller then advertises `COWBOY_PUBLIC_ORIGIN` as the HTTPS browser approval
page, so the link opens on another device without exposing the API port. A
non-loopback API connection accepts only its own HTTPS origin.

`cowboy login https://cowboy.example` may be run ahead of Zed to complete the
same flow. If product login is explicitly disabled, the bridge verifies that
policy through `/api/auth/status` and preserves local-owner access without
creating a credential. Network errors, unsupported controllers, and malformed
status responses never downgrade to anonymous access. `COWBOY_USER_TOKEN` and
`--token` remain hidden, migration-only compatibility inputs for existing
deployments; new configurations must not use them.

Use `cowboy serve-acp --provider grok --machine ovh --workspace matrix` to
create sessions on an explicit registered Machine and workspace ID. For remote
creation, `--workspace` is required and replaces ACP's client-local `cwd` in
the Controller request. The Controller resolves the ID through the Machine's
trusted workspace inventory; arbitrary remote paths are rejected. The default
remains `--machine local` and uses ACP's `cwd` for existing callers.
Offline or unavailable targets return the Controller's error; the
bridge never retries creation on another Machine. Loading an existing session
preserves that session's original placement. With a remote Machine selected,
history and loading are scoped to its registered workspace ID; the ACP client's
directory maps to that workspace even when the remote session uses a prepared
worktree path. Local mode retains exact-directory matching.
Remote creation waits for workspace preparation and Provider readiness, and
reports terminal asynchronous preparation failures as ACP errors. It still
waits for initial config options so Zed can construct its model/effort controls.

Do not override Registry agent IDs (including `codex-acp`, `claude-acp`, and
`gemini`) unless replacing a native agent is intentional. Zed's custom-agent settings have no
icon field, so the independent Cowboy entries use its generic sparkle icon.
Distinct provider icons require separate published ACP Registry entries.

When Zed is attached to a remote project, the command must resolve on that
remote host. Set `COWBOY_DAEMON_URL` or add `--daemon-url` when the daemon is not
on the default loopback address. Non-loopback device authorization requires
HTTPS.

The bridge supports:

- `initialize`, `session/new`, `session/list`, `session/load`, `session/close`,
  and `session/delete`; `session/close` detaches released Zed threads so the
  bridge stops publishing updates to dead thread entities, and succeeds for a
  session Cowboy already deleted;
- `session/delete` answers only after the Controller acknowledged the deletion
  (the session left its list, or is published as `closing`). A refusal or a
  30-second silence is returned as an error instead of a false success;
- complete retained-history replay on `session/load`, followed by any
  permission request the running turn is still waiting on, so a reopened
  thread can answer it;
- title changes: a session Cowboy titles from its first prompt, or that is
  renamed on another surface, updates the open Zed thread through
  `session_info_update`;
- prompt streaming, permission requests, cancellation, and session config
  options. Either `session/cancel` or JSON-RPC request cancellation stops an
  in-flight bridge prompt;
- daemon WebSocket recovery without terminating Zed's stdio ACP process:
  disconnects publish `reconnecting`, commands wait in the bridge, and a fresh
  bootstrap restores attached sessions, statuses, and config options. Each
  attached session then catches up: the transcript events, turn end and
  unanswered permission requests published during the outage are delivered
  once, in order, ahead of the resumed live stream. In-flight prompts remain
  pending through the outage and settle from the missed turn end, or from the
  session being idle again, even when no further live event arrives;
- provider-filtered import/load, preventing a thread from acquiring the wrong
  provider identity;
- ACP subagents: each subagent row of the session reaches Zed as the standard
  `subagent_update`, linked `session_message`s and child-session activity when
  Zed declares `clientCapabilities.subagents`, otherwise as one ordinary
  `tool_call` per subagent updated in place. A subagent's permission request
  names it, and cancelling its child session stops only that subagent. See
  [ACP subagents](../subagents.md#acp-bridge-clients);
- `_cowboy/session/status` snapshots and
  `_cowboy/session/status_changed` notifications. `turnRunning` is derived from
  the Hub's authoritative `Busy` lifecycle and the original `session/prompt`
  remains pending through the matching `TurnEnd`.

The bridge negotiates the lazy WebSocket bootstrap (`/ws?bootstrap=lazy`): a
connection carries the session index only, and one session's transcript tail
and config options are fetched from `/api/sessions/{id}/bootstrap` when it is
loaded or caught up. The size of other sessions' transcripts therefore never
decides whether a connection completes within the bootstrap deadline. An
unknown frame from a newer Controller is skipped instead of failing the
connection.

Prompt failures are attributed narrowly. The Controller's addressed
`command_result` fails exactly the prompt it names. Its session-wide `error`
frame is broadcast to every surface without naming a command, so it fails only
a prompt the Controller has not dispatched yet; a running turn ends through
its own turn end and is never failed by another surface's rejected command. A
session deleted in Cowboy fails its pending prompts with that reason and
leaves `session/list`.

On a fresh bridge, session listing and loading wait for the Controller's
WebSocket bootstrap before consulting the local session cache, using the same
bounded readiness wait as session creation. `initialize` still replies
immediately. A slow connection must not turn a retained session into an
"unknown session" error or an empty history list. If the Controller cannot
connect within the readiness deadline, the request returns a connection error;
it does not create a replacement session.

## Separate AI and Project Machines

For native Remote execution, select both enrolled Machines explicitly:

```bash
cowboy serve-acp --provider codex --runtime-machine ovh \
  --machine hawk --workspace columbus \
  --daemon-url https://cowboy.stormbird.xyz
```

The AI runs on `--runtime-machine`; `--machine` owns the registered Project
and its prepared worktree. The bridge uses the native execution-session API
and retains the server's placement, Provider, credential and executor checks.
If creation fails it reports the failure without creating a local session.
Session listing and loading require the same runtime, Project Machine and
workspace binding. Without `--runtime-machine`, the existing same-Machine
interface remains unchanged. This selects application placement, not a network
route; the host network owns ordinary DNS and sockets.

## Known TODOs

- `Cowboy · Provider` names with provider icons require published ACP Registry
  entries. Reusing an official provider ID preserves its icon but replaces the
  native agent, so it is not appropriate when both must coexist.
- Zed-provided MCP servers and `additionalDirectories` are logged but not yet
  forwarded into the daemon-owned provider session.
- Codex foreground turns and commands are authoritative today. Detached
  subagent/terminal aggregation stays unknown until `codex-acp` forwards
  `subAgentActivity` and `thread/backgroundTerminals/list`; the bridge reports
  `backgroundRunning: null` instead of a false idle.
- Stock Zed ignores the Cowboy status extension for its native spinner when a
  turn was started from Cowboy Web. It still receives the transcript updates;
  native out-of-band activity UI requires a small Zed-side integration.
- Zed Preview 1.11.2 (and Zed `main` as of 2026-07-11) keeps an ACP connection
  cached as connected after its stdio I/O task has failed. Cowboy keeps its
  bridge alive across daemon restarts and supports `session/close`, but it
  cannot repair a pipe that Zed itself has already closed; Zed must evict and
  respawn that cached connection.
