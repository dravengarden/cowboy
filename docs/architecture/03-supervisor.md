# Supervisor & lifetime

`src/supervisor.rs` is the Hub-facing lifetime API, decoupled from any client
connection. It has two backends: a local sender map for development/tests, and
the production `RemoteRuntime`, which routes through agentd to detached
per-session workers. The supervisor also holds the shared Hub, workspace root,
and monotonic id counter.

The guiding rule: **a client disconnecting never stops the agent.** Close the
phone, the agent keeps running. In production, restarting the HTTP daemon does
not restart a live agent.

## Methods

| Method | What it does |
|---|---|
| `new_session(provider, cwd, origin, system)` | create the session in the Hub, then `spawn_agent()` |
| `spawn_agent(session_id, spec, cwd, resume)` | local mode starts an OS thread; remote mode sends an idempotent `EnsureSession` |
| `send(session_id, cmd)` | local mode forwards to a channel; remote mode emits an idempotent runtime command |
| `ensure_alive(session_id)` | revive **without** sending a turn — warm the agent on open/reconnect |
| `revive(session_id)` | spawn a fresh agent for a restored session, passing its prior `agent_session_id` for `session/load` |
| `delete_session(session_id)` | best-effort `Cancel`, then drop the sender so the loop terminates |

Session ids are `sess-N`, with `N` seeded **past any restored session's max** so
a fresh daemon never collides on the storage primary key.

## Lazy revive

The supervisor never eagerly keeps dead agents around. Two entry points bring an
agent back:

```mermaid
flowchart TB
    SEND["send(id, cmd)"] --> CHK{"alive?"}
    CHK -->|yes| FWD["forward"]
    CHK -->|no| DROP["drop stale<br/>sender"]
    DROP --> REV["revive(id)"]
    OPEN["ensure_alive"] --> REV
    REV --> SP["spawn_agent<br/>(resume)"]
    SP --> FWD

    style REV fill:#dcfce7,stroke:#16a34a
    style DROP fill:#fde68a,stroke:#d97706
```

`ensure_alive` is what makes typing feel instant after a restart: opening a
session warms its agent before the user sends anything, so the first prompt
doesn't pay the spawn + handshake cost.

## Restart recovery

When systemd restarts `cowboy.service`, detached worker units remain alive.
Recovery rests on four facts:

1. **Transcripts are on disk** (Postgres) → history is never lost.
2. **Worker snapshots are authoritative** for whether a persisted Busy turn
   survived; restore does not falsely mark that turn Interrupted.
3. **Unacked runtime events replay** after the new controller takes its fenced
   lease, while command IDs prevent duplicate ACP submission.
4. If a worker really died, **resumable providers** re-attach via ACP
   `session/load(agent_session_id)`; non-resumable failures use the existing
   `Exited`/`Interrupted` recovery.

```mermaid
flowchart TB
    BOOT["daemon boots"] --> RESTORE["Hub.restore<br/>from Postgres"]
    RESTORE --> META["sessions + event logs +<br/>queue/drafts/settings"]
    META --> INT{"status =<br/>Interrupted?"}
    INT -->|"auto-resume on"| ENQ["enqueue continuation<br/>(__cont__ cmid)"]
    INT -->|off| IDLE["leave idle,<br/>revive on open"]
    ENQ --> REV["revive agent<br/>(session/load)"]

    style RESTORE fill:#eef2ff,stroke:#6366f1
    style ENQ fill:#dcfce7,stroke:#16a34a
```

Workers that survived are adopted at boot. Dead historical sessions are **not**
all revived — that would spawn dozens of subprocesses nobody is using. They are
revived lazily on the first `send`/`ensure_alive`, except interrupted sessions
opted into auto-resume, which are continued proactively.

Generation drain, broker restart, heartbeat isolation, and rollback are covered
in [Zero-interruption rolling updates](12-rolling-updates.md).

## System sessions

The session schema retains a `system` marker for machine-driven, view-only
sessions. They use the same supervisor machinery but can be hidden from normal
interactive surfaces and driven through the backend prompt endpoint. No
production subsystem currently creates one; the marker remains as a generic
control-plane primitive and for persisted-schema compatibility.
