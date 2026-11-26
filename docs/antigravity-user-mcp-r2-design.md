# Antigravity User MCP Servers (R2) — Design Document (Revision 4)

Status: **Design Revision 4 — Ready for Implementation** (Owner-approved Option 1
pragmatic same-UID parity; addresses all 14 Round 1 findings from
`call-471f3a5bc14a499bcbc732e8`, all 15 Round 2 findings from
`call-ae42c7df5b133bae153ba7cf`, and all 5 Medium + 6 Low findings from Round 3
`call-719fbc3668635f53f30cd8fa`, which confirmed zero critical/high blockers).

Related documents:
- [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) — Round 2 parity requirements (R2 and D1 aligned to the 2026-11-26 Option 1 decision)
- [`antigravity-real-accounts-design.md`](antigravity-real-accounts-design.md) — Account-sandboxed Antigravity runtime and session extension broker
- [`antigravity-post-edit-r4-design.md`](antigravity-post-edit-r4-design.md) — R4 post-edit diagnostics and §7.1 three-tier populated-ledger fallback verification
- [`host-isolation-design.md`](host-isolation-design.md) — Target-host same-UID threat model and Part I control-UID boundary
- [`uncertain-effect-recovery.md`](uncertain-effect-recovery.md) — Uncertain effect durability, L1c drain, and recovery UX
- [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md) — `retained-worker-interface-compatible` and worker pin advance
- [`agent-api.md`](agent-api.md) — Cowboy MCP schema discovery and write-action policy exceptions

---

## 1. Decision Record, Trust Model & Guardrails

### 1.1 Owner-Approved Trust Model (Option 1 — 2026-11-26)

On target hosts such as Hawk and Falcon today, the Controller (`User=draven`),
Machine, execution keeper (`cowboy-execution-host`), ordinary session
`RunCommand` processes, Claude Code's native stdio MCP servers, and
`~/.config/cowboy/client-auth` all execute under the same OS UID (`1000`,
[`host-isolation-design.md`](host-isolation-design.md)). Within a single UID, a
same-UID `bwrap` or filesystem blacklist cannot establish a genuine security
boundary against arbitrary user-level code (`client-auth`, `systemd-run --user`,
Docker socket, writable `$HOME`).

On 2026-11-26 the owner explicitly approved **Option 1**:

1. **Honest same-UID parity with Claude Code**: Antigravity R2 adopts the exact
   same target-user trust anchor as Claude Code's existing `~/.claude.json`
   stdio MCP servers on Hawk/Falcon. R2 makes **no false claim** of cross-UID
   sandbox confinement or `bwrap` isolation.
2. **Explicit residual risk**: Until [`host-isolation-design.md`](host-isolation-design.md)
   Part I establishes a separate control UID / peer-credential boundary, any
   process running as UID `1000` (including `RunCommand` or a user MCP server)
   shares OS privilege with `~/.claude.json` and same-UID Cowboy control files.
   This residual risk is documented here and in
   [`antigravity-parity-round-2.md`](antigravity-parity-round-2.md) (with frozen,
   credential-isolated launch plans scoped to D1 / Part I).
3. **All real, enforceable guardrails stay intact**: Everything that genuinely
   improves safety, privacy, durability, and blast-radius control without
   needing Part I is strictly enforced below.

### 1.2 Enforceable Guardrails in R2

1. **Zero secrets or config values cross off the target Machine**:
   `~/.claude.json`, server `env` values, `args`, and executable paths are
   parsed and held exclusively inside the target `cowboy-execution-host` keeper.
   Only `{ name, scope, fingerprint }` leaves the target to the OVH worker. No
   change is made to `LaunchContract`, `StartSession`, `contract.json`, or the
   OVH account-sandbox unit.
2. **Anti-accidental-repo-execution guardrail (neutral cwd + lexical repo-path checks)**:
   - Repository `.mcp.json` files are **never** read (deferred to D1).
   - User MCP servers start in a neutral, read-only, mode-`0500` directory
     located **outside** both `<keeper_state_dir>` and Machine state:
     `/tmp/cowboy-user-mcp-<uid>/<incarnation>` (created and verified for
     current UID, non-symlink `O_NOFOLLOW`, `0700` parent / `0500` leaf,
     mirroring `control_root()` in `components/machine/src/machine_cli/execution.rs:1122`,
     and removed on keeper shutdown).
   - Lexical repo-path checks (explicitly documented as an accidental-execution
     guardrail, not an OS confinement boundary) reject:
     - relative commands containing `/` (e.g. `./server.js`, `../bin/mcp`);
     - executables whose logical OR canonicalized path lies inside either
       `binding.workspace.source_path` (including its git toplevel) OR
       `binding.workspace.cwd` (worktree);
     - arguments starting with `./` or `../`, or referencing either
       `source_path` or `cwd` (logical or canonicalized).
3. **Clean child environment (empirically verified `envPolicy.inherit = "none"`)**:
   - Keeper's dedicated `cowboy/startUserMcp` handler **never** calls
     `wire::with_call_context`.
   - Native `process/start` is sent with `envPolicy: { inherit: "none",
     ignoreDefaultExcludes: true, exclude: [], set: <clean map>, includeOnly: [] }`
     (empirically verified on pinned `codex 0.159.3`: yields *exclusively* the
     keys in `set`, guaranteeing zero leakage of `COWBOY_CALL_CONTEXT`,
     `COWBOY_EXECUTION_FILE_HELPER`, or keeper-private
     `CODEX_HOME=<keeper_state_dir>/executor-home`).
   - `set` is constructed from `contract.environment` minus all `COWBOY_*` keys
     and minus any `CODEX_HOME` pointing inside `<keeper_state_dir>`, overlaid
     with the server's expanded `env` (rejecting any `COWBOY_*` key).
4. **Live-reloadable target Machine kill-switch & allowlist (fail closed)**:
   - Device config `[execution]`: `user_mcp` (`bool`, default `true`,
     `Reload::Live`) and `user_mcp_allowlist` (`Kind::McpServerNames { max_items: 32 }`,
     default `[]`, `Reload::Live`).
   - Target `execution::Manager` injects a canonical `managerGate` on every
     `cowboy/listUserMcp` and `cowboy/startUserMcp`; the keeper **fails closed**
     if `managerGate` is absent (protecting against old-Machine rollback with a
     retained new keeper).
   - A **level-triggered** 5-second reconciliation watcher in
     `execution::Manager` (mirroring `src/config/file_buffers.rs`) tracks a
     monotonic `config_revision` (starting at `1` on Manager startup) and
     per-session `confirmed_clean_rev`. On startup and every 5 s tick while
     `!enabled || !allowlist.is_empty()`, it sweeps all unconfirmed active
     keepers with bounded concurrency (8), bypassing the 24-permit regular call
     semaphore, and retries on failure until `cowboy/stopUserMcp` succeeds with
     `failed == 0`.
