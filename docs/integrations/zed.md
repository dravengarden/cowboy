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
      "args": ["serve-acp", "--provider", "codex"],
      "env": {
        "COWBOY_USER_TOKEN": "cow_…"
      }
    },
    "cowboy-claude": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "claude-code"],
      "env": {
        "COWBOY_USER_TOKEN": "cow_…"
      }
    },
    "cowboy-gemini": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "gemini"],
      "env": {
        "COWBOY_USER_TOKEN": "cow_…"
      }
    },
    "cowboy-grok": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "grok"],
      "env": {
        "COWBOY_USER_TOKEN": "cow_…"
      }
    }
  }
}
```

`serve-acp` requires a product personal access token. Create one on `/` →
account menu → tokens (or `POST /api/auth/tokens`), then set
`COWBOY_USER_TOKEN` or pass `--token`. The bridge sends
`Authorization: Bearer cow_…` on `POST /api/sessions` and `/ws`. There is
no headerless or loopback ACP path. An unset token fails at start; a
401/403 from the daemon exits the process instead of reconnecting.

Do not override Registry agent IDs (including `codex-acp`, `claude-acp`, and
`gemini`) unless replacing a native agent is intentional. Zed's custom-agent settings have no
icon field, so the independent Cowboy entries use its generic sparkle icon.
Distinct provider icons require separate published ACP Registry entries.

When Zed is attached to a remote project, the command must resolve on that
remote host. Set `COWBOY_DAEMON_URL` or add `--daemon-url` when the daemon is not
on the default loopback address.

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
