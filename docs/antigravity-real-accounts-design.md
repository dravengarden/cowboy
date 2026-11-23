# Antigravity real accounts on OVH — design

Status: design, revision 6, 2026-11-20 (implementation in progress; the
"Revision 6: concurrent sessions per account" section takes precedence over
"Revision 5 simplification", which takes precedence over earlier sections). Implemented so far: the sandbox
daemon (`packaging/account-sandbox/`), Machine step 3a (manifest isolation,
runtime archive, worker launch, native login/logout through the daemon) and
step 3b (Unix execution bridge, trusted process ledger in the worker endpoint,
sealed install probes, daemon-confirmed watchdog recycle; see
[Providers](architecture/04-providers.md)) and step 4 (the `agy-proxy` ACP
proxy and the Cowboy MCP facade,
[`cowboy-antigravity-acp`](../components/provider-runtime/packages/antigravity-acp/README.md))
and step 5a (the Controller's account controls, queue parking on
`uncertain_effect` and the kill switch; see
[Controller account controls](#controller-account-controls-step-5a)); the
controls UI (step 5b) remains. Native acceptance on 2026-11-18 found that
`settings.json` `permissions.deny` does not deny built-in tools in `agy acp`
1.5.9, so the second denial layer of [R2-14] does not hold for read-only
built-ins (see "Step 4 acceptance"); on 2026-11-20 the owner accepted the hook
as their only layer and Antigravity 0.2.2 declares split execution (see
[Split execution declaration](#split-execution-declaration-owner-decision-2026-11-20)). It extends
[`antigravity-provider-plan.md`](antigravity-provider-plan.md) Phases 3 and 4
under the
[Antigravity consumer-subscription boundary](requirements.md#antigravity-consumer-subscription-boundary).
Revision 1 was design-reviewed by Codex Astra Max (call
`call-b27d513a33f554b8f7097883`, 18 findings, **[R1-n]**) and revision 2 too
(call `call-448741b8535e72fe7823b378`, 18 findings, **[R2-n]**). Findings that
assume a hostile `ubuntu` process (R2-1, R2-2, R2-4 and the caller parts of
R2-5) are outside the owner's trust model below; the rest are answered inline.
Revision 3's review (call `call-52d3994a3d9ece0fbc1bdc27`, 17 findings,
**[R3-n]**) is answered in [Revision 4 amendments](#revision-4-amendments),
which take precedence over earlier sections where they differ.

## Owner decisions (2026-11-18)

- Four real Google accounts, each a named Cowboy account the owner names, each
  signed in by the owner personally.
- Runtime and sign-in on **OVH**: sign-in only in that account's isolated
  Machine browser on OVH; `agy` runs on OVH while workspace tools execute on
  the session's development Machine.
- The dedicated test account goes through the whole flow on OVH and a soak
  first; real accounts are then admitted one at a time, each after a soak.
- All accounts share OVH's IPv4 egress.
- Isolation is done properly before any real account: an OS identity per
  account, not same-user confinement.

## Trust model (owner decision, 2026-11-18)

The design keeps Cowboy's recorded trust boundary
([`requirements.md`](requirements.md), [Machine browser](machine-browser.md#account-isolation-core-requirement)):
root, the Machine supervisor and same-user host processes on a Machine are
trusted. Revision 2's review asked to treat every `ubuntu` process on OVH as
hostile; the owner declined that model because no model-driven code runs as
`ubuntu` on OVH: Claude and Codex sessions there are split-execution runtimes
whose tools execute on the development Machine, and Antigravity's run in its
own identity (below). A malicious vendor runtime is trusted to the same degree
as the Machine itself.

**Untrusted:** every model-driven action of every Provider (tool calls, prompt
injection, repository content); Google's responses; and each Antigravity
account's whole process tree (official `agy`, proxy, hooks) relative to every
other account and to the rest of the host.

**Goals:**

1. No Antigravity account's processes can read, write or use another
   account's state, or any other Provider's credentials or the Machine's state.
2. No other Provider's native process reads Antigravity state through ordinary
   file access (different owner, mode 0700).
3. Even if an Antigravity hook fails, the model cannot read its own account's
   OAuth token through `agy`'s tools (two independent denial layers).
4. Every Antigravity network connection, including login and logout, is IPv4.
5. No credential leaves OVH; Cowboy never reads it.

## Why separate OS identities

Same-user Landlock confinement (revision 1) fails on these kernels: pathname
Unix-socket restriction needs Landlock ABI 9, and Hawk reports 7, OVH 8
**[R1-1]**. Other Providers' same-user processes would keep plain file access
to the tokens **[R1-2]**.

**Spike on OVH (2026-11-18, transient units, removed afterwards):** a
`systemd-run --pipe --wait` system unit with a dedicated identity,
`ProtectHome=tmpfs`, `ProtectSystem=strict`, `PrivateTmp=yes`,
`NoNewPrivileges=yes` and `RestrictAddressFamilies=AF_INET AF_UNIX
AF_NETLINK` saw an empty `/home`, no `/run/user/1000/bus`, failed
`socket(AF_INET6)` with `EAFNOSUPPORT`, piped stdio, and ran the official
`agy` 1.5.9 bound read-only (`--version`, ACP `initialize`).

## Architecture

```
Controller ── Machine (ubuntu) ── worker (ubuntu)
                    │                 │ stdio (ACP), unit handle
                    ▼                 ▼
        cowboy-account-sandbox (root daemon, Unix socket, group-restricted)
                    │ systemd-run --pipe (system unit per process)
                    ▼
   account unit  User=cga-<key> (static system user)
      agy-proxy (Node, signed runtime) ── agy acp (official, pinned)
         └─ facade MCP (in-process, stdio pipe) ── execution endpoint
```

### Sandbox daemon

A static Rust binary `cowboy-account-sandbox`, built on Hawk from this
repository and installed by Columbus `machines/ovh` host configuration with an
activation receipt (OVH builds nothing), as a socket-activated root system
service on `/run/cowboy-account-sandbox/socket`, mode 0660, group
`cowboy-sandbox` (members: `ubuntu`). The caller is the trusted Machine; the
protocol is still narrow (defense in depth, simple audit):

- **Accounts.** `ensure-account <service_id> <account_id>` creates the static
  system user `cga-<key>` (`key = hex(sha256("cowboy-account-v1" ||
  service_id || account_id))[..16]`, 20-character names within systemd's
  31-character limit **[R2-15]**; no shell, no home in `/etc/passwd`, no
  groups) and the account tree. Removing the account removes the user after
  its tree.
- **Runtime store.** `register-runtime` receives the generation's runtime as a
  bounded archive over the socket (bytes, not a path; never a root reopen of a
  caller path **[R2-5]**), verifies its SHA-256 against the value the Machine
  supplies and the vendor `agy` executable's SHA-512 against a root-held
  allow-list written by the host configuration, unpacks it with no links or
  special files into `/var/lib/cowboy-account-sandbox/runtime/<sha256>/`
  (root-owned, read-only), and bounds the store (size and count, retaining
  every runtime a live unit or a pending cleanup uses) **[R2-18]**.
- **Fixed argv.** The caller names a mode (`runtime`, `login`, `logout`,
  `probe`), account key, generation and runtime digest; the daemon builds the
  command line, environment and unit properties. Per-turn execution
  descriptors arrive as bytes and are written by the daemon into a private,
  daemon-created credential file passed with `LoadCredential=` **[R2-5]**.
- **Replies** are fixed enums, unit handles, and the login unit's own output
  stream for the challenge; the daemon never returns account file content.

### Account directory layout **[R2-7]**

```
/var/lib/cowboy-account-sandbox/            root 0711
  accounts/<key>/                           root 0711
    meta.json                               root 0600  (generation states, ops)
    home/                                   cga-<key> 0700  (the account's HOME)
    units/<invocation>/                     root 0711
      process-ledger/                       cga-<key> 0700  (facade process IDs)
```

One persistent home per account: conversations, `installation_id` and caches
stay across sign-ins, so the account always looks like the same installation
to Google and history survives sign-out **[R2-13]**. The unit binds only
`home/` (read-write) and its own `process-ledger/`; nothing else under the
account or the store is writable, and lifecycle metadata is root-owned outside
every mount.

### Generations and login **[R2-3, R2-8]**

The Controller remains the single generation authority: it reserves the exact
generation and request (existing native custody). The daemon records each
generation idempotently by `(generation, operation_id)`. A `login` unit runs
only when the account's token file is absent; it writes the token into the
persistent home. An interrupted login (unit gone without a completed receipt)
leaves the generation `login_interrupted`; the daemon removes a partial token
and the Controller may retry with a new request. `runtime` units run only on
the current `signed_in` generation. Machine-local home checks
(`native_profile.rs:428,648`) are replaced by daemon receipts.

`owner_declared_v1` binds the generation to the Cowboy account the owner chose;
the owner is responsible for signing the intended Google account into that
card's browser. The proxy's mode-specific ACP allow-list **[R2-3]** permits in
`runtime` mode only `initialize`, `session/new|load|resume|prompt|cancel|
set_mode|set_model|set_config_option` and notifications, and refuses
`authenticate`, `logout` and every unknown method, so a runtime cannot change
or clear the account's sign-in. `login` and `logout` modes run only their own
fixed exchange.

### Account units

Each mode runs as one transient system unit
`cga-<key>-<mode>-<nonce>.service` started with `systemd-run --pipe`; stdio is
returned to the worker as daemon-created pipes. Properties:

| Property | Value |
|---|---|
| Identity | `User=cga-<key>`, `Group=cga-<key>`, `SupplementaryGroups=` none |
| Filesystem | `ProtectSystem=strict`, `ProtectHome=tmpfs`, `PrivateTmp=yes`, `PrivateDevices=yes`, `TemporaryFileSystem=/var/lib/cowboy-account-sandbox`, `BindPaths=` the account `home` and this unit's ledger, `BindReadOnlyPaths=` the runtime store entry |
| Process | `NoNewPrivileges=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `ProtectKernelTunables/Modules/Logs=yes`, `ProtectControlGroups=yes`, `ProtectClock=yes`, `ProtectHostname=yes`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`, `MemoryDenyWriteExecute=no` (Node JIT), `SystemCallArchitectures=native`, `SystemCallFilter=@system-service` without `io_uring_*`, `SystemCallErrorNumber=ENOSYS` (a filtered call fails instead of killing the process: Node's libuv probes `io_uring_setup` and falls back on `ENOSYS`), `CapabilityBoundingSet=` empty, `ProtectProc=invisible`, `ProcSubset=pid`, `PrivateIPC=yes`, `RemoveIPC=yes`, `UMask=0077` |
| Network | `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK`; `IPAddressDeny=localhost link-local multicast` so no host loopback service is reachable **[R2-6]**; acceptance enumerates OVH's abstract Unix sockets and pathname sockets reachable from the unit |
| Environment | closed: `HOME`, `PATH`, `LANG`, `TZ`, `TMPDIR`, `AGY_CLI_DISABLE_AUTO_UPDATE=true`, `DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent`, `CREDENTIALS_DIRECTORY` **[R1-4]** |
| Resources | per unit `MemoryMax`, `TasksMax`; `Slice=cowboy-accounts-<key>.slice` (at most `max_runtime_units` `runtime`/`probe` units and one `login`/`logout` unit per account; see Revision 6) under `cowboy-accounts.slice` with a host-wide `MemoryMax` and unit count; home size quota checked by the daemon before each start **[R2-18]** |

**Unit ownership [R2-9].** The daemon ties each unit to the requesting
connection: when the worker's connection closes, the daemon stops the unit and
confirms the whole unit is gone (no remaining PIDs, `InvocationID`, boot ID).
The worker's handle (unit name, `InvocationID`, boot ID) is part of its launch
record, so the existing teardown and recycle paths ask the daemon to stop and
confirm instead of killing a child. On Machine start, `list-units` reconciles:
a unit no live worker claims is stopped. Login and logout receipts report the
account unit's own exit and the daemon's state transition, not the relay's.
Machine maintenance and reboot: units are stopped with their workers; reboot
convergence holds because the daemon is socket-activated and stateless apart
from `meta.json`.

## Kill switch and logout **[R1-6]**

- **Fence first.** A per-account `blocked` state is written durably on the
  Controller before anything else; every dispatch, revival, login and session
  creation checks it (see controls).
- **Stop.** The daemon `stop-account <key>` stops every unit of the account
  (`systemctl stop` of the account slice) and reports the confirmed unit set.
- **Native logout without deleting history [R2-13].** Logout runs the ACP
  `logout` exchange in a `logout` unit; the daemon then removes **only** the
  vendor token file (`.gemini/antigravity-cli/antigravity-oauth-token`) if
  `agy` left it, and records `credential_cleared, history_retained`. The
  persistent home keeps conversations and `installation_id`, so the next
  sign-in continues them. Machine and Controller receipts gain these explicit
  outcomes instead of equating logout with `profile_cleaned`
  (`machine_cli/command_login.rs:65`). Account removal (owner action, with
  confirmation) is a separate destructive operation that requires a prior
  logout, stops every unit, deletes the account tree and removes the system
  user.
- **Offline Machine.** The operation is durable with one operation ID and stays
  pending until the Machine reconnects; the fence holds meanwhile.

## Phase 4: split execution

**Proxy.** `agy-proxy` (Node, `node_npm` recipe, runs inside the account unit)
spawns `agy acp --disable-telemetry`, relays ACP and rewrites
`session/new`, `session/load` and `session/resume`:

- replaces `mcpServers` entirely with the one facade server **[R1-5]**;
- sets `_meta.antigravity.systemPrompt.append` to the projection;
- keeps the target path as `cwd`; the unit's mirror directory is an empty
  tmpfs path inside the unit, never a shared runtime directory **[R1-13]**.

**Machine changes.** Antigravity's signed manifest declares
`runtime.isolation: "account_sandbox_v1"` (new optional runtime field, SDK
floor raised; older hosts reject it). A Machine without a reachable daemon
reports the Provider unavailable and refuses every launch, login, logout and
probe; the Hawk local mode is removed. Native login, logout and cleanup call
the daemon; the worker starts the agent through it and relays the returned
pipes; install probes use the `probe` mode.

**Facade identity [R1-5].** MCP server name `cowboy`; exact tool set
`ReadFile`, `ListDirectory`, `SearchFiles`, `WriteFile`, `EditFile`,
`RunCommand`, `StopCommand` (and, since 2026-11-20, `UpdatePlan`, which never
reaches the target; see "Feature inventory and Cowboy adaptation"). The hook allows exactly
`mcp_cowboy_ReadFile` … `mcp_cowboy_UpdatePlan` plus `ask_question` (and,
since 2026-11-20, the web built-ins; see "Web search and URL reading"), and
denies everything else, including unknown names. Startup verifies `agy`'s
tool listing does not contain any other MCP tool. `agy`'s user and workspace
configuration sources are fixed: the daemon bind-mounts read-only, from the
runtime store, `~/.gemini/config/hooks.json` and
`~/.gemini/antigravity-cli/settings.json` over the account home, so neither
the model nor `agy` can change them, and runs `agy` with
`--include-default-customizations=false`.

**Two denial layers [R2-14].** (1) The `PreToolUse` hook `hook.mjs` allows
only the exact facade tools and `ask_question`. (2) `settings.json`
`permissions.deny` lists every built-in file, command, subagent, schedule and
workflow tool, which `agy` enforces in its permission layer independently of
hooks. Native acceptance must show, on the pinned `agy`, that a missing,
timed-out, malformed, crashing and killed hook each still leaves the tool
denied (or ends the session); if any case fails open on either layer, split
execution stays unavailable. Even then, both layers failing would expose only
the account's own home: the unit has no other account's or Provider's data.

**One connection owner [R1-14].** The proxy owns the single
`execution-connection.mjs` `Connection` (the endpoint admits one connection).
The facade is an in-process MCP server exposed to `agy` over a stdio pipe the
proxy creates; readiness and tools share that connection.

**Target permissions [R1-3].** Ordinary execution bindings forward requests
unchanged; the facade is a model-driven agent with the same rights as a Claude
or Codex session on the development Machine (full workspace and user access
there). This is accepted and stated: the four Antigravity tokens are on OVH,
not on the target, so target access cannot cross Antigravity accounts. The
facade still resolves every path against the session cwd and rejects paths
outside the workspace root after `fs/canonicalize` (symlinks resolved on the
target) for `ReadFile`/`WriteFile`/`EditFile`/`ListDirectory`; `RunCommand` is a
shell on the target by design.

**Mutations [R1-11, R1-12, R2-16].** `WriteFile` and `EditFile` use the
execution helper's conditional write (compare the observed content hash,
publish atomically, report a conflict). The execution protocol carries no
caller idempotency key, so a mutation or process start whose reply is lost is
**not** resolved by observation: the facade reports the tool call as
`outcome unknown`, ends the turn, and marks the session `uncertain_effect`;
the session's queue parks until the owner reviews the target and continues.
No automatic retry exists anywhere. Tested with repeated MCP calls and cold
`session/load`.

**Cancellation [R1-10, R2-10].** The facade allocates each process ID before
`process/start` and appends it to the unit's `process-ledger/` (bound outside
the home, readable by the daemon) before sending. Cancel order: mark the turn
cancelled (no new start); for every ledger ID, wait for its start reply if
one is in flight, then `process/terminate` until `process/exited`. An unknown
ID during an in-flight start is not treated as terminated. If the proxy dies,
the daemon returns the ledger to the worker with the unit's exit, and the
worker (outside the unit, holding the same execution binding) terminates every
listed ID through a fresh `Connection` before the turn's reservation is
released.

**Bounds [R1-16].** `ReadFile` ≤ 256 KiB per call (offset/length paging),
`ListDirectory` ≤ 2,000 entries, depth 1; `SearchFiles` ≤ 500 matches and
512 KiB output, 30 s; `RunCommand` output ≤ 256 KiB retained (head and tail),
default timeout 120 s, maximum 600 s; every request below the connection's
14 MiB frame. Projection: Git status ≤ 200 lines, instruction snapshot ≤ 64 KiB
total, each file ≤ 16 KiB, explicit `[truncated]` markers.

**Context suppression [R1-15, R2-17].** Native sources of runtime context are
enumerated and closed: the cwd is an empty unit-private directory with the
target's path (no files, no Git); the home has no guidance, skills, workflows
or memories except what Cowboy mounts (`GEMINI.md` absent; `agy`'s own
`brain/` and memories are per account and contain only that account's
Antigravity history); `--include-default-customizations=false`. The runtime
OS equals the target's on Hawk (Linux); a different target OS is refused.
Acceptance plants distinct canary guidance on the runtime side (a `GEMINI.md`
in a test home and cwd) and the target, and requires that new, load, resume
and compacted sessions never surface the runtime canary. If `agy` exposes a
runtime context source this design does not close, split execution stays
unavailable. Repository text in the projection is fenced as untrusted
content, separate from Cowboy's routing instructions.

**Declaration and placement.** `provider.execution-jsonrpc.v1` with the
executor digest, only after native acceptance on an exact generation; installed
on OVH; `--provider-runtime-machine antigravity=ovh`.

## Account controls

**Admission list.** `accounts.antigravity_admitted` (list of account IDs,
default empty) replaces the boolean. Membership is checked at login start,
login completion, session creation **and every dispatch and worker revival**
**[R1-9]**.

**Account state.** Durable per account on the Controller: `ok`,
`verification_required`, `location_unsupported`, `rate_limited_until`,
`suspended`, `blocked` (kill switch), `unknown_refusal`. Any state other than
`ok` parks the account's queues; `rate_limited_until` parks them until that
time, and the next dispatch after it resumes the account (owner-paced since
2026-11-22; earlier only a new owner send did); `suspended` and `blocked` stop the account's
units and need an explicit owner action to clear.

**One active turn per account [R1-8, R2-11].** The Controller reserves the
account slot atomically (durable row: account, session, turn, worker epoch)
before dispatch, and records the dispatch and the unit handle (name,
`InvocationID`, boot ID) the worker reports. The slot is released when one of
these is durably recorded: the turn's single terminal ACP response with its
refusal evidence settled; or **proven** loss of the turn's unit (the daemon
reports the exact invocation gone, or the boot ID changed) after the worker's
ledger termination, which settles the turn `interrupted` without replaying
anything; or a reservation that never reached dispatch (no dispatch record),
settled `not_dispatched`. Anything else keeps the slot and shows the account
"needs attention"; the owner can force-release after a confirmed account stop.
Late replies after a terminal record are discarded.

**Refusal evidence [R1-7, R2-12].** Log-watching is not accepted as
authoritative until proven. Acceptance with the test account records the exact
`agy` log lines and ACP shapes for successful turns (single- and multi-step),
`VALIDATION_REQUIRED` and `UNSUPPORTED_LOCATION` (both observable without
provoking enforcement), and how each line is attributed to a turn and backend
step. Parsing reads only backend status fields (code, reason enum), never
model text, from the unit's own log file with a per-turn cursor and a bounded
reader. Rules: every backend request of the turn must have a matching success
record; any refusal record takes precedence over successes; a turn is settled
only after an evidence-completion barrier (the turn-end record `agy` writes
after its last backend step, or the unit's exit with the log closed). Missing
or ambiguous evidence → `unknown_refusal` (parks the account, owner decides).
A proven infrastructure interruption (unit lost, target lost) is
`interrupted`, not a refusal.

## Sign-in

- The account's Machine browser on OVH, with `accounts.machine_browser` and
  the account's `browser_enabled`; one browser identity per account.
- Browser profile state remains within the recorded same-user trust
  boundary of the Machine browser (owner decision); browsers of different
  accounts stay strictly isolated from each other. The explicit exception is
  an owner-configured browser share (`docs/machine-browser.md`, "Shared
  browser profiles"): an Antigravity account (family `google`) may join one
  share with accounts of other families, never with another Antigravity
  account, only while the share's browser lives on a Machine that enforces
  IPv4-only browsers (its Device setting `browser.ipv4_only`, which runs
  every browser unit there IPv4-only and is reported in its browser
  capability; OVH) where Antigravity is installed and its native login may
  run. An Antigravity account's own browser keeps naming `ipv4_only_v1` in
  its `Open` (R1-17).
- **[R1-17]** The browser runs IPv4-only (R3-15) for accounts of a
  Provider whose manifest declares `ipv4_only_v1`, before the first sign-in;
  acceptance records the address family of the OAuth requests through the
  host's connection table during a real sign-in.
- Dialog copy becomes Provider-neutral; the browser policy's new-tab
  shortcuts add `antigravity.google`.
- The code the callback page shows is pasted into the dialog; the native login
  runs in a `login` unit.

## UX

**Account card**: `Antigravity · <name>`, Machine OVH, state (Not signed in ·
Signed in · Verification required · Location unsupported · Rate limited until
… · Suspended · Blocked · Needs attention), "In use by 1 turn", usage "Not
available", the notice "Use of a personal Google account in Cowboy is not
supported by Google", Admitted/Not admitted, buttons Sign in, Open browser,
Sign out (keeps history), Block/Unblock, Remove account (confirmation).

**Sign-in dialog**: native login state, the account browser entry, the code
field; no link outside the account browser.

**New session picker**: one entry per admitted, signed-in, `ok` account;
preselected only when it is the sole choosable one; others disabled with the
reason; no default entry.

**Session header and Settings**: account name and state; a parked queue shows
why.

## Rollout **[R1-18]**

1. Implement: sandbox daemon and host configuration; Machine integration;
   proxy and facade; controls; UX. Gates and Codex review per step.
2. Release; OVH host activation; OVH Machine maintenance; install on OVH.
3. Test account: admit; sign-in; isolation canaries (from account A's unit,
   and from an `ubuntu` process, reads of account B's token, `ubuntu`'s home
   and another Provider's credential fail; daemon refuses foreign argv and
   reused generations); IPv4 evidence; Phase 4 native acceptance; refusal
   evidence; controls.
4. Test-account soak: 3 days of ordinary use with no verification or
   enforcement signal; receipts kept.
5. Real account 1; soak 2 days; then 2, 3, 4, each after the previous soak.
   Stop conditions: any verification, location, suspension or unknown refusal
   on any account halts further admissions until the owner decides.

## Revision 4 amendments

- **[R3-1] Owner decision: accepted.** Every agent session on Hawk runs as
  `draven`, whose operator SSH reaches OVH as root (`ssh ovh`, passwordless
  sudo). A prompt-injected session of any Provider can therefore reach every
  account's state through root on OVH. The owner accepted this residual risk on
  2026-11-18 rather than restricting operator access; it is listed below.
- **[R3-2] Privileged traversal.** Every daemon operation inside an account
  tree (cleanup, token removal, ledger and size reads) opens paths with
  `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
  RESOLVE_NO_XDEV)` relative to a directory descriptor of the account root
  opened at daemon start-up, never by pathname.
- **[R3-3] Host IPC and addresses.** The unit is started through the daemon's
  own `exec-confined` mode as `ExecStart` wrapper, which applies Landlock
  scoping (`LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET | LANDLOCK_SCOPE_SIGNAL`, ABI 6,
  available on OVH's ABI 8) before exec, so abstract sockets and signals outside
  the unit are unreachable. `/run` and `/var/run` are masked
  (`TemporaryFileSystem=`), re-exposing only the unit's own bridge socket and
  `/run/systemd/resolve` read-only. `IPAddressDeny=localhost link-local
  multicast` plus OVH's own addresses and RFC 1918/6598 ranges, with
  `IPAddressAllow=127.0.0.53/32` for the resolver stub. Acceptance attempts
  connections to host sockets and services and requires refusal.
- **[R3-4] Daemon account fence.** `stop-account` first persists
  `fenced(epoch)` in `meta.json`; every start takes the account lock and
  refuses when fenced; only operation-scoped cleanup and logout run while
  fenced; `admit-account` clears it.
- **[R3-5, R3-6] Trusted process ledger.** The ledger lives in the worker's
  execution endpoint (`src/worker_execution.rs`, Cowboy code outside the
  unit), which already relays every request: it records each `process/start`
  per session and turn before forwarding, refuses starts after a cancel
  fence, and terminates recorded processes on cancel, turn end, unit loss and
  teardown, observing completion through `process/read` rather than waiting
  for a consumed `process/exited`. The Machine persists pending terminations
  per execution binding; if the worker dies or OVH reboots, the Machine
  re-opens the binding on reconnect and completes them. A reservation (even a
  forced release) settles only with target termination evidence or the
  target's proof the process is gone.
- **[R3-7] Bounded storage.** Each account home is an ext4 image of fixed size
  (default 4 GiB, fixed inode count) that the daemon loop-mounts per unit;
  `/tmp` is `TemporaryFileSystem=/tmp:size=256M`; `agy`'s log directory is
  inside the image. The daemon keeps a host-wide free-space reserve and refuses
  starts below it.
- **[R3-8] Exclusive authentication transitions.** `login` and `logout` take
  the account fence, stop every runtime unit of the account and confirm them
  gone first; `meta.json` records the credential generation; runtime units
  carry it, and a runtime unit of an older credential generation is never
  started. Cleanup of an interrupted login applies only to the generation it
  owns.
- **[R3-9] Reservation states.** `reserved` → `may_have_dispatched` (durable,
  with worker epoch and turn ID, written before the prompt is enqueued) →
  terminal. Only a reservation still `reserved` can settle `not_dispatched`.
- **[R3-10] Evidence on every path.** Every settlement, including unit loss,
  runs evidence reconciliation over the unit's log (read by the daemon from the
  image after the unit stops): a refusal record always wins; complete success
  evidence with infrastructure loss settles `interrupted`; anything else is
  `unknown_refusal`.
- **[R3-11, R3-17] Transports.** The worker's execution endpoint additionally
  listens on a Unix socket the daemon creates per unit
  (`/run/cowboy-account-sandbox/units/<invocation>/exec.sock`, owner `ubuntu`,
  connectable by `cga-<key>` only, bearer token unchanged), bound into the unit
  as its only reachable host socket. Inside the unit the proxy owns the single
  execution connection and listens on a unit-private socket; `mcpServers`
  names the signed stdio relay `facade-relay.mjs`, which `agy` launches and
  which connects to that socket.
- **[R3-12] Daemon restart.** Unit ownership (worker connection identity,
  unit name, `InvocationID`, boot ID) is persisted before a launch is
  acknowledged; on every activation the daemon reconciles persisted units with
  systemd, stops unclaimed ones, and keeps unresolved cleanups. A worker whose
  control connection drops stops relaying, asks the daemon (after it returns)
  for its unit's state, and reports its session crashed unless the unit is
  confirmed alive and still owned.
- **[R3-13] Sessions across sign-ins.** After a new sign-in, existing
  sessions of the account move to the new credential generation through the
  existing native-session transition (stop evidence, `ReadmitAccountSession`),
  keeping their `agy` conversation IDs because the home is persistent.
  Acceptance covers logout → login → resume.
- **[R3-14] Immutable policy paths.** `~/.gemini` is a read-only bind of a
  daemon-assembled tree; `~/.gemini/antigravity-cli` is the writable state bind
  inside it; `hooks.json` and `settings.json` are read-only file binds, so
  neither they nor any ancestor can be renamed or replaced (`EBUSY`/`EROFS`).
  Acceptance tests ancestor replacement besides hook failures.
- **[R3-15] Browser IPv6.** For accounts of an `ipv4_only_v1` Provider, the
  account browser runs under the IPv4-only sealed AppArmor profile
  (`cowboy-browser-sealed-ipv4`, no `inet6` socket, also for `io_uring`
  socket creation). The unit carries no seccomp property: a user unit's
  seccomp filter sets `no_new_privs`, which makes AppArmor refuse the
  launcher's transition to the sealed profile (`docs/machine-browser.md`). Acceptance
  attempts IPv6 connections from the browser and requires failure, besides
  recording a real sign-in's address family.
- **[R3-16] Several idle sessions.** Each session has its own runtime unit
  (at most 4 per account plus 1 authentication unit); the one-turn limit is
  enforced by the Controller's reservation, not by the unit count.

## Controller account controls (step 5a)

Implemented as specified in "Account controls", "Kill switch and logout" and
R3-4, R3-9, R3-10, R3-13 and R3-16, with these concrete choices:

- **Admission.** `accounts.antigravity_admitted` (Service, live, at most 16
  account IDs) replaces the boolean. `named_only_admitted` (login start,
  native login completion and its recovery, session creation) also refuses a
  blocked account; the dispatcher's account gate checks it before any
  execution recovery or revival, and the wake/recovery admission of
  `session_maintenance` (`revival_gate`) refuses every revival of a governed
  session that is not admitted or not `ok`.
- **Records** (migration `0085` / SQLite `0059`):
  `provider_account_controls` (state, bounded reason, `until_ms`, revision),
  `provider_account_turns` (one row per account: session, a
  Controller-generated turn ID, the prompt's delivery ID (`cmid`), the worker
  epoch and runtime Machine at marking, the session's event cursor
  `after_seq`, state `reserved | may_have_dispatched`, and the turn's terminal
  record once delivered) and `provider_account_fences` (one current fence per
  account and Machine, `stop | admit`, `pending | confirmed`). A state change
  writes its fences in the same transaction.
- **Parking.** A refused dispatch requeues the prompt unchanged and shows the
  reason as the session's resource wait; the maintenance loop rechecks the
  cached controls before draining again. Any state but `ok`, a missing
  admission, another session's turn or an unacknowledged uncertain effect
  parks. A rate limit (default one hour; the proxy reports no reset time)
  parks until `until_ms`; the first dispatch after it, owner or automatic,
  resumes the account. Admission and the
  kill switch at login and session creation are read from the store, never
  from the cache; a failed reload marks the cache unloaded, which parks.
- **One active turn.** Every dispatch of a governed session, user prompts
  and supervised follow-ups alike, reserves the account before any
  preparation. The final eligibility check (admission from the live
  configuration, state, uncertain effect, the reservation itself) and the
  `may_have_dispatched` mark run under the control writers' lock, which is
  held until the prompt was handed to its runtime; admission edits apply to
  the live configuration under the same lock. Every transition matches
  account, session and the Controller-generated turn ID. The slot is
  released when a path never enqueued the prompt (positive in-process
  proof). The runtime reports every `TurnEnded` with the delivery ID of the
  prompt it ends and the worker epoch; only the record of exactly the
  reservation's delivery and epoch settles it, classified only from the
  proxy's structured result (the closed code and the same code in the error
  data's `cowboy` member; anything else is `unknown_refusal`). A stopped,
  replaced or deleted worker proves nothing: loss settles only on the
  worker/Machine ledger's termination evidence for the turn's original
  binding (`Env::termination`, `MachineCommand::QueryAccountTurnTermination`
  to the turn's runtime Machine), and its closed-log class decides between
  `interrupted`, a refusal and `unknown_refusal`. The Machine reports the
  target terminated only when no worker of the turn's epoch runs (a
  connected worker of another epoch, or no connected worker and a launch
  journal that is absent or records the tree exit), the session's execution
  ledger records no process and no account unit of the session survives
  its worker. Two parts are conservative: the Machine never reports a
  closed-log class (the turn's log window, the unit's `agy` process, byte
  cursor and conversation, is known only to the proxy inside the unit), so
  a proven loss settles `UnitLost` without complete evidence; and a ledger
  that still records processes is drained only by the next worker of the
  same binding, because the Controller forwards execution requests only
  from the session's current worker epoch, so the Machine answers
  `terminated: false` and the reservation keeps the slot. A `reserved` row no process
  is dispatching settles `not_dispatched`; anything else keeps the slot and,
  after 2 minutes, marks the account `needs_attention`. Settlement is one
  transaction (state, stop fences, release) that compares with the stored
  state, so it never lowers a more severe state and never replaces `blocked`;
  a failed write keeps the slot. Every worker launch, revive, reset, reload
  and replacement passes the account gate (`Supervisor::start_session`).
- **Fences.** `MachineCommand::AccountSandboxFence` (Machine protocol 48) asks
  the Machine's daemon for `stop_account` or `admit_account`; only the typed
  `AccountSandboxFenceObserved` for the same request confirms it, and
  `stop_account` answers only after it confirmed every unit gone. The
  operation ID is derived from the account, Machine, action and control
  revision, so retries after a lost reply repeat the same request; pending
  fences are retried while the Machine is connected. The request carries
  that originating revision (recovered from the operation ID, never the
  current one), and the daemon orders fences by it under the account lock:
  an older revision, or another operation at the applied one, is its
  `conflict`, which never confirms the fence. The Machines are the
  account's native generations plus its sessions' runtime Machines.
- **Kill switch.** `block` (with confirmation) writes `blocked` and the stop
  fences first, then the fences stop and confirm the units; `unblock` writes
  `ok` with admit fences and first cancels every logout target no Machine
  was asked. `force_release` of an unsettled turn requires every fence of
  the account to be a confirmed stop and the ledger's termination evidence
  for the turn. The account's Machines include its sessions' execution
  targets; a failed lookup refuses the write instead of fencing fewer.
- **Logout (step 5b).** `logout` (with confirmation) needs `blocked` and a
  confirmed stop. The request is recorded durably (internal setting
  `provider_account_logout:<account>`, one operation ID per control revision)
  with one target per Machine holding a signed-in generation; the reconciler
  delivers each target through the Machine's native logout command (the
  daemon's `logout` unit and `logout_cleanup`: `credential_cleared`,
  `history_retained`) when the Machine is connected, so an offline Machine
  stays `pending`. The outcome is recorded on that generation's login receipt,
  which then admits no session and reads `signed_out`; the account and its
  home remain. Unblocking before a target was ever sent cancels it. A later
  revocation accepts the recorded logout instead of asking the Machine for a
  second logout of the same generation.
- **Revocation removes the sandbox.** Unlike logout, revoking a named-only
  account must destroy its persistent home and system user on every Machine
  it signed in or ran a session on. Once every session is stopped and every
  native profile cleaned up, the revocation asks each Machine's daemon for
  `remove_account` under an operation ID derived from the revocation,
  account and Machine (so every retry repeats the same request), and records
  the receipt only when it answers exactly that request with both removals
  confirmed (`MachineCommand::AccountSandboxRemove`). The Machine first sets
  the daemon's fence with an unrevisioned stop (it stops every unit and only
  tightens), then calls `remove_account`, which requires the fence and a
  logged-out credential, records the removal in the fence tombstone and
  deletes the home, then the user, then the metadata, so a retry after a
  crash finishes it and a retry after a lost reply is its replay.
  Completion requires a receipt from every such Machine; an offline Machine
  or one below protocol 48 keeps the removal `pending`.
- **Login recovery.** A sandboxed login abandoned by a Machine restart is
  reconciled against the daemon's record of exactly its generation once no
  login unit of it is alive: `signed_in` completes it (the Controller's
  native recovery then admits it), anything else fails it; an unanswered
  daemon leaves it reserved.
- **Detection.** `suspended` and a policy violation (`needs_attention`,
  reason `policy_violation:<tool>`) stop the account's units like a block.
- **Uncertain effects.** The worker ledger counts lost replies
  (`UncertainEffect.sequence`); the Controller records the newest observed
  sequence per session durably and parks that session until the owner
  acknowledges exactly that sequence.

## Account UX (step 5b)

- `accounts.list()` adds `named_only_providers`; the Web reads it so a
  named-only Provider never offers a default account.
- **Account card** (`AccountControlsPanel`): title `Antigravity · <name>`,
  the state badge (sign-in observation for `ok`; reason and `until` for the
  others), "In use by 1 turn" with the holder session, the plan `agy`
  reported ("Plan · Antigravity Starter Quota", see "Antigravity features"),
  usage "Not available", the Google notice, the Admitted switch, Clear state
  (with its reason), Block / Unblock, Sign out (blocked, stop confirmed), Stop units (pending
  fences), Force release (turn held, stop confirmed); Block, Sign out and
  Force release confirm first. Sign-in, Machine browser and Remove account
  stay the existing controls; "Use by default" is hidden.
- **Picker**: no default entry; the sole choosable account is preselected,
  several leave the choice to the user. An account is choosable only when
  admitted, signed in and `ok`, otherwise listed disabled with the reason.
- **Session**: Session Settings show the account state; a parked queue (or an
  uncertain effect) shows why in Settings and in the composer's register line,
  with Acknowledge for an uncertain effect (method shown; the target must be
  checked first; nothing is resent).
- **Sign-in dialog** copy is Provider-neutral.

## Step 4 acceptance (Hawk, 2026-11-18)

Run with the dedicated test account's home, the official `agy` 1.5.9, no
daemon, and `bwrap` standing in for the unit: the target path hidden by an
empty tmpfs, `hooks.json` and `settings.json` bound read-only, the closed
environment, and the pinned Codex exec-server 0.159.3 serving a temporary
target directory as a `unix:` execution endpoint.

- Proxy and facade (1 prompt): `read_file` was attempted and denied by the
  hook (`failed`), `mcp_cowboy_ReadFile` returned the target-only sentinel,
  `session/prompt` carried `antigravity_backend.class = ok` (4 success
  records, barrier seen; one retried `UNAVAILABLE (code 503)` attempt counted
  as transient), `authenticate` was refused, and the endpoint saw one
  connection. The hook saw the facade tool as `mcp_cowboy_ReadFile`.
- Hook failures (direct `agy`, prompt: read a probe file with `read_file`):

| Hook | `settings.json` deny | Result |
|---|---|---|
| (a) command does not exist | absent | denied: `exit status 127` |
| (b) times out (`timeout: 3`) | absent | denied: `signal: killed` |
| (c) prints malformed JSON | absent | denied: `failed to unmarshal result from hook` |
| (d) exits non-zero | absent | denied: `exit status 3` |
| (a) command does not exist | present | denied: `exit status 127` |
| (d) exits non-zero | present | denied: `exit status 3` |
| `hooks.json` itself unparseable | present (`"read_file"` names) | **read executed** |
| hook allows | present (`read_file(*)`, `command(*)` rules) | **read executed**; `run_command` asked permission |

  Every hook failure mode fails closed, but `agy` logs `Failed to parse hooks
  file` and runs without hooks when `hooks.json` does not parse, and neither
  deny-list syntax stopped `read_file`. The proxy therefore also validates the
  gate's shape at readiness and rejects every `session/request_permission`
  for a non-facade tool itself (an independent layer only for tools that ask:
  `run_command`, `write_file`). Read-only built-ins remain gated by the hook
  alone; split execution stayed undeclared until a second layer was proven or
  the owner accepted the hook as the only one for reads, which the owner did
  on 2026-11-20 (next section).
- The (b) and (c) cases with `settings.json` present were not run (prompt
  budget); with the deny layer shown ineffective they reduce to the absent
  rows.

## Split execution declaration (owner decision, 2026-11-20)

The owner accepted the `PreToolUse` hook as the only layer that denies `agy`'s
read-only built-ins (`read_file` and the other tools that never ask
permission), so Antigravity runs like every other Provider: `agy` on OVH,
workspace tools on the session's development Machine. Antigravity 0.2.2
(component release 3.117.0) declares `provider.execution-jsonrpc.v1` with the
same `jsonrpc_v1` executor digest as Claude and Codex.

The residual risk is bounded by isolation, not by the deny layers: should the
hook fail open, a read-only built-in reads inside the account's own sandbox,
which holds only that account's home and token, never another account's,
another Provider's credentials, `ubuntu`'s home or the target. The one failure
the acceptance found that silently disables the hook, an unparseable
`hooks.json`, cannot arise at runtime: the daemon bind-mounts it read-only from
the runtime store and the proxy refuses `initialize` unless the file has the
gate's exact shape. Tools that ask permission keep the proxy as their second,
independent layer.

## Session extensions (2026-11-20)

An Antigravity session gets what a Claude or Codex session on the same
runtime gets: Cowboy MCP (with managed calls), the folder's MCP bindings
(Suger, ...) and Matrix memory (recall, learning and its tools). The worker,
not the confined proxy, runs and admits all of it (first review: call
`call-a72162e8a6fca973f1a15f84`).

- **Worker-run servers.** From its verified extension descriptor (the
  frozen copy `mcp_launch` checked, read once) the worker starts Cowboy MCP
  (`<cowboy> mcp`, always, as for any ordinary bound session) and each
  binding (`<cowboy> mcp --binding <descriptor>`) on the target through its
  own execution client, with its own launch template (workspace cwd, closed
  `env`, the keeper's environment; the target keeper adds the session's
  call context). It records them in the ledger as session servers: they
  outlive turns, end at teardown, and every unit request naming one of
  their process ids is refused. A required binding that fails to start
  makes `cowboy/extensions` refuse, so the session does not start (Claude
  and Codex refuse too); Cowboy MCP or memory failing is only noted.
- **Two unit requests.** `cowboy/extensions` lists the discovered tools
  (bounded wait); `cowboy/extensionCall` runs one `tools/call`, admitted by
  the ledger only inside the active turn and counted as that turn's
  admitted request, so the turn's completion waits for it. A reply lost
  after the request may have reached the server is an uncertain effect,
  exactly like a lost mutation. The worker frames each server's stdio
  itself: one exchange at a time, whole lines only, answers matched by id
  and kind, and lost output ends the server.
- **One namespace.** `agy` still has exactly one MCP server, `cowboy`; the
  facade lists every extension tool beside its own (a name a facade tool or
  an earlier server holds takes its server's name as a prefix), so the
  PreToolUse gate allows `mcp_cowboy_<tool>` and `ask_question` and nothing
  else.
- **Memory.** Recall (`POST /v1/context`) and learning (`POST /v1/observe`)
  run in the proxy's `@cowboy/memory-client` over the worker's
  `cowboy/memory`, which forwards only the actions the signed configuration
  enables, to this session's relay only, with the relay token the unit never
  receives. Memory's tools are the worker's own MCP client to the relay,
  limited to the signed tool list. The outbox is private to the Cowboy
  session. Evidence is collected only inside an owner turn, each tool once,
  and a turn that ran a forbidden built-in delivers none.
- **What the unit learns.** Its execution descriptor gains `extensions`:
  the session id (the outbox key), memory's recall/learning switches and
  whether tools exist, and the installed call guidance. Server commands,
  relay addresses, tokens and Machine paths stay with the worker.
- **Permissions and modes.** A permission request for a `cowboy` tool is
  approved once (`allow_once`, never a standing grant) only in a mode `agy`
  confirmed (a session result, a mode update, or the answer to the last
  outstanding switch) other than `plan`; while the mode is unknown or any
  switch is outstanding the owner decides.
- **Second review (call `call-0b56f87a42e322bfe461ab3f`).** Tools route by
  server handle, memory's signed list is checked again at dispatch; worker
  server ids carry a reserved prefix no unit request may name (also one an
  earlier worker left) and their execution events never reach the unit;
  the readiness probe must name no environment of its own; a call queued
  behind another rechecks its turn before it is written; at most 8 calls
  run at once; teardown ends the servers first so a running call ends with
  an unknown outcome inside the shutdown deadline; a definitive MCP error
  is a tool error, not an unknown outcome; lost or undecodable server
  output ends the server; required bindings must report the binding
  contract (`mcp --binding-capabilities`); learning is accepted only during
  a turn and for two minutes after it; recall no longer records the prompt,
  which is delivered with the turn's evidence only if the turn passed the
  gate; guidance travels with the tool list, not in the bounded descriptor.
- **Accepted residuals.** The outbox lives in the account's own home, so a
  later session of the same account can read or drop an earlier session's
  undelivered evidence: same owner, same account, never another account.
  An extension call in flight when its worker dies leaves no durable
  dispatch record; the keeper's reap ends its server and the next turn
  starts without it.

## Revision 5 simplification (owner decision, 2026-11-18)

Two code reviews of the implementation (calls `call-c61a3380faee24839460a248`
and `call-68f0580fdd70787294431c14`) kept finding crash-safety and race
defects concentrated in three cross-component state machines. The owner chose
to simplify them; account isolation (L1–L4) is unchanged. This section takes
precedence over "Account controls", R1-8, R2-11, R3-5/R3-6, R3-9, R3-10 and
R3-16 where they differ.

- **One runtime per account, enforced by the daemon** (superseded by
  [Revision 6](#revision-6-concurrent-sessions-per-account-owner-decision-2026-11-20)).
  `max_runtime_units` is
  1 for every account; a second runtime start is refused (`limit`). There is
  no Controller turn reservation, no reservation settlement, no cross-Machine
  termination query and no per-turn evidence settlement. The Controller only
  explains the state: creating or waking another session of an account whose
  runtime is live is refused with "in use by session X; hibernate it first".
  One runtime serves one session, so at most one turn per account runs.
- **Backend signals set account state only.** The proxy's structured
  classification (`antigravity_backend_refusal:<class>`,
  `antigravity_policy_violation`) moves the account to the matching state;
  `suspended`, `blocked`, `needs_attention` and policy violations also fence
  the account and stop its unit. Nothing waits for, or is settled by, turn
  terminal events. Unit loss has no settlement.
- **Process ledger for cancel and teardown only.** The worker records every
  `process/start` before forwarding, refuses starts and filesystem mutations
  outside an active turn (the readiness Git probe is authorized by worker
  state, never by a caller-chosen ID), fences the turn on cancel, turn end
  and teardown, and terminates recorded processes best-effort. Processes left
  behind by a dead worker are not inherited; the target keeper's unattended
  reap (30 minutes) is the backstop. A maybe-admitted mutation still sets
  `uncertain_effect`, which parks the session until the owner acknowledges
  that exact sequence.
- **Facade paths are a guardrail.** `RunCommand` already gives the model a
  shell on the development Machine, so workspace path confinement in the
  file tools prevents mistakes, not attacks; it is not an isolation boundary.
  Conditional writes reuse the existing execution helper primitive.
- **Logout and removal bind to the Machine enrollment** that holds the
  credential, so a re-enrolled Machine cannot certify them.

This section also supersedes the turn reservation, settlement, termination
query and force release of "Controller account controls (step 5a)" and the
"In use by 1 turn" line and Force release action of "Account UX (step 5b)".

### Revision 5 implementation decisions

- **Records.** The unreleased migrations `0085` (PostgreSQL) and `0059`
  (SQLite) were amended: `provider_account_turns` is gone;
  `provider_account_controls` and `provider_account_fences` remain.
- **In use** (removed by Revision 6). The launch gate (`Supervisor::start_session`: create, wake,
  revive, reset, reload) refuses a governed session while another session of
  the same account has a live worker: "Antigravity · <name> is in use by
  another session (<id>); hibernate it first". Session creation refuses with
  `antigravity_account_in_use`. The account read model exposes `in_use_by`
  (derived from live workers, never stored); the card shows "In use by
  <session>" and the picker lists the account disabled with the same reason.
  The daemon's `limit` is the backstop: the worker reports "Antigravity ·
  account <id prefix> is in use by another session; hibernate it first" (the
  Machine does not know the account's name).
- **Signals.** The Controller observes every `TurnEnded` of a governed
  session (`remote_runtime::turn_end`, no delivery correlation) and
  classifies its stop reason: the proxy's structured refusal moves the
  account to that state, a policy violation to `needs_attention`
  (`policy_violation:<tool>`), any other error ending or unknown stop reason
  to `unknown_refusal`; normal endings change nothing. The proxy classifies
  vendor error endings itself, so an error without a refusal record already
  arrives as `unknown_refusal`. The raise is one transaction compared with
  the stored state: it never lowers a more severe state and never replaces
  `blocked`. `suspended`, `blocked` and `needs_attention` write stop fences.
  A rate limit holds one hour and the first dispatch after it resumes it,
  and only the expired rate limit that admitted that dispatch (compared by
  revision in the write) is cleared. A signal is registered before the
  turn ending is acknowledged; the account's dispatches and revivals park
  until it commits, and a failed write is retried, never dropped.
- **Cache and fences.** Control reloads apply in the order they started and
  never move a record to a lower revision, so a slow reload cannot reopen a
  blocked account; a failed reload also advances the order, so only a later
  successful one restores availability. Fences go only to Machines that can hold the account's
  sandbox (its sign-in Machines and its sessions' runtime Machines), never to
  execution targets, and a fence whose originating revision cannot be
  established is not sent unordered; it stays pending. A stopped account's
  fence list is completed by the reconciler: a Machine that gained the
  account after the stop was written (a first sign-in racing the block)
  gets a stop fence of the current revision. Login rechecks the block once
  its native generation is durable.
- **Ledger.** The readiness window is open while the worker's session is
  `Starting` (until `Ready`/`Running` or the first turn): one probe at a
  time, whatever its id, and only the projection's exact Git probe
  (`/bin/sh -c <READINESS_SCRIPT> sh <cwd>`). `fs/writeFile`, `fs/remove`, `fs/copy`,
  `fs/createDirectory` and `process/start` need the active turn; a turn's
  completion is reported after its admitted requests drained and its
  processes were asked to terminate (bounded, 5 s per process). The ledger
  file keeps only the uncertain effect and its sequence. A lost reply of a
  mutation, of `process/terminate`, or of a `process/read` of any process
  but a readiness probe (a publish's outcome, also after its exit) is an
  uncertain effect, and so is one the facade detects with every reply
  received (a timed-out publish), which it reports to the ledger
  (`cowboy/uncertainEffect`) before the tool answers. The worker publishes
  each new sequence at once (`cowboy_uncertain_effect`), every copy merges
  it by sequence, the Controller writes its observation durably under the
  writer lock the acknowledgement holds, and the prompt handoff rechecks
  the account gate after any asynchronous recovery. Acknowledgement
  records exactly the newest sequence and never overwrites a newer
  observation.
- **Logout.** Targets are every Machine's newest dispatched sign-in without a
  recorded logout, whatever the Controller recorded about its outcome (the
  daemon's credential state decides what is cleared). Delivery requires the
  Machine's current enrollment to be the one the sign-in was dispatched to.
  A receipt-store failure keeps the target pending; an unblock cancels
  undispatched targets only once the unblock is written, and a delivery
  refused before enqueue stays undispatched. Only the block that requested a
  logout dispatches it (its exact revision). A later block's logout keeps
  the operation of an earlier one that may have reached the Machine for the
  same generation. A completed kill switch logout satisfies a later
  revocation's login cleanup.
- **Removal.** Required only on the account's sign-in Machines (the daemon
  creates a sandbox only where the credential lives), recorded only from the
  enrollment that dispatched a planned sign-in there, and sent only to it. A
  Machine whose planned sign-ins were all refused before enqueue needs no
  removal. A Machine without the daemon (no socket and no sandboxed native
  profile), or whose daemon never created the account, confirms that
  nothing exists. A refused logout start (a failed first login) is settled by
  the daemon's token cleanup, which certifies an unused generation only
  with no token present, so revocation can finish.
- **Daemon.** `max_runtime_units` other than 1 is a configuration error
  (Revision 6: optional and unset by default; see below).
  Loopback, link-local, multicast, RFC 1918 and `100.64.0.0/10` are always
  denied; `ip_deny` only appends (the host's public address). A unit counts
  as live while its cgroup still holds a process. A unit whose
  `systemd-run` client ended stays owned until systemd confirms its
  invocation gone; a `Starting` record of an earlier boot is retired, one of
  this boot without an observed client end stays held. `systemd-run` is
  cancelled through a pidfd. Created and removed directories, unpacked
  runtime files and their directories are synced before they are relied
  on; a damaged registered runtime is replaced.
- **Facade write limit.** WriteFile and EditFile accept at most 3 MiB: the
  bytes are base64 twice on the way to the helper and must fit one 7 MiB
  execution invocation.
- **Conversations.** A re-login keeps the native conversation only on the
  same Machine and the same signed Provider generation; a home of another
  generation (one that predates the sandbox) never resumes it.
- **Configuration.** `accounts.antigravity_admission` (the boolean the
  admitted list replaced) is accepted and ignored, so a deployed
  configuration still starts.
- **Facade.** WriteFile and EditFile publish through the execution helper's
  conditional `apply` (`$COWBOY_EXECUTION_FILE_HELPER`, the Claude runtime's
  primitive). Guidance files are read only as regular files whose canonical
  path is themselves inside the repository.

## Revision 6: concurrent sessions per account (owner decision, 2026-11-20)

The owner runs many sessions of one Antigravity account at once, as with a
Claude or Codex account. Revision 5's "one runtime per account" refused a second session
with "in use by another session; hibernate it first". This section supersedes
that bullet, the "In use" implementation decision and the `max_runtime_units`
part of the "Daemon" decision; everything else in Revision 5 holds.

**What changes.**

- **Daemon.** `max_runtime_units` is optional and unset by default: like a
  Codex or Claude account, one account runs as many sessions as the
  Machine's session admission allows (the owner rejected a fixed per-account
  bound of 8 on 2026-11-20). Every unit keeps its own `MemoryMax`, and the
  host-wide `cowboy-accounts.slice` budget bounds the total. Each session's
  worker starts its own `runtime` unit; `probe` units share an operator-set
  bound; a start beyond it is still `limit`, which the worker explains as
  "Antigravity · account <prefix> already runs its maximum number of
  sessions; hibernate one first".
- **Controller.** The launch gate (create, wake, revive, reset, reload) no
  longer looks at other sessions of the account, session creation no longer
  refuses `antigravity_account_in_use`, and the account read model drops
  `in_use_by` (with the label cache that only named the holder). Admission,
  account state, signal parking, uncertain effects and fences gate every
  session exactly as before.
- **Web.** The picker, the account card and the session's parked text no
  longer show "In use by … hibernate it first"; a Controller that still
  reports `in_use_by` is accepted and the field ignored.

**Why it is safe (analysis of N units of one account).**

- **Credential custody.** No unit receives a seeded copy of the credential
  and nothing is written back: every unit binds the same persistent account
  home (one ext4 image, mounted once), where `agy` keeps its own token file,
  exactly like several `agy` processes of one desktop user. The daemon never
  reads the token; it only checks its presence (login outcome) and removes
  it (interrupted login, logout cleanup). The daemon's credential generation
  lives in root-owned `meta.json`, outside every mount, and changes only on
  login and logout transitions under the account lock; runtime units never
  move it, so concurrent units cannot race or regress it. Token refresh is
  `agy`'s own concern: Google OAuth refresh tokens of installed apps do not
  rotate on refresh, so two units refreshing at once each obtain a valid
  access token and neither invalidates the other; `agy` 1.5.9 also links a
  file-lock library around its token storage. The runtime-mode ACP
  allow-list still refuses `authenticate` and `logout`, so no runtime can
  change or clear the sign-in.
- **Authentication exclusivity.** Unchanged and per account: a `login` or
  `logout` start first stops and confirms every `runtime` and `probe` unit of
  the account, and while an authentication unit is recorded every runtime
  start is refused (`fenced`). Tests start several runtimes and check that a
  login or logout stops all of them.
- **Fence and stop.** `stop_account` already stops every recorded unit, every
  live `cga-<key>-*` unit systemd lists and the account slice, then confirms
  none is live; a kill switch, `suspended` or `needs_attention` therefore
  stops all N sessions' units. A test fences an account running three
  runtimes.
- **Signals.** Turns of several sessions may now end concurrently. Each
  ending registers its signal before acknowledgement (a per-account counter,
  not a flag), and each raise is one store transaction compared with the
  stored state that never lowers a more severe state, so concurrent raises
  converge to the most severe one. The account state remains one value that
  parks every session of the account.
- **Unit naming and private files.** Unit names carry a random 8-byte nonce
  (`cga-<key>-<mode>-<nonce>.service`); each unit's descriptor, execution
  socket and process ledger live under its own `units/<nonce>/` directory, so
  units of one account never share a per-unit path.
- **Resources.** A live runtime unit measured about 190 MiB (peak 220 MiB,
  29 tasks) on OVH; each unit keeps `MemoryMax=3G`, `MemorySwapMax=0` and
  `TasksMax=512`. Eight per account covers the owner's parallel work (OVH's
  Machine admits 40 sessions over every Provider) while a runaway client
  cannot start unbounded units. The bound is per account; the host-wide
  budget of all accounts belongs to the host configuration (an OVH
  `cowboy-accounts.slice` memory and task drop-in), not to the daemon.

**Residuals.**

- Between a backend refusal of one session's turn and the stop fence that
  follows (`suspended`, `blocked`, `needs_attention`), turns other sessions of
  the account already dispatched keep running; only new dispatches park.
  Revision 5 bounded this window to the one refused turn.
- Concurrent turns of one account are visible to Google as concurrent
  requests of one installation, as from one user running several `agy`
  terminals.
- Several `agy` processes share the account's conversation store and caches
  in the home. Each session uses its own conversation; how `agy` arbitrates
  concurrent writers to shared state is the vendor's, and an `agy` failure
  there would surface as an ordinary turn error (`unknown_refusal`, owner
  clears it).
- A rate limit is account-wide: the first dispatch after it ends, in any
  session of the account, resumes all of them.

## Antigravity features (0.2.1)

Built on what `agy` 1.5.9 returns over ACP during owner-driven sessions; no
request is added for any of it.

- **Recommended cards.** `configuration_presets` follow the `configOptions`
  of `session/new` (`agy` 1.5.9, 2026-11-21): Gemini 4 Neon High, Gemini 4
  Hydrogen Extra and High (default), then Claude Opus 5.5 High and Claude
  Sonnet 5.5 High, which `agy` serves on the same Google account's quota.
  Each card sets `model` and `thought_level`. The reasoning list is
  model-scoped (Gemini 4 offers `low`…`xhigh`, the Claude models
  `low`/`medium`/`high`), which the generic preset rules already handle: a
  card that changes the model is applied model first and its reasoning is
  judged against the new model's options. Values reach `agy` through the
  generic serial `session/set_config_option` queue, which the proxy
  forwards. GPT-OSS 120B and the Orchestrator models stay selectable
  through the session's own `model` option without a card.
- **No Plan card.** Recommended cards are (model, reasoning) configurations:
  the picker labels them so, each binds one `models/targets.toml` evaluation
  target, and a card is current when all its values match. A card setting
  `mode` would force every card to pin `mode=default` and silently reset the
  owner's live mode on each card switch. `agy`'s own `mode` option (`default`, `accept-edits`,
  `auto`, `plan`) is already shown per session as "Agent mode" and persists
  as a session preference, so Plan stays one choice away. How `plan` behaves
  with every built-in tool denied is not observed yet; it is an acceptance
  item for the next owner-driven session, not something a card should
  preselect.
- **Account plan.** `session/new|load|resume` results carry
  `_meta.antigravity.tier` (`{name: "Antigravity Starter Quota", id:
  "free-tier", upgradeUri}`). The proxy keeps `{id, name}` only and adds it
  to every `usage_update` as `_meta.cowboy.account_plan`, beside
  `_meta.cowboy.sanitizer: 1`, which marks a `_meta.cowboy` the proxy rebuilt
  itself; the worker already relays that reading as the session's usage, and
  the Controller persists it. The Controller ignores a plan without that
  marker, and one from a session pinned to an Antigravity release before
  0.2.1: the 0.2.0 proxy forwarded `agy`'s `_meta` verbatim, marker
  included, so a stored reading or a still-pinned worker could carry a plan
  nobody validated. `accounts.list()` reports, per named-only account, the
  newest valid plan among its sessions' current usage readings (`plan: {id,
  name, observed_at_ms}` or `null`); the account
  card shows "Plan · <name>" beside "Usage · Not available". One rule holds
  at the proxy, the Controller and the Web: `id` is a slug of at most 64
  characters; `name` has no character of Unicode category C or a line or
  paragraph separator anywhere, and is 1 to 80 code points once trimmed. The
  proxy and the Web evaluate `[\p{C}\p{Zl}\p{Zp}]` with their engine's
  Unicode tables; the Controller uses a dependency-free range table
  generated from that same expression by the pinned Bun
  (`tools/generate-plan-name-table.ts`, Unicode 17.0), unassigned code
  points included. The proxy checks the reported name before trimming (so a tab or U+2028 at
  either end clears the plan) and forwards it trimmed; the Controller and
  the Web accept only an already trimmed name. Anything else is dropped; a
  valid plan another session of the account reported keeps showing (the
  fallback exists only across sessions, never to an older reading of the
  same session). The proxy forwards a session result
  without awaiting anything, so the result reaches the worker (which drops
  session updates until a load completes) before a `usage_update` that `agy`
  wrote right behind it, and that reading already carries the new plan. A
  prompt result instead waits for its turn's evidence to settle, and the
  turn stays observed until then. A worker's counter-only usage snapshot
  after a reconnect replaces the stored reading like any other, so the
  session's plan may be absent until its next `usage_update`. The plan never
  gates anything.
- **No raw tier.** The proxy removes `_meta.antigravity.tier` from every
  `_meta`, at any depth, of every message it forwards to the worker
  (results, error data, notifications, session updates, permission requests,
  tool inputs and outputs), together with the `antigravity` and `_meta`
  objects that removal leaves empty, however they nest. Nothing else is
  altered or dropped: one iterative post-order walk with no depth limit
  visits each value once and copies an object once, after its children,
  only when one of them changed. The `upgradeUri` is therefore never copied, so no link to
  Google appears outside the account browser.
- **Generic paths, unchanged.** `usage_update` (`size` 1,048,576) drives the
  context meter; `promptCapabilities.image` needs no declaration (image
  blocks are forwarded verbatim); `available_commands_update` feeds the
  command list; `agent_thought_chunk` renders as thought. `session_info_update`
  titles are not adopted: Cowboy titles a session from its first prompt and
  only while the title is the creation default, so a later agent title could
  not tell an owner's rename from Cowboy's own title without a new title
  source, which would also change every other Provider that emits one.

## Web search and URL reading (2026-11-20)

`agy`'s network-only built-ins are allowed for parity with Claude's and
Codex's WebSearch/WebFetch: `search_web` (a Google-backend search the model
summarizes; it never touches a file or process), `read_url_content` (an HTTP
GET from `agy` that converts HTML to Markdown) and `view_content_chunk` (pages
of a document `read_url_content` fetched). The hook allows them by exact name;
`read_url_content` only when every argument whose key mentions `url` is an
`http:`/`https:` URL, so `file:` and other schemes are denied. `settings.json`
no longer denies `read_url(*)`; `execute_url(*)` (browser) stays denied and the
browser tools stay outside the allow-list.

Reachability is the unit's, not the hook's: the account unit has only
`AF_INET`/`AF_UNIX`/`AF_NETLINK` and `IPAddressDeny=` always covers loopback,
link-local (cloud metadata), multicast, RFC 1918 and `100.64.0.0/10`, plus the
host's own public address on OVH (`ip_deny`); only the resolver stub
`127.0.0.53` is allowed. These are packet filters, so `0.0.0.0` or a DNS name
that resolves to a private address are refused too; the 2026-11-18 daemon
acceptance saw TCP to `127.0.0.1`, the host's public address and a Tailscale
peer denied. A URL read therefore reaches only public Internet hosts, like a
WebFetch, and its content is untrusted model input like any web page.

A hook-denied built-in is not a violation. `agy` 1.5.9 announces every call
as an `in_progress` `tool_call` before the hook decides, then reports a denied
call `failed` (the 2026-11-17 spike's denied `read_file`, `run_command`,
`schedule`, `swarm` and `invoke_subagent`; production session
`sess-1795107755038`'s two `search_web` calls). The proxy used to read that
announcement as execution; it now counts only a later `in_progress` update or
a completion. A built-in that ran and failed is indistinguishable from a
denial; the hook remains the boundary and isolation bounds the rest
("Split execution declaration").

The same session showed why the proxy's turn errors no longer use JSON-RPC
`-32000`: the worker treats ACP's `auth_required` code as rejected
credentials, replaced the message with "Provider authentication required",
and the Controller, no longer seeing `antigravity_policy_violation:<tool>`,
settled the account `unknown_refusal` and parked its sessions. The proxy now
answers `-32603` and adds the owner-facing sentence as `data.message` (shown
by the Web) beside `data.cowboy` (read by the Controller).

## Account quota (investigation, 2026-11-20)

The account card shows the plan only. What `agy` 1.5.9 exposes:

- Cloud Code `v1internal:retrieveUserQuota` (buckets: `modelId`,
  `tokenType`, `remainingFraction`, `remainingAmount`, `resetTime`),
  `retrieveUserQuotaSummary` and `fetchAvailableModels` (per-model
  `quotaInfo`), called with the account's OAuth token.
- Print-mode slash commands: `agy -p "/usage"` (also `/quota`, `/credits`)
  emits one tab-separated record per line, `--json` with UTC reset times, and
  forces a fresh quota reload.
- `agy acp` sends only the context window (`usage_update`) and the tier in
  `_meta.antigravity.tier`; no quota reaches the ACP stream, so a session's
  runtime cannot observe it.

A collector must not run outside the account's custody (the Machine's
signed Plugin collectors run as `ubuntu` with no access to the token), so the
quota is a daemon `usage` unit.

## Account quota (Antigravity 0.2.10)

**Path.** The Controller's existing native account usage pipeline
(`provider_accounts/usage.rs`: owner/binding admission, the shared five-minute
budget, coalesced refresh, persisted observations, transient-error rules)
dispatches `CollectNativeUsage` to the account's sign-in Machine, as for a
named Claude account. Antigravity 0.2.10 declares `host.account_usage`
(`antigravity`) and a `usage` binding without a collector command, so the
Service-level aggregator runs nothing for it and the Web shows no Service card
(named-only). Unlike a native Claude login, which binds its exact signed
release (an incompatible update asks to sign in again), an account-sandbox
login resolves its usage target on the sign-in Machine's active trusted
generation: that generation's `account_usage` declaration decides, and the
daemon admits only the login's current signed-in credential generation of an
unfenced account, as for session launches. For an `account_sandbox_v1` Provider
the Machine (`native_usage/sandbox.rs`) runs no collector: it asks the daemon
(`collect_usage`) on the active generation's runtime for the login's credential
generation, holding the account's usage guard until the daemon answers, and
maps the daemon's projection onto the generic `rateLimitsByLimitId` document.
No wire, SDK or worker interface changes; the account card renders the rows
with the existing usage components.

**Daemon.** `collect_usage` starts a `usage` unit:
`bin/agy-launch -p /usage --output-format json --log-file /tmp/agy-usage.log`
(the launcher execs `agy` unchanged in the sandbox), the account's identity
over its own home, no descriptor or execution socket. Admission is a runtime
unit's: the current `signed_in` generation, not fenced, no authentication unit,
inside a configured runtime bound; a login or logout stops it. The daemon
closes stdin, reads at most 256 KiB, waits up to 25 s, stops and confirms the
unit, and answers only `{"groups":[{"name","buckets":[{"name","id"?,"window"?,
"disabled"?,"remaining_fraction"?,"reset_at"?}]}]}` or
`authentication_required`; vendor text never leaves it. `start` refuses mode
`usage`, so no caller gets the raw output.

**No model request.** In `agy` 1.5.9's print mode (`printmode.run`), a prompt
whose leading token is a supported slash command branches to `runCommand`
before `session.start`/`runTurn`; `/usage` (`usageReport`) only reads Cloud
Code's quota summary ("retrieving quota summary") and prints it. This was read
from the binary's code (the Go symbol table recovered from `.gopclntab` and
the disassembled branch), not observed on a signed-in account. The
`--output-format json` envelope is `{conversation_id, status, response,
duration_seconds, num_turns, usage, command: {name, data}}` with `data` the
report `{description, groups: [{name, description, buckets: [{id, name,
description, window, disabled, remaining_fraction, remaining_amount,
reset_time}]}]}` (struct tags read from the same binary). The daemon refuses a
report whose `num_turns` is not 0 or whose command is not `usage`. `/usage`
takes no arguments (`--json` is not a flag), and without a usable token print
mode starts its interactive sign-in, which the closed stdin and the bound end
(reported as authentication required).

**Eligibility fallback (2026-11-20).** Print mode checks eligibility before
it runs a command, and `agy` 1.5.9 refuses an account whose `loadCodeAssist`
lists an ineligible free tier (`UNSUPPORTED_LOCATION`) even when a paid tier
(`g1-pro-tier`, Google AI Pro) serves it: "Eligibility check failed: ... not
currently available in your location". Such an account's ACP sessions still
run every model, and Cloud Code's
`retrieveUserQuotaSummary` answers the same token normally. In the sandbox the
launcher therefore hands this exact usage command to the proxy component's
helper (`bin/agy-proxy --cowboy-usage`, `proxy/app/usage.mjs`). The helper
runs the same print-mode command and passes its report through unchanged;
only for that eligibility refusal does it read the access token `agy` just
refreshed and persisted in the account home (print mode refreshes before its
eligibility check, observed with an expired token), POST `{}` to
`https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary`
(the host `agy` uses, IPv4, `User-Agent: antigravity`, which the endpoint
requires), and print a print-mode envelope with the mapped document
(`displayName` → `name`, `bucketId` → `id`; proto3 omits a zero
`remainingFraction`, so a bucket with a reset time and no fraction is
exhausted). The helper runs as the account's identity like `agy`; it never
refreshes, writes or prints the token, and without a token or a valid summary
`agy`'s own refusal stands (`location_unsupported`). The daemon still
validates and projects the result; its command and wire are unchanged.

**Gemini quota (2026-11-21).** `retrieveUserQuotaSummary` covers only
third-party models: for Husky (Google AI Pro) it answered one group, "Claude
and GPT models" (`3p-5h`, `3p-weekly`), so the card showed no Gemini row.
Gemini's quota is reported per model by Cloud Code's
`v1internal:fetchAvailableModels` (the catalogue `agy` loads), as
`quotaInfo {remainingFraction, resetTime}`; on that account every Gemini
model shared one quota (0.874, one reset time) while the editor's completion
models (`tab_*`) carried a fraction without a reset time. The usage helper
therefore asks the catalogue with the same token and appends a "Gemini
models" group to every successful report that has no Gemini group yet:
print mode's own report (the envelope is otherwise unchanged) and the
eligibility fallback, whose two requests run concurrently within the
daemon's 25 s bound. Each Google model with a reset time is one bucket
named after the model (proto3: a reset without a fraction is spent); the
Machine folds buckets sharing one quota into a single "Gemini models" row.
The catalogue states no window, so the row has a reset time but no window
label. Without the catalogue the report is the summary alone, as before.

**Client identity decides the summary (2026-11-21).** The summary's groups
depend on the request's `User-Agent`, not on the account: a capture of
`agy` 1.5.9's own request (body `{"project":"aicode-consumers"}`, user agent
`antigravity/cli/1.5.9 (aidev_client; os_type=linux; arch=amd64; cl=…;
auth_method=consumer)`) returned "Gemini Models" for a free account, and for
Husky the same token answers three groups as soon as the user agent starts
with `antigravity/cli/<version>`: "Gemini Models" (`gemini-weekly`,
`gemini-5h`), "Claude and GPT models" (`3p-weekly`, `3p-5h`) and "Gemini 4
Argon" (`gemini-argon-wtu-weekly`, Pro's separate Argon quota). `antigravity`,
`antigravity/cli`, `antigravity/1.5.9` and `antigravity/ide/1.5.9` get only
the third-party group. The helper therefore asks the pinned `agy` for its
version (`agy --version`, beside print mode, at most 3 s) and sends
`antigravity/cli/<version>` with both requests, the identity prefix `agy`
itself sends (owner-approved); without a version it sends `antigravity` as
before. With the summary's Gemini groups the catalogue group is not added.

**Egress decides location (2026-11-21).** Gemini models answer
`FAILED_PRECONDITION` "User location is not supported for the API use" when
`agy` reaches Google over the host's IPv6 address, while the same account,
model and prompt succeed over IPv4 (Husky, Gemini 4 Hydrogen; the
Claude models succeed either way). The account units are IPv4-only
(`network: ipv4_only_v1`), so production sessions are unaffected; a
diagnostic `agy` run outside a unit must also be restricted to IPv4
(`RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK`), or it reports a
location refusal production never sees.

**Presentation.** Each vendor group (model family: Gemini Pro, Gemini Flash,
...) is one row when its buckets share one quota; otherwise one row per distinct
quota, named by its bucket. Disabled buckets and buckets without a remaining
fraction are omitted. A parseable window (`5h`, `daily`, `weekly`) becomes the
row's window; reset times show as on Claude and Codex cards. The plan comes from
the account controls (the plan its sessions report); the Subscriptions card's
`Usage ·` line summarises the rows.

**Costs and residuals.**

- Each report starts `agy` (its language server) for a few seconds, at most
  once per account per five-minute budget; the CLI log goes to the unit's
  private `/tmp`. `agy` also updates its own state in the home (installation
  state, caches), as every runtime does; the daemon never reads or moves the
  credential.
- Print mode has no telemetry switch; `agy` may record a local analytics event
  for the command.
- Acceptance items on a signed-in OVH account: the exact report and window
  labels, the absence of any turn in `num_turns`/the quota, and a report
  beside running `acp` units.
- No new sign-in is needed after an upgrade: the account's credential lives in
  the daemon's account home and does not belong to a Provider release. Until
  the account's sign-in Machine runs 0.2.10 or later, the card keeps "This
  Provider does not support background account usage".

## Feature inventory and Cowboy adaptation (2026-11-20)

What `agy` 1.5.9 offers (its ACP `initialize`/`session/new` results, the
`available_commands_update` it sends, the built-in tool registry
`third_party/oneharness/internal/tools/*.go` and the tool descriptions in the
binary, the 2026-11-17 spike trace and production session
`sess-1795107755038`), whether it reaches a Cowboy session, and how Cowboy
adapts it. The rule is unchanged: every workspace read, write and command
goes through the facade to the session's execution binding; nothing built in
touches the runtime Machine's files or starts a process there.

### Tools and their presentation

`agy` 1.5.9 reports an MCP call as kind `other`, titled "Run mcp_cowboy_<tool>",
with its arguments (plus its own `toolSummary`) and **no result**: neither
`content` nor `rawOutput` (every completed facade call of
`sess-1795107755038`). Built-ins report their output (`run_command`:
`rawOutput.output` and text `content`). The facade runs in the proxy, so the
proxy attaches each answer itself: `agy` announces a call (`tool_call`, with
its arguments) before it calls the MCP server, and the MCP request carries no
ACP call ID (only `_meta` conversation IDs), so the facade reports each
answered call (`onResult`) before replying, and the proxy binds it to the
oldest unanswered announcement of the same tool with the same arguments
(`toolSummary` ignored), else of the same tool. The answer is put on `agy`'s
terminal update for that call; it adds no message to `agy` and no request to
the target. Memory evidence sees the same presented update, so a tool's output
now counts as it does for Claude and Codex.

| Feature | agy native | Before | Gap | Cowboy now |
| --- | --- | --- | --- | --- |
| `ReadFile` | `read_file` (denied) | Read, path, location | no content | text (with the paging note) on the call |
| `ListDirectory` | `list_dir` (denied) | List, path | no listing | the listing |
| `SearchFiles` | `grep_search` (denied) | kind `read`, pattern title | no matches | the matches |
| `WriteFile` | `write_file` (denied) | diff (new file) | result/error unseen | diff + result; a refused write shows only the error |
| `EditFile` | `edit_file` (denied) | diff | a refused edit still showed its diff | diff + result on success, the error alone on failure |
| `RunCommand` | `run_command` (denied) | command title, `toolSummary` as heading | no output, no exit code; a multi-line command titled by a fragment ("node -e '") | output so far while it runs (last 16 KiB, at most every 2 s, only when changed, from the reads the command needs anyway), then `Exit code: N` and the retained 256 KiB; title = first non-empty line + " …" |
| `StopCommand` | `manage_task kill` (denied) | Stop title | no output | final output |
| Extensions (Cowboy MCP, folder MCP, memory) | MCP | `mcp__<server>__<tool>`, title "Run mcp_cowboy_…", `toolSummary` among the arguments | no result | title `<server> · <tool>`, `toolSummary` dropped, result blocks (text, image, resource) as content, at most 1 MiB |
| `search_web` | built-in, allowed | WebSearch, query title | `agy` reports no result | unchanged (the result never reaches ACP; the model's reply summarizes it) |
| `read_url_content`, `view_content_chunk` | built-in, allowed for http(s) | WebFetch, URL title | `agy` reports no result | unchanged |
| `ask_question` | built-in, allowed | owner permission prompt | none | unchanged |
| Plan / task tracking | planning mode writes `task.md`/`implementation_plan.md` artifacts with `write_file` (denied); no todo tool | nothing | no checklist, no Plan panel | facade tool `UpdatePlan` (`plan: [{step, status}]`, like Codex's `update_plan`): validated by the facade without a target request, sent as ACP `plan`, its tool call hidden |

Per call this adds nothing on the wire to `agy` or the target; the system
prompt grows by one tool schema and one routing sentence (`UpdatePlan`).

### Agent features

| Feature | agy native | Reaches Cowboy | Adaptation |
| --- | --- | --- | --- |
| Thinking | `agent_thought_chunk`, `thought_level` low/medium/high | yes | rendered as thought; level is a session option and a card |
| Models | Gemini 3.8 Flash, Claude Sonnet/Opus 4.6 (Thinking), GPT-OSS 120B | yes | `model` option; cards for Gemini levels |
| Modes | `default`, `accept-edits`, `auto`, `plan` | yes | "Agent mode"; facade calls auto-approved except in `plan` |
| Context meter | `usage_update` (`size` 1,048,576) | yes | context ring; plan tier on `_meta.cowboy` |
| Images in prompts | `promptCapabilities.image` | yes | forwarded verbatim |
| Session load/resume | `session/load`, `session/resume` | yes | fresh projection appended |
| Session list/close | `session/list`, `session/close` | refused | Cowboy owns its session list |
| Conversation title | `session_info_update` | dropped by the worker | Cowboy titles from the first prompt (see "Antigravity features") |
| `agy` status lines | `agent_message_chunk` "\n⚠ <text>\n" ("Model produced invalid output.", "API error (attempt N): <status>"), `agy` then retries itself | shown as the model's words and stored in memory as them | when the worker advertises `session.notices`, a whole such chunk (optionally with its "Created At/Completed At" lines) becomes an ACP `notice` (`warning`; "Antigravity API error (attempt N)" with the status as description), which the Web shows as a muted status row; it is not part of the reply or memory. Production frequency: one in 5 turns of `sess-1795107755038` (its event log does not record the model) |
| Compaction | `/compact` | yes | typed `compaction_update`, context restated |
| Account plan / quota | tier in `_meta`; `/usage` in print mode only | plan yes, quota no | quota is a separate daemon `usage` unit ("Account quota") |

### Slash commands (`available_commands_update`)

| Command | What it does | Cowboy |
| --- | --- | --- |
| `/compact` | summarize history | offered (typed compaction) |
| `/goal` | run until a goal is finished | offered |
| `/plan`, `/grill-me` | planning prompts | offered |
| `/agy-customizations`, `/antigravity-guide` | built-in reference skills | offered (read-only guidance) |
| `/schedule` | timer or cron prompt | removed from the list: `schedule` is denied (below) |
| `/browser` | browser agent | removed: no browser |
| `/swarm`, `/run-workflow` | parallel agents, Python workflows | removed: denied (below) |
| `/learn` | writes skills/rules into the account home | removed: needs `write_file` on the runtime Machine |

The proxy drops exactly these five from `available_commands_update`; any
other or future command is offered.

**Repository skills (Antigravity 0.2.13).** Claude and Codex discover a
repository's skills natively; `agy` sees only its own empty directory. When
the proxy builds a session's projection it also reads, through the execution
connection, each `<dir>/.agents/skills/*/SKILL.md` and
`<dir>/.claude/skills/*/SKILL.md` from the repository root down to the
working directory, with the guidance files' path rules (canonical paths
inside the repository, no symlinked directory or file, so a `.claude/skills`
link to `.agents/skills` is read once). Only the frontmatter `name` and
`description` are read (at most 64 skills, 600 characters per description,
a 24 KiB section); the deepest definition of a name wins. The projection
lists them, fenced as repository content, with each `SKILL.md` path and the
rule to read the file with `mcp_cowboy_ReadFile` before using a skill. Each
skill is also offered as a slash command after `agy`'s own (a skill never
shadows an `agy` command): a prompt whose first text block is `/<skill>
[request]` reaches `agy` as the instruction to read that `SKILL.md` and
follow it for the request, while the transcript and memory keep the owner's
words. Workflows (`.agents/workflows/*.md`) are not projected.

### Denied built-ins and why

| Tool(s) | Reason | Alternative named in the hook's denial |
| --- | --- | --- |
| `read_file`, `write_file`, `edit_file`, `run_command` (and `list_dir`, `grep_search`, `find_by_name`, `command_status`, …) | they act on the runtime Machine | the facade tools |
| `invoke_subagent`, `define_subagent`, `manage_subagents`, `send_message`, `swarm` (`post_message`, `read_board`, `swarm_wait`, `claim_next`), `run_workflow` | see "Subagents" | `mcp_cowboy_Subagent` (a proxy-hosted subagent); Cowboy managed calls (`mcp_cowboy_cowboy_execute`, action `calls.start`) for work of another session |
| `schedule`, `manage_task` | timers and background tasks live in the `agy` process, which Cowboy stops when the session idles; a fired timer would start a turn no client prompted | `RunCommand` with `wait_seconds`; a scheduled message in Cowboy; for work handed to another session, `notify:true` and ending the turn after the admission is confirmed ([session completion notices](agent-api.md#session-completion-notices)) |
| `execute_url`, browser and computer-use tools (`bind_app`, `ax_*`, `focus_app`) | no browser or desktop in the account unit | `search_web`, `read_url_content`, a browser MCP tool of the folder |
| `generate_image` | writes the image to the account's artifact directory on the runtime Machine, which the session cannot see | none yet |

The denial reason is said only when the model tries a tool (the projection
already names the rule), so it costs nothing otherwise.

### Subagents

Evidence: `invoke_subagent` "invokes one or more subagents ... Each subagent
runs in the background with its own prompt and reports back when done"; in
the spike the parent's tool call completed at once and its prompt ended
(`end_turn`) while the child was still running; the child's own tool calls
were **not** reported over ACP, but did pass the `PreToolUse` hook (the spike
hook logged the child's `read_file` right after `invoke_subagent`). `agy`
adds `_meta["antigravity.google/root_conversation_id"]` to MCP calls made by
nested subagents, so children can call the `cowboy` server.

So a native subagent would be gated by the same hook, but (1) it outlives the
prompt that started it, and the facade answers only inside a turn, so its
workspace tools would fail after `end_turn`; (2) its result wakes the parent
outside any client prompt, a turn the worker never started (no evidence
settlement, no fencing, no queue ownership); (3) none of its steps are
visible over ACP, so the Web cannot show it as a subagent row and the
proxy's detection behind the gate cannot see it; (4) Cowboy stops the idle
process, killing it. Native subagents, swarms and workflows therefore stay
denied (the hook's denial names the alternatives), and `agy` 1.5.14, the
newest at 2026-11-22, still declares no subagent capability in its ACP
`initialize` (`agentCapabilities` is identical to 1.5.9's for a client that
declared `clientCapabilities.subagents`), so waiting for it reports nothing.

**Proxy-hosted subagents (owner decision, 2026-11-22).** The proxy runs the
delegation itself, so each of the four problems has an answer inside the
turn. The facade's `Subagent` tool (`subagents.mjs`) starts another native
`agy` session in the same process, with the parent's working directory,
projection (plus the subagent guidance), mode, model and effort settings and
its own facade channel and turn on the one execution connection, and sends the
delegation prompt. The tool call stays open while the child works: it waits at
most `wait_seconds` (default and maximum 600, like a command) and returns the
child's final report, or its `subagent_id` for another `Subagent` call to keep
waiting (or `StopSubagent` to stop it). `wait_seconds: 0` starts it without
waiting, so a model can run two at once and collect both.

- **Reporting.** The worker's `initialize` declared
  `clientCapabilities.subagents` (the proxy offers the tool only to such a
  client; otherwise the call fails). The proxy sends the ACP subagent form the
  worker already routes ([subagents](subagents.md)): a `subagent_update` on the
  parent before any child traffic (`capabilities.cancel`, `state: running`),
  the delegation prompt as a linked `session_message`, the child's own
  `session/update`s and permission requests under its session id (its prompt
  echo and command list are dropped), and a final `subagent_update` with
  `idle` and a stop reason (`end_turn`, `cancelled` or `error`). The row, its
  observed transcript, Stop and the permission attribution need nothing new
  in the Controller or Web.
- **Same gate.** The child is a session of the same `agy` process, so the same
  `PreToolUse` hook, marker, allow-list and violation detection apply; the
  proxy keeps a turn record per child so a built-in tool that ran is seen. A
  child approves facade tools in its parent's *current* mode (a parent that
  moves to plan mode, or whose mode is unconfirmed, makes its children ask the
  owner); a child's permission admission closes in the same tick it is
  stopped or its prompt is answered, a request after that is rejected and a
  late approval answered as cancelled. The mode is rechecked right before the
  child's prompt is sent, so a switch to plan mode during the start stops it. A child's violation, unknown effect or backend
  refusal ends the parent's turn the same way: the parent's facade turn, its
  other children and any start still opening are fenced in the same tick,
  then `agy` is cancelled; the child's settled backend class is merged into
  the parent's reply (the worst class among the parent and every child by
  the classifier's precedence; their backend lines also lie inside the
  parent's evidence window), and a child starts only with a log cursor. A lost
  effect is announced the moment it is lost, before the ledger report is
  awaited, and latches on the exact parent turn that started the child;
  the parent's end waits for its children's calls, and for any running
  publish helper (an admitted effect, at most 30 s).
  Backend evidence is account-scoped: `agy`'s success lines carry no session
  id, so a child is `ok` with its own barrier, no refusal in its window and a
  success in the window (its own or a sibling's), and a sibling's refusal
  fails it too; as for the parent, an errored prompt with otherwise clean
  evidence is an unproven outcome. A helper process that cannot be confirmed
  closed (a failed read, a timeout whose termination is not confirmed) is a
  lost effect.
- **Bounded.** One level only (a child's `Subagent` call fails), at most 2
  running at once and 8 started per turn, a prompt of at most 64 KiB, a report
  of at most 64 K characters, nothing in plan mode or before the mode is known.
- **Nothing outlives the turn, and a stop fails closed.** The parent's prompt
  ending, its cancel or its Stop cancels every child (their facade turns are
  fenced and `agy` is asked to stop in the same tick) after the parent's own
  fence; a child's own Stop (its row) cancels only that child and reaches
  `agy` at once, the facade's cleanup draining beside it. `agy` has 10 s
  from the moment the cancel reached it to confirm a stopped child by ending
  its prompt (an answer disarms the watchdog at once, whatever its
  settlement still takes); if it does not, the unit's `agy` process is killed
  (nothing could show the native work stopped), every parent prompt still open
  is answered with what is already known (a latched violation, a child's
  refusal, a lost effect, else an unproven outcome, including one `agy` had
  already answered and whose settlement was still running, once) after the
  children's rows are ended and before the process exits, and the row is
  failed, never reported stopped. A turn's end waits separately for a child
  `agy` did answer to settle its evidence (it never kills that child). A settled child's native session is
  closed (also one `agy` opens after its request timed out; if that cannot be
  closed, or never answers within 2 minutes, the unit's `agy` is ended), its facade
  channel revoked and its facade session forgotten; `agy` keeps its
  conversation on disk. The frames of a session still opening are held until
  the worker has been told of the child, so its setup traffic is never read as
  the parent's. Child text is never recorded as the owner's memory.
- **Account risk.** The child's prompts are agent-initiated requests on a
  consumer subscription inside the owner's own turn: the owner accepted that
  (requirements, Antigravity consumer-subscription boundary). Managed calls
  keep the read-only profile and no subagents.

Accepted with real traffic on 2026-11-22 ([rollout](releases/antigravity-subagents-3-134-rollout-2026-11-22.md)):
`agy` 1.5.9 runs the child session while the parent's tool call is open, sends a
per-child `model` configuration (the row shows "Gemini 4 Hydrogen"), and
answers a cancelled child prompt; two children ran at once and a row Stop
stopped one of them. Not covered: a child's token usage (`agy` reports none per
session), and real-traffic runs of a parent Stop with children and the
fail-closed paths (unit tests only).

### Remaining (needs work outside this adapter)

- `/schedule` as a Cowboy scheduled message: the Web can schedule a queued
  message, but Cowboy MCP's `queue.submit` has no `scheduled_at_ms`; adding
  it (Controller) would let the model schedule its own follow-up through the
  extension tools.
- Repository workflows (`.agents/workflows/*.md`) are not seen by `agy`
  (skills are, since 0.2.13: "Repository skills").
- `search_web`/`read_url_content` results stay invisible until `agy` reports
  them over ACP.
- `generate_image`: allowing it needs the image returned over ACP or copied
  to the target, neither of which `agy` 1.5.9 offers.
- Quota per model: the daemon `usage` unit ("Account quota").

## Residual risks

- **Accepted by the owner (R3-1):** a prompt-injected agent session of any
  Provider on Hawk can reach root on OVH through the operator SSH route and
  read every account's state.

- Linkage of the accounts by shared egress, host fingerprint and timing.
- Same-user (`ubuntu`) host processes and the Machine are trusted (owner
  decision): a malicious vendor runtime of another Provider could drive the
  daemon or the account browser. No model-driven code runs as `ubuntu` on
  OVH.
- Workspace tools run with the session's full rights on the development
  Machine.
- Root on OVH can read every account's state.
