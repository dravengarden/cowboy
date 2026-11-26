# Antigravity User MCP Servers (R2) — Design (Revision 2)

Status: **Revision 2 (owner-approved Option 1 boundary, pending Round 2 design review)**

This document specifies **R2** from
[`docs/antigravity-parity-round-2.md`](antigravity-parity-round-2.md):
exposing owner-configured **stdio** MCP servers from the target home's
`~/.claude.json` to ordinary (non-managed) Antigravity sessions running on OVH
against a Linux execution target (Hawk/Falcon).

Revision 2 resolves all 7 High, 6 Medium, and 1 Low findings from Round 1 design
review `call-471f3a5bc14a499bcbc732e8` (Claude Fable Max) and incorporates the
owner's explicit **2026-11-26 Option 1 scope decision** aligning R2 with the
documented host-isolation reality in [`docs/host-isolation-design.md`](host-isolation-design.md).

---

## 1. Scope, Trust Boundary, and Owner-Accepted Residual Risk

### 1.1 Honest host-isolation reality (resolves Round 1 High 1 & High 2)

As documented in [`docs/host-isolation-design.md`](host-isolation-design.md)
(lines 41–78 and 675–687):

- On Hawk today, the Controller (`cowboy.service`, `User=draven`), Machine
  broker, workers, keepers (`cowboy-execution-host`), `RunCommand`, and owner
  CLI device credentials (`~/.config/cowboy/client-auth`) all share UID 1000
  (`draven`).
- Consequently, any same-UID `--bind / /` blacklist sandbox or same-UID
  Controller admission store **cannot** provide a true cross-UID security
  boundary against another compromised `draven` process. Real user-MCP
  sandboxing (static `cowboy-sbx-<n>` users, `ProtectHome=tmpfs`, positive FS
  view, network allowlist, and credential separation) belongs to
  `docs/host-isolation-design.md` **Part I** (I-1 / I-8 / Future user-MCP
  sandbox).
- Meanwhile, Cowboy's Claude Code remote runtime
  (`plugins/claude-code/runtime/mcp.mjs`) already runs the target user's
  `~/.claude.json` stdio MCP servers directly under `draven` on Hawk/Falcon,
  and the owner's actual `~/.claude.json` servers on Hawk require target host
  capabilities incompatible with a loopback-blocked/tmpfs-`$HOME` sandbox
  (`chrome-devtools*` connect to loopback `127.0.0.1:9222..9225`; `ios-sim`
  runs `ssh macbook-air ...` using `~/.ssh`).

**Owner Decision (2026-11-26, Option 1)**: Ship R2 now with parity to Claude
Code's existing same-UID `~/.claude.json` trust model on Hawk/Falcon — explicitly
recording the same-UID residual risk until Part I lands rather than claiming an
unenforceable separate-UID sandbox — while enforcing every real guardrail that
holds cleanly under the current architecture:

1. **Zero secrets or config cross off the target Machine**: `~/.claude.json`,
   server `env` values, `args`, and host executable paths NEVER leave the target
   keeper (`cowboy-execution-host`), NEVER enter `StartSession`, OVH worker
   argv, `contract.json`, Controller storage, or the account sandbox unit.
2. **No repository `.mcp.json` execution** (deferred to D1): ONLY
   `$HOME/.claude.json` (`mcpServers` user scope + `projects[projectKey].mcpServers`
   local scope minus `disabledMcpServers`) is read.
3. **Pre-turn worktree / repository confinement**: user MCP servers run with
   `cwd` set to a keeper-owned empty mode-`0500` neutral directory
   (`<keeper_state_dir>/user-mcp-cwd`), NEVER the mutable session worktree.
   Commands with relative `/` paths (`./`, `../`), executables resolving inside
   the worktree/workspace/source root, and relative `./`/`../` or worktree-path
   arguments are refused.
4. **No `COWBOY_CALL_CONTEXT` or `COWBOY_*` env inheritance (defence-in-depth)**:
   user MCP servers start through a dedicated keeper method
   (`cowboy/startUserMcp`) that **never** calls `wire::with_call_context(...)`
   and explicitly excludes `COWBOY_CALL_CONTEXT`,
   `COWBOY_EXECUTION_FILE_HELPER`, and all `COWBOY_*` keys from the spawned
   process's environment. (This prevents accidental credential inheritance; it
   does not claim kernel-level isolation against a malicious same-UID binary.)
5. **Live Device-level kill-switch and server-name allowlist**: target Machine
   Device configuration (`execution.user_mcp`, default `true`, `Reload::Live`;
   `execution.user_mcp_allowlist`, default `[]`, `Reload::Live`) lets the owner
   disable user MCP or pin exact server names without restarting keepers or
   breaking worker adoption.