5. **Durable pending-call recovery & bounded correlated proxy acks**:
   - Pending user-MCP calls are recorded in Schema-1 `record.active_processes`
     as `cowboy-pending-call:<call_id>` (zero ledger schema bump), gated on an
     explicit rollback worker floor (first generation containing `d3b59b8ca` /
     `f2aa86175`, verified via R4 §7.1 three-tier populated-ledger proof).
   - Correlated proxy ack (`cowboy/extensionCallAck`, `15 s` deadline started at
     registration, `MAX_ACK = 8` dedicated lane on proxy & worker, top-level
     `cowboyAck` frame field) guarantees no silent effect loss and no unbounded
     turn hang even if the socket closes mid-call.
6. **Keeper-authoritative orphan & process-tree lifecycle**:
   - User-MCP processes use process IDs under the **existing protected prefix**:
     `cowboy-worker-mcp-user-<32hex>`. Every previously released worker
     generation already blocks account-unit events and requests for
     `cowboy-worker-mcp-*` without modification.
   - Keeper owns `cowboy/stopUserMcp` (tracking active user servers,
     `stop_epoch`, and bounded per-`processId` tombstones), eliminating sibling
     ledger files and unconfirmed-start deadlocks.
7. **Zero wire-interface churn (`retained-worker-interface-compatible`)**:
   - `components/wire/src/` is **not modified**. The 4 string constants are
     defined in keeper and worker modules and kept identical by a workspace unit
     test (the existing `READINESS_SCRIPT` pattern), allowing Hawk and Falcon to
     deploy target support as resident host releases without replacing live
     Hawk/Falcon workers.

---

## 2. Protocol & Configuration Design

### 2.1 Shared String Constants (Zero `components/wire/src` Changes)

To avoid tripping `retained-worker-interface-compatible` (`flake.nix:1016-1028`)
and forcing unnecessary worker generation swaps on Hawk/Falcon, the four string
constants live in `components/machine/src/execution_host/user_mcp.rs` and
`src/worker_execution/extension_broker.rs`, verified byte-identical by a unit
test:

```rust
pub const USER_MCP_INITIALIZATION_KEY: &str = "cowboyUserMcp";
pub const USER_MCP_LIST_METHOD: &str = "cowboy/listUserMcp";
pub const USER_MCP_START_METHOD: &str = "cowboy/startUserMcp";
pub const USER_MCP_STOP_METHOD: &str = "cowboy/stopUserMcp";
pub const USER_MCP_PROCESS_PREFIX: &str = "cowboy-worker-mcp-user-";
```

No struct with `#[serde(deny_unknown_fields)]` (`LaunchContract`,
`SessionExtensions`, `execution_protocol::Command`, `machine_protocol::execution::Action`)
is modified.

### 2.2 Keeper Capability Advertisement

In `components/machine/src/execution_host.rs` (right after `MANAGED_INITIALIZATION_KEY`
annotation):
- When `contract.managed.is_none()` (ordinary non-managed sessions only):
  the keeper creates its neutral cwd `/tmp/cowboy-user-mcp-<uid>/<incarnation>`
  and ephemeral 256-bit HMAC/SHA-256 fingerprint key, and inserts into
  `initialization`:
  ```json
  "cowboyUserMcp": { "schema": 1 }
  ```
- Managed read-only children (`contract.managed.is_some()`): omit
  `cowboyUserMcp` and refuse all three `cowboy/*UserMcp` methods.
- Retained legacy keepers omit `cowboyUserMcp`; the R2 worker skips Phase 2 and
  the startup orphan sweep cleanly. (If a legacy keeper ever received
  `cowboy/*UserMcp`, `codex 0.159.3` returns JSON-RPC `-32601`.)

### 2.3 Device Configuration & Cowboy MCP Policy Exception

In `src/config.rs` and `src/config/schema.rs`:

1. Add `Kind::McpServerNames { max_items: usize }` (matching the MCP server name
   grammar `^[A-Za-z0-9._-]{1,64}$`, supporting names such as `ios_sim`,
   `GitHub`, `chrome-devtools-draven`, unlike `Kind::Identifiers` which only
   accepts lowercase hyphenated slugs).
2. Add two live-reloadable Device fields under `[execution]`:
   - `DEVICE_EXECUTION_USER_MCP`: `execution.user_mcp`, `Kind::Bool`,
     default `true`, `Reload::Live`.
   - `DEVICE_EXECUTION_USER_MCP_ALLOWLIST`: `execution.user_mcp_allowlist`,
     `Kind::McpServerNames { max_items: 32 }`, default `[]` (empty = allow all
     valid stdio servers from `~/.claude.json`), `Reload::Live`.
3. **Cowboy MCP policy exception (`docs/agent-api.md:1820-1826`)**: Both fields
   are host security switches controlling what target processes Antigravity may
   launch. They are **discoverable in the Device config schema** (`cowboy config
   schema` / JSON Schema), and **intentionally expose NO AI-callable Cowboy MCP
   write action** — only the host owner editing `device.toml` may change them.
   This exception is also recorded in [`agent-api.md`](agent-api.md).
4. **Rollback note for `[execution]`**: Older Machines reject unknown TOML
   sections (`docs/configuration.md:64`). Rolling back a target Machine to a
   pre-R2 release requires removing the **entire `[execution]` section** (not
   merely its keys) from `device.toml` if present. Even with `[execution]`
   removed, a retained R2 keeper still **fails closed** under an old Machine
   because the old Manager omits `managerGate` (§2.5).

### 2.4 Target-Local `~/.claude.json` Parser & Claude Code Parity

Implemented in `components/machine/src/execution_host/user_mcp.rs`:

1. **Source**: `$HOME/.claude.json` resolved from `contract.environment["HOME"]`
   (refusing symlinks to non-regular files or files > **4 MiB**, matching
   `plugins/claude-code/runtime/tools.mjs:2848`). Missing file yields empty
   list without error. Repository `.mcp.json` is **never** read.
2. **Project key resolution (Claude Code parity)**:
   - Determine `project_root` from `binding.workspace.source_path`: walk upward
     (bounded, no child process spawn) to locate a `.git` file/directory to find
     the repository root if `source_path` is a subdirectory, falling back to
     `source_path` (`plugins/claude-code/runtime/mcp.mjs:62-64`).
   - Match `projects` entries in `~/.claude.json` by comparing canonicalized
     paths of each key against both `canonicalize(project_root)` and
     `canonicalize(source_path)` (preferring exact/canonical `project_root`
     match, then `source_path`), so symlinked workspace aliases match reliably.
3. **Precedence & filtering**:
   - Merge top-level `mcpServers` (scope `"user"`) overridden by matching
     `projects[<key>].mcpServers` (scope `"local"`).
   - Remove any server listed in `projects[<key>].disabledMcpServers`.
   - Accept **stdio only** (`type` absent or `"stdio"`, non-empty `command`,
     optional string array `args`, optional string-to-string `env`). Non-stdio
     entries (`http`, `sse`, `ws`) are skipped with bounded reason code
     `"unsupported_transport"`.
   - Validate server name against `^[A-Za-z0-9._-]{1,64}$`; reject `"cowboy"`
     and any name starting with `"cowboy"` (case-insensitive) at the keeper as
     well as in worker Phase 2.
   - Apply `managerGate.allowlist` (when non-empty) **BEFORE** enforcing the
     maximum list size (`MAX_USER_MCP_SERVERS = 16`, sorted deterministically by
     server name). This guarantees allowlisted servers are never dropped by
     truncation when `~/.claude.json` defines >16 entries.
