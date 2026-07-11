# Server & wire API

`src/server.rs` stands up a single **axum** server that does three jobs at once:
serve the embedded SPA, expose the REST endpoints, and host the WebSocket that
carries the real-time event/command stream. One process, one port (`:3333`) →
the same binary is both frontend and backend.

## REST endpoints

| Route | Purpose |
|---|---|
| `GET /healthz` | readiness → `"ok"`, or 503 after persistence loss / exhausted retries |
| `GET /version` | `{ version }` = SHA-256 of `index.html`, for build-id / stale-bundle detection |
| `GET /api/metrics` | storage/session/RSS plus persistence pending, dropped, and failed-batch counters |
| `GET /api/workspaces` | selectable session roots — Columbus projects + `workspace_root` subdirs |
| `GET /api/sessions/{id}/files?q&limit` | the composer `@` picker (gitignore-aware fuzzy search) |
| `GET /api/sessions/{id}/info` | metadata + event / queue / draft counts |
| `POST /api/sessions` | create a session, returns the cowboy `session_id` |
| `POST /api/sessions/{id}/prompt` | machine-driven session wake |
| `GET /api/history/{id}/{page}` | fixed-size, seq-aligned history page (`immutable` once the next page exists) |
| `ANY /ws` | WebSocket upgrade |
| `*` (fallback) | the embedded SPA (`index.html` for client routes) |

The SPA is embedded at compile time via `rust-embed` (`#[folder = "web/dist"]`),
so a release binary is fully self-contained — no static-file directory to deploy.

## The `/api/workspaces` picker

The New Session picker lets you open a session in **any Columbus project's
worktree**. The chosen `cwd` becomes the agent's working directory, so Codex
loads that project's guidance files and trusted project configuration.

## History pagination

The WebSocket `Snapshot` only carries the last ~200 events. Older history is
pulled lazily over REST: `GET /api/history/{id}/{page}` returns a fixed-size,
seq-aligned page, marked `immutable` once a later page exists (so the client can
cache it forever). The frontend requests older pages as the user scrolls up. It
uses canonical message/tool rows plus CSS off-screen containment; retaining the
browser's `column-reverse` anchor avoids iOS jumps from unmounting
variable-height rows in a JavaScript virtualizer.

## The WebSocket loop

```mermaid
flowchart TB
    CONN["client connects /ws"] --> HELLO["Hub sends Sessions +<br/>Snapshot + Settings +<br/>ConfigOptions + Skills"]
    HELLO --> TAIL["live tail<br/>(broadcast subscribe)"]
    CIN["client → Inbound cmd"] --> HUB["Hub handles,<br/>stamps seq"]
    HUB --> TAIL
    TAIL --> COUT["Outbound → client"]

    style HELLO fill:#eef2ff,stroke:#6366f1
    style HUB fill:#dcfce7,stroke:#16a34a
```

On connect the Hub pushes the full bootstrap (`Sessions`, per-session
`Snapshot`, `Settings`, `ConfigOptions`, `Skills`). After that it's symmetric:
the client sends `Inbound` commands, the Hub fans `Outbound` messages back. The
wire types are mirrored in `web/src/protocol.ts`. `protocol_contract` parses the
Rust enums and fails tests when their serialized discriminants differ from the
TypeScript unions; representative field shapes remain covered by Rust serde and
TypeScript checks. See [Core — the Hub](02-core-hub.md) for the full catalogs.

## Background tasks

`serve` spins up, alongside the HTTP server:

- **store writer** — drains `StoreWrite` intents ([Storage](05-storage.md))
- **purge sweeper** — hard-deletes soft-deleted sessions past 3 days (every 6h)
- **dispatcher** — drains the Hub→supervisor hand-off and forwards prompts

## Auth

There is **no auth in v0** — a deliberate LAN-only choice. The deployed service
binds localhost and is reached over Tailscale / a reverse proxy (which also
resolves browser mixed-content for `wss://`). Token-based device pairing and
`wss` are a v1 follow-up; the design treats remote exposure as the largest
attack surface, so this is a known gap, not an oversight.
