# Antigravity User MCP Servers (R2) — Design

Status: **Proposed (design review pending)**

This document specifies **R2** from
[`docs/antigravity-parity-round-2.md`](antigravity-parity-round-2.md)
(user-configured stdio MCP servers from `~/.claude.json` for Antigravity
sessions), resolving all four High and two Medium findings from design review
`call-8dc781e16a7ca99702fb61b0`.

---

## 1. Scope and Non-Goals

- **In scope**: Owner-authored **stdio** MCP servers from the target home's
  `~/.claude.json` (`mcpServers` user scope + `projects[projectKey].mcpServers`
  local scope, minus `disabledMcpServers`), matching Claude's remote runtime
  precedence (`plugins/claude-code/runtime/mcp.mjs:118-138`) after owner
  admission into a frozen launch plan.
- **Explicitly excluded**:
  - Repository `.mcp.json` servers (deferred to **D1**; never read).
  - HTTP / SSE / streamable-HTTP user MCP servers (stdio only).
  - Automatic execution of unreviewed edits to `~/.claude.json` on session open.
  - Managed read-only calls (`binding.managed.is_some()`): user MCP servers are
    never loaded for managed calls.
- **Preserved invariants**:
  - `agy`'s PreToolUse hook keeps a single closed namespace (`mcp_cowboy_*`);
    user MCP tools are exposed through the existing `cowboy` facade and worker
    extension broker (`src/worker_execution/extension_broker.rs`,
    `components/provider-runtime/packages/antigravity-acp/extensions.mjs`).
  - No user-server credentials, gateway context, or `COWBOY_CALL_CONTEXT` ever
    enter the account sandbox unit or any user MCP server process.
  - Required folder bindings (e.g., `suger`) strictly retain startup, tool, and
    transport priority over optional user MCP servers.
  - No wire-breaking commit lands on `origin/main` ahead of an owner-approved
    `inputs.cowboy-workers` pin advance.

---

## 2. Threat Model and Frozen Owner-Admitted Launch Plan (High 1)

### 2.1 Why live `~/.claude.json` reading on session open is unsafe

1. **Branch-controlled repository execution**: Launching a server pre-turn with
   the worktree as `cwd` allows entries like `node ./server.js` or repo-relative
   args/env to run untrusted branch files before the owner sends a prompt.
2. **Cross-session privilege escalation via shared UID**: Target `RunCommand`
   runs under the target user UID (e.g. `draven` on Hawk), so a previous
   session's shell command can rewrite `~/.claude.json` or overwrite a mutable
   helper script. Reading live `~/.claude.json` at session open would silently
   execute attacker-injected commands.

### 2.2 Frozen owner-admitted launch plan

Execution NEVER auto-discovers live `~/.claude.json` entries on session open.
Instead, admission and execution are strictly separated across trust boundaries:

1. **Owner admission (Controller-stored, separate UID from target `RunCommand`)**:
   - An explicit owner-authenticated action (Cowboy UI / owner CLI invoking a
     Controller endpoint — never callable from an agent session's `RunCommand`
     or `COWBOY_CALL_CONTEXT` grant) inspects candidate stdio servers via a
     target-side **credential-isolated resolver** (Section 3) that parses
     `~/.claude.json` (`user` + `projects[projectKey]` minus `disabledMcpServers`,
     excluding `.mcp.json` and non-stdio entries).
   - For each admitted server, the Controller freezes and persists an immutable
     `AdmittedUserMcpPlan` scoped to `(machine_id, project_key)` in its own
     separate-UID SQLite store:
     - `schema: 1`, `plan_digest` (SHA-256 over canonical JSON).
     - Ordered `servers[]` (max `MAX_USER_MCP_SERVERS = 16`):
       - `name` (`^[A-Za-z0-9_-]{1,64}$`, never `"cowboy"` or colliding with
         required folder binding names; collisions drop the user server).
       - `executable`: canonical absolute path outside workspace, worktree, and
         git repository root, regular file, owner- or root-owned, non-group/world
         writable.
       - `executable_sha256`: SHA-256 of the executable file at admission time
         (mandatory for all executables outside read-only `/nix/store/`; also
         verified for script arguments when the executable is an interpreter such
         as `node`, `python3`, `bun`, `sh`, `bash`, `env` — see 2.3).
       - `args`: exact admitted argument list after `${VAR}` / `${VAR:-default}`
         expansion against a sanitized owner environment (with all `COWBOY_*`
         variables stripped before expansion).
       - `env`: exact admitted `BTreeMap<String, String>` (keys matching
         `COWBOY_*` or loader/injection variables `LD_PRELOAD`,
         `LD_LIBRARY_PATH`, `LD_AUDIT`, `NODE_OPTIONS`, `NODE_PATH`,
         `PYTHONPATH`, `PYTHONSTARTUP`, `PERL5OPT`, `RUBYOPT`, `BASH_ENV`,
         `ENV`, `ZDOTDIR`, `IFS` are rejected at admission and stripped at launch).
