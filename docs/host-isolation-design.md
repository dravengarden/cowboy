# Host isolation: heartbeat attribution now, liveness split and identity boundary next

Status: **design, revision 3, not implemented.** Nothing here is activated on
any host. Every step that touches networking, firewall rules, sudo policy or a
deployment activation needs explicit owner confirmation before switching (see
[Flags](#flags-needing-explicit-owner-confirmation)).

Review history (Codex Astra Max, risk high):

- **Revision 1** proposed the whole boundary at once: a `cowboy` control UID, a
  root spawner, peer credentials on every socket, an agent-identity workspace
  bridge, and a migration of Machine and Controller state.
  - Round 1, `call-2e66a341853f6559a49f6e30`, ended `review_incomplete`: the
    Columbus host files were outside the review root.
  - It still reported 3 critical and about 20 high findings, almost all in the
    identity migration.
- **Revision 2** reduced the identity boundary to requirements and kept a
  same-UID worker/adapter split as the implementable scope.
  - Round 2, `call-26a3d3cda3ad9354de781d50`, ended `review_incomplete`: the
    round-1 findings were not in its input.
  - It reported 1 critical and about 14 high findings, all in that split and
    its heartbeat lane.
- **Revision 3 (this one)** narrows again:
  - **[L0](#l0-requirements), heartbeat stall attribution, is the only
    implementable scope.** It is broker-side observation with no behavior
    change, and it ships as a resident Machine release.
  - The split and the lane become
    [deferred requirements](#l-split-requirements-deferred), gated on L0 data.
  - **[Part I](#part-i-identity-boundary-target-and-binding-requirements)**
    stays a target with binding requirements, and is not
    implementation-ready.

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

## Part L: heartbeat liveness

Part L has three pieces, and only the first is implementable:

- **L0, stall attribution.** This is the surviving scope. It is broker-side
  observation with no behavior change.
- **The split and the lane.** The same-UID worker/adapter unit split and the
  heartbeat lane from revision 2 are deferred. They stay as binding
  requirements until L0 data selects a remedy.

### Why L0 first

Design review round 2 (`call-26a3d3cda3ad9354de781d50`) found 1 critical and
about 14 high findings in revision 2's unit split and heartbeat lane. They are
recorded in [L-split](#l-split-requirements-deferred) and
[L-lane](#l-lane-requirements-deferred).

The mechanism behind the 48-58 s heartbeat gaps is also **not established**.
It could be any of these, and each calls for a different remedy:

| Candidate | Remedy it would call for |
|---|---|
| Throttling at the shared `cowboy-agents.slice` `MemoryHigh` | A memory policy change, or the split |
| CPU competition at equal weight | Weights, or the split |
| I/O pressure | IO weights, or the split |
| One large data frame blocking `write_all` (up to `MAX_FRAME_BYTES`, `components/wire/src/runtime_wire.rs:47,1082-1109`), or the broker's read of it | An independent liveness transport |
| A broker-side stall | Neither the split nor the lane helps |

So L0 measures the mechanism before committing to the expensive, review-heavy
remedies.

### L0 requirements

- **L0-1. Gap record.** On every worker heartbeat gap above 20 s, and on every
  heartbeat-timeout isolation, the broker records one structured event
  (Machine telemetry and journal). It records no secrets. The fields are:
  - the session, worker epoch and unit name;
  - the gap, and the last and current heartbeat receive times;
  - broker read state for that connection: whether a frame is in progress,
    its declared length and bytes read so far, the unread backlog, and
    whether the backlog deferral (`WORKER_BACKLOG_ISOLATION_LIMIT`) was
    applied;
  - for the worker unit's cgroup and each ancestor up to `user@<uid>.service`:
    the `cpu.pressure`, `memory.pressure` and `io.pressure` totals and the
    delta since the previous heartbeat, `memory.events` (`high`, `max`, `oom`)
    deltas, and `memory.current`;
  - the same pressure totals for the broker's own cgroup, plus the host
    `/proc/pressure/*`, so a broker stall is distinguishable;
  - the broker's own event-loop lag over the gap, measured by a periodic tick.
- **L0-2. Baseline sampler.** For PSI deltas to cover the gap, the broker
  keeps the previous sample of each live worker's cgroup counters at heartbeat
  receive. It reads only files below the worker's verified unit cgroup
  (`ControlGroup` from the user manager). It reads with
  `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` under
  `/sys/fs/cgroup`, and fails closed per field (`unavailable`), never per
  session.
- **L0-3. Bounded cost.** The sampler reads about a dozen small files per
  worker per heartbeat (10 s). At `--max-sessions 128` that is well under 1%
  of one CPU. A configured kill switch disables it.
- **L0-4. No behavior change.** L0 changes none of these:
  - isolation decisions, timeouts or the backlog limit;
  - the wire, worker or adapter;
  - units or slices.

  It also changes no `Cargo.toml`, `Cargo.lock`, `components/wire/src` or SDK
  file. Those are the retained-worker interface inputs that host releases
  require to be byte-identical (`flake.nix:956-969`). L0 therefore ships as a
  resident Machine fix.
- **L0-5. Reproduction.** In a NixOS test VM with the same slice settings
  (not on production), run `stress-ng` CPU, VM at `MemoryHigh`, I/O, and a
  32 MiB-frame flood separately. Record which loads reproduce gaps above 45 s
  and what L0-1 shows for each. The Hawk record from production then
  classifies real incidents against this matrix.
- **L0-6. Decision gate.** After two weeks, or after five recorded gaps, the
  owner selects the remedy from the data:
  - shared memory throttling → first apply the policy the execution units
    already use: drop the slice `MemoryHigh` in favor of `MemoryMax` with
    per-unit limits, and measure again; take the split only if that is
    insufficient;
  - frame blocking → L-lane;
  - CPU or I/O → weights, then the split;
  - broker stall → a broker-side design.

### L0 rollout

| Step | What | Deploys via | Rollback | Flags |
|---|---|---|---|---|
| L0-a | Broker gap record and sampler | `.#cowboy-machine-writer-host-release` on Hawk (active deletion writer; resident-only, because `machine_broker.rs` is not among the host-release byte-identity inputs). OVH and Falcon use the resident-fix output that their maintenance runbooks select. **Never `.#cowboy-machine-release` on Hawk while the writer is active** | Previous resident release | Machine component activation (restarts only the resident Machine; workers are retained) |
| L0-b | VM reproduction | No host | n/a | None |

L0 needs no worker pin advance, no Columbus change and no network change.

### L-split requirements (deferred)

These requirements bind any future unit split. They come from revision 2 plus
round 2.

- **L-S1. Exit proof covers both trees.** Persist the worker's and the
  agent's boot, launch, `InvocationID` and cgroup identities. Require
  broker-confirmed exit of both trees before revocation, cleanup or
  replacement. A killed worker cannot report its agent's exit.
  - Today named-account tree exit checks only the worker cgroup
    (`machine_broker/systemd_exit.rs:308-319`, `machine_broker.rs:1684-1692,1751-1759`).
- **L-S2. Scope.** The new launcher applies only to the local-child branch in
  `SystemdUser` mode. `account_sandbox_v1` sessions keep daemon-owned launch,
  network restrictions and exit handling (`src/acp_process.rs:75-81`,
  `components/provider-sdk/src/runtime_policy.rs:17-27`).
- **L-S3. Gate before Provider code.** A trusted gate runs first in the agent
  unit, binds the observed invocation and placement to the reserved launch,
  then releases the Provider. A failed gate is reconciled by exact
  invocation. The named-worker precedent is `systemd_exit.rs:18-41`.
- **L-S4. Working directory.** Pass the exact admitted runtime cwd as
  `WorkingDirectory`. Managed Codex reads `process.cwd()` before ACP
  initialization for ExecPolicy and MCP enumeration
  (`src/acp_process.rs:111-116`, `plugins/codex/runtime/launch.mjs:172-184`).
- **L-S5. Sidecars.** Gateway sidecars (`src/provider/mod.rs:524-529,876-913`,
  `src/worker.rs:597-601`) move under the task budget with their readiness,
  network and teardown contracts. Otherwise those launches stay in legacy
  mode.
- **L-S6. Environment.** Build a clean exec environment from the effective
  launch spec, not an overlay of the manager environment. Carry secret values
  privately through both unit launches. Note that the broker already exports
  projected credentials in the worker's `Environment` property
  (`machine_broker.rs:3364-3366,3394-3398`), which is itself an I-2 concern.
- **L-S7. Containment owners.** Keep the independent native-auth and
  Direct-worker containment owners (`machine_cli/native_login.rs:128-135`,
  `src/cgroup/containment/linux.rs:92-115,212-241`). Add a separate owner for
  split systemd sessions.
- **L-S8. Selected executable.** Check split capability on the actually
  selected worker executable, including the generation fallback, before
  assigning slice and policy (`machine_broker.rs:2567-2585`,
  `worker_launch.rs:24-43`).
- **L-S9. Rollback.**
  - Older Machines reject unknown Device keys (`docs/configuration.md:45-69`).
  - Specify a drain, verify both units have exited, and remove the key before
    a downgrade.
  - Validate the previous writer and cold-recovery binaries against the
    resulting config and persisted receipts.
- **L-S10. Adoption is not cleanup.** Keep Hello/snapshot adoption of a
  surviving worker. `prepare_transient_unit` must not stop a late reconnect.
  Reconcile a leftover agent by exact invocation after fencing its prior
  launch (`machine_broker.rs:3337-3345,3685-3697,4179-4208`).
- **L-S11. OVH.** Both user slices descend from OVH's capped
  `user-1000.slice`. Gate any OVH rollout on a resource transaction that
  places the task limit below the split and verifies every worker ancestor
  (`machine_cli/host_resources.rs:375-383`, `docs/ovh-machine-maintenance.md:39-55`).
- **L-S12. Placement and slices.** Revision 2's layout remains the target:
  `cowboy.slice/cowboy-control.slice` for workers,
  `cowboy.slice/cowboy-agents.slice` for tasks, and no `MemoryHigh` or
  `MemoryMax` on any worker ancestor. So do its network-policy-follows-socket
  rule and its fail-closed placement.

### L-lane requirements (deferred)

- **L-L1. Independent liveness transport.** One legal 32 MiB frame can hold
  `write_all` past 45 s, and the broker must read the whole payload before it
  sees the next heartbeat. Batch priority cannot meet a 15 s bound.
  - Liveness needs an independently serviced transport bound to the worker
    epoch and connection, including during replay.
  - Stalled data progress is tracked separately, so heartbeats cannot mask
    the 300 s limit (`runtime_wire.rs:47,1082-1109`,
    `machine_broker.rs:996-1023,4848-4850`).
- **L-L2. ACK ordering.** `CommandAck` is released only after its required
  runtime sequence (for example `TurnStarted`) has been sent
  (`src/worker.rs:755-763,1086-1101`).
- **L-L3. No self-deadlock.**
  - Reserve outbox capacity for command and terminal events.
  - Keep ACK processing runnable while adapter stdout is paused.
  - Account for deliberate read pauses in the prompt watchdog
    (`src/worker.rs:746-768,1086-1094`, `src/acp.rs:3463-3509`).
- **L-L4. Ingress bounds.** Bound individual records at ingress on both pipes,
  including an unterminated stderr line and the ACP decoder before event
  admission. Serialization must check size before allocating the payload.
  Define how oversized records truncate or terminate
  (`src/acp.rs:2901-2904,2940`, `runtime_wire.rs:1071-1078`).

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
  - Keep the account daemon's fixed command table
    (`packaging/account-sandbox/src/units.rs:14`, `protocol.rs:57`).
- **Every verb, not just start.** Account-wide stop, admission, logout
  cleanup and removal (`packaging/account-sandbox/src/daemon.rs:253-291`) are
  deny-by-default, each with a defined caller and account scope. Lifecycle
  administration is reserved to its trusted owner.
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
  runtimes through its keepers. L0 comes first, then any split, then Part I.
- **OVH.** Runs the Machine and the runtime, with no Controller. The agent
  identity is `ubuntu`, which already has no sudo.
  - Today the account-sandbox control socket is `root:ubuntu 0660`, and the
    daemon reads the peer but does not authorize with it
    (`packaging/account-sandbox/src/server.rs:31-38`). Any `ubuntu` tool can
    therefore request account units.
  - I-2 closes this. An interim fix that authorizes `uid == ubuntu` does not
    help while tools share `ubuntu`, so the fix waits for Part I.
- **Falcon (Machine only).** L0 follows Hawk. Part I is a follow-up.
- **macbook-air.** A non-goal: it runs no Cowboy component.

## Flags needing explicit owner confirmation

- **L0.** A resident Machine component activation on each host. It restarts
  only the resident Machine and retains workers. It needs no network,
  firewall, Columbus or worker pin change.
- **A later split or lane.** These need a worker pin advance, which replaces
  every live Hawk worker generation. They also need a Columbus machine
  transaction, which changes user units only, and OVH maintenance with its
  resource transaction (L-S11).
- **Part I.** Every sub-part, because they touch sudo policy, SSH identities,
  durable Machine namespaces, a Controller restart, nftables and per-unit
  network policy.

## Owner decisions needed

- **L0 go.** Resident Machine release on Hawk, then OVH.
- **L0-6.** The remedy selection once the data is in.
- **D1. Agent identity on Hawk.** Either:
  - keep `draven`, with the operator SSH identity and owner credentials moved
    out (I-1); or
  - create a new `cowboy-agent` UID, with a chown of the agent tree and
    separate Git and SSH credentials.
- **D2. Admin path.** Either a `hawk-admin` login whose key is held only on
  owner devices (mirroring OVH's `ovh-operator`), or `pam_u2f`.
- **D3. R1.** Accept agent-initiated activation as recorded authority, or
  require owner approval for it.
- **D5. Budgets for a later split.** The `cowboy-control.slice` `MemoryLow`,
  the task `TasksMax`, and the OVH numbers.
- **D6. Origin ordering.** Whether per-session agent identities (I-5) must
  exist before Antigravity round 3 uses `agent(S)` as an origin.
