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
answers permission requests. See **[docs/architecture/](docs/architecture/)**
for the implementation architecture; [design.md](design.md) records the
original design direction.

Auth/token pairing remains deliberately out of process (the deployment VPN is
the boundary). Postgres persistence, restart resume, history pagination, queue /
draft sync, and the CodeMirror composer are implemented. Code-editor / file-tree
/ git views remain intentionally out of scope — cowboy is the agent panel only.

## Shape (one-paragraph summary)

A single Rust binary, run as a systemd daemon, that is itself the **ACP client**
(via the official `agent-client-protocol` crate — no Zed fork). It spawns each
agent over ACP/stdio, normalizes the stream into a provider-agnostic
event/command model, persists it in Postgres, and fans it out over
WebSocket to all connected web clients equally. The web UI (React/MUI/Vite,
embedded in the binary, omega's frontend recipe) is one responsive app:
incrementally paged transcript, and a CodeMirror 6 composer with vim support.

## Stack

- **Backend**: Rust — axum + tokio (WS/HTTP), `agent-client-protocol` (pinned),
  sqlx/Postgres, clap, rust-embed.
- **Frontend**: React 19, MUI 7, Vite 7,
  TypeScript (strictest); built by Deno, linted by oxlint.
- **Providers**: pluggable (trait + registry), all over ACP; Claude Code and
  Codex use maintained ACP adapters, while Gemini uses its native ACP mode.
- **Storage**: service-private Postgres (sessions, canonical events, settings, secrets).
- **Deploy**: systemd, `StateDirectory=cowboy` → `/var/lib/cowboy/`.

## Two subcommands, one source of truth (design §13a)

cowboy is **one** long-running daemon. `cowboy serve` on `:3333` owns the
`Hub`, supervisor, and Postgres write-behind — that's the source of truth. Every
other surface is a *client* of it:

| Subcommand | Transport | Run by | When to use |
| --- | --- | --- | --- |
| `cowboy serve` | HTTP + WebSocket on `:3333` | systemd (always) | the daemon — start once, leave it |

### Session sync semantics (v0)

| Action | Visible to other clients? |
| --- | --- |
| Web UI / phone creates a session | ✅ — daemon assigns id, broadcasts; every WS client sees it appear |
| Either client deletes a session | ✅ to WS clients (badge disappears) |

Each session in the UI carries a source chip (`Cowboy` / `External`) so you can
distinguish sessions opened in Cowboy's own UI from sessions opened by an ACP
client or another external caller. Per-session **delete** button is in the
sidebar.

## Quick start

```sh
just build               # deno + cargo (embeds web/dist)
./target/release/cowboy serve            # the daemon on :3333 (systemd in prod)

# Run cowboy's quality gates
just check               # fmt + clippy + tsc + cargo build
```