2. **Pre-turn worktree confinement**:
   - Every user MCP server launches with `cwd` set to a keeper-created, empty,
     mode-`0500` neutral directory (`<keeper_state_dir>/user-mcp-cwd`), NEVER the
     mutable workspace or worktree.
   - At admission AND again at target launch, the keeper rejects:
     - Any relative `command` containing `/` (e.g., `./server`, `../bin/mcp`).
     - Any bare `command` resolved via sanitized `PATH` (no `.`, no empty entry,
       no workspace/repo dir) whose `realpath` lies inside the workspace,
       worktree, or git repository root.
     - Any argument starting with `./`, `../`, or `~/` or whose resolved path
       lies inside the workspace, worktree, or git repository root.
3. **Interpreter & mutable script integrity pin (2.3)**:
   - If `executable` is a general interpreter (`node`, `bun`, `deno`, `python*`,
     `ruby`, `perl`, `sh`, `bash`, `zsh`, `fish`, `env`, `npx`, `uvx`), admission
     MUST reject inline code flags (`-e`, `-c`, `--eval`) targeting unpinned
     paths or mutable worktree inputs, and MUST pin the SHA-256 of any local
     script file argument outside `/nix/store/` (`pinned_files: Vec<{ path, sha256 }>`).
   - Before launch on the target, `cowboy-execution-host` re-verifies
     `plan_digest`, re-checks canonical paths, ownership, modes, non-worktree
     confinement, and every `executable_sha256` / `pinned_files` SHA-256 by
     opening each file with `O_NOFOLLOW` + `fstat` + hashing the open fd (and
     passing `/proc/self/fd/<fd>` or refusing on `ENOSYS`/drift). Any drift
     refuses that server cleanly with a bounded notice asking the owner to
     re-admit.
4. **Launch declaration & adoption**:
   - The Controller carries the frozen `user_mcp_plan` alongside `mcp_plan` in
     the session's signed launch declaration (`StartSession` / `LaunchContract`,
     protected by `mcp_launch.rs` adoption checks so reload/adopt never drops or
     silently swaps it).
   - Unadmitted changes in `~/.claude.json` NEVER execute automatically; a
     read-only drift check (running inside the isolated resolver) may surface a
     passive notice in the UI/prompt (`~/.claude.json changed since admission`).

---

## 3. Target-Side OS Credential & Socket Isolation (High 2)

### 3.1 Why omitting `COWBOY_CALL_CONTEXT` is insufficient

`cowboy-execution-host` runs under the target user UID (`components/machine/src/execution_host.rs`).
On Linux targets:
- Managed-call context files live at `<state_dir>/calls/sessions/<session_id>.json`
  (`components/store/src/managed_calls/host.rs:150`).
- Managed-call Unix domain sockets live at
  `<canonical_temp_dir>/cowboy-calls-<uid>/<instance>/...`
  (`components/store/src/managed_calls/host.rs:132`; pathname sockets only, no
  TCP listener).
- Keeper private files live at `<keeper_state_dir>/{keeper.sock,contract.json,executor-home,ledger.json}`.

Any ordinary child process under the same UID can open those files or connect to
that Unix socket even when `COWBOY_CALL_CONTEXT` is omitted from its env.

### 3.2 Mandatory Linux kernel boundary for user MCP servers and their resolver

