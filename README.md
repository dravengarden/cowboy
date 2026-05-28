# cowboy

Drive coding-agent CLIs — **Claude Code, Codex, OpenCode**, and more — from
anywhere (remote machine, phone browser, PC browser), with **all clients
sharing one live progress**. Agents run on a remote box under systemd; cowboy
owns their lifetime; humans drive them from thin web clients.

The core constraint: **don't constrain the agent CLI's functionality**. cowboy
is a conduit and control plane, not a reduced reimplementation.

## Status

**v1 — working multi-agent panel.** A single Rust daemon spawns claude-code /
codex over ACP, normalizes their streams (messages, thinking, tool calls,
plan, permissions), and fans them out over one WebSocket to all clients
equally; the React/MUI web UI (embedded in the binary, responsive for
iPad/iPhone) lists sessions, shows a live transcript, sends prompts, and
answers permission requests. See **[design.md](design.md)** for the full
architecture (it is the source of truth).

Deferred from v1 (see design): auth/token pairing + QR (§9; v1 is no-auth,
LAN-only by choice), the vim/hint-mode keyboard layer (§8), SQLite persistence
+ restart `session/load` resume (§6/§7), and the code-editor / file-tree / git
views (intentionally out of scope — cowboy is the agent panel only).

## Shape (one-paragraph summary)

A single Rust binary, run as a systemd daemon, that is itself the **ACP client**
(via the official `agent-client-protocol` crate — no Zed fork). It spawns each
agent over ACP/stdio, normalizes the stream into a provider-agnostic
event/command model, persists it (SQLite, event-sourced), and fans it out over
WebSocket to all connected web clients equally. The web UI (React/MUI/Vite,
embedded in the binary, omega's frontend recipe) is one responsive app:
virtualized transcript via `@tanstack/react-virtual`, QR pairing for phones,
and vim (CodeMirror 6) in the composer at PC widths.

## Stack

- **Backend**: Rust — axum + tokio (WS/HTTP), `agent-client-protocol` (pinned),
  rusqlite (WAL), clap, rust-embed.
- **Frontend**: React 19, MUI 7, TanStack Router/Query/Virtual, Vite 7,
  TypeScript (strictest); built by Bun, linted by oxlint.
- **Providers**: pluggable (trait + registry), all over ACP; OpenCode first
  (native ACP), then Claude Code and Codex.
- **Storage**: SQLite (sessions + events) + `config.json` (intent) + secrets.
- **Deploy**: systemd, `StateDirectory=cowboy` → `/var/lib/cowboy/`.
