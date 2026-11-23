# Host isolation: heartbeat attribution now, liveness split and identity boundary next

Status: **design, revision 3.1, not implemented.** Nothing here is activated on
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
- **Revision 3** narrows again:
  - **[L0](#l0-requirements), heartbeat stall attribution, is the only
    implementable scope.** It is broker-side observation with no behavior
    change, and it ships as a resident Machine release.
  - The split and the lane become
    [deferred requirements](#l-split-requirements-deferred), gated on L0 data.
  - **[Part I](#part-i-identity-boundary-target-and-binding-requirements)**
    stays a target with binding requirements, and is not
    implementation-ready.
  - Round 3, `call-02abbc2e4c84c950790749d3` (Sol Max, routine), completed
    with verdict `needs-attention`: 5 high and 7 medium findings.
- **Revision 3.1 (this one)** folds all round-3 findings into L0, L-S1, L-L1,
  I-6, the sandbox claim and D6. These amendments are unreviewed, because the
  three-round review budget is used up. The next review starts with L0's
  implementation, not with this design.

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

Revision 3.1 amends this section with the round-3 findings
(`call-02abbc2e4c84c950790749d3`, verdict `needs-attention`). The review rounds
are used up, so these amendments are **unreviewed**.

- **L0-1. Gap record.** The broker records one bounded, typed event in two
  cases:
  - when a worker heartbeat gap exceeds 20 s;
  - on every isolation, including one with no closing heartbeat.

  The event goes to the journal and to Machine telemetry. It holds no secrets
  and contains:
  - the session, the worker epoch, the connection id, and the unit name with
    its bound `InvocationID` (L0-2);
  - **separate clocks**:
    - the last heartbeat receive time;
    - the last decoded-frame time (`last_seen` is refreshed by every frame,
      `machine_broker.rs:4848-4853`);
    - the last read-progress time;
    - the sweep decision actually taken: evict, backlog extension, or none
      (`machine_broker.rs:996-1032`);
  - **frame progress during the gap**, kept as bounded per-connection
    observations updated by the existing reader, not reconstructed at
    heartbeat receipt:
    - the header-read, payload-read, decode and admission phase;
    - the declared length, bytes read so far, and the last progress time;
    - the queued bytes (`FIONREAD`).

    The reader's framing semantics are unchanged. In particular L0 does not
    switch to `FrameReader`, whose partial-header EOF handling differs
    (`runtime_wire.rs:1094-1112,1148-1161`);
  - **pressure samples** (L0-2) covering the gap, with their coverage window.
    They are labelled correlation, not cause;
  - the broker's own event-loop lag, which is necessary but not sufficient:
    a healthy tick can coexist with one stalled connection.
- **L0-2. Sampler.**
  - **Binding.** The cgroup identity is resolved only on a new or adopted
    connection. It comes from a bounded, read-only user-manager query for the
    derived `.service` (`ControlGroup`, `InvocationID`, `MainPID`). The
    result is verified against the kernel peer of the broker connection
    (`SO_PEERCRED` pid membership in that cgroup) and bound to the
    connection. Unit names are reused across launches, so samples from a
    stale binding are discarded. Direct mode, and an identity that cannot be
    resolved, record `unavailable`.
  - **Scopes.** Fixed counter files are read from validated directories on
    the cgroup v2 mount:
    - the worker unit;
    - every ancestor up to the root, including OVH's capped
      `user-1000.slice` (L-S11);
    - the broker's own unit;
    - the host `/proc/pressure/*`, from a separate proc scope.
  - **Files.** `cpu.pressure`, `memory.pressure`, `io.pressure`,
    `memory.events`, `memory.events.local`, `memory.current`, and the outer
    `memory.high` and `memory.max` limits. Reads use
    `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` under each validated
    root. A failure marks that field `unavailable`, never the session.
- **L0-3. Bounded cost, off the critical paths.**
  - Sampling and logging never run on the connection reader, the heartbeat
    monitor or the sweep.
  - One sampler task runs on a fixed period. It deduplicates shared ancestor
    scopes (the slice, `user@`, root and host are read once per period, not
    once per worker), has a fixed concurrency and a bounded queue, and
    coalesces or drops when behind. Drops are counted.
  - Overhead is measured under the L0-5 loads before activation.
  - The kill switch is a Device setting in the schema of the release that
    introduces it ([rollback](#l0-rollout)).
- **L0-4. No behavior change.**
  - L0 does not change isolation decisions, timeouts, the backlog limit, the
    wire, the worker, the adapter, units or slices.
  - It does not change `Cargo.toml`, `Cargo.lock`, `components/wire/src` or
    any SDK file. These are the retained-worker interface inputs that host
    releases require to be byte-identical (`flake.nix:956-969`).
- **L0-5. Reproduction.** Run in a NixOS test VM with the same slice settings,
  never on production. Apply each load separately:
  - `stress-ng` CPU;
  - `stress-ng` VM at `MemoryHigh`;
  - I/O;
  - a 32 MiB-frame flood.

  Record which loads reproduce gaps above 45 s, and what L0-1 shows for each.
  Production records from Hawk are then classified against this matrix.
- **L0-6. Telemetry sinks.** The Machine telemetry bridge keeps an explicit
  field allowlist and has no structured value visitor
  (`src/logs/capture.rs:699-883`). The gap record is therefore a bounded,
  typed event with an explicit allowlisted projection. Its retrieval is
  verified from both sinks, the journal and VictoriaLogs. A dropped or
  truncated record is counted. An observation window with drops does not
  count as clean.
- **L0-7. Decision gate.** After two weeks, or after five recorded gaps, the
  owner selects the remedy from the data:

  | Data shows | Remedy |
  |---|---|
  | Shared memory throttling | First the execution-unit policy: drop the slice `MemoryHigh` in favor of `MemoryMax` plus per-unit limits. Measure again; split only if that is insufficient |
  | Frame blocking | L-lane |
  | CPU or I/O pressure | Weights, then the split |
  | A broker stall | A broker-side design |

### L0 implementation

The code is in `components/machine/src/machine_broker/liveness.rs` and
`liveness_counters.rs`. It is implemented on its branch and **not deployed**.

**Hot paths.** These touch only their own connection's atomics and `try_send`
into a 64-slot queue; a full queue counts drops:

- `handle_worker` reads through `liveness::read_frame`, which is
  `runtime_wire::read_frame` plus phase and progress stamps, with the same
  framing.
- The `Heartbeat` arm, and the sweep inside `take_stale_workers` (with the
  exact peer, its decision and its measured `FIONREAD`), add no lock, scan or
  await.

**Evidence.**

- A closing heartbeat freezes the interval's evidence into its notice before
  resetting it: slowest frame and size, longest first-byte wait, frames, and
  the in-flight frame's phase and bytes.
- A threshold crossing is detected on each 10 s tick, even if no heartbeat
  or isolation follows.
- A probe records the broker runtime's own scheduling lag every 250 ms.

**Binding.** At admission the broker pins the `SO_PEERCRED` peer's
`/proc/<pid>` directory before any await. A two-lane bind pool then:

1. runs a bounded `systemctl --user show` (5 s, 4 KiB of output);
2. pins the unit's `ControlGroup` directory;
3. verifies that the pinned peer is a member.

A recreated unit cannot be sampled under the old invocation. Results from
before a kill-switch transition are discarded, and enabling the sampler
rebinds live connections.

**Residual.** The window between `accept` and the `/proc` open is not
covered. Closing it needs `SO_PEERPIDFD`, which rustix 1.1.6 does not expose;
the crate forbids `unsafe`.

**Sampling.** Every 10 s the sampler reads worker scopes through their pinned
directories. It reads ancestors once each (capped) and always the broker unit
and the host. Reads run on a blocking thread with
`openat2(BENEATH|NO_SYMLINKS|NO_MAGICLINKS|NO_XDEV)` on validated roots.
Gap records are built from those periodic samples:

- `gap_baseline_lead_ms` and `gap_end_lag_ms` state the real coverage
  against the gap's own start and end.
- A gap waits at most one tick.

Off Linux, gap records stay and scopes are unavailable.

**Kill switch.** The Device setting `diagnostics.heartbeat_gap_sampler`
(default `true`, applies live) stops unit queries and sampling. Gap records
stay. Agents discover it through Cowboy MCP `configuration.schema("device")`;
writing Device configuration stays an operator action.

**Query** (VictoriaLogs on Hawk):

```text
event_name:"cowboy.machine.heartbeat_gap" OR event_name:"cowboy.machine.heartbeat_gap.scope" OR event_name:"cowboy.machine.heartbeat_gap.dropped"
```

- Join records on host, `cowboy.heartbeat_gap.instance` (one per broker
  start), session and `cowboy.heartbeat_gap.id`.
- A window containing a `.dropped` record is incomplete.

### L0 rollout

| Step | What | Deploys via | Rollback | Flags |
|---|---|---|---|---|
| L0-0 | **Pin compatibility preflight.** On the exact integrated commit, `retained-worker-interface-compatible` must hold. As of `789e64c24`, `Cargo.toml` and `Cargo.lock` already differ from the pinned worker source `1da02869a`, so no host release builds from `main` | Build check only | n/a | If incompatible, L0 waits for, or rides on, a **separately approved worker pin advance** (`docs/machine-worker-pin-advance-runbook.md`). That replaces every live Hawk worker generation |
| L0-a | The broker gap record and the sampler | `.#cowboy-machine-writer-host-release` on Hawk (active deletion writer). OVH and Falcon use the resident-fix output their runbooks select. **Never `.#cowboy-machine-release` on Hawk while the writer is active** | See the next row | Machine component activation: restarts only the resident Machine and retains workers |
| L0-a′ | Rollback | The previous admitted resident release that keeps both writers and the accepted worker generation | 1. Remove the whole `[diagnostics]` section (an empty table is still an unknown section to an older Machine) from the Device configuration, or restore a configuration the rollback binary accepts (older Machines reject unknown keys: `docs/configuration.md:45-69`, `machine_cli.rs:513-521`). 2. Validate with that binary's `cowboy config check`. 3. Activate, then verify worker and keeper identities are retained | Machine component activation |
| L0-b | VM reproduction | No host | n/a | None |

L0 needs no Columbus change and no network change. It needs a worker pin
advance only if L0-0 fails.

### L-split requirements (deferred)

These requirements bind any future unit split. They come from revision 2 plus
round 2.

- **L-S1. Exit proof covers both trees.** Persist the boot, launch,
  `InvocationID` and cgroup identities of both the worker and the agent.
  - Write the durable revocation fence first, as today
    (`docs/requirements.md:434-449`, `machine_broker.rs:2325-2359`).
  - Then require broker-confirmed exit of both trees before cleanup,
    replacement or declaring revocation complete. A killed worker cannot
    report its agent's exit.
  - Keep the separate account-sandbox confirmation
    (`machine_broker.rs:2815`).
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
    stalls. Define an explicit data-progress deadline; today's 300 s branch
    is a backlog allowance tied to frame inactivity (`runtime_wire.rs:47,1082-1109`,
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

- **Fresh inodes for control state.** The I-8 rule applies to Machine control
  state and credential files too. Publishing by rename and chown leaves a
  writable descriptor held by the old UID usable. Test with old-UID
  descriptors held open through both cutover and rollback. Task worktrees
  keep their inodes, because cleanup records require them.

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
- **Authentication.** A gateway peer check yields `sandbox(S, server)`. That claim is authority only when sandbox UIDs are not shared across sessions or servers, by the same rule as I-5.

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

Decided by the owner on 2026-11-23 (relayed by coordinator
`sess-1794888946545`):

- **L0: go.**
  - Hawk first, then OVH. The coordinator performs the activations.
  - A worker pin advance is allowed if L0-0 requires one. It goes through the
    Machine maintenance runbook.
  - Data collection runs for two weeks or 5 gaps. Then the owner selects the
    remedy (L0-7).
- **D1: keep `draven` as the Hawk agent identity.** Later Part I slices
  will:
  - migrate the operator SSH identity and the owner's CLI and browser
    credentials out of `draven` (I-1);
  - reduce sudo to fixed wrappers.

  The `cowboy-agent` UID alternative is not pursued.

- **D2: a `hawk-admin` login.** Its key lives only on owner devices,
  mirroring OVH's `ovh-operator`; `pam_u2f` is not used. The I-1 slice
  creates it before narrowing `draven`'s sudo.
- **D3: agent-initiated activation stays recorded authority (R1).** This
  matches the owner's default-deploy preference. Part I therefore never
  claims protection against a deliberate malicious activation. The audit
  trail is the machine transaction and component receipts, and the sudo
  wrappers in I-1 must keep them mandatory.

Still open:

- **D5. Budgets for a later split.** The `cowboy-control.slice` `MemoryLow`,
  the task `TasksMax`, and the OVH numbers.
- **D6 (now binding, not a choice).** Per-session execution identities and
  writable homes (I-5) are a prerequisite for using `agent(S)` as authority.
  Antigravity round 3 may carry `agent(S)` only as observed-unit provenance
  until then.