User MCP servers and the `~/.claude.json` resolver are NEVER started through
`wire::with_call_context(...)`. Instead, `cowboy-execution-host` launches them
through a dedicated **isolated-launch path** using pinned `bwrap` (already
shipped and used by `components/codeact-runtime` and `components/server-agent-api`,
and present on Hawk at `/run/current-system/sw/bin/bwrap`):

1. **Namespaces & privilege drop**:
   - `--unshare-user --unshare-pid --unshare-ipc --die-with-parent --new-session`
   - Automatic `PR_SET_NO_NEW_PRIVS` (blocks setuid escalation).
   - Dedicated `--proc /proc` inside the new PID namespace: the user server
     cannot see `cowboy-execution-host`, `cowboy-machine`, or peer sessions in
     `/proc` (`/proc/<keeper_pid>/environ` and `/proc/<keeper_pid>/fd` return
     `ENOENT`), and cannot `ptrace` them.
   - Network namespace stays shared (`--share-net`) so legitimate external MCP
     clients (GitHub, Linear, databases) can reach outbound endpoints; Cowboy's
     call gateway listens ONLY on a pathname Unix domain socket (never TCP).
2. **Filesystem & Unix-socket masking (logical AND canonical paths)**:
   - Mount host filesystem (`--bind / /`, `--dev /dev`, `--proc /proc`), set
     `--chdir <keeper_state_dir>/user-mcp-cwd`, and overlay empty mode-`0000`
     `tmpfs` mounts over BOTH the logical path AND the `realpath` (handling
     symlinks such as Hawk's `~/.cache/cowboy -> /srv/storage/fast0/...`) of:
     - `COWBOY_CALL_CONTEXT` parent `calls/` tree and the entire Machine state
       directory (`$XDG_STATE_HOME/cowboy-machine`, `~/.local/state/cowboy-machine`).
     - `<canonical_temp_dir>/cowboy-calls-<uid>` AND `$TMPDIR/cowboy-calls-<uid>`
       (if distinct) AND `/tmp/cowboy-calls-<uid>`.
     - `<keeper_state_dir>` (hiding `keeper.sock`, `contract.json`, `ledger.json`,
       `executor-home`, while keeping only the empty `user-mcp-cwd` bind visible
       if needed, or mounting a fresh `--tmpfs /run/cowboy-user-mcp-cwd` as
       `--chdir /run/cowboy-user-mcp-cwd` so the entire `<keeper_state_dir>` is
       100% masked!). Using `--tmpfs /run/cowboy-user-mcp-cwd --chmod 0500 --chdir /run/cowboy-user-mcp-cwd`
       ensures zero bytes of `<keeper_state_dir>` are visible!
     - `/run/user/<uid>/cowboy*` (if present).
     - The session's mutable workspace/worktree path (pre-turn startup masking
       for the resolver; for running user MCP servers that legitimately inspect
       files outside Cowboy state, workspace access is read/write only if
       explicitly needed, while `cwd` remains `/run/cowboy-user-mcp-cwd` and
       pre-turn executable/script validation strictly forbids worktree paths).
       For the `~/.claude.json` resolver specifically, mount the host read-only
       (`--ro-bind / /`) with all Cowboy state/socket dirs masked and zero write
       access anywhere.
3. **Environment sanitization**:
   - `--clearenv` followed ONLY by the sanitized non-secret base target
     environment (excluding `COWBOY_CALL_CONTEXT`, `COWBOY_EXECUTION_FILE_HELPER`,
     and every `COWBOY_*` variable) plus the server's admitted `env` entries.
4. **Fail closed when OS isolation is unavailable**:
   - If `bwrap` or unprivileged user+mount+pid namespaces are unavailable or
     fail a startup self-probe on the target Machine, user MCP servers and the
     resolver are **refused** (NEVER launched unsandboxed!). Required folder
     bindings and Cowboy MCP remain unaffected, and an availability note informs
     the owner.