6. **Managed read-only calls excluded**: when `binding.managed.is_some()` (or
   keeper `read_only == true`), user MCP servers are never advertised, listed,
   or started.
7. **Single closed `agy` tool namespace**: user MCP tools surface only through
   the worker extension broker inside `mcp_cowboy_*` with mandatory `<server>_`
   prefixing; required folder bindings and Cowboy MCP retain full priority and
   existing budgets.

### 1.2 Explicitly excluded (non-goals)

- Repository `.mcp.json` servers (D1).
- HTTP / SSE / streamable-HTTP user MCP servers (stdio only; non-stdio entries
  produce bounded availability notes).
- Non-Linux / Windows execution targets (`execution-connection.mjs` already
  refuses non-Linux executors; non-Unix targets return empty/refused cleanly).
- Cross-UID kernel sandboxing (`bwrap` / static `cowboy-sbx-*` UIDs — deferred
  to `docs/host-isolation-design.md` Part I).

---

## 2. Target-Side Keeper User-MCP Protocol (Resolves High 3, High 4, High 7, Medium 5, Medium 6)

### 2.1 Zero breaking wire/contract changes

R2 adds **zero fields** to `LaunchContract` (`contract.json`), `StartSession`,
`SessionExtensions`, `execution_protocol::Command`, or
`machine_protocol::execution::Action` (all of which use `deny_unknown_fields`
or `mcp_launch.rs` equality checks). Instead:

- `components/wire/src/execution_protocol.rs` adds three string constants only:
  - `USER_MCP_INITIALIZATION_KEY = "cowboyUserMcp"`
  - `USER_MCP_LIST_METHOD = "cowboy/listUserMcp"`
  - `USER_MCP_START_METHOD = "cowboy/startUserMcp"`
- Both methods ride inside the existing `Command::Invoke { invocation, wait_ms }`
  envelope (`Invocation::validate` already permits `[A-Za-z0-9/_]` up to 128
  bytes).

### 2.2 Explicit keeper capability negotiation (High 3)

At keeper startup (`components/machine/src/execution_host.rs`), when
`contract.managed.is_none()`:

1. The keeper creates `<keeper_state_dir>/user-mcp-cwd` with mode `0500` (if
   absent, failing startup only if `state_dir` is broken; verifying mode/owner
   and non-symlink).
2. The keeper generates a random 256-bit ephemeral per-keeper HMAC/keyed-hash
   secret (`user_mcp_key: [u8; 32]`) in memory (never persisted, never logged).
3. The keeper adds `"cowboyUserMcp": { "schema": 1 }` to `initialization`
   (matching `cowboyAgentCli` and `cowboyManaged`).

**Retained legacy keeper safety**:
- A retained legacy keeper from before this release does **not** advertise
  `"cowboyUserMcp"` in `initialization`.
- Before attempting user MCP discovery, the worker inspects `initialization`
  from `Command::Describe`. If `"cowboyUserMcp"` `{ "schema": 1 }` is absent,
  the worker skips user MCP discovery completely and emits a bounded
  availability note (`"User MCP servers require a refreshed target execution
  environment."`).
- Even if `cowboy/startUserMcp` were ever sent to a legacy keeper, the legacy
  keeper forwards the unknown method verbatim to the native executor, which
  rejects it with `-32601 Method not found` — it **never** falls back to
  `process/start` and **never** injects `COWBOY_CALL_CONTEXT`.

### 2.3 Target-local `~/.claude.json` resolution (`cowboy/listUserMcp`)

Handled locally inside `Host::request` (NEVER forwarded to the native executor):

1. Refuses immediately when `self.read_only` (`contract.managed.is_some()`),
   when `self.ledger.lock().lost`, or when `!self.current_workspace()`.
2. Reads `$HOME/.claude.json` from `self.environment.get("HOME")` (bounded to
   `MAX_CLAUDE_CONFIG_BYTES = 2 MiB`; symlink/non-regular check; missing file
   returns empty `servers: []`).
3. **Precedence & `projectKey` resolution** (matches `plugins/claude-code/runtime/mcp.mjs:118-138`
   without reading `.mcp.json`, and resolves Medium 2's worktree `project_key`
   mismatch):
   - User scope: top-level `mcpServers`.
   - Local project scope: `projects[key]` where `key` checks
     `contract.binding.workspace.source_path` first (e.g., `/home/draven/columbus`,
     NOT the ephemeral worktree `/srv/storage/fast0/agent/sessions/sess-...`),
     falling back to `canonicalize(source_path)` if symlinked and distinct.
   - Local scope overrides user scope by server name; names in
     `local.disabledMcpServers` are skipped.
   - Never reads `.mcp.json` or `disabledMcpjsonServers`.
