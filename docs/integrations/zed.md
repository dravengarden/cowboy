# Zed ACP integration

Zed starts `cowboy serve-acp` as a stdio External Agent. The bridge connects to
the already-running cowboy daemon (`http://127.0.0.1:3333` by default); it does
not start another Hub or another copy of a provider session.

Override the three official Registry IDs with custom commands so a thread's Zed
agent identity stays fixed to its Cowboy provider while Zed keeps the official
provider display name and icon:

```json
{
  "agent_servers": {
    "codex-acp": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "codex"],
      "env": {}
    },
    "claude-acp": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "claude-code"],
      "env": {}
    },
    "gemini": {
      "type": "custom",
      "command": "cowboy",
      "args": ["serve-acp", "--provider", "gemini"],
      "env": {}
    }
  }
}
```

The Registry IDs are intentional. Current Zed resolves display metadata and
SVG icons by agent ID even when the executable is overridden with
`type: "custom"`. Using unrelated IDs such as `cowboy-codex` works, but Zed
renders its generic sparkle icon instead.

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

- Formal `Cowboy · Provider` names with provider icons would require published
  ACP Registry entries. The official-ID overrides above preserve the original
  provider names and icons without modifying Zed-owned Registry files.
- Zed-provided MCP servers and `additionalDirectories` are logged but not yet
  forwarded into the daemon-owned provider session.
- Codex foreground turns and commands are authoritative today. Detached
  subagent/terminal aggregation stays unknown until `codex-acp` forwards
  `subAgentActivity` and `thread/backgroundTerminals/list`; the bridge reports
  `backgroundRunning: null` instead of a false idle.
- Stock Zed ignores the Cowboy status extension for its native spinner when a
  turn was started from Cowboy Web. It still receives the transcript updates;
  native out-of-band activity UI requires a small Zed-side integration.
