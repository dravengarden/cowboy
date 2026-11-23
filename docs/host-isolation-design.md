# Host isolation: a control identity and a task cgroup subtree

Status: **design, revision 1, not implemented.** Nothing here is activated on
any host. Every step that touches networking, firewall rules, sudo policy or a
deployment activation needs explicit owner confirmation before switching (see
[Flags](#flags-needing-explicit-owner-confirmation)).

## Why

On Hawk, all of the following run as `draven` (UID 1000):

- the Controller (system `cowboy.service`, `User=draven`);
- the Machine broker (user `cowboy-machine.service` in `user@1000`'s `app.slice`);
- the ACP workers (`cowboy-worker-*`);
- the execution keepers (`cowboy-execution-*`);
- every agent tool process.

The workers and keepers are transient units in one user slice,
`cowboy.slice/cowboy-agents.slice`. That slice has a memory cap and no CPU
weight. Three consequences follow:

1. **Secrets.** A tool process can read every Controller and Machine secret:
   - the Controller database (`/var/lib/cowboy/cowboy.sqlite3`);
   - the provider vault and its signing key
     (`/var/lib/cowboy/provider-auth-service/{vault.key,signer}`,
     `components/store/src/provider_service.rs:191-221`);
   - the Machine identity and provider replicas
     (`~/.local/state/cowboy-machine/provider-auth/`, `materialized/`).
2. **Forgery.** A tool process can forge any role on the local sockets. The
   broker socket trusts the `role` in the hello (`PeerRole::Core` attaches as
   the Controller lease, `machine_broker.rs:4131-4184`). The keeper, code,
   Zed and usage-spool sockets rely only on directory modes. The managed-call
   gateway checks `peer uid == euid` (`managed_calls/gateway.rs:143-160`),
   which every tool process satisfies.
3. **Starvation.** A tool process can starve the heartbeat path. Workers beat
   every 10 s. The broker isolates after `WORKER_HEARTBEAT_TIMEOUT=45s`, swept
   every 15 s (`machine_broker.rs:44-45`). Under load above 400 this produced
   the observed 48-58 s isolations and sessions with uncertain effects.

Same-UID equality also makes three things impossible:

- an authenticated delivery origin (blocking Antigravity round 3:
  `docs/antigravity-parity-round-3.md` on `cowboy/sess-1795399435396`, commit
  `4eed6b2d0`, "Owner decisions" item 5, review
  `call-1517db7ae578f9050a9232d4`);
- a guarded broker `Permission`/`SetConfigOption`;
- any user-MCP sandbox.

## Threat model

**Protected against.** A model-driven process running with the agent identity,
including one that is prompt-injected. It may run arbitrary code as the agent
UID, write anything the agent UID can write, and generate unlimited CPU,
memory, I/O and process load inside the task subtree.

**Assets:**

- **A1, control secrets.** Controller DB and artifacts, provider vault and
  signer, Machine identity key, provider replicas and materialized
  generations, Cardea/OIDC client secrets, the operator grant.
- **A2, control authority.** Claiming `Core` or `Worker` on the broker. Acting
  as the Controller to a Machine, or as a Machine to the Controller. Starting
  units through the spawner or the account sandbox.
- **A3, origin.** The kernel-attested identity of a caller (control code, or
  the agent of session S), on every control socket an agent can reach.
- **A4, liveness.** The worker↔broker heartbeat, the Machine↔Controller link,
  Controller HTTP, and the Stormbird data path keep their CPU, memory and I/O
  under task load.

**Not protected (stated residuals):**

- **R1, granted deployment authority.** Agents are allowed to build and
  activate machine generations and Cowboy components (owner deploy
  preference). Activating a closure the agent built is root and control
  authority by construction. The boundary makes that path explicit: it goes
  through the machine transaction or a component activator, which records
  receipts, pinned revisions and journal entries. It no longer happens through
  ambient file or socket access. Closing R1 needs owner-gated activation
  ([D3](#owner-decisions-needed)).
- **R2, cross-session reads inside the agent UID.** All sessions' agent units
  share the agent UID. One session's tools can read another session's
  worktree and materialized provider home: `/proc/<pid>/root` needs only
  `PTRACE_MODE_READ`, which Yama `ptrace_scope=1` does not restrict. Closing
  R2 needs per-account or per-session UIDs, as `cga-<key>` already does on
  OVH. That is the [follow-up F-UID](#follow-ups).
- **R3, agent escape from the task budget, not from the identity.** A tool
  can still use `systemd-run --user`, rootless Docker or `nohup` under
  `user@1000`. That moves load out of `cowboy-tasks.slice` but not into the
  control subtree: weights still favour control. It is treated as an
  accounting gap, not a liveness break ([Cgroups](#cgroup-layout)).
- **R4, the kernel and shared filesystems.** Kernel exploits, and filling a
  filesystem that control state shares. Control state moves to `/var/lib`
  (root fs), while task data stays on `fast0`/ZFS, so the fill case shrinks.
  Kernel exploits stay out of scope.

## Design summary

| Element | Decision |
|---|---|
| Control identity | New static system user and group `cowboy` (no login shell, no sudo, no supplementary groups) on Hawk and OVH |
| Agent identity | Hawk: `draven` (D1 default). OVH: `ubuntu`, unchanged |
| Runs as `cowboy` | Controller, `cowboy-plugin-converge`, `cowboy-model-intel-audit`, Machine (broker, Controller link, usage spool, code/Zed adapters), ACP workers (`cowboy-acp-worker`), execution keepers (`cowboy-execution-host`), signed Plugin MCP servers |
| Runs as agent | Provider adapters (codex-acp, claude-agent-acp, …) and all their tool children; `codex exec-server` and its commands; Machine browser; the new agent-side helper; future user-MCP sandboxes run under a further UID |
| Unit creation across identities | One root spawner, `cowboy-spawner`: a generalization of OVH's `cowboy-account-sandbox`. Socket-activated, authorizes by `SO_PEERCRED`/`SO_PEERPIDFD`, closed profile set |
| Peer authentication | Every Unix control socket checks the kernel peer: uid always, and the cgroup through a pidfd when the role is session-bound |
| Filesystem rule | Control identity never opens paths in agent-writable trees. It talks to an agent-identity helper over a pipe ([R-CD](#r-cd-no-confused-deputy)) |
| Cgroups | System-manager slices: `cowboy-control.slice` (protected) and `cowboy-tasks.slice` (budgeted), both directly under `-.slice` |

The process split is the smallest one that meets goal (1): the heartbeat sender
(the worker) must not share a UID with the tools. Today the provider adapter is
the worker's child (`src/acp_process.rs:110-127`), and its tools sit in a leaf
cgroup of the worker unit (`src/cgroup.rs:37-62`). The split moves the adapter
into its own agent unit. The worker keeps the adapter's stdio, which ACP
already speaks, so the wire is unchanged.

## Identities

### `cowboy` (control)

- A static system UID/GID allocated in the NixOS module (Hawk) and by the
  maintenance script (OVH). It does not use `DynamicUser`, because its state
  is durable and its UID appears in peer checks and ACL-free modes.
- It has no supplementary groups. The agent UID is never in group `cowboy`.
- It owns:
  - `/var/lib/cowboy` (Controller, 0750 → 0700);
  - `/var/lib/cowboy-machine/<service-id>` (Machine state, moved from
    `~/.local/state/cowboy-machine`);
  - `/run/cowboy-machine` (0700; broker `m`, usage spool `u`, code `c`, Zed
    `z`, keeper sockets, replacing `/tmp/cowboy-execution-1000` and
    `%t/cowboy`).
- Units use `ProtectHome=yes`, `ProtectSystem=strict` with explicit
  `ReadWritePaths`, `NoNewPrivileges=yes`, `PrivateTmp=yes`. The Hawk module's
  stated reason for running as `draven` ("workspace access",
  `services/cowboy/default.nix` header) disappears under R-CD: control code no
  longer needs workspace access.

### Agent identity

- **Hawk.** D1 default is `draven`: worktrees, SSH keys, `gh`, Git identity,
  rootless Docker and the Nix profile all keep working unchanged.
  - The alternative D1b is a new `cowboy-agent` UID. It would also separate
    the human's own credentials (`~/.ssh`, `~/.config/cowboy/client-auth`,
    shell rc files) from agents, at the cost of a full chown of
    `/srv/storage/fast0/agent` and separate GitHub/SSH credentials.
  - Under D1, any credential in `draven`'s home is agent-readable by
    construction. That includes the human's Cowboy CLI credential and
    signed-in Chrome profiles. Owner-class Cowboy authority must therefore
    never be stored there ([D1](#owner-decisions-needed)).
- **OVH.** `ubuntu` (UID 1000) stays the agent identity. It already has no
  sudo; `ovh-operator` is the admin. The Machine moves from `ubuntu`'s user
  manager to `cowboy`.

### What the agent user loses (Hawk, D1)

| Today | After | Why |
|---|---|---|
| `sudo` NOPASSWD `ALL` | NOPASSWD only for an allowlist: `machine-activate` (via `just sys-activate`), `cowboy-{web,controller,machine}-activate`, read-only `journalctl`, `systemctl status`/`show`, and the existing `sudo -u` paths the activators themselves use as root. Anything else needs the admin path (D2) | Root reads any secret; NOPASSWD ALL makes the UID split decorative |
| Nix `trusted-users` | Ordinary `allowed-users` | A trusted user can import unsigned store paths and set substituters, which is root-equivalent. `nix build`, `nix develop` and `just sys-build` keep working as an ordinary user (the deployment doc already builds as the normal user) |
| Supplementary group `root` (and review `argus`) | Removed | Group-root-readable files would leak |
| Reading `/var/lib/cowboy`, Machine state, provider replicas, the operator grant | Denied | A1 |
| `systemctl --user` control of `cowboy-machine`, `cowboy-worker-*`, `cowboy-execution-*` | Gone (they become system units owned by `cowboy`). Read status through `journalctl`/`systemctl status` (allowlisted) and Cowboy MCP | A2 |
| `cowboy operator` as the Service account | Through `sudo -u cowboy cowboy operator …` only if D4 allowlists it; otherwise the owner's admin path | The operator socket is Admin authority |
| Killing, ptracing or reading `/proc/<pid>/environ` of workers, the Machine or the Controller | Denied by the kernel (different UID) | A2, A4 |

The agent keeps the following:

- editing worktrees;
- `git`, `ssh`, `gh`, Nix builds and dev shells;
- rootless Docker;
- user services in `user@1000`;
- the Chrome debug bridge;
- the deployment transactions, through the allowlist;
- Cowboy MCP and the CLI through the managed-call gateway.

The human `draven` loses the same sudo breadth unless the owner takes the
admin path in D2.

## Processes, units and cgroups

### Cgroup layout

All Cowboy units move to the **system** manager. They no longer live in
`user@1000`, so control placement no longer depends on a user manager the
agent UID controls. `user@1000` already carries the retained
`restartIfChanged=false` pair for this reason.

```text
-.slice
├── system.slice                      MemoryLow=2G (unchanged)
│   ├── cowboy.service                Controller, User=cowboy, CPUWeight/IOWeight=1000, MemoryLow=1G
│   ├── cowboy-spawner.service        root, socket-activated
│   ├── cowboy-account-sandbox.*      (OVH only, root; merged into spawner later)
│   └── stormbird-device.service      unchanged
├── cowboy-control.slice              CPUWeight=1000, IOWeight=1000, MemoryLow=<sum, see D5>
│   ├── cowboy-machine.service        User=cowboy, MemoryLow=1G
│   ├── cowboy-worker-<h>-<sess>.service      User=cowboy, Restart=no, no Delegate
│   └── cowboy-keeper-<incarnation>.service   User=cowboy
├── cowboy-tasks.slice                CPUWeight=100, IOWeight=100, MemoryHigh=48G, MemoryMax=64G,
│   │                                 MemorySwapMax=4G, TasksMax=<D5>
│   ├── cowboy-agent-<sess>.service   User=<agent>, Delegate=yes (provider + tools)
│   ├── cowboy-exec-<incarnation>.service     User=<agent>, MemoryMax=24G, MemorySwapMax=2G, OOMPolicy=continue
│   ├── cowboy-browser-<instance>.service     User=<agent>, Delegate=yes
│   ├── cowboy-agent-helper.service   User=<agent> (workspace bridge, below), CPUWeight=1000 within slice
│   └── cowboy-sandbox.slice          reserved for future user-MCP units (below)
└── user.slice                        draven's logins and user@1000 (dev servers, rootless Docker)
```

The critical-resources policy is preserved and extended:

- `cowboy.service` and `cowboy-machine.service` keep `CPUWeight=1000`,
  `IOWeight=1000` and `MemoryLow=1G`. Their ancestors (`system.slice`,
  `cowboy-control.slice`) carry `MemoryLow` at least the sum of their
  protected children. No ancestor of a control unit carries `MemoryHigh` or
  `MemoryMax`.
- The heartbeat sender (the worker) is now in the protected subtree. At the
  root level, `cowboy-control.slice` at weight 1000 competes with
  `cowboy-tasks.slice`, `user.slice` and `system.slice` at 100 each. That gives
  control at least ~75% of the CPU whenever it is runnable. Today the workers sit at
  equal weight beside the commands.
- The task memory budget moves unchanged from `cowboy-agents.slice` to
  `cowboy-tasks.slice`.
- The documented execution-isolation gap (keeper, executor and commands in
  one `cowboy-execution-*` unit) closes for the keeper. The keeper is control;
  `codex exec-server` and its commands are the task unit. The exec-server
  stays with its commands. A stalled exec-server affects only its own session's
  commands, never the heartbeat path. The Columbus deletion trigger for
  `critical-resources.nix` is met once the Cowboy module provides these
  slices; the module then shrinks to its assertions.
- The Columbus assertions update in the same Columbus change:
  - `cowboy-machine` must be in `cowboy-control.slice`;
  - `cowboy` must be in `system.slice`;
  - no task unit may be in `cowboy-control.slice`;
  - the deployment check uses system paths, not the hard-coded UID-1000 user
    paths (`critical-resources.nix:20`).

R3 (tools escaping to `user@1000`) leaves control protected, because
`user.slice` is a weight-100 sibling of `cowboy-control.slice`. The extended
[acceptance](#acceptance) measures it explicitly.

### Unit split per session

| Unit | Identity | Contents | Started by |
|---|---|---|---|
| `cowboy-worker-<h>-<sess>` | `cowboy` | `cowboy-acp-worker`: broker connection, heartbeats, ACP framing, the delivery ledger | Machine, through the spawner profile `worker` (detached, as today: survives Machine restarts) |
| `cowboy-agent-<sess>` | agent | The `cowboy-agent-helper` init, then the provider adapter and all tool children | Worker, through the spawner profile `agent` (attached: stdio pipes and a helper control pipe come back over `SCM_RIGHTS`; closing the spawner connection stops the unit, as the account sandbox does) |
| `cowboy-keeper-<inc>` | `cowboy` | `cowboy-execution-host`: keeper socket, the contract and its lifetime | Machine, spawner profile `keeper` (detached) |
| `cowboy-exec-<inc>` | agent | `cowboy-agent-helper`, then `codex exec-server --listen stdio` and its commands | Keeper, spawner profile `exec` (attached) |

The attached lifetime matches today's: the adapter dies with its worker and
the exec-server with its keeper. Detached lifetimes are unchanged, so
Machine-restart retention and reboot convergence keep their current shape:
the Machine re-adopts workers and keepers by unit name after a restart. A
missing agent or exec unit is the same condition as a dead adapter child is
today.

Containment becomes fail-closed. Today the adapter runs uncontained when its
leaf cgroup cannot be created (`src/cgroup.rs:14-17`). Under the split, the
spawner refuses to report success unless the unit is active in its slice.

### `cowboy-spawner` (root)

This is a generalization of `packaging/account-sandbox`. It reuses that
package's unit-property code, `SCM_RIGHTS` return and close-to-stop
semantics; on OVH the account sandbox becomes the spawner's `account` profile.

- Socket `/run/cowboy-spawner/control.sock`, mode `root:cowboy 0660`.
- On accept it reads `SO_PEERCRED` and `SO_PEERPIDFD` (Linux ≥ 6.5; Hawk runs
  6.18, and OVH needs 26.04's ≥ 6.17, checked by the installer). It
  authorizes only `uid == cowboy`, and only when the pidfd's cgroup is
  `cowboy-control.slice/cowboy-machine.service` or a
  `cowboy-control.slice/cowboy-{worker,keeper}-*.service`. The account
  sandbox today reads the peer but does not authorize with it
  (`packaging/account-sandbox/src/server.rs:31-38`).
- **Closed profile set.** `worker`, `keeper` (User=cowboy,
  `cowboy-control.slice`); `agent`, `exec`, `browser`, `helper`
  (User=<agent>, `cowboy-tasks.slice`); `account` (`cga-<key>`, OVH); and a
  reserved `sandbox` that refuses until user MCP is designed. The caller
  supplies only:
  - the executable, from an allowlisted, digest-pinned store path or
    generation, or the provider command already listed in the Machine's
    configuration;
  - argv;
  - an environment filtered by the profile's allowlist;
  - a working directory, which must sit below a configured worktree root for
    agent profiles;
  - the session or incarnation id, which derives the unit name.

  A caller can never choose `User=`, `Slice=`, capabilities, `ReadWritePaths`
  or the root of a bind mount.
- Agent profiles do not set `NoNewPrivileges`, so the allowlisted `sudo`
  still works from tools. They do not use `ProtectHome`, because the
  developer workflow needs `$HOME`. They mount `TemporaryFileSystem=` over
  `/var/lib/cowboy`, `/var/lib/cowboy-machine` and `/run/cowboy-machine` as
  defense in depth; the modes alone already deny those paths.
- **Credentials.** Materialized provider credentials and per-session tokens
  reach an agent unit only through `SetCredential=` / `LoadCredential=` from a
  control-owned staging file. They land in `$CREDENTIALS_DIRECTORY`, which is
  readable only by that unit's user. The helper copies them into the
  provider home.

### `cowboy-agent-helper` (agent identity)

A small binary that runs first in every agent-identity unit. It owns all
filesystem access to agent-writable trees on behalf of control code:

- per unit: it prepares the provider home (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`
  from credentials), execs the adapter or exec-server as its child, and
  serves a control pipe to its starter;
- as the long-lived `cowboy-agent-helper.service`: the Machine's workspace
  bridge. It handles worktree preparation (`git fetch`, `git worktree add`),
  workspace listing, remote file buffers, code reads for review and Code
  Intelligence, credential-home observation for token refresh, and session
  cleanup of agent-owned paths.

The existing keeper file helper (`COWBOY_EXECUTION_FILE_HELPER`) is the
precedent; this widens it to one protocol.

### R-CD: no confused deputy

Control code never opens, stats, reads, writes, renames, deletes or
`exec`s a path inside a tree the agent UID can write. It asks the helper,
which acts with agent rights only and returns bytes.

- **Why it is a rule, not a hardening.** If the Machine (`cowboy`) read a
  refreshed `auth.json` or a workspace file directly, a tool could plant a
  symlink to `/var/lib/cowboy/provider-auth-service/vault.key`. The Machine
  would then ingest a control secret as a "provider credential", sync it to
  the Controller, and materialize it into an agent home. `O_NOFOLLOW` or
  `RESOLVE_BENEATH` on every call site is too easy to miss. Making the call
  impossible is not.
- **Enforcement:**
  - the `cowboy` units' `ProtectHome=yes` and `ReadWritePaths` exclude every
    worktree root and agent home, so a violation fails at the kernel;
  - a lint forbids `std::fs` and `tokio::fs` on workspace path types in
    control crates (a Rust module-layer rule beside the existing ratchets).
- Data that flows from the helper to control code is untrusted input, as
  the adapter's ACP output already is.
- The Controller's own workspace reads move behind the Machine:
  - the workspace picker (`--workspace-root /home/draven`) and any
    Controller-side git or code reads become Machine requests answered by the
    helper;
  - the Hawk `--workspace-root` flag then names a Machine-side root rather
    than a Controller-readable path.

## Peer authentication on control sockets

| Endpoint (after) | Owner, mode | Reachable by agent? | Kernel check |
|---|---|---|---|
| Broker `/run/cowboy-machine/<svc>/m` | `cowboy`, dir 0700 | No | uid == `cowboy`. `Core` additionally requires pidfd cgroup `cowboy-machine.service` (the in-process bridge) or `cowboy.service` (the Hawk co-located `remote_runtime` client). `Worker{session_id}` requires pidfd cgroup `cowboy-worker-*-<session_id>.service`. The hello's claimed role is ignored when it disagrees |
| Usage spool `u`, code `c`, Zed `z`, keeper sockets | `cowboy`, 0700 dir | No | uid == `cowboy` plus the expected cgroup per socket |
| Spawner | `root:cowboy` 0660 | No (not in group) | uid and cgroup as above |
| Account sandbox (OVH, until merged) | `root:cowboy` 0660 (today `root:ubuntu`) | No | Today **any `ubuntu` tool can reach it**; after this change uid == `cowboy` plus cgroup |
| Local operator `/var/lib/cowboy/local-operator/control.sock` | `cowboy` 0600 | No | Existing `peer uid ∈ {euid, 0}` (`src/local_operator.rs:111-127`), now meaning `cowboy` or root |
| Managed-call gateway, per session | `cowboy`. The socket directory is bind-mounted into exactly one agent unit by the spawner; socket 0666 inside a 0711 directory | **Yes, by design** (Cowboy MCP and CLI) | uid ∈ agent UIDs **and** the pidfd cgroup is under `cowboy-agent-<S>.service` or `cowboy-exec-<inc of S>.service` **and** the capability hash matches S. A peer from another session presenting S's capability is refused, even though R2 lets it read S's context. The result is the attested origin `agent(S)` |
| Agent helper control pipes | Pipes from the spawner | Only their own unit | Not a socket. The fd is the authority |
| Worker execution endpoint (`ws://127.0.0.1:<port>` + bearer, `src/worker_execution.rs:467-489`) | Loopback TCP | Yes | Moves to a Unix socket inside the agent unit with the gateway's check (slice S6). Until then the bearer arrives only through `$CREDENTIALS_DIRECTORY` |
| Memory relay, request-observation listener (loopback TCP + token) | Loopback TCP | Yes, by design | Same move to Unix with a peer check (S6). Tokens are per session and are no longer readable from control files |
| Controller HTTP `:3333` | TCP | Yes | Bearer/device authentication, not kernel. No owner-class token is readable by the agent UID (D1). Optional skuid fence (S7, firewall) |
| Machine→Controller link (Hawk loopback) | Outbound HTTP/WSS | n/a | Ed25519 challenge with a key only `cowboy` can read |

Implementation notes:

- A single helper crate exposes `PeerIdentity { uid, pid, pidfd, cgroup }`.
  It reads `/proc/self/fdinfo` of the pidfd, then
  `/proc/<pid>/cgroup`. It verifies the pidfd is still live after reading,
  which closes the PID-reuse race.
- The required checks come from configuration
  (`[host_identity] control_uid, agent_uids`; Service and Device TOML per
  `docs/configuration.md`, exposed read-only through Cowboy MCP).
- With both values equal to the running euid (the pre-cutover default) the
  checks reduce to today's behavior. That is what lets the code ship before
  the host changes.
- After cutover a mismatch refuses with a typed `peer_identity` error and a
  counter. Nothing falls back.

**Authenticated origin.** The attested `agent(S)` versus `control` origin on
the gateway and broker is the kernel fact the Antigravity redesign asked for.
Round-3 F1 can carry it as content origin. Owner authority still comes only
from a verified human credential, never from a socket.

## Future user-MCP sandbox (placement only)

- **Place:** `cowboy-tasks.slice/cowboy-sandbox.slice/cowboy-mcp-<sess>-<server>.service`,
  started through the spawner's `sandbox` profile. That profile stays refused
  until a user-MCP design defines it.
- **Identity:** a pool of static system users `cowboy-sbx-<n>`, allocated per
  account or session by the spawner as `cga-<key>` is today. Never the agent
  UID: the agent UID is exactly what a hostile MCP server must not obtain.
- **Filesystem:**
  - `ProtectSystem=strict`, `ProtectHome=tmpfs`, `PrivateTmp/Devices/IPC`,
    `TemporaryFileSystem` over Cowboy state;
  - read-only or read-write binds of the session worktree chosen by the
    owner's grant;
  - Landlock via the existing `exec-confined`.
- **Network:**
  - per unit, `RestrictAddressFamilies` and `IPAddressDeny=any` plus an
    explicit `IPAddressAllow` (BPF, per unit);
  - host nftables `meta skuid` / `socket cgroupv2` rules, as
    `matrix-agent` already uses on Hawk;
  - DNS through an allowlisting resolver to be designed.
- **Authentication:** it reaches Cowboy only through a gateway socket whose
  peer check yields `sandbox(S, server)`, never `agent(S)`.

## Hosts

### Hawk (Controller, Machine and execution for OVH runtimes)

Everything above applies. Hawk's `cowboy-execution-*` keepers serve runtimes
whose adapters run on OVH, so the keeper and exec split is what protects
Hawk's tools from Hawk's control plane.

### OVH (Machine and runtime; no Controller)

- **Identities.** `cowboy` becomes the Machine identity. `ubuntu` stays the
  agent identity, and `cga-<key>` keeps the Antigravity accounts.
- **Units.**
  - The Machine moves from `ubuntu`'s user unit
    `cowboy-machine-svc-….service` to a system unit `User=cowboy`, installed
    by the `service-managed-ubuntu` maintenance path (`ovh-operator`).
  - Workers become system units under `cowboy-control.slice` via the spawner;
    adapters become agent units as `ubuntu`.
- **Memory.** Today `user-1000.slice` (`MemoryHigh=9.5G`, `MemoryMax=10.5G`)
  holds the Machine, the workers and Matrix together. After the move:
  - `cowboy-control.slice` gets `MemoryLow` and no max;
  - `cowboy-tasks.slice` and `cowboy-accounts.slice` get the maxima;
  - the existing invariant is kept: sshd and Stormbird stay outside every
    budget.

  The exact numbers for the 11 GiB host are D5.
- **Account sandbox.** It moves to group `cowboy` with a uid and cgroup check
  (this fixes today's gap), and later merges into the spawner as the
  `account` profile.
- **State.** It moves from `/home/ubuntu/.local/state/cowboy-machine/services/<svc>`
  to `/var/lib/cowboy-machine/<svc>`.
- **Distribution.** OVH is runtime-only with no Nix daemon, so the spawner and
  helper ship inside the existing `cowboy-machine-release` closure. The
  maintenance tooling (`tools/ovh_machine_host_maintenance.py`) installs the
  system units.

### Falcon and macbook-air

- **Falcon (Machine only, `draven`, `systemd-user`): follow-up.** It applies
  slices S1-S5 after Hawk has run them for two weeks without isolation
  incidents. It needs no Controller step. Its sudo and trusted-user policy is
  decided separately.
- **macbook-air: non-goal.** It runs no Cowboy Machine or Controller. A
  future macOS Machine would use `Direct` spawn mode and needs its own design
  (no systemd, no `SO_PEERCRED`; `LOCAL_PEERCRED`/`getpeereid` instead).

## Migration

Every slice is independently deployable and reversible. Each follows the
machine transaction (`machines/docs/nixos-deployment.md`) or a Cowboy component
activator, and records its rollback. No durable data is deleted until a later
slice has run cleanly for a week. Previous copies are kept read-only, with a
SHA-256 manifest.

| Slice | What | Deploys via | Rollback | Flags |
|---|---|---|---|---|
| **S0** Peer-identity code, inert | The `PeerIdentity` crate; `[host_identity]` config defaulting to euid; checks on the broker, spawner client, gateway and Unix sockets; typed refusal and counters. No behavior change at default | Controller and Machine host releases | Previous release | None |
| **S1** Spawner and unit split, same UID | `cowboy-spawner` and `cowboy-agent-helper`; new Machine spawn mode `spawner` (worker, agent, keeper, exec units in system `cowboy-control.slice` and `cowboy-tasks.slice`, all still `User=draven`); fail-closed containment; R-CD workspace bridge with the helper; Columbus critical-resources assertions updated. **Delivers goal (2) on its own: the heartbeat fix** | Columbus machine transaction (spawner, slices, module) **and** `.#cowboy-machine-release` (new worker binary, a pool/adapter boundary change, so the Machine maintenance boundary) | Config flag `spawn_mode = "systemd-user"` restores the old path for new sessions; live sessions drain naturally; Columbus descendant revert | Activation; Machine maintenance |
| **S2** Agent privilege narrowing (Hawk) | sudo allowlist; drop `trusted-users` and group `root`; the D2 admin path | Columbus machine transaction | Descendant revert; activation runs as root, so a broken allowlist cannot lock it out, and `hawk-admin` or the console stays available | **sudo policy** |
| **S3** `cowboy` user and Machine move | Create `cowboy`. Drain or hibernate sessions, stop the Machine, `cp -a` state to `/var/lib/cowboy-machine/<svc>` and chown, write the manifest, leave the original read-only. System `cowboy-machine.service` as `cowboy`; spawner `worker` and `keeper` profiles run as `cowboy`; `[host_identity]` set | Columbus transaction plus Machine maintenance | Stop the Machine, copy the newer state back with the manifest, chown to `draven`, revert the unit. Durable namespaces (deletions, incarnations) are copied only while the Machine is stopped, in both directions | Activation; Machine maintenance; **high risk: durable namespaces** |
| **S4** Controller move | Stop `cowboy.service`, chown `/var/lib/cowboy` to `cowboy` (SQLite WAL checkpointed first), `User=cowboy`, `ProtectHome=yes`, `ReadWritePaths`; `cowboy-plugin-converge` and `cowboy-model-intel-audit` too; operator grant reissued; Controller `--workspace-root` behind the Machine | Columbus transaction (restarts the Controller; see the Deploy note in `AGENTS.md`) | chown back and revert the unit; the DB is unchanged by the move | Activation; **restarts the daemon driving sessions** |
| **S5** Enforce | Remove the euid default and the old spawn mode; refusal on any peer mismatch; acceptance suite | Controller and Machine host releases | Previous release (the config default returns) | Activation |
| **S6** Loopback listeners to Unix | Worker execution endpoint, memory relay and request observation move to agent-unit Unix sockets with peer checks | Machine release (maintenance) | Previous release | Machine maintenance |
| **S7** Optional TCP fence | nftables `meta skuid` denying agent UIDs from control-only loopback TCP (if any remain after S6) and optionally from `:3333` | Columbus transaction | Descendant revert | **firewall / networking** |
| **O1-O4** OVH | O1 = S0 (shared releases); O2 = S1 with spawner, slices and the account-sandbox socket check; O3 = S3 for `ubuntu`→`cowboy`, with no sudo step because `ubuntu` already lacks sudo; O4 = S5 and S6 | OVH maintenance (`ovh-operator`), Machine maintenance | As Hawk; the prior user unit and state stay installed until O4 | Activation; Machine maintenance; slice memory budgets |

The ordering makes the heartbeat fix (S1) available before any identity change.
S0 and S1 also unblock nothing security-wise by themselves: secrets stay
readable until S3 and S4, and sudo still bypasses everything until S2.

## Acceptance

- **Load.** Run `stress-ng` CPU, VM and I/O, plus a fork bomb capped by
  `TasksMax`, inside `cowboy-tasks.slice` at load above 400 for 30 minutes,
  and the same load from `user@1000`. Expect zero heartbeat isolations,
  `/healthz` p99 under 1 s, and every Machine presence intact.
- **Secrets.** As the agent UID, from inside an agent unit, read every A1
  path, `/proc/<pid>/{environ,mem,root}` of control processes, and the
  control runtime directories. All are denied.
- **Forgery.** As the agent UID:
  - connect to the broker, spawner, account sandbox and operator sockets
    (denied);
  - present session S's gateway capability from session T (refused with
    `peer_identity`).

  As `cowboy` from an unexpected cgroup, claim `Core` (refused).
- **R-CD.**
  - Plant symlinks to A1 paths in a worktree and an agent home. Trigger
    worktree preparation, a file buffer, a code read and a credential
    refresh. No A1 byte leaves.
  - Remove `ProtectHome` from a test unit: the lint still fails the build.
- **Workflow.** From a session, run:
  - `git push`, `ssh <alias>`, `nix develop -c just check-compact`;
  - `just sys-build` and `just sys-activate ./result` (allowlist);
  - rootless `docker run`, the Chrome bridge, Cowboy MCP read and execute.

  Then confirm `sudo cat /var/lib/cowboy/...` is refused.
- **Reboot convergence.** Verify in a NixOS test VM, not on production hosts.
  After reboot every unit returns, with no human step, in the right slice and
  under the right UID.

## Flags needing explicit owner confirmation

- **Activation.** S1, S2, S3, S4, S5 and every OVH slice. S3 and O3 include a
  Machine maintenance boundary. S4 restarts the Controller.
- **sudo / privilege policy.** S2, which also affects the human `draven`.
- **Networking and firewall.**
  - S7 (nftables).
  - The spawner's per-unit `IPAddressDeny`/`RestrictAddressFamilies` for
    agent profiles. Proposed: none for agent units, which keep today's
    network.
  - Any future sandbox network policy.
  - Stormbird, routing, DNS and TUN are untouched by every slice.
- **Durable data.** S3 and O3 copy Machine durable namespaces. Durable
  namespaces are high-risk under the review policy, so verify them with the
  manifest and keep the original.

## Owner decisions needed

- **D1. Agent identity on Hawk.**
  - Default `draven` (narrowest). The human accepts that Hawk shell sessions
    are agent-class, and owner-class Cowboy authority on Hawk lives only in
    the Web/PWA devices and the admin path.
  - Or D1b: a new `cowboy-agent` UID, a full chown of the agent tree, and
    separate Git and SSH credentials.
- **D2. Admin path.** A separate `hawk-admin` login with an SSH key held only
  on owner devices and NOPASSWD ALL (mirrors OVH's `ovh-operator`), or a
  password or `pam_u2f` on `draven`'s full sudo.
- **D3. R1.** Accept that agent-initiated activation remains root and control
  authority, recorded, as today's deploy preference requires. Or require an
  owner approval on activations (a future design).
- **D4.** Whether `sudo -u cowboy cowboy operator` is allowlisted for agent
  Plugin upgrades (`docs/agent-driven-plugin-release.md`).
- **D5. Budgets.** `TasksMax`, the control `MemoryLow` sum on Hawk, and the
  OVH slice numbers on the 11 GiB host.
- **D6.** Whether R2 (cross-session reads) is acceptable until the F-UID
  follow-up.

## Follow-ups

- **F-UID.** Per-account or per-session agent UIDs for provider homes, on the
  `cga-<key>` pattern, to close R2.
- **User MCP design**, inside the reserved `sandbox` profile.
- **Falcon rollout.**
- **macOS `Direct` mode equivalent**, if a Mac Machine is ever introduced.