4. **Validation, confinement & sanitization** per candidate server (capped at
   `MAX_USER_MCP_SERVERS = 16`):
   - `name` matches `^[A-Za-z0-9_-]{1,64}$` and is not `"cowboy"`.
   - Transport must be stdio (`type` absent or `"stdio"`, `command` non-empty
     string, `url` absent). Non-stdio entries go to `omitted`.
   - Expands `${VAR}` and `${VAR:-default}` in `command`, `args`, and `env`
     against `sanitized_env` (`contract.environment` with ALL `COWBOY_*` keys
     stripped).
   - Rejects (omits) if `env` contains any `COWBOY_*` key or loader/injection
     key (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`, `NODE_OPTIONS`,
     `NODE_PATH`, `PYTHONPATH`, `PYTHONSTARTUP`, `PERL5OPT`, `RUBYOPT`,
     `BASH_ENV`, `ENV`, `ZDOTDIR`, `IFS`), or if any key/value contains `\0` or
     exceeds bounded length (`MAX_ARGS = 64`, `MAX_ARG_BYTES = 4096`,
     `MAX_ENV_ENTRIES = 64`, `MAX_ENV_VALUE_BYTES = 4096`).
   - **Executable & argument confinement**:
     - Rejects relative `command` containing `/` (`./...`, `../...`).
     - Resolves bare `command` via sanitized `PATH` (skipping empty entries,
       `.`, and any directory inside forbidden workspace roots).
     - Canonicalizes the resolved executable; requires regular file with execute
       bit and `realpath` strictly outside all forbidden roots:
       `{ cwd, canonicalize(cwd), source_path, canonicalize(source_path) }`.
     - Rejects any argument starting with `./` or `../`, or any absolute path
       argument whose existing or ancestor-canonicalized path lies inside
       `cwd` / `canonicalize(cwd)`.
5. **Opaque keyed fingerprint (no secrets leave target — resolves High 7)**:
   - Computes `fingerprint = hex(SHA-256(user_mcp_key || canonical_json({ name, executable, args, env })))`.
   - Returns ONLY:
     ```json
     {
       "result": {
         "schema": 1,
         "servers": [{ "name": "chrome-devtools", "scope": "local", "fingerprint": "<64-hex>" }],
         "omitted": [{ "name": "remote-sse", "scope": "user", "reason": "non-stdio transport is not supported" }]
       }
     }
     ```
   - ZERO `env` values, ZERO `args`, and ZERO host executable paths leave the
     target Machine.

### 2.4 Target-local sanitized launch (`cowboy/startUserMcp`)

Handled inside `Host::request` and admitted through `execution_host::ledger::Ledger`:

1. Validates `!self.read_only`, `wait_ms <= MAX_WAIT_MS`, `current_workspace()`,
   `params.processId` matches `^cowboy-user-mcp-[0-9a-f]{32}$`,
   `params.serverName` matches `^[A-Za-z0-9_-]{1,64}$`, and
   `params.fingerprint` is 64 lowercase hex.
2. Re-resolves `serverName` from `$HOME/.claude.json` using the exact Section
   2.3 resolver and verifies `computed_fingerprint == params.fingerprint` (if
   `~/.claude.json` drifted between list and start, returns a clean refusal
   without starting a swapped entry).
3. Builds the native `process/start` parameters locally:
   - `processId`: validated `cowboy-user-mcp-<hex>`
   - `argv`: `[resolved_executable, ...expanded_args]`
   - `cwd`: `file://<keeper_state_dir>/user-mcp-cwd` (mode `0500` empty dir)
   - `env`: `{}`
   - `tty`: `false`, `pipeStdin`: `true`, `arg0`: `null`
   - `envPolicy`:
     - `inherit: "all"`, `ignoreDefaultExcludes: true`
     - `exclude`: explicit list of `COWBOY_CALL_CONTEXT`,
       `COWBOY_EXECUTION_FILE_HELPER`, plus EVERY `COWBOY_*` key present in
       `contract.environment` or the keeper process environment
     - `set`: the server's validated `env` entries
     - `includeOnly`: `[]`
4. Admits `invocation` into `execution_host::ledger::Ledger` (preserving
   idempotent `operation_id` deduplication AND recording `processId` in
   `ledger.processes` so `reap_unattended()` automatically cleans up orphaned
   user MCP processes if a worker vanishes for 30 min!).
5. On `Admission::New`, sends `{"id": id, "method": "process/start", "params": native_params}`
   directly to the native executor **WITHOUT calling `wire::with_call_context`**!

### 2.5 Target Machine Device config gate (`src/config/schema.rs` + `machine_cli/execution.rs`)

Add two live-reloadable Device settings (enforced by the target Machine
receiving `Action::Call` in `execution::Manager::call`, matching
`docs/configuration.md`):

- `execution.user_mcp` (`Kind::Bool`, default `true`, `Reload::Live`): master
  switch for target user MCP servers (`~/.claude.json`). When `false`,
  `execution::Manager::call` immediately answers `cowboy/listUserMcp` with
  `{ schema: 1, disabled: true, servers: [], omitted: [] }` and refuses
  `cowboy/startUserMcp` before reaching the keeper.
- `execution.user_mcp_allowlist` (`Kind::Identifiers { max_items: 32 }`,
  default `[]`, `Reload::Live`): when empty, all validated stdio servers from
  `~/.claude.json` are eligible; when non-empty, `execution::Manager::call`
  filters `cowboy/listUserMcp` responses to listed names (moving others to
  `omitted`) and refuses `cowboy/startUserMcp` for unlisted names.
- Rollback note in `Field.doc` (matching `diagnostics.heartbeat_gap_sampler`):
  if explicitly written into `device.toml`, remove the `[execution]` keys before
  rolling back to an older Machine release. When omitted (default), older
  Machines are unaffected.

### 2.6 Account-unit reachability fence (Resolves Medium 6)

In `src/worker_execution/ledger.rs` and `src/worker_execution.rs`:

1. Define `pub(super) const USER_SERVER_PREFIX: &str = "cowboy-user-mcp-";`
   alongside `SERVER_PREFIX = "cowboy-worker-mcp-"`.
2. `is_worker_server_process(id)` checks BOTH prefixes (`SERVER_PREFIX` and
   `USER_SERVER_PREFIX`) in:
   - `worker_event()` (execution events for user MCP servers NEVER reach the
     account unit);
   - `Ledger::admit()` (unit requests naming either prefix are refused with
     `"that process belongs to the worker; request refused"`).
3. `Ledger::admit()` (and `serve_mutation` before forwarding) locally **refuses**
   any unit-originated request whose `method` starts with `"cowboy/"` other than
   the explicitly handled local endpoints (`cowboy/extensions`,
   `cowboy/extensionCall`, `cowboy/extensionCallAck`, `cowboy/uncertainEffect`,
   `cowboy/drained`) — so the account unit can NEVER directly invoke
   `cowboy/listUserMcp` or `cowboy/startUserMcp`.

---

## 3. Zero-Schema-Migration Durable Tracking & Orphan Cleanup (Resolves High 5 & Medium 3)

### 3.1 Pending call durability in Schema-1 `ledger.json` (no schema bump!)

Note: the worker's execution ledger lives on the **runtime Machine** at
`<state>/execution-ledgers/<session_id>.json` (`src/account_sandbox.rs:114-134`),
NOT inside `<keeper_state_dir>` (resolves Low finding).

Rather than bumping `ledger.json` from Schema 1 to Schema 2 (which caused
per-session fallback breakage in Round 1 High 5), R2 stores in-flight durable
user-MCP call sentinels directly inside the existing Schema-1
`record.active_processes: BTreeSet<String>` using reserved prefix
`cowboy-pending-call:<call_id>`:

- **Pre-dispatch**: before writing `tools/call` to a user MCP server's stdin,
  the worker allocates a 128-bit hex `call_id`, inserts
  `"cowboy-pending-call:<call_id>"` into `record.active_processes`, and
  atomically persists `ledger.json` (still `schema: 1`!). If the write fails,
  the call is refused before dispatch (`Failure::NotSent`).
- **On worker crash or rollback to ANY older Schema-1 worker generation**
  (automatic, manual, or per-session fallback):
  - An **older worker** (`worker-11af6d9b0b7925e98a9d` or earlier) opens
    `schema: 1` cleanly, sees `!record.active_processes.is_empty()`, atomically
    clears `active_processes`, sets `uncertain_effect` and advances
    `uncertain_sequence` (`ledger.rs:271-281`) — zero `"unsupported execution
    ledger schema"` crash!
  - A **new R2 worker** reopening `ledger.json` distinguishes
    `cowboy-pending-call:*` entries from process IDs: if only pending-call
    sentinels exist it records `uncertain_effect = Some("tools/call")`; if
    process IDs also exist it records `"process/terminate"`; in both cases it
    clears `active_processes` and atomically advances `uncertain_sequence`.
- **On `mark_unresolved_effects_uncertain` (SIGTERM cutoff)**: if any
  `cowboy-pending-call:*` sentinel or unacked call remains, records
  `uncertain_effect` (`"process/terminate"` if turn processes remain, else
  `"tools/call"`) atomically.
- **No Phase-A / Phase-B reader-floor deadlock**: Schema stays `1`, so every
  existing rollback tier is immediately compatible with zero two-phase schema
  migration!

### 3.2 Retained-keeper orphan user-server cleanup via sibling file (Resolves Medium 3)

Because worker-owned MCP servers are never a session effect
(`ledger.rs:1301`: *"the worker's own servers are never a session effect"*),
running user-MCP server IDs are **not** stored in `active_processes` (which
would falsely park sessions on normal worker restarts). Instead, they are
tracked in a bounded sibling file next to the ledger:
`<ledger_path>.user-servers.json` (mode `0600`, max `64 KiB`, max `32` records):

```json
{
  "schema": 1,
  "servers": [
    {
      "process_id": "cowboy-user-mcp-<32-hex>",
      "server_name": "chrome-devtools",
      "scope": { "binding_id": "...", "revision": 1, "incarnation": "..." },
      "start_confirmed": true
    }
  ]
}
```

Lifecycle invariants (incorporating the three L2 start-tracking rules from
`docs/uncertain-effect-recovery.md:468-484`):

1. **Persist before start**: before issuing `cowboy/startUserMcp`, atomically
   persist the entry with `start_confirmed: false` (enforcing the 32-entry /
   64 KiB cap).
2. **Classified start with surviving completion owner**:
   - `cowboy/startUserMcp` runs via `invoke_classified` inside a spawned
     completion task that **survives** discovery timeout cancellation (so
     cancelling discovery never leaves a process stuck in `ledger.starting`!).
   - On `Ok(processId)` → update sibling entry to `start_confirmed: true` and
     call `ledger.server_started(process, true)`.
   - On `Err(maybe_sent = false)` (definitive refusal) → remove sibling entry
     and call `ledger.server_started(process, false)`.
   - On `Err(maybe_sent = true)` or discovery timeout → clear `starting` while
     keeping the process tracked for termination (`start_confirmed` stays
     `false`), trigger bounded cleanup termination of that process, disable that
     server name for this worker epoch with a bounded availability note, and do
     **NOT** park the session (optional user MCP failure never wedges a session
     before turn 1).
3. **Reopen orphan sweep before new user-server starts**:
   - On `Broker::start()`, read `<ledger_path>.user-servers.json`.
   - Entries whose `scope != Scope::from_binding(&client.binding)` belong to a
     retired/closed incarnation and are pruned immediately.
   - Entries matching the connected keeper's `scope` are terminated via
     `process/terminate` + `process/read` within `USER_SERVER_STOP_CONFIRM_MS = 10s`:
     - If `start_confirmed == true`: `closed` OR `E1 forgotten` confirms exit.
     - If `start_confirmed == false`: `E1 forgotten` is **NOT** accepted as
       closure proof (a queued `start` resend could still land!); only confirmed
       `terminate` + `closed` removes the record.
     - If termination is unconfirmed within 10 s, that `server_name` stays
       disabled for this worker epoch (no duplicate copy launched; session is
       not parked) and the record is bounded-pruned once the scope changes or a
       later sweep confirms exit.
4. **Clean worker teardown**: terminates all live user servers (`park_on_unconfirmed = false`),
   and removes confirmed-closed entries from `<ledger_path>.user-servers.json`.

---

## 4. Bounded Correlated Proxy Ack (`cowboy/extensionCallAck`) (Resolves High 6 & Medium 1)

### 4.1 Scope & legacy-proxy compatibility

- Correlated acks apply **strictly** to `Route::UserServer` calls. Cowboy MCP,
  required folder bindings (`suger`), and Matrix memory keep their existing
  protocol unchanged so retained older Antigravity Plugin generations (`0.2.39`)
  keep working with zero disruption.
- In `cowboy/extensions`, the proxy sends `{ "ackProtocol": 1 }`.
  - If `ackProtocol >= 1` is absent, `cowboy/extensions` omits all
    `Route::UserServer` tools (adding a bounded upgrade note) and
    `cowboy/extensionCall` refuses any `Route::UserServer` call before dispatch
    (`Failure::NotSent`).

### 4.2 Top-level frame field & spoofing resistance

- On `Route::UserServer` `tools/call`:
  - **`Failure::NotSent`**: remove `cowboy-pending-call:<call_id>` from
    `ledger.json` immediately, release `SettlementGuard`, return plain refusal
    (no ack required).
  - **`Failure::MaybeSent`**: atomically remove `cowboy-pending-call:<call_id>`
    AND record `uncertain_effect = Some("tools/call")` + `next_sequence(...)` in
    one atomic `ledger.json` write, release `SettlementGuard`, return
    `{ error: { code: -32000, message: "...", data: { outcome: "unknown" } } }`.
  - **`Ok(result)` OR `Failure::Refused(reason)`** (server actually processed the
    call and returned a result or MCP error): keep `cowboy-pending-call:<call_id>`
    durably in `ledger.json`, register an in-memory `PendingAck` holding the
    call's `SettlementGuard`, and attach **`"cowboyAck": { "callId": "<32-hex>" }`**
    as a **top-level member of the JSON-RPC execution reply frame** (sibling to
    `id` and `result`/`error`, NEVER inside `result`!).
- On the proxy (`components/provider-runtime/execution-connection.mjs`):
  - `Connection.request` reads top-level `frame.cowboyAck` cleanly and exposes
    it to `Extensions.call` on both resolved results (`requestWithMeta`) and
    rejected remote errors (`error.cowboyAck`).
  - A malicious MCP server returning `{"cowboyAck": ...}` or `{"_cowboyAck": ...}`
    inside its tool output can NEVER spoof or trigger an ack because the proxy
    only inspects the top-level execution frame member produced by the worker.

### 4.3 Dedicated ack capacity `>= MAX_CALLS` (8) and worker wiring

- **Worker `MutationInflightIds` (`src/worker_execution.rs`)**: adds a third
  independent admission set `ack: HashSet<String>` with `MAX_ACK = 8` (matching
  `MAX_CALLS = 8`), completely separate from `MAX_CONTROL = 4` (`uncertainEffect`,
  `drained`) and `MAX_FORWARDED = 16`.
- **Proxy `Connection` (`components/provider-runtime/execution-connection.mjs`)**:
  adds `cowboy/extensionCallAck` to `DRAIN_REQUESTS` with its own `kind = "ack"`
  slot class (`MAX_ACK = 8`), separate from `control` (4) and `forwarded` (16).
- **`src/account_sandbox.rs`**: adds `"cowboy/extensionCallAck"` to
  `cleanup(method)` so draining units can ALWAYS acknowledge delivered replies
  during graceful worker cutover/shutdown.
- **`src/worker_execution.rs` (`serve_mutation`) & `ledger.rs`**: handles
  `"cowboy/extensionCallAck"` locally BEFORE `ledger.admit()` (never forwarded
  to the keeper).
- **Zero duplicate L1c accounting**: in `src/worker_execution/relay.rs`,
  `Relays::delivered` skips recording `unread_effects` for replies that carry
  top-level `cowboyAck` because those calls are already tracked durably via
  `cowboy-pending-call:<call_id>` and converted to uncertain on socket close or
  ack deadline expiry.

### 4.4 Bounded ack deadline (`EXTENSION_ACK_TIMEOUT = 15s`)

Once the worker writes a reply frame carrying `cowboyAck: { callId }` to the
execution WebSocket, a bounded `15 s` timer runs for `callId`. Exactly ONE of
four terminal transitions settles the entry and drops its `SettlementGuard`:

1. **Valid `cowboy/extensionCallAck({ callId })`**: atomically removes
   `cowboy-pending-call:<call_id>` from `ledger.json`, drops `SettlementGuard`,
   and replies `{"result": {"acknowledged": true}}`.
2. **Proxy reports `cowboy/uncertainEffect` with matching `callId`**: atomically
   removes `cowboy-pending-call:<call_id>`, records `uncertain_effect = Some("tools/call")`,
   and drops `SettlementGuard`.
3. **Socket close OR `15 s` ack timeout**: atomically removes
   `cowboy-pending-call:<call_id>`, records `uncertain_effect = Some("tools/call")`
   + `next_sequence(...)`, and drops `SettlementGuard`. Thus `finish_turn` /
   `mutations_drained` / `TurnEnded` can **never** hang indefinitely!
4. **Late/unknown `callId` on `cowboy/extensionCallAck`** (e.g., arrived after
   timeout or reconnect): worker answers cleanly `{"result": {"acknowledged": false, "reason": "unknown_call_id"}}`
   without breaking the socket; proxy treats `acknowledged !== true` or any ack
   transport/timeout error as `UncertainEffect("tools/call")` and fences the turn.

---

## 5. Discovery Budgets & Namespace Isolation (Resolves Medium 4)

In `src/worker_execution/extension_broker.rs`:

1. **Phase 1 (Required folder bindings + Cowboy MCP + Matrix memory) keeps its
   full existing `40 s` budget (`DISCOVERY_TIMEOUT = 40s`) and existing
   per-server `MAX_TOOLS = 128` limit unchanged**:
   - Required bindings (`plan.required == true`) are never shortened to 20 s and
     never displaced by optional user MCP servers.
   - All Phase-1 tool names (plus `FACADE_TOOLS` and reserved Cowboy MCP /
     memory tool names) are claimed in `taken` BEFORE Phase 2 runs.
2. **Phase 2 (Optional user MCP servers) runs after Phase 1 succeeds with its
   own independent bounded budget**:
   - Only runs when `binding.managed.is_none()`, keeper `initialization`
     advertises `"cowboyUserMcp": { "schema": 1 }`, and `listUserMcp` returns
     servers.
   - Runs orphan cleanup first (Section 3.2), then launches admitted user
     servers in parallel with bounded concurrency (`MAX_USER_MCP_CONCURRENCY = 4`,
     per-server init timeout `USER_MCP_SERVER_INIT_TIMEOUT = 10s`, overall Phase
     2 cap `USER_MCP_DISCOVERY_TIMEOUT = 15s`) — preventing 2 slow servers from
     starving the remaining servers.
   - Dedicated user-MCP tool caps: `MAX_USER_MCP_TOOLS_PER_SERVER = 64`,
     `MAX_USER_MCP_TOOLS_TOTAL = 128` (independent of Phase 1).
   - **Mandatory `<server>_` prefix**: user MCP server tools are ALWAYS exposed
     as `<server>_<original>` (never bare `<original>`), capped at 64 chars
     matching `^[A-Za-z0-9_-]{1,64}$`, so a user server can never claim a bare
     Cowboy MCP, memory, or facade tool name even when Cowboy MCP or memory
     failed to start.
   - Any user server failure/timeout adds a bounded availability note (no
     secrets/paths) and never fails session startup.

---

## 6. Mixed-Version Compatibility Matrix & Single Clean Rollout (Resolves Medium 6)

Because R2 introduces **zero** breaking changes to `ledger.json` schema (`schema == 1`),
`LaunchContract`, `StartSession`, `SessionExtensions`, `execution_protocol::Command`,
or `machine_protocol::execution::Action`:

| Runtime Worker | Target Keeper | Antigravity Proxy | Behaviour |
|---|---|---|---|
| Old (`worker-11af6d9b`) | New | Old (`0.2.39`) | Old worker ignores `cowboyUserMcp`; 100% unchanged |
| New | Old (retained) | New (`0.2.40`) | Old keeper omits `cowboyUserMcp` → worker skips user MCP with note; `cowboy/startUserMcp` never sent (and would return `-32601`) |
| New | New | Old (`0.2.39`) | Old proxy omits `ackProtocol: 1` → worker omits user MCP with upgrade note; Cowboy MCP / `suger` / memory unaffected |
| Old (`worker-11af6d9b`) | Old or New | New (`0.2.40`) | Old worker returns no `cowboyAck` → proxy sends no ack; 100% compatible |
| New → Rollback to Old | New | Any | Schema-1 `ledger.json` opens cleanly; any `cowboy-pending-call:*` in `active_processes` atomically parks session; `.user-servers.json` ignored and keeper reaps |
| New | New | New (`0.2.40`) | Full R2 active: target-local `~/.claude.json` stdio user MCP with neutral cwd, `COWBOY_*` stripped, `<server>_` tools, durable acks |

### Rollout order

1. **PR / Push `R` + Worker Pin Advance `P` + Acceptance Receipt `P2`** (one
   clean pin advance; passes all 6 independent worker acceptance suites +
   existing `just session-deletion-production-conformance` per
   `docs/machine-worker-pin-advance-runbook.md`).
2. **Deploy Target Machine Hosts (Hawk writer host + Falcon host)**: activates
   `cowboy-execution-host` with `cowboyUserMcp` + `execution.user_mcp` Device
   config (new sessions get new keepers; existing retained keepers remain
   compatible).
3. **Deploy OVH Machine + Workers** (`activate-workers` / `accept-workers`,
   sync `machines/ovh/cowboy/10-plugin-admission.conf` in Columbus).
4. **Publish & install `antigravity 0.2.40`** (and companion `3.156.0` plugins)
   to `/var/lib/cowboy/plugins/catalog` and activate on OVH.
5. **Live OVH ↔ Hawk Canary Acceptance**:
   - Verify a fresh OVH `antigravity 0.2.40` session against a Hawk Columbus
     worktree discovers and invokes a real `~/.claude.json` stdio MCP server
     (e.g., `chrome-devtools` or a disposable canary stdio server), confirming:
     - `COWBOY_CALL_CONTEXT` and `COWBOY_*` absent from child env;
     - `cwd` is neutral mode-`0500` `user-mcp-cwd` (not worktree);
     - tool exposed with `<server>_` prefix;
     - correlated `cowboy/extensionCallAck` clears `cowboy-pending-call:*` with
       zero `UncertainEffect`;
     - `execution.user_mcp = false` kill-switch cleanly suppresses user MCP.

---

## 7. Required Test Matrix

1. **Keeper (`components/machine/src/execution_host/user_mcp.rs` + `execution_host.rs`)**:
   - `~/.claude.json` user + `projects[source_path]` + symlinked `source_path`
     precedence + `disabledMcpServers`; `.mcp.json` ignored.
   - Rejects `./`/`../` commands, executables resolving inside worktree/source
     root, worktree path args, non-stdio entries, `COWBOY_*` and loader env
     injection keys (`LD_PRELOAD`, `NODE_OPTIONS`, etc.).
   - `cowboy/listUserMcp` response contains zero `env` values, zero `args`,
     zero executable paths; fingerprint rejects drift between list and start.
   - `cowboy/startUserMcp` launches with neutral mode-`0500` `user-mcp-cwd`,
     NEVER calls `with_call_context`, excludes `COWBOY_CALL_CONTEXT` &
     `COWBOY_*`, and records `processId` in keeper ledger for `reap_unattended`.
   - Managed (`read_only`) keeper refuses `cowboy/listUserMcp` &
     `cowboy/startUserMcp` and omits `cowboyUserMcp` from `initialization`.
   - Target Machine `execution::Manager` live `execution.user_mcp` kill-switch
     and `execution.user_mcp_allowlist` filtering.
2. **Worker (`src/worker_execution/{ledger,extension_broker,relay,tests}.rs`, `src/account_sandbox.rs`)**:
   - Schema-1 `cowboy-pending-call:*` in `active_processes`: crash/reopen by
     new worker records `uncertain_effect = "tools/call"` + monotonic sequence;
     simulated legacy Schema-1 reader opens cleanly and parks session.
   - Correlated `cowboy/extensionCallAck`: happy path (`Ok` and `Failure::Refused`),
     `15 s` timeout converts to uncertain and releases `SettlementGuard`
     (unblocking `finish_turn` / `TurnEnded`), socket close before ack parks,
     ack accepted during `RelayQuiet` drain (`cleanup`), 8 concurrent acks pass
     under `MAX_ACK = 8` without starving `MAX_CONTROL = 4`, late/unknown
     `callId` returns `{ acknowledged: false }`, zero duplicate L1c accounting.
   - Legacy proxy (no `ackProtocol: 1`) omits user MCP while preserving Cowboy
     MCP / required bindings / memory.
   - Retained legacy keeper (no `cowboyUserMcp`) skips user MCP with note.
   - Account unit refused for `cowboy/listUserMcp`, `cowboy/startUserMcp`, and
     both `cowboy-worker-mcp-` / `cowboy-user-mcp-` prefixes; `worker_event()`
     filters both prefixes.
   - Sibling `.user-servers.json` orphan sweep: confirmed start accepts
     `closed`/`forgotten`, unconfirmed start rejects `forgotten`, different
     keeper scope pruned, unconfirmed stop disables server name without parking,
     capped at 32 entries.
   - Discovery isolation: Phase 1 keeps 40 s + `MAX_TOOLS = 128`; slow user
     servers never starve Phase 1 or other user servers; `<server>_` prefix
     cannot shadow Cowboy MCP / memory / facade names.
3. **Antigravity Proxy (`components/provider-runtime/packages/antigravity-acp/`)**:
   - Sends `ackProtocol: 1` on `cowboy/extensions` and `cowboy/extensionCall`.
   - Reads top-level `frame.cowboyAck` (ignoring any spoofed `result._cowboyAck`
     or `result.cowboyAck`), sends `cowboy/extensionCallAck` on both success and
     refused errors before returning tool output.
   - Ack failure / `{ acknowledged: false }` throws `UncertainEffect("tools/call")`.
   - `cowboy/extensionCallAck` runs during `Connection.idle()` drain and uses
     its own 8-slot `ack` lane independent of `control` (4) and `forwarded` (16).
   - Old worker (no `cowboyAck` in reply) works identically to `0.2.39`.
