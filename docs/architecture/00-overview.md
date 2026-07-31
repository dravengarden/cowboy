# cowboy — Architecture Overview

cowboy lets you drive coding-agent CLIs — **Claude Code, Codex, Gemini** — over
the **Agent Client Protocol (ACP)** from anywhere: a phone browser, a PC
browser, a Zed agent panel, or another machine. A Rust (axum) control plane
serves the JSON/WebSocket API, while a stable broker routes to **one detached
ACP worker per session** and a separately deployed React SPA serves the UI. It
is deployed through NixOS systemd units on hawk (`:3333`).

Operational growth and migration constraints are documented in
[`11-operations.md`](11-operations.md).

This document set is the **source of truth for the implementation as it stands**
— it describes the code, not the original design draft (`design.md`, which
predates the code and reads as a pre-flight plan).

## The one idea

cowboy is a **server-authoritative fan-out hub**. The daemon owns all state; every
client — phone, PC, Zed — is an equal subscriber to the same event stream. There
is no client-vs-client asymmetry, and **agent lifetime is decoupled from any
client connection**: close the phone, the agent keeps running under the
supervisor.

Three hard constraints shape everything:

1. **Don't constrain the agent CLI** more than the transport requires. cowboy is
   a conduit + control plane, not a reduced-feature reimplementation. Anything
   ACP can't model rides through as an opaque `Update` payload.
2. **One shared progress** across all clients, with a single global ordering.
3. **Agent lifetime is owned by a detached worker**, not by a client socket or
   the HTTP daemon.

## Topology

```mermaid
flowchart TB
    PH["phone"] --> WS
    PC["PC"] --> WS
    ZED["Zed (ACP)"] --> ACPF["serve-acp"]
    WS["WebSocket"] --> HUB
    ACPF --> HUB
    HUB["Hub<br/>seq · fan-out"] --> SUP["Remote Supervisor"]
    HUB --> PG[("Postgres")]
    SUP --> AD["agentd<br/>UDS broker"]
    AD --> AG["detached worker + agent<br/>per session"]

    style HUB fill:#eef2ff,stroke:#6366f1
    style SUP fill:#dcfce7,stroke:#16a34a
    style PG fill:#fef9c3,stroke:#ca8a04
```

The **Hub** is the single source of truth. Every surface (WebSocket clients, the
ACP server face) is a subscriber to the Hub's broadcast channel, so "new session
shows everywhere" and "an approval reflects everywhere" are just internal
broadcasts. In production the **Supervisor** talks to agentd. Per-session
worker units own ACP threads and subprocesses, survive daemon/broker restarts,
and replay unacked events when the control plane reconnects.

## Component map

| Layer | Module | Role |
|---|---|---|
| Entry / CLI | `src/main.rs`, `src/cli.rs` | clap dispatch: `serve`, `serve-acp`, `try-agent` |
| Core / bus | `src/core.rs` | `Hub`, `Event`/`Inbound`/`Outbound`, `seq`, fan-out |
| Transport | `src/acp.rs` | the **only** module touching `agent-client-protocol` |
| Lifetime | `src/supervisor.rs`, `src/agentd.rs`, `src/worker.rs` | route / detach / drain / resume / rollback |
| Runtime IPC | `src/runtime_wire.rs`, `src/remote_runtime.rs` | version negotiation, fencing, replay, idempotency |
| Providers | `src/provider/*` | launch specs + per-provider confirm rules |
| Server | `src/server.rs` | axum REST + WS + runtime static-file root |
| Storage | `src/store.rs`, `migrations/*` | Postgres write-behind, restore |
| Confirm | `src/skills/`, `src/inference/` | deterministic L1 + shared Codex Luna L2 |
| Files | `src/files.rs` | gitignore-aware `@` file picker |
| Frontend | `web/src/*` | independently built React SPA |

## End-to-end request flow

```mermaid
flowchart TB
    A["client sends Submit"] --> B["Hub: assign seq,<br/>enqueue or dispatch"]
    B --> C["Supervisor.send(Prompt)"]
    C --> D["agent thread →<br/>ACP prompt"]
    D --> E["agent streams<br/>session/update"]
    E --> F["Hub.push(Event)<br/>broadcast to all"]
    F --> G["clients render;<br/>writer persists"]
    E --> H["TurnEnd →<br/>confirm-detect judge"]
    H --> F

    style B fill:#eef2ff,stroke:#6366f1
    style F fill:#dcfce7,stroke:#16a34a
```

Read the chapters in order; each one zooms into a box above:

1. [Transport — ACP](01-acp-transport.md)
2. [Core — the Hub](02-core-hub.md)
3. [Supervisor & lifetime](03-supervisor.md)
4. [Providers](04-providers.md)
5. [Storage](05-storage.md)
6. [Server & wire API](06-server-api.md)
7. [Confirm-detect & inference](07-confirm-inference.md)
8. [Codex-owned memory boundary](08-memory.md)
9. [Frontend](09-frontend.md)
10. [Build & deploy](10-deploy-build.md)
11. [Operations](11-operations.md)
12. [Zero-interruption rolling updates](12-rolling-updates.md)
13. [Multi-machine runtime](15-multi-machine.md)

Zed setup and the ACP bridge's current compatibility boundary are documented in
[Zed ACP integration](../integrations/zed.md).
