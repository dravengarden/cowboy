<p align="center">
  <img src="web/public/favicon.svg" width="88" height="88" alt="Cowboy">
</p>

<h1 align="center">Cowboy</h1>

<p align="center">
  <strong>Drive Claude Code, Codex, Gemini, and Grok from your phone or desktop.</strong><br>
  One control plane. One live session. Close the tab — the agent keeps working.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-7c5cbf?style=flat-square" alt="MIT">
  <img src="https://img.shields.io/badge/rust-1.97-dea584?style=flat-square" alt="Rust 1.97">
  <img src="https://img.shields.io/badge/ACP-native-4a90d9?style=flat-square" alt="ACP">
</p>

## Product tour

<p align="center">
  <img src="docs/screenshots/desktop.webp" alt="Cowboy desktop: session rail, prompt, and a live conversation with tools, tables, and code" width="920">
</p>

<p align="center"><sub>Desktop · the full control plane, with a live transcript, tool states, tables, and code blocks in one view.</sub></p>

<div style="overflow-x:auto; padding: 4px 0 16px;">
  <table>
    <tr>
      <td align="center" valign="top">
        <img src="docs/screenshots/ios-agent.webp" alt="Mobile Agent view with a live transcript, tool calls, and composer" width="190"><br>
        <sub><b>Agent</b><br>tool calls + composer</sub>
      </td>
      <td align="center" valign="top">
        <img src="docs/screenshots/ios-code.webp" alt="Mobile Code review showing Rust syntax highlighting and an LSP symbol panel" width="190"><br>
        <sub><b>Code</b><br>syntax + LSP symbols</sub>
      </td>
      <td align="center" valign="top">
        <img src="docs/screenshots/ios-code-tree.webp" alt="Mobile Code review worktree file tree" width="190"><br>
        <sub><b>Worktree</b><br>files + Git context</sub>
      </td>
      <td align="center" valign="top">
        <img src="docs/screenshots/ios-sessions.webp" alt="Mobile session switcher with a live session list" width="190"><br>
        <sub><b>Sessions</b><br>switch agents anywhere</sub>
      </td>
    </tr>
  </table>
</div>

<p align="center"><sub>Light theme shown. On narrow screens, the feature strip stays one row and can be swiped horizontally; each view also reads well on its own.</sub></p>

## Why Cowboy exists

Coding agents are powerful CLIs. They are not a UI. Cowboy is the missing control plane:

- **Don't shrink the agent.** [ACP](https://agentclientprotocol.com) is the transport. Anything the protocol cannot model still rides through as an opaque update. Cowboy is a conduit, not a reduced reimplementation.
- **One shared timeline.** Phone, laptop, and ACP clients subscribe to the same Hub. Approving a permission on one device clears it everywhere.
- **Detached workers.** Each session is a Machine-hosted ACP worker. Close the browser. The turn continues.

Desktop and Mobile are separate products on the same protocol: keyboard-first density on a large screen, touch-first focus on a phone. They are not a squeezed layout of each other.

## What you get

| Surface | What it is |
| --- | --- |
| **Agent** | Live transcript, thinking, tools, plans, permissions, queue, and a CodeMirror composer (including Vim). |
| **Code** | Mobile-first review: worktree, Git changes, file view, syntax highlighting, LSP diagnostics, and symbol navigation. Long source lines remain horizontally scrollable, so a tree fetch never blocks a turn. |
| **Sessions** | Create, rename, reorder, pause, resume. Codex, Claude Code, Gemini, Grok — side by side. |
| **Machines** | Agents run on hosts you enroll. The controller is not the worker. |

Providers are independently versioned packages (`providers/*/provider.json`), not hardcoded adapters in the UI.

## Quick start

```sh
nix develop          # pinned Rust 1.97 + Deno + just
just build           # web bundle + release binaries
./target/release/cowboy serve \
  --database-url sqlite:///tmp/cowboy.sqlite3
```

Open `http://127.0.0.1:3333`. Point a Machine at the controller so sessions can actually spawn agents — see [machine operations](docs/machine-operations.md).

```sh
just check           # fmt, clippy, tsc, tests, release build
```

SQLite is the zero-ops local store. PostgreSQL speaks the same `Store` API (`--database-url postgresql://…`). Omit the URL and the daemon runs in-memory.

## How it is put together

```mermaid
flowchart LR
  phone[Phone PWA] --> ws[WebSocket]
  desktop[Desktop PWA] --> ws
  zed[Zed / ACP] --> hub
  ws --> hub[Hub · seq · fan-out]
  hub --> store[(SQLite / Postgres)]
  hub --> machine[cowboy-machine]
  machine --> worker[Detached ACP worker]
  worker --> agent[Claude / Codex / Gemini / Grok]
```

The **Hub** (`src/core.rs`) is the only writer of `seq`. Live clients receive compact deltas; durable history is canonicalized and large images become `/api/artifacts/…` before they clone across the persist queue and WebSocket fan-out.

Rolling updates keep workers alive across controller restarts: [architecture/12-rolling-updates.md](docs/architecture/12-rolling-updates.md).

## Documentation

| Start here | |
| --- | --- |
| [Architecture overview](docs/architecture/00-overview.md) | Topology, Hub, workers |
| [Requirements](docs/requirements.md) | Provider packages, auth, uninstall |
| [Code review](docs/architecture/13-code-review.md) | Worktree / Git data plane |
| [Multi-machine](docs/architecture/15-multi-machine.md) | Enrollment and placement |
| [Operations](docs/architecture/11-operations.md) | Runbooks |
| [Index](docs/INDEX.md) | Everything else |

## Status

Working multi-agent panel. Persistence, resume, history pagination, queue/draft sync, Code review, and Machine enrollment are in tree. The web UI is a separately switched immutable bundle, so a frontend-only rollout does not recycle agents.

## License

[MIT](LICENSE)