5. **Adversarial acceptance test**:
   - Automated integration test spawns a real `CallHost` (live context JSON +
     live Unix socket) and launches an adversarial stdio MCP server inside the
     target boundary, handing it the exact `call_context` path, `CallHost` socket
     path, and keeper PID as tool arguments.
   - Asserts: (a) `COWBOY_CALL_CONTEXT` absent from env; (b) opening logical or
     canonical `call_context` fails (`ENOENT`/`EACCES`); (c) connecting to the
     live `CallHost` Unix socket fails (`ENOENT`/`EACCES`/`ECONNREFUSED`); (d)
     reading `/proc/<keeper_pid>/environ` fails (`ENOENT`); (e) Cowboy's own
     trusted `cowboy mcp` server in the same session still succeeds on (a)–(c).

---

## 4. Correlated Proxy Ack for Pending Extension Calls (High 3)

### 4.1 Failure mode today

In `src/worker_execution/extension_broker.rs:301-385`, `call_guarded` holds an
in-memory `SettlementGuard` only until `send_bounded` writes the reply frame to
the local Unix WebSocket send buffer. There is no durable pending-call record
on disk and no proxy acknowledgement: if the worker crashes after the target MCP
server finishes executing a side effect, or if the WS drops before the proxy
receives and processes the reply, the effect disappears without parking the
session.

### 4.2 Protocol and durability invariants

1. **Older-proxy capability gate**:
   - The Antigravity proxy (`extensions.mjs`) passes `{ ackProtocol: 1 }` in
     `cowboy/extensions` and `{ name, arguments, ackProtocol: 1 }` in
     `cowboy/extensionCall`.
   - If the proxy does not advertise `ackProtocol >= 1`, `cowboy/extensions`
     omits user MCP servers (with an upgrade note) and `cowboy/extensionCall`
     refuses durable-tracked calls before dispatch (`refused("proxy does not support extension-call acknowledgements")`).
2. **Pre-dispatch durable write**:
   - Before writing `tools/call` to the target server's stdin, the worker
     allocates a 128-bit `call_id` and atomically persists
     `PendingCall { call_id, turn_id, server, tool }` into the execution ledger
     (`ledger.json`, Schema 2). If persistence fails, the call is refused before
     dispatch (`Failure::NotSent`).
3. **Reply carrying correlated `call_id`**:
   - On `Failure::NotSent` (proven nothing reached the server): remove
     `PendingCall` from the ledger and reply with refusal.
   - On `Failure::MaybeSent` (timeout/loss after possible dispatch): atomically
     remove `PendingCall` AND set `uncertain_effect = Some("tools/call")`,
     `uncertain_sequence = next_sequence(...)` in a single atomic ledger write,
     and reply with `code: -32000, data: { outcome: "unknown" }`.
   - On server `Ok(result)` or `Failure::Refused`: keep `PendingCall` **durably
     in `ledger.json`** and transition in-memory state to `AwaitingAck { call_id }`
     while returning `{ result, _cowboyAck: { callId } }` (keeping the turn's
     `SettlementGuard` open!).
4. **Proxy acknowledgement (`cowboy/extensionCallAck`)**:
   - In `extensions.mjs`, `Extensions.call` strips `_cowboyAck`, validates
     `callId`, and sends `cowboy/extensionCallAck({ callId })` on the execution
     connection BEFORE returning the tool output to `agy`.
   - `cowboy/extensionCallAck` is added to `DRAIN_REQUESTS` in
     `execution-connection.mjs` and exempted from the 15-slot ordinary cap (just
     like `cowboy/uncertainEffect` and `cowboy/drained`), so full slots or drain
     transitions can NEVER block acknowledgement.
   - Upon receiving `cowboy/extensionCallAck({ callId })`, the worker verifies
     `callId` matches the in-flight `AwaitingAck` entry, atomically removes
     `PendingCall` from `ledger.json`, and drops `SettlementGuard`.
   - If the proxy fails to receive the ack reply (disconnect/timeout), it throws
     `UncertainEffect("tools/call")`, fencing the turn locally.
5. **Atomic recovery on crash, disconnect, or SIGTERM**:
   - If the WS closes before `cowboy/extensionCallAck`, `mark_unresolved_effects_uncertain()`
     or `Ledger::open()` on worker reopen atomically converts EVERY remaining
     `PendingCall` in `ledger.json` into `uncertain_effect = Some("tools/call")`
     with `uncertain_sequence = next_sequence(uncertain_sequence)` and clears
     `pending_calls` in one atomic `write_record` before any new turn can run.