4. **Variable expansion & documented Claude Code parity gaps**:
   - Expand `${VAR}` and `${VAR:-default}` across `command`, `args`, and `env`
     values (matching `plugins/claude-code/runtime/mcp.mjs:39-57`) using
     `contract.environment` (minus `COWBOY_*` and keeper-private `CODEX_HOME`).
   - **Documented parity gap — `CLAUDE_PROJECT_DIR` omitted**: R2 intentionally
     does NOT expand or set `CLAUDE_PROJECT_DIR` (pointing it at `source_path`
     would risk mutating the shared stable checkout, while pointing it at the
     worktree `cwd` contradicts the neutral-cwd guardrail). Servers needing
     project paths receive them via tool arguments.
   - **Documented parity gap — host session/secret env vars**: Because
     `contract.environment` contains only the 12 BASE names plus operator-
     allowed non-secret names (`components/store/src/execution_target_environment.rs:28-66`),
     variables such as `${GITHUB_TOKEN}`, `SSH_AUTH_SOCK`, or `XDG_RUNTIME_DIR`
     absent from `contract.environment` are not expanded (literal values in
     `server.env` still work as authored). If any `${VAR}` placeholder remains
     unresolved in `command`, `args`, or `env` after expansion, the server is
     omitted with closed reason code `"unresolved_variable"` rather than
     launching with a literal `${VAR}`.
5. **Executable, argument & environment validation**:
   - `command` must not contain NUL, must not be a relative path containing `/`
     (e.g. `./foo`, `../foo`, `sub/foo`).
   - If `command` is a bare name (no `/`), resolve it against `PATH` from
     `contract.environment` to obtain `logical_executable` (an absolute path
     preserving symlinks!).
   - Canonicalize `logical_executable` (`realpath`) **solely for the lexical
     workspace check**; execute **`logical_executable`** so symlink-dispatched
     tools (`venv/bin/python`, `bunx`, `uvx`, Nix wrappers) preserve `argv[0]`
     and sibling directory behavior identically to Claude Code.
   - Reject if EITHER `logical_executable` OR `realpath(logical_executable)` is
     inside ANY of: `source_path`, `canonicalize(source_path)`, `project_root`,
     `canonicalize(project_root)`, `cwd`, `canonicalize(cwd)`.
   - Reject if ANY `arg` starts with `./` or `../`, or matches/contains a path
     prefix inside any of the above workspace/worktree directories (both logical
     and canonicalized forms).
   - Under Option 1, standard runtime variables (`LD_LIBRARY_PATH`,
     `NODE_OPTIONS`, `PYTHONPATH`, etc.) are **allowed** so legitimate NixOS
     configurations work; **only `COWBOY_*` keys (case-insensitive)** are
     rejected in server `env` (defense-in-depth).
   - Child `envPolicy` sent to executor:
     ```json
     {
       "inherit": "none",
       "ignoreDefaultExcludes": true,
       "exclude": [],
       "set": { "<clean contract.environment + server.env>" },
       "includeOnly": []
     }
     ```
     (`CLAUDE_PROJECT_DIR` is intentionally omitted per §2.4.4.)
6. **Keyed ephemeral fingerprint (HMAC-SHA-256)**:
   - Computed as standard RFC 2104 `HMAC-SHA-256(keeper_ephemeral_32B_key, canonical_json(name, scope, logical_executable, args, sorted_env))`
     (64 lowercase hex chars), implemented directly over the existing `sha2`
     crate (zero new crate / zero `Cargo.lock` change). Never leaks config
     content; detects drift between `listUserMcp` and `startUserMcp`.

### 2.5 Target Manager Enforcement, Fail-Closed Gate & Keeper Methods

#### Manager (`components/machine/src/machine_cli/execution.rs`)

`execution::Manager` holds `device_config: crate::config::Handle` and spawns a
5-second background watcher (like `src/config/file_buffers.rs`):

