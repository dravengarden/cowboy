# cowboy — design

> Status: design draft. No code yet. This document is the source of truth for
> the architecture; it consolidates the decisions taken during design and is
> meant to be read top-to-bottom before any scaffolding.

## 1. Goal

Let coding-agent CLIs — **Claude Code, Codex, OpenCode**, and more over time —
be **operated from anywhere** (a remote machine, a phone browser, a PC browser)
while **all clients share one live progress**. The agents run on a remote box;
the humans drive and watch them from thin web clients.

Hard constraints, in priority order:

1. **Do not constrain the agent CLI's functionality** more than the transport
   inherently requires. cowboy is a conduit and a control plane, not a
   reduced-feature reimplementation.
2. **One shared progress** across all connected clients (phone, PC, remote),
   with no asymmetry between them.
3. **Agent lifetime is owned by cowboy**, decoupled from any client connection:
   close the phone, the agent keeps running.

## 2. Transport decision — ACP

All agents are driven over the **Agent Client Protocol (ACP)**: JSON-RPC 2.0
over stdio between cowboy and each agent subprocess.

- cowboy depends on the official **`agent-client-protocol`** crate
  (Apache-2.0) — **the same crate Zed uses**. We do **not** copy or fork Zed's
  source. "Syncing Zed's protocol changes" == `cargo update`. The crate is
  pre-1.0, so we **pin a minor version** and **isolate all crate usage behind a
  thin `acp/` adapter module** so a version bump touches one place.
- cowboy implements the crate's **`Client`** trait and connects to each agent
  (the agent is the ACP *server*). This is the crate's primary supported use
  case — no "play both client and agent" proxy tricks.
- **Accepted limitation**: ACP is a lowest-common-denominator surface. Native
  CLI features not expressible in ACP are not exposed. We mitigate (not
  eliminate) this by carrying provider-specific payloads through an **`Opaque`
  event/command** channel and by **not over-normalizing** what ACP already
  gives us. Choosing ACP is choosing this tradeoff deliberately.

### Minimal client capabilities

