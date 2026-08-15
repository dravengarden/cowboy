# cowboy

Drive coding-agent CLIs — **Claude Code, Codex, Gemini, Grok Build**, and more — from
anywhere (remote machine, phone browser, PC browser), with **all clients
sharing one live progress**. Agents run on a remote box under systemd; cowboy
owns their lifetime; humans drive them from thin web clients.

The core constraint: **don't constrain the agent CLI's functionality**. cowboy
is a conduit and control plane, not a reduced reimplementation.

## Status

**v1 — working multi-agent panel.** A Rust control plane coordinates detached
per-session Claude Code / Codex / Gemini / Grok Build workers over ACP, normalizes their streams (messages, thinking, tool calls,
plan, permissions), and fans them out over one WebSocket to all clients
equally; the separately deployed React/MUI web UI (responsive for
iPad/iPhone) lists sessions, shows a live transcript, sends prompts, and
answers permission requests. See **[docs/architecture/](docs/architecture/)**
for the implementation architecture; [design.md](design.md) records the
original design direction.

The normative contract for independently built Provider packages, typed
Provider UI, dynamic per-Machine installation, Service-scoped login, and
bounded uninstall cleanup is [Cowboy core requirements](docs/requirements.md).
The numbered architecture chapters describe the running implementation and
its remaining legacy-component compatibility boundaries.

End-user access pairing for the Cowboy application remains deliberately out of
process (the deployment VPN is the boundary); this is separate from the
Service-owned Provider authentication contract. PostgreSQL/SQLite persistence,
restart resume, history pagination, queue /
draft sync, and the CodeMirror composer are implemented. Code-editor / file-tree
/ git views remain intentionally out of scope — cowboy is the agent panel only.

## Shape (one-paragraph summary)

A Rust HTTP control plane plus a stable local broker and one detached systemd
worker per session. Workers are the **ACP clients** (via the official
`agent-client-protocol` crate — no Zed fork). They spawn each agent over
ACP/stdio, normalize the stream into a provider-agnostic
event/command model, persist it through a backend-neutral storage API, and fan it out over
WebSocket to all connected web clients equally. The web UI (React/MUI/Vite,
served from an independently switched immutable output) is one responsive app:
incrementally paged transcript, and a CodeMirror 6 composer with vim support.

## Stack

- **Backend**: Rust — axum + tokio (WS/HTTP), `agent-client-protocol` (pinned),
  sqlx/PostgreSQL/SQLite, clap, and a versioned local runtime protocol.
- **Frontend**: React 19, MUI 7, Vite 7,
  TypeScript (strictest); built by Deno, linted by oxlint.
- **Providers**: pluggable (trait + registry), all over ACP; Claude Code and
  Codex use maintained ACP adapters, while Gemini uses its native ACP mode.
- **Storage**: PostgreSQL or SQLite behind one stable API (sessions, canonical
  events, settings, machines, incidents, and provider usage). Hawk uses its
  service-private PostgreSQL database.
- **Deploy**: NixOS systemd units: system `cowboy`, user `cowboy-machine`, and
  transient per-session workers. See
  [zero-interruption rolling updates](docs/architecture/12-rolling-updates.md).

## Two subcommands, one source of truth (design §13a)

cowboy is **one** long-running daemon. `cowboy serve` on `:3333` owns the
`Hub`, supervisor, and database write-behind — that's the source of truth. Every
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
just build               # deno + cargo
./target/release/cowboy serve            # the daemon on :3333 (systemd in prod)
./target/release/cowboy serve \
  --database-url sqlite:///tmp/cowboy.sqlite3  # durable single-node setup

# Run cowboy's quality gates
just check               # fmt + clippy + tsc + cargo build
```