1. **On every `Manager::call` with `Command::Invoke { invocation, .. }`**:
   - For `cowboy/listUserMcp` and `cowboy/startUserMcp`:
     - Strip any caller-supplied `managerGate` field from `invocation.params`.
     - Read live `enabled = device_config.current().get(&DEVICE_EXECUTION_USER_MCP)`
       and `allowlist = device_config.current().get(&DEVICE_EXECUTION_USER_MCP_ALLOWLIST)`.
     - If `!enabled`:
       - Issue a best-effort local `cowboy/stopUserMcp {}` to this session's
         keeper so any already-running user servers are stopped.
       - Return **`Outcome::Completed`** (NOT `Response::Refused`!) with:
         ```json
         {"error": {"code": -32000, "message": "user MCP servers disabled by device configuration", "data": {"code": "user_mcp_disabled"}}}
         ```
     - If `enabled`:
       - Inject `"managerGate": { "enabled": true, "allowlist": allowlist }`
         into `invocation.params` and forward to the keeper.
   - For `cowboy/stopUserMcp`:
     - Forward directly to keeper regardless of `enabled` (always safe; only
       terminates user servers; strips any caller-supplied `keepAllowlist` so
       only the Manager's live watcher can supply allowlist-scoped stops).
2. **Level-triggered 5-second reconciliation watcher**:
   - Track `(config_revision: u64, last_enabled, last_allowlist)` starting at
     `config_revision = 1` on Manager startup (bumping `config_revision` on any
     change to `enabled` or `allowlist`), plus `confirmed_clean_rev: HashMap<String, u64>`.
   - On startup AND on every 5-second tick whenever `!enabled || !allowlist.is_empty()`:
     enumerate active non-stopped, non-managed session directories under
     `self.root` whose `confirmed_clean_rev.get(session) < Some(&config_revision)`.
   - Sweep unconfirmed keepers with **bounded concurrency (8)**, sending
     `cowboy/stopUserMcp {}` (if `!enabled`) or
     `cowboy/stopUserMcp { "keepAllowlist": allowlist }` (if `enabled`),
     **bypassing** the per-session 24-permit regular call semaphore (using a
     dedicated control path so a saturated session cannot block kill-switch
     enforcement).
   - Record `confirmed_clean_rev[session] = config_revision` ONLY when the
     keeper responds `Outcome::Completed` with `result.failed == 0` (or the
     session is gone / `EnvironmentLost`); any timeout, transport error, or
     `failed > 0` leaves the session unconfirmed so the next 5 s tick retries
     automatically.

#### Keeper (`components/machine/src/execution_host.rs` & `execution_host/ledger.rs`)

1. **`cowboy/listUserMcp`**:
   - Handled locally in `Host::request` (never hits `exec-server`, never
     consumes a ledger operation slot).
   - Refuses managed/read-only keepers (`Outcome::Completed` error
     `"user_mcp_unsupported"`).
   - Requires `params.managerGate` with `enabled == true` (otherwise returns
     `Outcome::Completed` error `"user_mcp_manager_gate_required"` or
     `"user_mcp_disabled"` — **fails closed under old Machine rollback**!).
   - Parses `~/.claude.json`, applies `managerGate.allowlist` **before**
     truncating to 16 servers, and returns:
     ```json
     {"result": {"servers": [{"name": "...", "scope": "user"|"local", "fingerprint": "<64hex>"}], "omitted": [{"name": "...", "reason": "<closed_code>"}]}}
     ```
     (Zero paths, args, or env values in response.)

2. **`cowboy/startUserMcp`**:
   - Params: `{ "name": string, "processId": string, "fingerprint": string, "managerGate": { "enabled": true, "allowlist": [...] } }`.
   - **Exact execution order (idempotency-first!)**:
     1. Validate `managerGate` is present and `enabled == true` (otherwise
        return `Outcome::Completed` error `"user_mcp_manager_gate_required"` /
        `"user_mcp_disabled"` — fails closed even under old Machine).
     2. Strip `managerGate` from `invocation.params` into a local variable so
        the ledger digest is computed strictly over `{ name, processId, fingerprint }`
        (immune to allowlist edits across retries).
     3. **Check `ledger.observe(&invocation.operation_id)` FIRST** (matching
        native `execution_host.rs:294-298`). If the operation already exists
        (`Completed` or `Pending`), return/await that exact outcome immediately
        **without** re-reading `~/.claude.json` or re-checking fingerprint!
     4. Validate `processId` matches `^cowboy-worker-mcp-user-[0-9a-f]{32}$`.
     5. Snapshot `let epoch = self.user_mcp.lock().stop_epoch` and check that
        `processId` is NOT in `stopped_process_ids` tombstone (else return
        `Outcome::Completed` error `"user_mcp_stopped"`).
     6. Re-read `~/.claude.json`, apply `managerGate.allowlist`, resolve
        `name`, verify `fingerprint` matches (else return `Outcome::Completed`
        error `"user_mcp_config_drift"` / `"user_mcp_not_allowed"` /
        `"user_mcp_invalid"` — NEVER `Refused`!).
     7–8. **Single atomic critical section & strict lock order (`user_mcp` → `ledger` everywhere)**:
        Hold `self.user_mcp` lock continuously across: verifying `stop_epoch == epoch`,
        verifying `!stopped_process_ids.contains(&processId)`, verifying
        `latest_allowlist` (if set by a scoped stop) still permits `name`,
        verifying active count `< 32`, reserving `self.writer` permit, calling
        `ledger.admit(&clean_invocation)` (extending `execution_host/ledger.rs:132-137`
        so **both** `method == "process/start"` AND `method == "cowboy/startUserMcp"`
        record `processId` in `self.processes` for `reap_unattended()`),
        inserting `(processId -> UserMcpProcess { server_name, start_state: Pending(rx) })`
        into `self.user_mcp.active_processes`, and sending native `process/start`
        (`cwd = file:///tmp/cowboy-user-mcp-<uid>/<incarnation>`,
        `argv = [logical_executable, ...args]`, `envPolicy = { inherit: "none", set: ... }`,
        `tty: false`, `pipeStdin: true`, `arg0: null`) on `self.writer`
        **WITHOUT** calling `wire::with_call_context`. Release `self.user_mcp`.
     9. **Backend-loop bookkeeping (AFTER dropping `self.ledger` lock!)**:
        In the keeper's native stdout loop (`execution_host.rs:618`), drop the
        `backend_host.ledger.lock()` guard FIRST before acquiring `self.user_mcp`
        (never reversing `user_mcp` → `ledger`!):
        - When the `process/start` reply for a tracked user-MCP `processId`
          arrives: mark `start_state = Confirmed` (on `result`) or `Failed`
          (on `error`, removing from `active_processes`), notifying its waiter.
        - When `process/exited` or `process/closed` arrives for a confirmed
          user-MCP `processId`, remove it from `self.user_mcp.active_processes`.

3. **`cowboy/stopUserMcp`**:
   - Params: `{ "processId"?: string, "keepAllowlist"?: string[] }`.
   - Does **not** require `managerGate` (safe-only destructive cleanup, so R2
     workers can still sweep orphans in a retained new keeper during Machine
     rollback).
   - **Rule E1 confirmation discipline (never accept `unknown process id` for an unconfirmed start)**:
     For each target entry in `active_processes`:
     - If `start_state` is `Pending(rx)`: await `rx` (bounded within the `5 s`
       stop budget) BEFORE sending `process/terminate`.
       - If `process/start` replied with `error` (`Failed`): process never
         started → remove entry and count as `stopped += 1`.
       - If `process/start` replied `Ok` (`Confirmed`): send `process/terminate`
         and await reply within remaining budget; now `unknown process id` IS
         authoritative confirmation (matching `closed_if_confirmed` in
         `src/worker_execution/ledger.rs:692-717`) → remove entry, `stopped += 1`.
       - If `process/start` or `process/terminate` does NOT confirm within the
         `5 s` budget: **retain** the entry in `active_processes` and increment
         `failed += 1` so the Manager's level-triggered reconciliation retries!
   - Modes:
     - **Targeted (`processId` present)**: validate `^cowboy-worker-mcp-user-[0-9a-f]{32}$`;
       insert `processId` into bounded `stopped_process_ids` (FIFO cap 512);
       apply E1 stop above if present in `active_processes`.
     - **Allowlist-scoped (`keepAllowlist` present)**: bump `stop_epoch += 1`
       AND store `latest_allowlist = Some(keepAllowlist)` (fencing any
       in-flight `startUserMcp` carrying a stale `managerGate` allowlist!);
       select every active entry where `!keepAllowlist.contains(&server_name)`,
       insert their IDs into `stopped_process_ids`, and apply E1 stop.
     - **Stop-all (both omitted)**: bump `stop_epoch += 1`, clear
       `latest_allowlist`, select all entries in `active_processes`, insert
       their IDs into `stopped_process_ids`, and apply E1 stop.
   - Returns `Outcome::Completed` with `{"result": {"stopped": u32, "failed": u32}}`
     (zero paths/secrets).

### 2.6 Account-Unit Isolation & `cowboy/*` Dispatch Boundary

1. **Process prefix**: Because user-MCP process IDs are
   `cowboy-worker-mcp-user-<32hex>`, they inherit the existing
   `SERVER_PREFIX = "cowboy-worker-mcp-"` protection in `worker_event()` and
   `Ledger::admit()` (`src/worker_execution/ledger.rs:51-58, 507-511`) across
   **both** R2 workers AND all previously released worker generations.
2. **No second `cowboy/*` allowlist**: In `Client::relay_guarded`
   (`src/worker_execution/ledger.rs:1058-1120`), all existing local endpoints
   continue to be dispatched explicitly (`cowboy/drained`, `cowboy/memory`,
   `cowboy/extensions`, `cowboy/extensionCall`, `cowboy/extensionCallAck`,
   `cowboy/uncertainEffect`). Immediately **after** those local branches — and
   **before** `ledger.admit()` or target forwarding — reject ANY remaining
   method where `method.starts_with("cowboy/")` with
   `refused("reserved cowboy method")`!
   - This guarantees `cowboy/memory` keeps working without maintaining a
     duplicate allowlist, while preventing the account unit from ever invoking
     `cowboy/listUserMcp`, `cowboy/startUserMcp`, `cowboy/stopUserMcp`, or any
     future keeper-internal `cowboy/*` method.
3. **Worker `effectful()` classification**: Add `USER_MCP_START_METHOD`
   (`"cowboy/startUserMcp"`) to `effectful()` in `src/worker_execution/ledger.rs:113-127`.
   Thus any transport-level `Response::Refused` is classified `Err(true)`
   (ambiguous → worker calls `cowboy/stopUserMcp { processId }` to fence/clean
   up), whereas keeper policy/validation/drift rejections arrive as
   `Outcome::Completed` with `{ "error": ... }` (definitive non-start → zero
   orphan possible!).

---

## 3. Ledger Durability, Rollback Floor & Orphan Lifecycle

### 3.1 Schema-1 Sentinel Durability (No Schema Bump)

Instead of bumping `SCHEMA` from `1` to `2`, pending user-MCP tool calls reuse
Schema-1's existing `Record.active_processes: BTreeSet<String>` field with a
reserved sentinel format:

```text
cowboy-pending-call:<32hex-call-id>
```

1. **Before writing `tools/call` to a user server**: `Ledger::record_pending_call(call_id)`
   inserts `cowboy-pending-call:<call_id>` into `record.active_processes` and
   persists atomically. If persist fails, nothing is sent (`Failure::NotSent`).
2. **On `Ledger::open()` (crash / restart recovery)**:
   - Partition persisted `record.active_processes` entries:
     - Any entry starting with `cowboy-pending-call:` → record
       `uncertain_effect = "tools/call"` (incrementing `uncertain_sequence` via
       `next_sequence()`).
     - Any non-sentinel entry → existing behavior (`uncertain_effect = "process/terminate"`).
   - Persist cleared `active_processes` + incremented `uncertain_sequence`
     atomically. (Same partitioning in `mark_unresolved_effects_uncertain()`.)
3. **On `MaybeSent` (timeout / lost reply from user server)**:
   `Ledger::convert_pending_call_to_uncertain(call_id)` removes the sentinel AND
   sets `uncertain_effect = "tools/call"` + increments `uncertain_sequence` in
   **one atomic persist**.
4. **On `Failure::NotSent` (including calls to a kill-switch-stopped server)**:
   `Ledger::clear_pending_call(call_id)` removes the sentinel and persists
   atomically **before** returning the refused reply (with `AckState::None`;
   no `PendingAck` registered and no `uncertain_effect` recorded). In addition,
   when `cowboy/stopUserMcp` or a `process/exited`/`process/closed` event
   arrives for a user server, the worker marks that server `io.closed = true`
   so subsequent tool calls refuse locally (`NotSent`) before ever persisting a
   sentinel.
5. **On valid proxy `cowboy/extensionCallAck`**:
   `Ledger::clear_pending_call(call_id)` removes the sentinel and persists
   atomically. If persist fails, fail closed (convert to uncertain in memory and
   return `{ acknowledged: false }`).

### 3.2 Mandatory Rollback Worker Floor (R4 §7.1 Verification)

`active_processes` and its reopen recovery were introduced in commit `d3b59b8ca`
(2026-11-24); the first pinned worker generation containing it is **`f2aa86175`**
(2026-11-25). Workers older than `d3b59b8ca` (e.g. `76c69b882`, `2d8e0d85c`,
`198a7a479`) lack `active_processes` and `deny_unknown_fields` on `Record`, so
reopening a populated ledger on them would silently strip the sentinel on next
persist.

**Binding rollout invariant**:
- **Minimum rollback floor**: worker generation `>= f2aa86175` (contains
  `d3b59b8ca`).
- Before enabling R2 user-MCP writes in production on OVH, run the
  [`antigravity-post-edit-r4-design.md`](antigravity-post-edit-r4-design.md) §7.1
  **three-tier populated-ledger proof** (automatic `previous.conf`, manual
  rollback targets, and every per-session `fallback_pins` / live retained worker
  binary) using a ledger populated with `cowboy-pending-call:<call_id>`,
  verifying BOTH first reopen (consumes sentinel, records `uncertain_effect`,
  increments `uncertain_sequence`) AND second reopen (retains incremented
  `uncertain_sequence` and fence). Any selectable fallback binary below floor
  MUST be retired first.
- (Live audit on 2026-11-26 confirmed all live OVH worker binaries including
  the sole retained older generation already contain `active_processes` and
  `cowboy-worker-mcp-`; formal §7.1 execution on every fallback binary remains a
  required gate before rollout.)

### 3.3 Orphan Cleanup Without Sibling Files

Sibling `.user-servers.json` and `start_confirmed` are **eliminated**.
Because the keeper is authoritative via `cowboy/stopUserMcp`:

1. **Unconditional startup sweep**: When `Broker::start` launches on a keeper
   whose `initialization` advertises `cowboyUserMcp: { schema: 1 }`, it spawns a
   one-time background sweep task calling `cowboy/stopUserMcp {}` (bounded
   `5 s`). This runs **unconditionally** — even when `execution.user_mcp` was
   flipped to `false` or when a retained `0.2.39` proxy never triggers Phase 2!
2. **Phase 2 barrier**: Phase 2 awaits completion of this startup sweep before
   calling `cowboy/listUserMcp` / `cowboy/startUserMcp`.
3. **Ambiguous / straggler cleanup**: Whenever `cowboy/startUserMcp` returns
   `Err(true)` (ambiguous transport failure) OR a server fails/times out during
   Phase 2 `initialize`/`tools/list`, the worker invokes
   `cowboy/stopUserMcp { "processId": id }` (which both tombstones an unseen ID
   and terminates a started one).
4. **Worker shutdown**: `Broker::shutdown` invokes `cowboy/stopUserMcp {}`
   alongside Phase-1 server termination.

---

## 4. Correlated Proxy Acknowledgement & Concurrency Lanes

### 4.1 Wire Protocol (`ackProtocol: 1`)

1. **`cowboy/extensions`**: Proxy `0.2.40` sends `{"ackProtocol": 1}` (timeout
   `70_000 ms`, safely above Phase 1 `40 s` + Phase 2 `15 s` + margin). Legacy
   `0.2.39` sends `{}` (`ackProtocol` omitted / `0`) → worker skips Phase 2 and
   returns Phase 1 tools only.
2. **`cowboy/extensionCall`**: Proxy `0.2.40` sends
   `{"name": ..., "arguments": ..., "ackProtocol": 1}`. Worker checks
   `ackProtocol >= 1` **strictly for `Route::UserServer`**; legacy proxies calling
   a user tool receive a clean refusal (zero `tools/call` sent). Phase-1 Cowboy
   MCP, folder bindings, and Memory tools remain unchanged (`ackProtocol` not
   required).
3. **Top-level `cowboyAck` frame field (spoof-proof)**:
   - For `Route::UserServer` replies (`Ok(result)` AND `Failure::Refused` — i.e.
     any completed server exchange that is not `NotSent` and not already-marked
     `MaybeSent`), `Done` carries `ack_call_id: Option<String>` outside `reply`.
   - `Done::frame()` inserts top-level `"cowboyAck": { "callId": "<32hex>" }`
     adjacent to `"id"` and `"result"`/`"error"` (NEVER inside `"result"`, so a
     malicious user server cannot spoof or suppress ack metadata).
   - Proxy `execution-connection.mjs` extracts `frame.cowboyAck?.callId` ONLY
     from the top-level JSON-RPC response frame (for both `result` and `error`
     frames).
4. **`cowboy/extensionCallAck`**:
   - Proxy sends `{"callId": "<32hex>"}` with explicit **`timeoutMs: 12_000`**
     before returning the tool result or refused error to `agy`.
   - If the ack request fails, times out, or returns `acknowledged !== true`,
     proxy throws `UncertainEffect("tools/call")` (never returning the tool
     result as clean!).

### 4.2 Dedicated Concurrency Lanes (`MAX_ACK = 8`)

Because `MAX_CALLS = 8` extension calls can complete simultaneously while
`MAX_CONTROL = 4`:

- **Worker (`src/worker_execution.rs` & `src/account_sandbox.rs`)**:
  - Add `EXTENSION_CALL_ACK: &str = "cowboy/extensionCallAck"` to
    `account_sandbox::cleanup(method)` so acks are admitted while draining.
  - Add a dedicated `ack: HashSet<String>` lane with **`MAX_ACK = 8`** inside
    `MutationInflightIds` (separate from `forward: 16` and `control: 4`).
  - Handle `cowboy/extensionCallAck` directly inside `serve_mutation` **before**
    `ledger.admit()`, with zero target round-trip.
- **Proxy (`components/provider-runtime/execution-connection.mjs`)**:
  - Add `"cowboy/extensionCallAck"` to `DRAIN_REQUESTS` with dedicated slot
    class `kind = "ack"` (`MAX_ACK = 8`), separate from ordinary (15), cleanup
    (1), and control (4) slots.
  - **Immediate slot release on local 12 s ack timeout**: When a `kind === "ack"`
    request hits its `12_000 ms` local `timeoutMs`, release its `ack` slot
    **immediately** (recording its request id directly in `retired` so a late
    reply is silently absorbed without failing the socket, rather than holding
    the slot in `abandoned` for `185 s - 12 s = 173 s`). Because the worker
    already fails closed on its own 15 s timer, this prevents transient stalls
    from starving the `MAX_ACK = 8` lane.

### 4.3 Bounded `PendingAck` State Machine & Deadlock-Free Lock Ordering

Every `Route::UserServer` call that completes with `Ok` or `Failure::Refused`
registers a `PendingAck` entry holding `{ call_id, connection_id, ledger, guard: SettlementGuard, timer_abort: AbortHandle }`:

1. **Explicit `AckState` enum on `Done` (`None | Pending(String) | AlreadySettled`)**:
   - `Done` carries `ack: AckState` instead of `Option<String>` so the relay
     layer distinguishes *not broker-managed* (`AckState::None`) from
     *broker-owned pending ack* (`AckState::Pending(call_id)`) and *already
     settled by broker* (`AckState::AlreadySettled`).
   - `Broker::register_pending_ack(call_id, connection_id, ledger, guard)`
     checks under `pending_acks` mutex whether `connection_id` is already closed
     (closed connection IDs are tracked in a small bounded set per Broker).
   - **If `connection_id` is already closed**: do NOT insert! Release
     `pending_acks` lock, call `ledger.convert_pending_call_to_uncertain(&call_id)`,
     then drop `guard` outside every lock, and return **`AckState::AlreadySettled`**
     (so downstream `Relays::drop`/`SendError`/`undelivered` skip `unread_effects`
     and never double-bump `uncertain_sequence`!).
   - **Otherwise**: insert entry AND immediately spawn the
     `EXTENSION_ACK_TIMEOUT = 15 s` deadline task right here, returning
     **`AckState::Pending(call_id)`**.
2. **Exclusive ownership across ALL four relay paths (zero hang, zero duplicate L1c)**:
   - Across **`Relays::delivered`**, **`Relays::undelivered`**, **`Relays::drop`**, and **`mpsc::SendError` / WebSocket `send_bounded` error**:
     - `AckState::None`: existing Phase-1 behavior unchanged.
     - `AckState::AlreadySettled`: **skip** `unread_effects` and `client.undelivered(...)` completely (broker already converted sentinel & incremented `uncertain_sequence` exactly once).
     - `AckState::Pending(call_id)`: on `delivered`, **skip** `unread_effects` (PendingAck owns settlement until ack / timeout / close / uncertainReport); on `undelivered` / `Relays::drop` / `SendError`, **skip** `client.undelivered(...)` and call `broker.fail_pending_ack(&call_id)` (converts sentinel → `uncertain_effect = "tools/call"`, aborts timer, drops `SettlementGuard`).
3. **Socket close (`serve_mutation` exit)**:
   - Calls `broker.connection_closed(connection_id)` which records
     `connection_id` closed, removes all `PendingAck` entries matching
     `connection_id`, releases `pending_acks` lock, converts all their sentinels
     to `uncertain_effect = "tools/call"` in one batch persist, and drops all
     their `SettlementGuard`s outside every lock.
4. **`cowboy/uncertainEffect({ method, callId? })`**:
   - If `callId` matches a `PendingAck`, settles it via
     `broker.fail_pending_ack(&call_id)` (single atomic conversion + guard drop,
     no duplicate `uncertain_sequence` increment).
5. **Strict lock-ordering invariant (zero self-deadlock)**:
   - `SettlementGuard::drop` acquires `ledger.state`.
   - Therefore EVERY settlement path follows the exact 3-step sequence:
     1. Lock `broker.pending_acks` → remove `PendingAck` → **unlock `broker.pending_acks`**.
     2. Call `ledger.clear_pending_call` or `ledger.convert_pending_call_to_uncertain`
        (acquires & releases `ledger.state` internally).
     3. **`drop(entry.guard)`** AFTER both locks are released!
   - While step 2 executes, `entry.guard` is still alive (`admitted[turn] >= 1`),
     so `finish_turn` cannot observe `mutations_drained` until after the durable
     ledger update has already persisted.

---

## 5. Two-Phase Discovery, Bounds & Sanitization

### 5.1 Independent Phase 1 & Demand-Triggered Phase 2

1. **Phase 1 (unchanged, published immediately)**:
   - Runs Cowboy MCP, folder bindings (required & optional), and Memory tools
     under the existing `DISCOVERY_TIMEOUT = 40 s` and `MAX_TOOLS = 128`.
   - `Broker::discovered()` waits **only** for Phase 1 (`45 s` ceiling).
     Required binding failures still refuse session startup; Phase 1 latency and
     retained `0.2.39` sessions are completely untouched.
2. **Startup orphan sweep (unconditional, non-blocking for Phase 1)**:
   - If keeper advertises `cowboyUserMcp: { schema: 1 }`, `Broker::start` spawns
     one background `cowboy/stopUserMcp {}` call (`5 s` timeout) tracked by a
     `watch::Receiver<bool>` (`sweep_done`).
3. **Phase 2 (demand-triggered on first `cowboy/extensions` with `ackProtocol >= 1`)**:
   - If `ackProtocol < 1`, keeper lacks `cowboyUserMcp`, or session is managed:
     `Broker::list()` returns Phase 1 immediately (zero user servers started!).
   - On the first `cowboy/extensions` with `ackProtocol >= 1`, a single-flight
     `tokio::sync::OnceCell<UserDiscovery>` runs Phase 2 under a dedicated
     **`USER_MCP_DISCOVERY_TIMEOUT = 15 s`** total wall budget (starting when
     triggered):
     1. Wait for `sweep_done` within the remaining 15 s budget (if sweep did not
        complete, record note `orphan_sweep_timeout` and return Phase 1 only).
     2. Call `cowboy/listUserMcp {}`. If disabled or error, record bounded
        reason-code note (or empty if disabled) and return Phase 1 only.
     3. Filter out any server whose name:
        - starts with `"cowboy"` (case-insensitive);
        - collides (case-insensitive) with ANY Phase-1 server name;
        - collides (case-insensitive) with `plan.memory_server` (e.g. `"matrix"`).
        Record omitted note with reason code `"reserved_name_collision"`.
     4. Start remaining servers (up to 16) with bounded concurrency
        **`MAX_USER_MCP_CONCURRENCY = 4`**, per-server init + `tools/list`
        timeout **`10 s`**, all bounded by the outer **`15 s`** Phase 2
        deadline.
     5. **Partial-success guarantee**: When the 15 s deadline expires, Phase 2
        **always succeeds** and publishes whatever user servers completed in
        time alongside Phase 1! Any still-initializing straggler is cancelled
        and cleaned up via background `cowboy/stopUserMcp { "processId": id }`
        and recorded as a concise note (`reason = "init_timeout"`). An optional
        or hanging user server (e.g. `ios-sim` when Mac is offline) can **never**
        fail `cowboy/extensions` or block session startup beyond 15 s.

### 5.2 Tool Namespacing, Result Size, Instructions & Logging Bounds

- **Mandatory `<server>_` prefix**: Every user-MCP tool is exposed as
  `<server>_<tool>` (sanitized to `^[A-Za-z0-9_-]{1,64}$`, never colliding with
  facade tools or Phase-1 tools).
- **Tool count caps**: Max **64 tools per user server**, max **128 total user
  MCP tools** (separate budget from Phase 1's `MAX_TOOLS = 128`).
- **Instructions cap**: User server `initialize.instructions` is capped at
  **1,024 chars per server** and **4,096 chars total** across all user servers
  (UTF-8 safe truncation), preventing prompt bloat.
- **Result size cap**: Worker enforces **`MAX_USER_MCP_RESULT_BYTES = 4 MiB`**
  (4,194,304 bytes — comfortably below proxy's 14 MiB `maxPayload`) on user
  server `tools/call` responses. Oversized results return a structured MCP tool
  error (`isError: true`, bounded text `"user MCP tool result exceeded 4 MiB limit"`)
  and complete the normal `cowboyAck` handshake so other in-flight requests on
  the socket are never disrupted.
- **Call duration limit**: User server `tools/call` shares `CALL_TIMEOUT = 170 s`
  (below proxy's 185 s socket timer). Calls exceeding 170 s (e.g. long external
  Xcode builds) trigger `MaybeSent` / `UncertainEffect("tools/call")`; async
  MCP progress / streaming calls are out of scope for R2.
- **Bounded OVH logs & prompt notes**: For user servers, OVH worker `tracing`
  logs and system-prompt notes emit **only** `{ server_name, reason_code }` from
  a closed enum (`disabled`, `init_timeout`, `process_exited`, `protocol_error`,
  `start_failed`, `reserved_name_collision`, `result_too_large`, `call_timeout`,
  `config_drift`, `not_allowed`, `invalid_config`, `unsupported_transport`,
  `unresolved_variable`, `orphan_sweep_timeout`) — NEVER raw server stderr/error text, paths, args, or
  env values.

---

## 6. Mixed-Version & Rollback Compatibility Matrix

| Target Machine | Target Keeper | OVH Worker | Antigravity Proxy | Behavior |
|---|---|---|---|---|
| **Old** (pre-R2) | **Old** (pre-R2) | **New** (R2) | **New** (`0.2.40`) | Keeper omits `cowboyUserMcp`; worker skips sweep & Phase 2; Phase 1 works identically. |
| **Old** (rolled back; `[execution]` removed) | **New** (retained R2) | **New** (R2) | **New** (`0.2.40`) | Old Manager omits `managerGate` → retained R2 keeper **fails closed** on `listUserMcp`/`startUserMcp` (`user_mcp_manager_gate_required`); `stopUserMcp` still works for orphan cleanup! |
| **New** (R2) | **Old** (retained pre-R2) | **New** (R2) | **New** (`0.2.40`) | Keeper omits `cowboyUserMcp`; worker skips sweep & Phase 2; zero errors. |
| **New** (R2) | **New** (R2) | **Old** (`>= f2aa86175` floor) | **Old** or **New** | Old worker ignores `cowboyUserMcp`; any leftover `cowboy-worker-mcp-user-*` processes remain unit-isolated by `cowboy-worker-mcp-` prefix; any `cowboy-pending-call:*` in ledger converts to `uncertain_effect` on reopen (`>= f2aa86175` floor proven via R4 §7.1); leftover processes persist until keeper shutdown or next R2 worker sweep (not reaped by `reap_unattended` while old worker polls). |
| **New** (R2) | **New** (R2) | **New** (R2) | **Old** (retained `0.2.39`) | `0.2.39` omits `ackProtocol`; worker skips Phase 2 completely (zero startup latency added, zero unused user servers spawned); startup orphan sweep still cleans prior orphans. |
| **New** (R2) | **New** (R2) | **New** (R2) | **New** (`0.2.40`) | Full R2: `managerGate` enforced live, Phase 1 + demand-triggered Phase 2, `<server>_` tools, Schema-1 sentinel + correlated `cowboyAck` (`15 s` / `MAX_ACK = 8`), `4 MiB` result cap, keeper-authoritative `stopUserMcp`. |

---

## 7. Verification & Test Matrix

### 7.1 Unit & Integration Tests (Deterministic Gate)

1. **Cross-module constant parity test**: Asserts keeper and worker string
   constants (`USER_MCP_INITIALIZATION_KEY`, `USER_MCP_LIST_METHOD`,
   `USER_MCP_START_METHOD`, `USER_MCP_STOP_METHOD`, `USER_MCP_PROCESS_PREFIX`)
   match byte-for-byte (`retained-worker-interface-compatible` stays green).
2. **Device config & `Kind::McpServerNames` tests**:
   - Accepts uppercase/underscore/dot/hyphen names (`ios_sim`, `GitHub`,
     `chrome-devtools-draven`); rejects invalid chars and `>32` items.
   - Example config (`cowboy config init --print`) and JSON Schema snapshots
     stay synchronized.
3. **Keeper parser & validation tests (`execution_host/user_mcp.rs`)**:
   - Merges user + local `projects` (via git toplevel & symlinked alias
     canonicalization), honors `disabledMcpServers`, ignores `.mcp.json`,
     enforces 4 MiB file cap, skips non-stdio entries with closed reason code.
   - Expands `${VAR}` / `${VAR:-default}` from `contract.environment`,
     omits `CLAUDE_PROJECT_DIR` (asserted absent from child env), omits
     servers retaining unresolved `${VAR}` with `unresolved_variable`, and
     strips `COWBOY_*` and keeper-private `CODEX_HOME`.
   - Symlink-preserving `argv[0]`: executes logical PATH path while checking
     realpath against both `source_path` and `cwd`.
   - Rejects relative `/` commands, executables inside `source_path` OR `cwd`,
     `./`/`../` args, args referencing `source_path` OR `cwd`, and `COWBOY_*`
     env keys; allows `LD_LIBRARY_PATH`/`NODE_OPTIONS`/`PYTHONPATH`.
   - Allowlist applied **before** 16-server truncation.
   - `envPolicy.inherit = "none"` yields clean env (no `COWBOY_CALL_CONTEXT`,
     no `COWBOY_EXECUTION_FILE_HELPER`, no keeper `CODEX_HOME`); neutral cwd
     `/tmp/cowboy-user-mcp-<uid>/<incarnation>` is mode `0500` outside keeper
     state.
4. **Keeper lifecycle, idempotency & fail-closed tests (`execution_host.rs`)**:
   - Missing `managerGate` fails closed (`Outcome::Completed` error).
   - `cowboy/startUserMcp` checks `ledger.observe(operation_id)` BEFORE reading
     `~/.claude.json` or fingerprint (retry after config edit/drift replays
     cached outcome cleanly).
   - Validation/drift/tombstone failures return `Outcome::Completed` error
     (never `Refused`).
   - `execution_host/ledger.rs` registers `cowboy/startUserMcp` processes in
     `self.processes` for `reap_unattended()`.
   - `cowboy/stopUserMcp` enforces E1 confirmation (waits for `Pending` start,
     never accepts `unknown process id` for an unconfirmed start, returns
     `failed > 0` and retains entry on timeout), bumps `stop_epoch` + updates
     `latest_allowlist` on allowlist-scoped stop (fencing in-flight starts),
     and preserves strict `user_mcp` → `ledger` lock ordering.
   - Managed read-only keeper omits `cowboyUserMcp` and refuses all three
     methods.
5. **Target Manager live-reload tests (`machine_cli/execution.rs`)**:
   - Strips spoofed caller `managerGate`; injects live `enabled` & `allowlist`.
   - When `user_mcp == false`, returns `Outcome::Completed` `"user_mcp_disabled"`
     and stops running user servers.
   - Level-triggered watcher stops running servers on Manager restart when
     `user_mcp = false` is already set, retries after transient failure,
     bypasses the 24-permit call semaphore, and stops newly disallowed servers
     on allowlist narrowing.
6. **Worker isolation, Phase 2 & PendingAck tests (`src/worker_execution/`)**:
   - Unit sending `cowboy/memory` **succeeds**; unit sending `cowboy/listUserMcp`,
     `cowboy/startUserMcp`, `cowboy/stopUserMcp`, or `cowboy/anythingUnknown` is
     refused locally.
   - `cowboy-worker-mcp-user-*` process events are hidden from the unit and unit
     `process/read`/`write`/`terminate` against them are refused.
   - `effectful("cowboy/startUserMcp") == true`; transport `Refused` triggers
     `cowboy/stopUserMcp { processId }`.
   - Phase 1 publishes independently (`45 s`); retained `0.2.39` (`ackProtocol`
     absent) skips Phase 2; `0.2.40` triggers Phase 2 once; hanging user server
     times out at `15 s` returning ready servers + Phase 1 without failing
     `cowboy/extensions`.
   - Name collisions with `cowboy*`, Phase-1 servers, or `memory_server` are
     dropped (`reserved_name_collision`); instructions capped at 1,024/4,096;
     `>4 MiB` tool result returns bounded `isError: true` and still completes
     normal `cowboyAck` without closing socket.
   - **High-1 regression**: Mutation socket closes **while user-MCP `tools/call`
     is in flight** → `register_pending_ack` returns `AckState::AlreadySettled`,
     turn terminates boundedly (no hang, no self-deadlock), and records
     **exactly one** `uncertain_effect = "tools/call"` (no duplicate from
     `Relays::drop` / `undelivered`).
   - **`Failure::NotSent` & stopped-server call**: Calling a server stopped by
     kill-switch or failing before send clears `cowboy-pending-call:<id>`
     atomically (zero leftover sentinel on reopen) and marks the server closed
     on stop/`process/closed`.
   - **High-1 timeout & ack**: Ack received within 15 s clears sentinel cleanly
     (no uncertain); ack omitted times out at 15 s and converts sentinel once;
     `cowboy/uncertainEffect({ method, callId })` settles immediately without
     double sequence bump; 8 concurrent user-MCP calls complete simultaneously
     via `MAX_ACK = 8` without starving control lane.
   - **High-2 floor & counterexample**: Schema-1 ledger with
     `cowboy-pending-call:<id>` reopens on R2 worker as `uncertain_effect = "tools/call"`
     and survives second reopen; counterexample test with a pre-`d3b59b8ca`
     reader struct (no `active_processes`) proves silent sentinel drop and
     documents the `f2aa86175` rollback floor.
7. **Proxy JS tests (`components/provider-runtime/`)**:
   - `execution-connection.mjs`: `cowboy/extensionCallAck` uses dedicated `ack`
     lane (`MAX_ACK = 8`), permitted during drain, extracts ONLY top-level
     `frame.cowboyAck` (ignores spoofed `result.cowboyAck`), enforces
     `timeoutMs: 12000`, and releases the `ack` slot immediately on local 12 s
     timeout (8 consecutive ack timeouts do NOT starve subsequent acks).
   - `antigravity-acp/extensions.mjs`: sends `ackProtocol: 1` on
     `cowboy/extensions` (`70 s` timeout) & `cowboy/extensionCall`; acks both
     result and refused error; throws `UncertainEffect("tools/call")` if ack
     fails or returns `acknowledged !== true`.

### 7.2 Production Rollout & Live Canary Acceptance

1. **Order of deployment**:
   - **Step 1 (Target hosts first — resident host releases)**: Deploy R2
     Machine host release to Hawk and Falcon (`retained-worker-interface-compatible`
     holds → zero live Hawk/Falcon worker replacement!).
   - **Step 2 (Pre-rollout floor proof on OVH)**: Run R4 §7.1 three-tier
     populated-ledger reopen + second-reopen proof with
     `cowboy-pending-call:*` against EVERY selectable fallback binary on OVH;
     confirm all are `>= f2aa86175`.
   - **Step 3 (Worker pin advance + all 6 independent worker acceptance suites
     + `session-deletion-production-conformance`)**: Advance
     `inputs.cowboy-workers`, publish `3.156.0` / `antigravity 0.2.40` plugins,
     upgrade OVH Machine + Workers (`--workers`), converge fleet plugins, and
     activate `antigravity 0.2.40` on OVH.
2. **Live OVH `antigravity 0.2.40` ↔ Hawk canary**:
   - Positive control: real stdio server from `~/.claude.json` discovered with
     `<server>_` prefix; live tool call returns expected output via correlated
     ack with `parked: null, uncertain_effects: null`; child process environment
     verified clean (no `COWBOY_*`, no keeper `CODEX_HOME`) and cwd verified at
     `/tmp/cowboy-user-mcp-<uid>/<incarnation>`.
   - Kill-switch control: flip `execution.user_mcp = false` live on Hawk →
     running user server terminates promptly and new session lists zero user
     MCP tools; restore `execution.user_mcp = true` afterward and clean up all
     canary artifacts.
