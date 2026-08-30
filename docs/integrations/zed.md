# Zed ACP integration

Zed starts `cowboy serve-acp` as a stdio External Agent. The bridge connects to
the already-running cowboy daemon (`http://127.0.0.1:3333` by default); it does
not start another Hub or another copy of a provider session.

Add four custom agents alongside Zed's official Registry agents so native
Codex/Claude/Gemini/Grok remain available and each Cowboy thread stays fixed to its
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
    "cowboy-gemini": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "gemini"]
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
uses 10-minute sender-constrained access tokens, and rotates its 30-day refresh
token on every refresh. The Service stores only the public key and refresh-token
hash. Account → Authorized devices lists and revokes these clients.

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
  bridge stops publishing updates to dead thread entities;
- complete retained-history replay on `session/load`;
- prompt streaming, permission requests, cancellation, and session config
  options. Either `session/cancel` or JSON-RPC request cancellation stops an
  in-flight bridge prompt;
- daemon WebSocket recovery without terminating Zed's stdio ACP process:
  disconnects publish `reconnecting`, commands wait in the bridge, and a fresh
  bootstrap restores attached sessions, statuses, and config options. In-flight
  prompts remain pending through the outage and settle normally after the
  revived daemon reports the session idle;
- provider-filtered import/load, preventing a thread from acquiring the wrong
  provider identity;
- `_cowboy/session/status` snapshots and
  `_cowboy/session/status_changed` notifications. `turnRunning` is derived from
  the Hub's authoritative `Busy` lifecycle and the original `session/prompt`
  remains pending through the matching `TurnEnd`.

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
