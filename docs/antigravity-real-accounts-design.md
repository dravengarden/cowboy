# Antigravity real accounts on OVH — design

Status: design, revision 4, 2026-11-18. Implemented so far: the sandbox
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
controls UI (step 5b) remains. Split execution stays
unavailable: native acceptance on 2026-11-18 found that `settings.json`
`permissions.deny` does not deny built-in tools in `agy acp` 1.5.9, so the
second denial layer of [R2-14] does not hold (see "Step 4 acceptance"). It extends
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
| Process | `NoNewPrivileges=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `ProtectKernelTunables/Modules/Logs=yes`, `ProtectControlGroups=yes`, `ProtectClock=yes`, `ProtectHostname=yes`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`, `MemoryDenyWriteExecute=no` (Node JIT), `SystemCallArchitectures=native`, `SystemCallFilter=@system-service`, `CapabilityBoundingSet=` empty, `ProtectProc=invisible`, `ProcSubset=pid`, `PrivateIPC=yes`, `RemoveIPC=yes`, `UMask=0077` |
| Network | `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK`; `IPAddressDeny=localhost link-local multicast` so no host loopback service is reachable **[R2-6]**; acceptance enumerates OVH's abstract Unix sockets and pathname sockets reachable from the unit |
| Environment | closed: `HOME`, `PATH`, `LANG`, `TZ`, `TMPDIR`, `AGY_CLI_DISABLE_AUTO_UPDATE=true`, `DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent`, `CREDENTIALS_DIRECTORY` **[R1-4]** |
| Resources | per unit `MemoryMax`, `TasksMax`; `Slice=cowboy-accounts-<key>.slice` (at most one `runtime` and one `login`/`logout` unit per account) under `cowboy-accounts.slice` with a host-wide `MemoryMax` and unit count; home size quota checked by the daemon before each start **[R2-18]** |

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
`RunCommand`, `StopCommand`. The hook allows exactly
`mcp_cowboy_ReadFile` … `mcp_cowboy_StopCommand` plus `ask_question`, and
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
`ok` parks the account's queues; `rate_limited_until` additionally requires a
new owner send after the time; `suspended` and `blocked` stop the account's
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
  accounts stay strictly isolated from each other.
- **[R1-17]** The browser's unit gains
  `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK` for accounts of a
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

**New session picker**: one entry per admitted, signed-in, `ok` account; never
preselected; others disabled with the reason; no default entry.

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
  account browser's unit adds `SystemCallArchitectures=native`, a syscall
  filter without `io_uring`, `RestrictAddressFamilies=AF_INET AF_UNIX
  AF_NETLINK`, and its AppArmor sealed profile denies `inet6`. Acceptance
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
  `provider_account_turns` (one row per account: session, turn = the prompt's
  `cmid`, worker epoch, the session's event cursor `after_seq`, state
  `reserved | may_have_dispatched`) and `provider_account_fences` (one current
  fence per account and Machine, `stop | admit`, `pending | confirmed`). A
  state change writes its fences in the same transaction.
- **Parking.** A refused dispatch requeues the prompt unchanged and shows the
  reason as the session's resource wait; the maintenance loop rechecks the
  cached controls before draining again. Any state but `ok`, a missing
  admission, another session's turn or an unacknowledged uncertain effect
  parks. A rate limit (default one hour; the proxy reports no reset time)
  parks until `until_ms`; after it only an explicit owner send (send, send
  now) resumes the account, an automatic drain does not.
- **One active turn.** The dispatcher reserves the account before any
  preparation, persists `may_have_dispatched` before the Provider lifecycle
  fence and the enqueue, and releases the slot when a path never enqueued the
  prompt (a requeue before the send, or the enqueue's synchronous refusal:
  positive in-process proof, not inferred absence). The reconciler (every 3 s
  and at start) settles a `may_have_dispatched` turn only from the first turn
  end after `after_seq` (classified by the proxy's closed codes) or from proven
  loss: the Machine reports a sandboxed worker stopped only after the daemon
  confirmed its unit gone, and a connected Machine shows no live worker for a
  stopped session. No backend evidence survives a lost unit, so loss settles
  `unknown_refusal`. A `reserved` row no process is dispatching settles
  `not_dispatched`; anything else keeps the slot and, after 2 minutes, marks
  the account `needs_attention`. Settlement never lowers a more severe state
  and never replaces `blocked`.
- **Fences.** `MachineCommand::AccountSandboxFence` (Machine protocol 48) asks
  the Machine's daemon for `stop_account` or `admit_account`; only the typed
  `AccountSandboxFenceObserved` for the same request confirms it, and
  `stop_account` answers only after it confirmed every unit gone. The
  operation ID is derived from the account, Machine, action and control
  revision, so retries after a lost reply repeat the same request; pending
  fences are retried while the Machine is connected. The Machines are the
  account's native generations plus its sessions' runtime Machines.
- **Kill switch.** `block` (with confirmation) writes `blocked` and the stop
  fences first, then the fences stop and confirm the units; `unblock` writes
  `ok` with admit fences. `force_release` of an unsettled turn requires every
  fence of the account to be a confirmed stop.
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
- **Detection.** `suspended` and a policy violation (`needs_attention`,
  reason `policy_violation:<tool>`) stop the account's units like a block.
- **Uncertain effects.** The worker ledger counts lost replies
  (`UncertainEffect.sequence`); the Controller records the newest observed
  sequence per session durably and parks that session until the owner
  acknowledges exactly that sequence.

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
  alone; split execution stays undeclared until a second layer is proven or
  the owner accepts the hook as the only one for reads.
- The (b) and (c) cases with `settings.json` present were not run (prompt
  budget); with the deny layer shown ineffective they reduce to the absent
  rows.

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