---

## 5. Two-Phase Ledger Schema 2 Rollout & Conformance (High 4)

Currently `src/worker_execution/ledger.rs:35,921` hard-checks
`record.schema == 1`. Writing `schema: 2` before every rollback target (OVH
timer, broker fallback, per-session fallback, manual rollback) understands
Schema 2 would cause fallback workers to fail `Ledger::open` with
`"unsupported execution ledger schema"`.

### 5.1 Phase A — Dual-Schema Reader Floor (ships FIRST, writes `schema: 1`)

- `Ledger::read` accepts `record.schema == 1 || record.schema == 2`.
- `Record` adds optional fields with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`
  (or `BTreeMap`):
  - `pending_calls: Vec<PendingCallRecord>`
  - `user_servers: Vec<UserServerRecord>`
- On reopen (`Ledger::open`) of ANY `schema: 1` or `schema: 2` file:
  - Preserves existing `uncertain_sequence` monotonically via `next_sequence`
    (never resets or regresses).
  - If `pending_calls` is non-empty, atomically clears `pending_calls`, sets
    `uncertain_effect = Some("tools/call")`, and advances `uncertain_sequence`.
  - Retains `user_servers` termination obligations until confirmed closed
    (Section 6).
- **Write fence in Phase A**: `Ledger::persist` writes `schema: 1` ONLY (never
  emits `pending_calls`/`user_servers` or `schema: 2`), and user MCP server
  activation stays fenced off until Phase B.
- Roll out Phase A until automatic fallback, manual fallback, and per-session
  retained worker generations on Hawk and OVH all carry the dual-schema reader.

### 5.2 Phase B — Schema-2 Writer Enablement & Hawk Production Conformance

- Only after the Phase-A reader floor is verified live across all three rollback
  tiers, enable `schema: 2` writes (explicit `--ledger-schema-v2` / release
  flag) and activate user MCP server execution.
- Because Phase-A workers already read `schema: 2` cleanly and preserve
  `uncertain_sequence`, rolling back from Phase B to Phase A (automatic, manual,
  or per-session) recovers populated Schema-2 ledgers without error.
- **Required Hawk production-conformance artifact matrix** (matching
  `docs/machine-worker-pin-advance-runbook.md` and `call-8dc781e16a7ca99702fb61b0`):
  1. Two distinct writer revisions and ELFs (proving upgrade & downgrade between
     distinct Schema-2 writer builds).
  2. A reader-only fallback binary on the same worker generation (Phase-A
     binary opening a populated Schema-2 ledger with active `pending_calls` +
     `user_servers` + non-zero `uncertain_sequence`, verifying sequence
     preservation and atomic conversion to `uncertain_effect`).
  3. A default-reader artifact (fresh Schema-1 ledger upgraded to Schema-2 and
     downgraded back to reader-only).
  4. Fresh, unique receipt path per run (never reusing an existing receipt).

---

## 6. Retained-Keeper Orphan User-Server Lifecycle (Medium 1)

Because `cowboy-execution-host` is a **retained keeper** that outlives worker
replacements, a user MCP server launched by worker epoch `E1` can remain alive
on the keeper when replacement worker `E2` reconnects before
`UNATTENDED_PROCESS_REAP` fires.

1. **Persist before launch**:
   - Each user MCP server uses a dedicated process-ID prefix
     `cowboy-user-mcp-<uuid>` (covered by `worker_event()` filtering so its
     events never reach the account unit).
   - BEFORE sending the isolated `process/start` request to the keeper, the
     worker atomically persists
     `UserServerRecord { process_id, server_name, keeper_scope: { environment_id, incarnation, workspace_cwd } }`
     into `ledger.json` (Schema 2).
2. **Fence and confirm termination on uncertain dispatch or reopen**:
   - On **reopen** (`Ledger::open()` finds persisted `user_servers` from a prior
     worker): BEFORE discovery launches ANY new user server or reuses ANY server
     name, the broker issues `process/terminate` for every persisted
     `UserServerRecord` matching the connected keeper scope and waits for
     confirmed closure (`process/read` closed or `E1 forgotten`) within a
     bounded stop budget (`USER_SERVER_STOP_CONFIRM_MS = 10_000`).
   - On **uncertain dispatch** (`Failure::MaybeSent` on `initialize`,
     `tools/list`, or `tools/call`): the affected user server is immediately
     fenced (no further calls admitted), terminated, and removed from
     `user_servers` ONLY after confirmed exit.
   - If termination cannot be confirmed (or keeper scope differs while a
     pending call existed): the server stays fenced, `uncertain_effect` is
     recorded (`"process/terminate"`), and discovery refuses to start a second
     copy of that server.
   - Clean worker shutdown terminates all `user_servers`, confirms exit, and
     clears them from `ledger.json`.

---

## 7. Required Bindings Budget Reservation & Bounded Concurrency (Medium 2)

In `src/worker_execution/extension_broker.rs`:

1. **Strict two-phase discovery**:
   - **Phase 1 (Required folder bindings only, `plan.required == true`)**:
     - Run exclusively first with guaranteed budget (`REQUIRED_DISCOVERY_BUDGET = 20s`
       inside `DISCOVERY_TIMEOUT = 40s`) and zero contention from optional
       servers.
     - Required bindings claim tool slots first from `MAX_TOOLS = 128` (and
       server count slots). If required bindings alone exceed `MAX_TOOLS` or
       fail to initialize within budget, **refuse session startup** (`discovery.failed = Some(...)`).
       Required binding tools are NEVER truncated or displaced by optional
       servers.
   - **Phase 2 (Optional servers: Cowboy MCP, Matrix memory, then owner-admitted
     user MCP servers)**:
     - Runs only after Phase 1 succeeds, drawing strictly from the remaining
       tool budget (`remaining_tools = MAX_TOOLS - required_tools`) and
       remaining discovery deadline.
     - Optional server launches are gated by a concurrency semaphore
       (`MAX_OPTIONAL_LAUNCH_CONCURRENCY = 2`), capped at
       `MAX_USER_MCP_SERVERS = 16`, with a per-server init timeout of
       `USER_SERVER_INIT_TIMEOUT = 10s`.
     - If an optional user server fails, times out, or exceeds remaining tool
       slots, it is terminated with confirmed cleanup (removing its
       `UserServerRecord` once closed), omitted with a bounded availability
       note, and never disrupts required bindings or session startup.

---

## 8. Wire, Protocol, and Rollout Sequencing

- **Protocol changes**:
  - `LaunchContract` / `StartSession` carries optional frozen `user_mcp_plan`
    (verified by `plan_digest` and included in `mcp_launch.rs` placement
    equality checks).
  - Keeper (`execution_host.rs`) adds an isolated user-MCP launch mode (never
    calling `with_call_context`, invoking pinned `bwrap` with masked Cowboy
    state/socket paths and isolated PID/IPC/mount namespaces) and a read-only
    isolated `~/.claude.json` resolver command for owner admission.
  - Execution connection (`execution-connection.mjs`) adds
    `cowboy/extensionCallAck` to `DRAIN_REQUESTS` and `extensions.mjs`
    advertises `ackProtocol: 1` + sends correlated acks before returning tool
    output.
- **Two-phase pin advance (post-R4 baseline `worker-11af6d9b0b7925e98a9d`)**:
  - R4 (`antigravity 0.2.39`, `worker-11af6d9b0b7925e98a9d`) is already landed on
    `origin/main` and active in production.
  - R2 ships as its own two-phase rollout on top of `origin/main`:
    1. **Phase A (Reader Floor)**: Advance `inputs.cowboy-workers` with
       dual-schema (`schema: 1 || 2`) ledger reader + additive protocol fields
       (writing `schema: 1` only, user MCP execution fenced off) and verify all
       rollback tiers across Hawk and OVH carry the Schema-2 reader.
    2. **Phase B (Writer Enablement)**: Enable `schema: 2` ledger writes,
       isolated `bwrap` user MCP execution, correlated `cowboy/extensionCallAck`,
       and owner admission APIs, verified by the Hawk four-artifact
       production-conformance matrix and live OVH <-> Hawk canary acceptance.
