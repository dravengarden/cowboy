# Host isolation: liveness split now, identity boundary next

Status: **design, revision 2, not implemented.** Nothing here is activated on
any host. Every step that touches networking, firewall rules, sudo policy or a
deployment activation needs explicit owner confirmation before switching (see
[Flags](#flags-needing-explicit-owner-confirmation)).

Revision 1 proposed the whole boundary at once: a `cowboy` control UID, a root
spawner, peer credentials on every socket, an agent-identity workspace bridge,
and a migration of Machine and Controller state. Design review round 1,
`call-2e66a341853f6559a49f6e30` (Codex Astra Max, risk high), ended
`review_incomplete`, because the Columbus host files were outside the review
root. It reported 3 critical and about 20 high findings. Almost all of them
were in the identity migration, which is too large to converge in the remaining
rounds. Revision 2 therefore narrows the scope:

- **[Part L](#part-l-liveness-split-same-uid), the liveness split, is the
  surviving implementable scope.**
  - It moves the heartbeat sender out of the task cgroup subtree.
  - It bounds the heartbeat behind bulk output.
  - It keeps one UID and makes no security claim, so the round-1 identity
    findings do not apply to it.
- **[Part I](#part-i-identity-boundary-target-and-binding-requirements), the
  identity boundary, is reduced to a target and binding requirements.**
  - Every round-1 finding is recorded there, along with the owner decisions
    it needs.
  - Each sub-part needs its own design round before any implementation.

## Why

On Hawk, all of the following run as `draven` (UID 1000):

- the Controller (system `cowboy.service`, `User=draven`);
- the Machine broker (user `cowboy-machine.service`, in `user@1000`'s
  `app.slice`);
- the ACP workers (`cowboy-worker-*`);
- the execution keepers (`cowboy-execution-*`);
- every agent tool process.

The workers and keepers are transient units in one user slice,
`cowboy.slice/cowboy-agents.slice`. That slice has `MemoryHigh=48G`,
`MemoryMax=64G` and `MemorySwapMax=4G`, and no CPU weight.

The provider adapter is a child process of its worker
(`src/acp_process.rs:110-127`). Its tool processes sit in a leaf cgroup,
`agent-<session>`, inside the worker's own unit (`src/cgroup.rs:37-62`). So:

- **Liveness.** The worker's heartbeat is subject to the same `MemoryHigh`
  reclaim throttling as the tools it hosts. That throttling
  (`mem_cgroup_handle_over_high`) is the stall documented in Columbus
  `critical-resources.md`. At equal CPU weight, the worker also competes with
  every other session's tools.
  - Workers beat every 10 s (`src/worker.rs:744,812`).
  - The broker isolates a worker after `WORKER_HEARTBEAT_TIMEOUT=45s` and
    sweeps every 15 s (`machine_broker.rs:44-45`).
  - Together these produced the observed 48-58 s isolations under load above
    400.
- **Identity.** Any tool process can read every Controller and Machine
  secret. It can also claim any broker role, because the broker trusts the
  `role` sent in the hello (`machine_broker.rs:4131-4184`). And it passes the
  managed-call gateway's `peer uid == euid` check
  (`managed_calls/gateway.rs:143-160`). This blocks three things:
  - an authenticated delivery origin (Antigravity round 3:
    `docs/antigravity-parity-round-3.md` on `cowboy/sess-1795399435396`,
    commit `4eed6b2d0`, owner decision 5; review
    `call-1517db7ae578f9050a9232d4`);
  - a guarded broker `Permission`/`SetConfigOption`;
  - any user-MCP sandbox.

## Part L: liveness split (same UID)

### Goal and non-goals

**Goal.** Task load inside the task subtree, whether CPU, memory or I/O, never
delays a worker heartbeat past the broker's timeout. Neither does adapter
output, however large.

**Non-goals.**

- Part L makes no security claim. A tool can still stop a worker unit with
  `systemctl --user stop`, or read secrets, as it can today.
- Part L does not move the execution keeper. That split needs the
  execution-lifetime work in [I-7](#i-7-lifetime-and-recovery).

### Cgroup layout (user manager of the Machine user)

systemd derives slice nesting from dashes. The existing `cowboy-agents.slice`
is already `cowboy.slice/cowboy-agents.slice`, so Part L adds a sibling to it
rather than a new root:

```text
user@1000.service
├── app.slice                         MemoryLow=1G (unchanged)
│   └── cowboy-machine.service        CPUWeight/IOWeight=1000, MemoryLow=1G (unchanged)
├── cowboy.slice                      no MemoryHigh/MemoryMax (asserted)
│   ├── cowboy-control.slice          NEW: CPUWeight=1000, IOWeight=1000, MemoryLow=<D5>
│   │   └── cowboy-worker-<h>-<sess>.service    worker process only; no Delegate
│   └── cowboy-agents.slice           unchanged budget: MemoryHigh=48G, MemoryMax=64G, MemorySwapMax=4G;
│       │                             add CPUWeight=100, IOWeight=100
│       ├── cowboy-agent-<h>-<sess>.service     NEW: provider adapter + tools, Delegate=yes
│       ├── cowboy-execution-<inc>.service      unchanged (keeper+exec+commands; gap stays documented)
│       └── cowboy-browser-<instance>.service   unchanged
└── (other user services, rootless Docker)
```

- **CPU.** Inside `cowboy.slice`, workers have weight 1000 and tasks 100, so
  workers get at least about 90% of the slice whenever they are runnable.
  `cowboy.slice` itself still competes with `app.slice` and other user
  services at weight 100; Part L does not change that level. The heartbeat
  needs microseconds of CPU. Its starvation came from the shared `MemoryHigh`
  throttling and the equal weights, not from the `user@1000` level.
  [Acceptance](#acceptance-part-l) measures this.
- **Memory.** No ancestor of a worker carries `MemoryHigh` or `MemoryMax`.
  `cowboy-control.slice` gets a `MemoryLow` sized from the measured worker RSS
  times `--max-sessions` (D5). The Columbus assertions propagate it through
  `cowboy.slice` and `user@1000`.
- **Columbus.** The `critical-resources` assertions gain three checks:
  - the worker unit's exact `ControlGroup` is under `cowboy-control.slice`;
  - no task unit name is in that slice;
  - `cowboy.slice` and `cowboy-control.slice` carry no `MemoryHigh` or
    `MemoryMax`.

  The deployment check reads the effective `memory.low`, `memory.max` and
  `cpu.weight` at each ancestor. The working rule "never place task processes
  in a protected control service" holds, because the worker unit contains only
  `cowboy-acp-worker`.

### Unit split

The worker starts its adapter as a separate transient unit instead of a child
process:

```text
systemd-run --user --quiet --collect --pipe --service-type=exec \
  --unit=cowboy-agent-<h>-<sess> --slice=cowboy-agents.slice \
  -p KillMode=control-group -p Restart=no -p Delegate=yes -p TimeoutStopSec=15s \
  -p BindsTo=cowboy-worker-<h>-<sess>.service -p After=cowboy-worker-<h>-<sess>.service \
  [-p RestrictAddressFamilies=… from the signed network policy] \
  -- <provider command> <args>
```

Requirements:

- **L-1. Stdio.** `--pipe` hands the worker's pipe descriptors to the service
  directly, with no relay process. The ACP wire and the worker's framing are
  unchanged.
- **L-2. Placement is verified before use.** After the start, the worker reads
  the unit's `ControlGroup` and `InvocationID` from the user manager. If the
  unit is not active in `cowboy-agents.slice`, the worker refuses to send the
  first ACP frame. This mode does not use the fail-open leaf cgroup
  (`src/cgroup.rs:14-17`).
- **L-3. Teardown.**
  - "Kill the agent subtree" (`cgroup::kill_and_remove`,
    `src/acp_process.rs:245,314`) becomes `systemctl --user kill
    --signal=SIGKILL` on the exact unit, followed by waiting for `inactive`.
  - `BindsTo=` stops the agent unit whenever the worker unit stops, for any
    reason.
  - The worker reports exit only once the agent unit is inactive, or is gone
    and the recorded invocation is not running.
- **L-4. Containment parity.** The fail-closed owner for named authentication
  (`src/cgroup/containment`) and the proc-watcher (which reads the agent
  cgroup) both switch to the agent unit's verified `ControlGroup`. A worker
  that cannot establish that containment refuses the launch.
- **L-5. Network policy follows the socket owner.** The signed Provider
  network policy (`worker_launch::network_property`, `machine_broker.rs:3296`)
  moves from the worker unit to the agent unit, which is where sockets are
  opened. The broker keeps refusing it outside `systemd-user` mode. A worker
  that cannot apply the policy to the agent unit refuses the launch.
- **L-6. Environment.**
  - The agent unit gets exactly the environment the adapter gets as a child
    today, passed by `--setenv` or a 0600 environment file. It does not get
    the user manager's default environment. Shell-init guards are removed as
    they are today.
  - No secret appears in argv or in any `systemd-run` property visible on the
    user bus. A secret is only ever passed as a path to a 0600 file.
- **L-7. Mode selection and rollback.**
  - A Machine flag, `worker_agent_unit = true|false`, selects the mode for
    each launch. It lives in Device configuration and is readable through
    Cowboy MCP.
  - The worker records the mode in its launch receipt, and live sessions keep
    the mode they launched with.
  - Rolling back means setting the flag to false. Split sessions then drain on
    their next revive.
  - A release that does not understand split sessions must never be
    activated while any are live. The rollback runbook checks for this.
- **L-8. Recovery.**
  - A broker restart re-adopts workers by unit name, as it does today.
  - Each agent unit belongs to its worker through `BindsTo=`. After a Machine
    restart, a live worker still holds its agent's pipes, so nothing on the
    agent side needs re-adoption.
  - After a host reboot both units are gone, and the session recovers through
    its existing revive path.
  - A stale `cowboy-agent-*` unit whose worker is gone is stopped by
    `BindsTo=`. If one outlives that, the broker's existing
    `prepare_transient_unit` cleanup removes it before relaunch.

### Heartbeat lane

Slice weights do not help when the heartbeat waits behind bulk output on the
same writer. Today `heartbeat.tick()` writes the heartbeat frame and then the
entire outbox (`src/worker.rs:812-813`). The broker also defers isolation for
up to 300 s while it has unread bytes (`WORKER_BACKLOG_ISOLATION_LIMIT`).

- **L-9. Bounded outbox writes.** `send_outbox` writes at most a bounded batch,
  in bytes and in events, per wakeup, and yields between batches.
- **L-10. Liveness is never queued behind data.** When a `Heartbeat` is due,
  the writer sends it before the next data batch. Frames are never
  interleaved.
- **L-11. Broker side.** The broker acts on heartbeat frames ahead of data
  frames it has already read from the same connection. A heartbeat counts as
  liveness even while that backlog is still being processed.
- **L-12. Worker memory bound.** The outbox has a byte ceiling.
  - Reaching it applies backpressure to the read of the adapter's stdout.
  - The existing delivery ledger keeps ordering.
  - The protected worker never buffers unbounded adapter output.

### Acceptance (Part L)

- **Load.** Run `stress-ng` (CPU, VM at `MemoryHigh`, I/O) and a fork storm
  bounded by `TasksMax` inside `cowboy-agents.slice`, at load above 400 for 30
  minutes.
  - Expect zero heartbeat-timeout isolations.
  - Expect a maximum heartbeat gap under 15 s.
  - Repeat with the same load in a non-Cowboy `user@1000` service, and again
    in a Nix build.
- **Output flood.** An adapter writes 1 GiB to stdout while the broker reader
  is deliberately slowed. Heartbeats still arrive within 15 s, worker RSS
  stays under the ceiling, and no events are lost.
- **Lifecycle.**
  - Killing the worker stops the agent unit.
  - Killing the agent makes the worker report exit.
  - Restarting the Machine leaves workers and agents running.
  - Rebooting a NixOS test VM converges the units with no human step.
- **Policy.**
  - With the IPv4-only network policy, a tool cannot open `AF_INET6`.
  - A session whose agent unit cannot be placed is refused, never started
    uncontained.

### Rollout (Part L)

| Step | What | Deploys via | Rollback | Flags |
|---|---|---|---|---|
| L-a | Worker code (L-1 to L-12) with `worker_agent_unit` defaulting to false | The **worker pin advance** (`docs/machine-worker-pin-advance-runbook.md`), because the worker binary is in the retained `cowboy-workers` bundle. The resident fix then uses `.#cowboy-machine-writer-host-release`, because Hawk has an active deletion writer. **Never `.#cowboy-machine-release` on Hawk while the writer is active** (it builds `writer=false`) | Previous pin, per the runbook | Machine maintenance; owner go for the pin advance (it replaces every live Hawk worker generation) |
| L-b | Columbus: `cowboy-control.slice` and its assertions, plus `CPUWeight`/`IOWeight` on `cowboy-agents.slice` | Columbus machine transaction from a fresh Columbus worktree | Descendant revert | Activation (user units only; no network change) |
| L-c | Set `worker_agent_unit = true` on Hawk | Device configuration (run `cowboy config check` first) | Set the flag to false | None |
| L-d | The same on OVH, then Falcon | OVH maintenance by `ovh-operator`; Falcon machine transaction | Set the flag to false | Activation |

L-a and L-b are independent. L-b alone only creates an empty slice, and L-a
alone is inert at its default. Each step can be rolled back on its own.

## Part I: identity boundary (target and binding requirements)

Part I is **not implementation-ready**. It records the target and the
requirements every sub-design must satisfy, including all round-1 findings.
Each sub-part I-1 to I-10 needs its own design document and review round.

### Target

- **Identities.** A static system user `cowboy` runs the Controller, Machine,
  workers and keepers. Agents run as an agent UID (D1). Future user MCP
  servers run as a separate sandbox UID per account or per session.
- **Spawner.** A root spawner starts units across identities. The precedent is
  OVH's `cowboy-account-sandbox` (`packaging/account-sandbox`): file-backed
  `LoadCredential`, a fixed command table, `SCM_RIGHTS` return and
  close-to-stop.
- **Peer checks.** Every Unix control socket checks its kernel peer.
- **R-CD.** Control code never handles filesystem content that a task
  controls.
- **Placement.** Part L's slices move to the system manager with the same
  shape.

### Threat model and stated residuals

**Protected against:** a model-driven process running under the agent
identity, including a prompt-injected one.

**Stated residuals:**

- **R1.** Activating a closure an agent built remains root and control
  authority (D3).
- **R2.** All sessions share one agent UID, so one session can read **and
  write** another session's homes and worktrees.
  - A peer-cgroup check therefore identifies only the *observed unit*, not an
    uncompromised session S.
  - Authorizing on `agent(S)` requires per-session writable homes and
    execution identities (I-5).
- **R4.** Kernel exploits are out of scope.

### Binding requirements from round 1

#### I-1 Credential and operator identity separation (critical)

- **The operator SSH identity.** `draven`'s SSH identity reaches OVH's
  `ovh-operator`, which has NOPASSWD ALL
  (`docs/antigravity-real-accounts-design.md:431`). An agent UID that holds
  it is OVH root, with no activation and no receipt. The owner accepted this
  as residual R3-1 for Antigravity on 2026-11-18. Part I cannot claim
  control-secret protection while that acceptance stands, so D1 revisits it.
  Before any Part I step:
  - the operator SSH identity moves to an admin identity that agents cannot
    read;
  - agents reach OVH only through a scoped key or a fixed wrapper.
- **Owner product credentials.** CLI device logins (`src/cli.rs:305`,
  `docs/product-cli.md:14`) and browser sessions held under `draven` bypass
  the managed-call gateway with account-wide authority.
  - Inventory them, then revoke or relocate them.
  - Give the automation that needs them a scoped credential. One example is
    OVH rollout hibernation (`docs/ovh-machine-maintenance.md:59`).
- **sudo.**
  - Implement the allowlist as administrator-owned wrappers. Each wrapper
    fixes its verbs, targets, arguments, environment and pager behavior.
  - Never allow an unrestricted `cowboy operator`. It can enable its own
    grant, freeze convergence and force session updates
    (`src/local_operator.rs:183,209,385`).
  - The admin login's startup files must be outside the agent UID's control.
- **Root-equivalent memberships.** Remove Nix `trusted-users` and group `root`
  from the agent UID.

#### I-2 Spawner contract

- **Caller-by-operation matrix.** Each caller may start only its own units:
  - a worker starts only its own agent unit, for its own session;
  - a keeper starts only its own exec unit;
  - the Machine starts only workers and keepers.

  No Provider command ever runs under a profile with the `cowboy` UID.
- **Trusted launch reservation.** The target unit, exact executable,
  arguments, environment and credential source come from a reservation the
  trusted Machine records, never from caller fields.
  - Keep the account daemon's fixed command table and its separate
    administrative verbs (`packaging/account-sandbox/src/units.rs:14`,
    `protocol.rs:57`).
- **Secrets.**
  - Deliver them only through `LoadCredential` from a spawner-owned private
    file or descriptor.
  - Never use `SetCredential`, whose literal data is visible over systemd IPC,
    and never unit properties or argv.
  - Verify manager visibility as the agent UID.
- **Runtime projection.** Installed Provider executables and private
  components live inside Machine state.
  - Project the verified runtime read-only.
  - Expose only the admitted working directory.
  - Keep contracts and ownership markers private (`machine_plugins.rs:4538`,
    `machine_cli/execution.rs:89`).

#### I-3 Peer authentication

- **What a pidfd proves.** A pidfd check authenticates the *connecting*
  process at connect time. A descriptor can later be inherited or passed, and
  cgroup membership can change.
  - Bind admission to the exact Service, session, boot and systemd
    `InvocationID`.
  - Keep the broker namespace already encoded in the unit name
    (`machine_broker.rs:3752`, `systemd_exit.rs:104`).
  - For per-request origin, use per-message `SCM_CREDENTIALS`, or limit the
    claim explicitly to the connecting process.
- **Gateway clients.** Each client requires both the context-file owner and
  the gateway peer to equal its own euid (`managed_calls/cli.rs:147`). The
  Machine also replaces grant sockets and namespaces
  (`managed_calls/host.rs:188`).
  - Update both client checks.
  - Publish rotating contexts and sockets through a stable, control-owned
    per-session directory, never through a one-time bind mount.
- **Account units.** Antigravity's Unix listener runs in a `cga-<key>` account
  unit (`src/acp_process.rs:178`). Authenticate that exact account-unit
  invocation; the generic agent and exec rule does not cover it.

#### I-4 Filesystem view (R-CD)

- **Hiding task roots.** `ProtectSystem=strict` only makes paths read-only,
  and `ProtectHome` still leaves the `/srv` worktrees visible. Control units
  need either a positive filesystem view (`TemporaryFileSystem=/` plus
  explicit binds) or an explicit mask of every physical task root and alias.
  - A Rust lint is only supplementary. Untyped paths, subprocesses and
    inherited handles bypass it.
  - Keep the keeper's workspace identity check through a trusted handle
    (`execution_host.rs:174,403`).
- **Subprocesses that process project or credential input.** Each one needs a
  dedicated unprivileged profile, and the Machine keeps only coordination:
  - Code host tools and Zed language servers for trusted workspaces
    (`machine_code_plugins.rs:629`, `plugins/zed/adapter/src/trust.rs:3`);
  - Provider install and refresh probes (`machine_plugins/auth_refresh.rs:208`).
- **Execution environment.** Execution contracts capture
  `HOME`/`USER`/`PATH`/`XDG_*` from the Machine. After a UID move they must
  come from the agent account. Provision its SSH agent, bus, rootless Docker
  socket and Nix profile explicitly.

#### I-5 Provider credentials

- **One refresh owner per account.** Per-unit credential copies would hand
  two sessions of one account the same single-use refresh token. Keep one
  account-scoped refresh directory, or serialize redemption through an
  account owner, usage collectors included
  (`docs/provider-credential-observation.md:28`, `machine_plugins.rs:1676`).
- **Per-session identities first.** Per-session writable homes and execution
  identities must exist before `agent(S)` is used for authorization (R2).

#### I-6 Machine state move

This covers S3 and O3 from revision 1.

- **Writer-capable releases only.**
  - Never deploy a `writer=false` release on a host with an active deletion
    writer (`flake.nix:1053`, `machine_broker.rs:1403`).
  - Advance the worker pin, then deploy a writer-host release
    (`docs/machine-worker-pin-advance-runbook.md:81`).
- **Recovery floors.** The floors compare absolute dataset paths and the
  canonical state directory (`src/session_deletion_admission/owner_writer.rs:101`,
  `reader_floor.rs:82`).
  - Migrate the floors, the selected release and the cold bootstrap in one
    transaction.
  - Run startup checks in both the forward and the rollback direction.
- **One owner at a time.**
  - Persist one boot selector.
  - Fence the old user unit and its socket. OVH's rollback helper otherwise
    starts the old unit (`tools/ovh_machine_host_maintenance.py:124`;
    `src/machine_install.rs:744`).
  - Publish verified deletion, incarnation and cleanup state atomically,
    because an absent record means empty (`machine_broker/namespace.rs:75`).
- **No `cp -a` of the state.**
  - Rebuild component links and execution GC roots
    (`src/machine_components.rs:184`, `execution/retention.rs:30`).
  - Keep task worktrees in place, because cleanup records identify them by
    device and inode (`components/store/src/session_workspace.rs:865`).
- **The identity key.**
  - Any rollback copy holding `identity_ed25519` goes under an
    administrator-only parent.
  - Rotate the keys and credentials the old UID could read, through an
    enrollment-preserving transition (`components/wire/src/machine_auth.rs:54`).

#### I-7 Lifetime and recovery

- **Unit names alone do not prove an exit.** After a split or a spawn-mode
  rollback, a user-manager unit can be gone while a system-manager exec unit
  still runs.
  - Persist the manager, boot and invocation of both units.
  - Reconcile the actual exit before any replacement.
- **Spawner restarts.** Preserve uncertain effects across a spawner restart.
  Closing the spawner connection must not silently stop attached units
  (`machine_cli/execution/liveness.rs:64`,
  `packaging/account-sandbox/src/daemon.rs:164`).
- The keeper/exec split belongs here and follows Part L.

#### I-8 Controller move

An in-place chown leaves existing database descriptors usable by a `draven`
process.

- Quiesce writers.
- Publish a verified copy on fresh inodes in a private directory.
- Test with an old-UID descriptor held open across the cutover.

#### I-9 Intermediate configurations

Every intermediate step defines its callers and socket policy, or switches
mutually dependent units atomically.

- Hawk's co-located Core client runs in the Controller. It must move together
  with the Machine socket policy, or keep a compatible path.
- Ship a compatible cold bootstrap before introducing any new spawn flag.

#### I-10 Transports, browser and network

- **Unix sockets for loopback transports.** Moving the worker execution
  endpoint and the memory relay to Unix sockets needs a versioned transport,
  and the consumers must ship first. The pinned adapters accept only
  `ws://127.0.0.1` (`provider-runtime/packages/codex-acp/launch.mjs:24`,
  `plugins/claude-code/runtime/connection.mjs:26`, `src/acp_memory.rs:227`).
- **Browser.** It needs:
  - its own identity;
  - a trusted verifier for the `/proc` and namespace checks the manager makes
    today (`machine_browser/manager.rs:451`, `verify.rs:285`);
  - only authorized channels exported.
- **Network policy.** Enforce every signed or Device network policy on the
  unit that opens the sockets, or refuse the launch. This covers IPv4-only and
  the OVH account IP denials (`packaging/account-sandbox/src/units.rs:208`,
  `docs/requirements.md:59`). Verify OVH native egress and its proxy route.
- **Escaped load.** User services, rootless containers and `nix-daemon` builds
  can exhaust global memory or PIDs regardless of CPU weights.
  - Budget those subtrees and reserve host headroom.
  - Test sshd, Stormbird and the heartbeat under memory and PID pressure.

### Future user-MCP sandbox (placement only)

- **Units.** `cowboy-mcp-<sess>-<server>.service`, in a dedicated sandbox slice
  under the task subtree.
- **Identity.** A pool of static `cowboy-sbx-<n>` users, never the agent UID.
- **Filesystem.**
  - `ProtectSystem=strict`, `ProtectHome=tmpfs` and `PrivateTmp/Devices/IPC`;
  - `TemporaryFileSystem` over Cowboy state;
  - grant-scoped worktree binds;
  - Landlock through `exec-confined`.
- **Network.** Per-unit `IPAddressDeny=any` plus an allowlist, and nftables
  `meta skuid` / `socket cgroupv2` rules, as `matrix-agent` uses on Hawk.
- **Authentication.** A gateway peer check yields `sandbox(S, server)`.

Nothing beyond placement is designed here.

### Hosts

- **Hawk.** Runs the Controller and a Machine, and executes work for OVH
  runtimes through its keepers. Part L comes first, then Part I.
- **OVH.** Runs the Machine and the runtime, with no Controller. The agent
  identity is `ubuntu`, which already has no sudo.
  - Today the account-sandbox control socket is `root:ubuntu 0660`, and the
    daemon reads the peer but does not authorize with it
    (`packaging/account-sandbox/src/server.rs:31-38`). Any `ubuntu` tool can
    therefore request account units.
  - I-2 closes this. An interim fix that authorizes `uid == ubuntu` does not
    help while tools share `ubuntu`, so the fix waits for Part I.
- **Falcon (Machine only).** Part L follows Hawk. Part I is a follow-up.
- **macbook-air.** A non-goal: it runs no Cowboy component.

## Flags needing explicit owner confirmation

- **Part L.**
  - The worker pin advance (L-a) replaces every live Hawk worker generation.
  - The Columbus transaction (L-b) changes user units only. It makes no
    network, firewall, DNS, routing or Stormbird change.
  - The OVH maintenance (L-d).
- **Part I.** Every sub-part, because they touch sudo policy, SSH identities,
  durable Machine namespaces, a Controller restart, nftables and per-unit
  network policy.

## Owner decisions needed

- **Part L go**, including the worker pin advance.
- **D1. Agent identity on Hawk.** Either:
  - keep `draven`, with the operator SSH identity and owner credentials moved
    out (I-1); or
  - create a new `cowboy-agent` UID, with a chown of the agent tree and
    separate Git and SSH credentials.
- **D2. Admin path.** Either a `hawk-admin` login whose key is held only on
  owner devices (mirroring OVH's `ovh-operator`), or `pam_u2f`.
- **D3. R1.** Accept agent-initiated activation as recorded authority, or
  require owner approval for it.
- **D5. Budgets.** The `cowboy-control.slice` `MemoryLow`, the task
  `TasksMax`, and the OVH numbers.
- **D6. Origin ordering.** Whether per-session agent identities (I-5) must
  exist before Antigravity round 3 uses `agent(S)` as an origin.
