# Zed ACP integration

Zed starts `cowboy serve-acp` as a stdio External Agent. The bridge connects to
the already-running cowboy daemon (`http://127.0.0.1:3333` by default); it does
not start another Hub or another copy of a provider session.

Add three custom agents so a thread's Zed agent identity stays fixed to its
Cowboy provider:

```json
{
  "agent_servers": {
    "cowboy-codex": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "codex"],
      "env": {}
    },
    "cowboy-claude": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "claude-code"],
      "env": {}
    },
    "cowboy-gemini": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "gemini"],
      "env": {}
    }
  }
}
```

When Zed is attached to a remote project, the command must resolve on that
remote host. Set `COWBOY_DAEMON_URL` or add `--daemon-url` when the daemon is not
on the default loopback address.

The bridge supports:

- `initialize`, `session/new`, `session/list`, `session/load`, and
  `session/delete`;
- complete retained-history replay on `session/load`;
- prompt streaming, permission requests, cancellation, and session config
  options;
- provider-filtered import/load, preventing a thread from acquiring the wrong
  provider identity;
- `_cowboy/session/status` snapshots and
  `_cowboy/session/status_changed` notifications. `turnRunning` is derived from
  the Hub's authoritative `Busy` lifecycle and the original `session/prompt`
  remains pending through the matching `TurnEnd`.

## Known TODOs

- Custom `settings.json` agents have no icon field. Publish three ACP Registry
  entries (or a small Zed extension) with the Codex, Claude, and Gemini SVGs.
  One dynamic Cowboy agent cannot change icons per session in stock Zed.
- Zed-provided MCP servers and `additionalDirectories` are logged but not yet
  forwarded into the daemon-owned provider session.
- Codex foreground turns and commands are authoritative today. Detached
  subagent/terminal aggregation stays unknown until `codex-acp` forwards
  `subAgentActivity` and `thread/backgroundTerminals/list`; the bridge reports
  `backgroundRunning: null` instead of a false idle.
- Stock Zed ignores the Cowboy status extension for its native spinner when a
  turn was started from Cowboy Web. It still receives the transcript updates;
  native out-of-band activity UI requires a small Zed-side integration.