During ACP `initialize`, `fs` (`readTextFile`/`writeTextFile`) and `terminal`
capabilities are **optional**; anything omitted is treated as UNSUPPORTED, and
agents that ship their own tools (Claude Code's Read/Write/Bash, etc.) then
operate **directly on disk themselves**.

cowboy **advertises minimal capabilities to start** (no `fs`, no `terminal`).
Consequence: the agent does its own file I/O and shell on the remote box;
cowboy stays a **session manager + permission broker + fan-out hub** and does
**not** have to build a PTY/terminal backend or fs sandbox in v1. This is the
single biggest scope-reducer in the design. (Capabilities can be turned on
later per provider if a future agent needs the client to mediate fs/terminal.)

## 3. Topology

```
  phone web ─┐
  PC web   ──┤  WebSocket (events + commands)   ┌───────── cowboy daemon (systemd, remote) ──────────┐
  remote   ──┘                                  │  normalized SessionEvent/Command bus + transcript   │
                                                │  Supervisor: owns agent process lifetime            │
                                                │  ┌─ Provider registry (pluggable) ────────────────┐ │
                                                │  │ claude-code │ codex │ opencode │ <future…>     │ │
                                                │  └──────┬──────────┬────────┬───────────────────┘ │
                                                │   ACP client side (stdio) to each agent subprocess  │
                                                └─────────────────────────────────────────────────────┘

  Reviewer (optional, no integration): Zed-remote / SSH / git diff on the same
  remote working directory. cowboy does not depend on any editor.
```

cowboy is the **single source of truth**. Every frontend is an equal subscriber
to cowboy's own WebSocket server, so "new session shows everywhere" and
"confirm reflects everywhere" are just internal broadcasts — none of the
client-vs-client asymmetry that a Zed-side proxy would suffer.

## 4. Provider abstraction (pluggable)

Providers are **case-by-case** (each agent's ACP on-ramp differs) but sit behind
one trait. Adding a provider = implement the trait + register; the core never
changes.

```rust
trait Provider {
    fn id(&self) -> &str;                    // "claude-code" / "codex" / "opencode"
    fn caps(&self) -> ProviderCaps;          // { resume: bool, ... }
    async fn start(&self, spec: SessionSpec) -> Result<Box<dyn AgentSession>, ProviderError>;
}

trait AgentSession {
    fn events(&self) -> EventStream;                 // normalized events + Opaque(native)
    async fn send(&self, cmd: SessionCommand) -> Result<(), SessionError>;
    async fn resume(&mut self, agent_session_id: &str) -> Result<(), SessionError>; // if caps.resume
    async fn shutdown(&self, mode: ShutdownMode);
}
```

- All implementations use the shared `acp/` client backend; a provider mostly
  declares **how to launch its ACP adapter binary**, its **capabilities**, and
  any **Opaque payload mapping**.
- **v1 registry is in-tree** behind a registry map. The trait is deliberately
  shaped so providers can later become **out-of-process plugins** (a provider
  spawned as its own subprocess, talking to cowboy over a local socket) — that
  way third parties add providers **without recompiling cowboy**. Not built in
  v1, but the interface must not preclude it.

### Provider on-ramps (current reality)

| Provider | ACP on-ramp | Resume |
|---|---|---|
| **OpenCode** | native built-in ACP server (`opencode acp`) — first class | yes (own session store) |
| **Claude Code** | via adapter (`claude-code-acp`) | yes (own session store) |
| **Codex** | via `zed-industries/codex-acp` (open-sourced, usable standalone) | TBD per adapter |

Start with **OpenCode** (cleanest, native ACP) to prove the spine, then add
Claude Code and Codex.

## 5. Normalized session model (over the WebSocket)

The wire contract to frontends is **provider-agnostic**, so heterogeneous agents
present one UI.

**Events (cowboy → clients):**
- `MessageDelta` — live streaming token chunks (**not persisted**, see §6).
- `Message` — a coalesced, final assistant/user message (**persisted**).
- `ToolCall{started,updated,done}` — tool invocations with status.
- `PermissionRequest` — agent is asking to proceed; carries `request_id`.
- `PermissionResolved` — a request was answered (so other clients clear it).
- `Lifecycle{starting,running,exited,crashed,resumed}` — process state.
- `Opaque{provider, json}` — provider-specific payload ACP/normalization can't
  model; UI renders it if it has a handler, else degrades gracefully.

**Commands (clients → cowboy):**
- `Prompt{session_id, text}` — send a turn.
- `PermissionResponse{session_id, request_id, decision}` — answer a request.
- `Cancel{session_id}` — cancel the current turn (ACP `session/cancel`).
- `NewSession{provider, workspace}` / `LoadSession{...}`.
- `ProviderSpecific{provider, json}` — escape hatch for native actions.

**Multi-device semantics:**
- cowboy is the sole writer → it assigns a monotonic `seq` per session, so
  ordering is global and unambiguous.
- **Permissions: first response wins.** cowboy forwards the first
  `PermissionResponse` to the agent and emits `PermissionResolved` to everyone
  else so their button disappears.
- A connecting client gets a **snapshot** (session list + recent events with the
  max `seq` as cursor), then a **live tail** from that cursor.

## 6. Storage

Split by access pattern; one mechanism cannot serve both.

| Data | Write rate | Read pattern | Store |
|---|---|---|---|
| Session **events** (transcript) | very high (streaming) | replay, paginate-older, restart recovery | **SQLite (WAL)** |
| Session **index** (id, provider, cwd, status, agent_session_id) | medium | list, sort, lookup | SQLite |
| **config / intent** (providers, launch cmds, server) | low (human-edited) | read at startup | `config.json` (atomic rewrite) |
| **devices / tokens** (pairing) | low | auth | `secrets` file, `0600`, hashed |

**Why SQLite, not the omega "no-DB / file-as-truth" rule.** omega is a small,
near-stateless config panel, so file-only fits. cowboy is an **event-sourced
stateful daemon** with growing event logs, mobile **pagination**, search, and
**restart recovery** — exactly SQLite's job. Embedded single-file SQLite in WAL
mode is "close to a file", not "run a database server": ACID, crash-safe, and
**concurrent reads while the single writer appends**. (Pure-file fallback if we
ever want it: per-session `events.jsonl` + `meta.json` — but then pagination and
search are hand-rolled.)

**Schema (event-sourced):**

```sql
sessions(
  id TEXT PRIMARY KEY,          -- cowboy session id
  provider TEXT,
  agent_session_id TEXT,        -- agent's own id, for ACP session/load resume  ★
  workspace TEXT,
  title TEXT, status TEXT,      -- running | exited | crashed | idle
  created_at, updated_at, last_seq INTEGER
);
events(
  session_id TEXT, seq INTEGER, -- monotonic per session
  ts, kind TEXT, payload JSON,
  PRIMARY KEY (session_id, seq)
);
pending_permissions(session_id, request_id, payload JSON, created_at);
devices(id, token_hash, name, paired_at, last_seen);
```

**Do not persist token deltas.** `MessageDelta` is broadcast live from memory
only; once a turn completes, persist **one coalesced `Message`** row (and tool
calls in their final state). Keeps the DB small and replay fast without hurting
the live experience.

**Rust:** `rusqlite` behind a **single dedicated writer task** (mirrors the
single-writer model), WAL for concurrent reader queries. (`sqlx` is the async
alternative; `rusqlite` + writer-actor is simpler and a better fit here.)

**On-disk layout (systemd `StateDirectory=cowboy`):**

```
/var/lib/cowboy/
  state.db        SQLite (sessions + events + devices), WAL
  config.json     provider/server intent (human-editable, atomic rewrite)
  secrets         tokens, 0600
```

CLI flags `--data-dir` / `--config` override for dev (omega convention).

## 7. Daemon & lifetime (systemd)

- **Lifetime decoupled from client connections.** A client disconnecting (phone
  sleeps) never stops the agent; the agent keeps running under the Supervisor.
- **Supervisor responsibilities:** spawn, health-check, crash-restart policy,
  idle reaping (idle timeout), explicit kill, per-session resource bounds (cwd,
  concurrency caps).
- **cowboy restart (systemd).** Agent subprocesses are cowboy's children and die
  with it. Recovery:
  1. transcripts are already on disk (SQLite) → history never lost;
  2. for providers with `caps.resume`, re-attach via ACP `session/load(
     agent_session_id)` and continue;
  3. otherwise mark `status = exited` and offer a one-click restart in the UI.
  `agent_session_id` (§6) is the bridge between storage and resume.

## 8. Frontend — one responsive web app

Reuse omega's frontend stack; **single web build**, embedded in the cowboy
binary (omega pattern, via `rust-embed`). "PC" and "phone" are the **same app at
different widths**, not separate builds — no desktop/Tauri app.

- **Stack:** React 19, MUI 7 + Emotion, TanStack Router, TanStack Query, Vite 7,
  TypeScript (strictest), built with Bun, linted with oxlint.
- **Realtime:** WebSocket client + a small store accumulating each session's
  timeline. TanStack Query (`useInfiniteQuery`) handles non-stream REST and
  **cursor-based history pagination** (`seq < cursor`, see §6).
- **Virtualized transcript:** **`@tanstack/react-virtual`** (same family as
  Query). Three must-handle cases for chat-style logs:
  1. **variable row heights** → `measureElement` dynamic measurement (no fixed
     `estimateSize`);
  2. **stick-to-bottom** during live streaming, releasing when the user scrolls
     up;
  3. **scroll anchoring on prepend** so loading older messages doesn't jump.
- **Pairing:** `qrcode.react` — PC shows a QR with a token; phone scans to join.
### vim — two layers (PC widths / physical keyboard only)

Mobile (touch) gets neither layer; this is the "capability layered by target"
theme again. The two layers are independent subsystems:

**(a) Composer text editing — CodeMirror 6.** The composer/editor is a
pluggable component. At PC width inject **CodeMirror 6 +
`@replit/codemirror-vim`** (modes, registers, `.`, macros, `:` commands); at
mobile width fall back to a plain `textarea`. This is *only* text editing inside
the input box.

**(b) App-wide keyboard navigation — a global "keyboard layer".** Drive the
whole UI without the mouse, in the spirit of **LazyVim + Vimium**. This is NOT
CodeMirror's job; it is a separate global subsystem, suppressed while focus is
in the composer's insert mode (focus/mode arbitration so typing a prompt never
triggers navigation).

- **Leader / which-key (LazyVim style):** `<Space>` opens a **which-key popup**
  — a grouped, labeled menu of available actions (e.g. `s` sessions, `a`
  approve, `g` go-to, `c` cancel turn), hierarchical and self-documenting. Fed
  by a central **keymap registry**; the popup is a custom MUI overlay.
- **Hint mode (Vimium style):** activate hint mode → every actionable item
  (permission Allow/Reject buttons, session list rows, tool-call expanders, nav)
  gets a small **yellow label** anchored at its **bottom-right**; type the
  label's letters to "click" it. Port Vimium's mechanism:
  1. collect **visible, non-occluded** hintable elements;
  2. generate labels from a home-row **hint alphabet**, minimal-length unique
     strings (more items → longer labels);
  3. render an overlay (React portal) of yellow badges over each target's
     bounding rect;
  4. capture keys: **filter as you type**, dim non-matching, trigger on unique
     match; `Esc` cancels.
  Elements opt in via a `data-hint` attribute / `<Hintable>` wrapper (plus a
  role/selector fallback scan), so the engine stays decoupled from components.

**Implementation notes:** use **`tinykeys`** (tiny, good at leader/chord
sequences) for the keymap manager; the which-key popup and the hint engine are
both custom (no off-the-shelf React lib matches Vimium well — port the
label-generation + overlay-filter loop). The keymap registry is the single
source of truth shared by which-key, hint mode, and direct bindings.

## 9. Security

cowboy is a remote control plane that **runs arbitrary agent code and lets it
modify files / run commands**, exposed to phones. Non-negotiable:

- **auth + wss**; token-based device pairing, **store only token hashes**.
- **workspace-root scoping** for every session's cwd.
- **audit log**: who sent which prompt, who approved which permission, what ran.
- default **bind localhost**; reach it over **Tailscale / reverse proxy / dev
  tunnel** (also resolves browser mixed-content for `wss://`).
- per-session **resource bounds** and concurrency limits.

## 10. Repo & build (columbus external project)

cowboy is an **`external`** project — it has its own GitHub repo
(`git@github.com:dravengarden/cowboy.git`) and is the single source of truth for
its own code. Under columbus it lives as a **bare clone with per-branch
worktrees** at `projects/cowboy/.bare/`; columbus's root `.gitignore` already
ignores `/projects/*`, so no whitelist entry is needed. To register it:

- add to `project-defs/projects.cue`:
  ```cue
  cowboy: {
      kind:           "external"
      repo:           "git@github.com:dravengarden/cowboy.git"
      default_branch: "main"
  }
  ```
- this file, `README.md`, and all code are committed and pushed to **cowboy's
  own remote** (not columbus) — per the AGENTS.md external-project rule.
- the repo carries its **own** `.gitignore` for build artifacts
  (`/target`, `web/node_modules`, embedded `dist/`, `/result*`).

```
projects/cowboy/
  Cargo.toml
  flake.nix              single binary embedding the built web UI (rust-embed)
  justfile               build + codegen + quality gates (omega style)
  design.md              (this file)
  README.md
  deploy/cowboy.service  systemd unit (StateDirectory=cowboy)
  src/
    main.rs              clap CLI/daemon: cowboy serve --bind --workspace-root --config
    acp/                 thin adapter — the ONLY module touching agent-client-protocol
    core/                SessionEvent/Command model, bus, seq allocation
    store/               rusqlite writer task + queries; config.json; secrets
    supervisor/          process lifetime, restart policy, idle reap, resume
    provider/
      mod.rs             Provider/AgentSession traits + registry
      opencode.rs        first (native ACP)
      claude.rs          claude-code-acp
      codex.rs           codex-acp
    server/              axum: WS (events+commands) + REST + embedded UI + auth
  web/                   React/MUI/Vite (omega recipe) → built dist embedded
```

**Backend crates:** `agent-client-protocol` (pinned), `tokio`, `axum` +
`tokio-tungstenite`, `clap`, `rusqlite` (bundled), `rust-embed`, `serde` /
`serde_json`. Backend language is Rust (diverging from omega's Go); only the
**frontend recipe** and the "single binary embeds UI / file-as-config / no
service DB" *habits* are inherited.

**Build cache — sccache.** All Rust builds use **sccache** as the compiler
cache, wired in the **flake `devShell`** (dev-time `cargo build` iteration), not
in the hermetic `nix build` (which is sandboxed and uses Nix's own
crane/cargoHash caching — sccache there adds nothing and hurts hermeticity).
In the devShell:

- add `sccache` to `nativeBuildInputs`;
- set `RUSTC_WRAPPER = "sccache"`;
- set `CARGO_INCREMENTAL = "0"` — sccache and cargo incremental compilation
  conflict; disabling incremental maximizes cache hits (sccache's documented
  usage).

Cache backend starts as **local disk** (`~/.cache/sccache`); a shared backend
(S3 / redis) can be swapped in later for cross-machine/CI reuse without changing
the wrapper config.

## 11. Build order (first vertical slice)

1. Register the subdir project; scaffold Cargo + flake + justfile + empty axum
   server serving an embedded placeholder UI.
2. `acp/` + `provider/opencode.rs`: spawn `opencode acp`, `initialize` with
   minimal caps, `session/new`, relay one `Prompt`, receive `session/update`.
3. `core/` + `store/`: normalize events, assign `seq`, persist coalesced
   messages to SQLite; snapshot + tail over WS.
4. Web: connect WS, render a virtualized transcript, send a prompt, answer a
   `PermissionRequest` — verify **two browsers see one shared session**.
5. Supervisor + restart/resume; then add Claude Code and Codex providers.

## Validation log

- **2026-05-27 — claude-code end-to-end verified.** `cowboy try-agent
  --provider claude-code` against `@agentclientprotocol/claude-agent-acp`
  (renamed from `@zed-industries/claude-code-acp`) over `npx -y`: spawn →
  initialize → new_session → prompt → streamed `agent_message_chunk` →
  `EndTurn`. Works.
- **Observed protocol drift (concretizes §12 risk 2).** The adapter emits a
  `sessionUpdate: "usage_update"` (token usage + USD cost) that
  `agent-client-protocol` 0.4.7 does **not** model; the crate **errors and
  drops** the notification rather than passing it through. Consequences:
  (a) the turn is unaffected; (b) useful usage/cost telemetry is lost; (c) the
  crate trails the adapters, and it is *strict* (rejects unknown
  `sessionUpdate` variants) rather than lenient. Action items: prefer a crate
  version that models `usage_update` when available, and/or add a lenient
  sidecar parse that captures unknown notification variants as `Opaque` events
  (§5) so cost/usage reaches the UI. codex provider not yet verified
  (`codex-acp` binary not installed).

## 12. Open risks

1. **ACP is a subset** — native CLI features outside ACP aren't exposed
   (accepted tradeoff of choosing ACP; `Opaque` softens it).
2. **`agent-client-protocol` is pre-1.0** — breaking changes; mitigated by
   pinning + the `acp/` isolation module.
3. **Per-provider resume capability is uneven** — restart recovery quality
   varies by agent.
4. **Remote exposure** is the largest attack surface; §9 is mandatory, not
   optional.

## 13. Zed remote-development coexistence

cowboy is **not an editor** (§3). For code review / direct editing of the
files an agent is touching, the chosen integration with Zed's SSH
remote-development mode is **coexistence (option a): cowboy does nothing
Zed-protocol-specific**. A local Zed app connects to the same box over SSH as a
normal remote project; Zed manages its own `~/.zed_server/zed-remote-server-
<channel>-<version>` binary, version matching, and server lifecycle entirely.

Why not reimplement Zed's remote server (so a local Zed connects to cowboy)?
Researched and rejected: the SSH remote protocol is the same internal,
**unversioned** ~600-message `zed.proto` Envelope shared with Zed
collaboration (`crates/proto/proto/buf.yaml` self-declares "internal to Zed
only"), with **no protocol negotiation** — the client computes one *exact*
`zed-remote-server-<channel>-<version>` filename from its own build and refuses
anything else. A third-party server side would be a perpetual reverse-
engineering treadmill that breaks on every Zed release. Not a compatibility
guarantee.

What coexistence requires of cowboy — only deployment alignment, no code:

- **Run cowboy as the human SSH user** (e.g. `draven`), not a locked-down
  system user. Then the agent and the Zed-over-SSH session share one identity
  and one view of the files: Zed's filesystem watcher sees the agent's edits
  live (exactly the review story), and the agent can read the user's
  credentials (`~/.claude`, `~/.codex`).
- **Put `--workspace-root` under the SSH user's home** so both cowboy's agents
  and a Zed connection open the same paths.

For restricted-internet / air-gapped remotes, do **not** have cowboy stage the
Zed server binary (option b): Zed's built-in `upload_binary_over_ssh: true`
already solves first-connect download better, because the *local* app inherently
knows its own exact version while a remote daemon can only guess it.
