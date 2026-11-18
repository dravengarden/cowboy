# Antigravity real accounts on OVH — design

Status: design, revision 2, 2026-11-18. Nothing here is implemented. It extends
[`antigravity-provider-plan.md`](antigravity-provider-plan.md) Phases 3 and 4
under the
[Antigravity consumer-subscription boundary](requirements.md#antigravity-consumer-subscription-boundary).
Revision 1 was design-reviewed by Codex Astra Max (call
`call-b27d513a33f554b8f7097883`, 18 findings); each finding is answered below
as **[R1-n]**.

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

## Why not same-user confinement

Revision 1 confined `agy` with Landlock under the Machine's `ubuntu` user.
Landlock restricts pathname Unix sockets only from ABI 9; measured ABIs are
7 (Hawk, 6.18) and 8 (OVH, 7.0), so a confined process could still reach the
user's D-Bus and systemd manager and start an unconfined process **[R1-1]**.
Same-user processes of other Providers also keep full access to the tokens
**[R1-2]**. OVH has passwordless root for the operator and systemd 259, whose
system units provide per-unit identities and full sandboxing without user
namespaces (which Ubuntu restricts for unprivileged users).

**Spike on OVH (2026-11-18, transient units, removed afterwards):** a
`systemd-run --pipe --wait` system unit with `DynamicUser=yes`,
`User=cowboy-agy-spike-a`, `StateDirectory=`, `ProtectHome=tmpfs`,
`ProtectSystem=strict`, `PrivateTmp=yes`, `NoNewPrivileges=yes` and
`RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK` ran as uid 65419; `/home`
was empty, `/run/user/1000/bus` absent, `socket(AF_INET6)` failed with
`EAFNOSUPPORT`, stdin/stdout piped through, and the official `agy` 1.5.9
bound read-only answered `--version` and ACP `initialize`.

## Architecture

```
Controller ── Machine (ubuntu, user unit) ── worker (ubuntu)
                     │                          │ stdio (ACP)
                     │ register / login / ...   │
                     ▼                          ▼
          cowboy-account-sandbox (root daemon, Unix socket)
                     │ systemd-run --pipe (system unit per process)
                     ▼
  account unit  User=cowboy-acct-<key> (DynamicUser), StateDirectory
     agy-proxy (Node, signed)  ── agy acp (official, pinned)
        └─ facade MCP (stdio) ── execution endpoint (target Machine)
```

### Account sandbox daemon (new, root, OVH host configuration)

A small static Rust binary `cowboy-account-sandbox` built on Hawk from the
Cowboy repository, installed by Columbus `machines/ovh` host configuration
(with an activation receipt) as `/usr/local/libexec/cowboy/account-sandbox`,
root-owned, plus a socket-activated root system service listening on
`/run/cowboy-account-sandbox.sock` (mode 0660, group `cowboy-sandbox`, of which
`ubuntu` is a member). No sudo rule is added.

**The caller is not trusted.** Every process running as `ubuntu` can reach the
socket, including other Providers' native agents. On OVH the Machine and its
workers share one cgroup, so the daemon cannot distinguish them. The daemon is
therefore built so that a hostile caller can at most drive an admitted account
through the same fixed, policy-enforcing entrypoint the worker uses, never run
its own code as an account identity or read account state:

- **Runtime store.** `register-runtime <dir> <sha256>`: the daemon copies the
  generation's runtime directory (the `agy-proxy` Node archive and the vendor
  `agy`) into `/var/lib/cowboy-account-sandbox/runtime/<sha256>/`, root-owned,
  read-only, and refuses unless the content digest matches. It additionally
  checks the vendor executable's SHA-512 against a root-held allow-list of
  pinned `agy` releases written by the host configuration. Account units run
  only from this store.
- **Fixed argv.** The caller chooses a mode (`runtime`, `login`, `logout`,
  `probe`), an account key and a generation; the daemon builds the whole
  command line, environment and unit properties itself. No caller-supplied
  argv, environment, path or property reaches the unit.
- **Account keys.** `key = hex(sha256("cowboy-account-v1" || service_id ||
  account_id))[..20]`; the identity is `cowboy-acct-<key>`. Keys are validated
  as exactly 20 lowercase hex characters.
- **Generations.** `create-generation <key>` allocates the next generation
  directory; `login` runs only once and only on a generation that has never
  been used; `runtime` only on a generation whose login completed; `logout`
  and `archive` on a completed one. A caller cannot log another Google account
  into an existing generation.
- **No reads.** The daemon never returns file content from an account
  directory. Its replies are fixed enums (state, generation numbers, exit
  status), plus the login process's own stdout/stderr stream for the challenge.

### Account units

Each `runtime`, `login`, `logout` or `probe` invocation is one transient system
unit `cowboy-acct-<key>-<mode>-<nonce>.service`, started with
`systemd-run --pipe --wait` by the daemon; its stdio is passed back to the
caller over the socket (`SCM_RIGHTS`) so ACP flows directly between worker and
unit. Properties:

| Property | Value |
|---|---|
| Identity | `DynamicUser=yes`, `User=cowboy-acct-<key>` (stable per account), `Group=` same |
| State | `StateDirectory=cowboy-accounts/<key>`; `HOME` = `…/<key>/generations/<n>/home`; nothing else writable |
| Filesystem | `ProtectSystem=strict`, `ProtectHome=tmpfs` (no `/home`, so no `ubuntu` state, no other Provider credential), `PrivateTmp=yes`, `PrivateDevices=yes`, `BindReadOnlyPaths=` the runtime store entry, `ReadWritePaths=` only the generation home, `TemporaryFileSystem=/var/lib/cowboy-accounts` with only this key re-bound |
| Process | `NoNewPrivileges=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `ProtectKernel*`, `ProtectControlGroups=yes`, `RestrictNamespaces=yes`, `SystemCallFilter=@system-service`, `CapabilityBoundingSet=` empty, `RemoveIPC=yes`, `PrivateIPC=yes` |
| Network | `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK` (IPv4 only) for every mode, so login and logout are covered **[R1-17 for `agy`]** |
| Environment | closed: only `HOME`, `PATH`, `LANG`, `TZ`, `TMPDIR`, `AGY_CLI_DISABLE_AUTO_UPDATE=true`, `DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent`, and for `runtime` the execution descriptor path; nothing inherited **[R1-4]** |
| Credentials | the execution descriptor arrives as `LoadCredential=` from a file the worker created, readable inside only through `$CREDENTIALS_DIRECTORY` **[R1-13]** |
| Resources | `MemoryMax`, `TasksMax`, `CPUQuota` per unit; one slice per account |

Different accounts are different uids with 0700 state, separate `/tmp`,
`/dev/shm`, IPC and no shared bus; `ubuntu` processes cannot read account state
and account processes cannot read `ubuntu`'s home or another account's state.
The only shared inputs are the read-only runtime store and the host's
read-only system tree. The trusted-root exception of `requirements.md` stays:
root and the daemon can read everything.

### Machine changes

- Antigravity's signed manifest declares `runtime.isolation:
  "account_sandbox_v1"` (new optional runtime field, SDK floor raised; older
  hosts reject it). A Machine without a reachable sandbox daemon reports the
  Provider unavailable and refuses every Antigravity launch, login and logout.
  There is no fallback to same-user execution; the Hawk local mode is removed.
- Native generations of a sandboxed Provider keep their Machine-side metadata
  (receipts, identity, generation numbers) under the Machine state root, but
  the home itself is the daemon's generation. `native_login`, `native_logout`
  and cleanup call the daemon instead of `authentication_process`.
- The worker starts the agent by asking the daemon for a `runtime` unit
  instead of spawning the entrypoint, and relays the returned stdio.
- Install probes run through the daemon's `probe` mode.

## Kill switch and logout **[R1-6]**

- **Fence first.** A per-account `blocked` state is written durably on the
  Controller before anything else; every dispatch, revival, login and session
  creation checks it (see controls).
- **Stop.** The daemon `stop-account <key>` stops every unit of the account
  (`systemctl stop` of the account slice) and reports the confirmed unit set.
- **Native logout without deleting history.** For sandboxed Providers,
  logout runs `agy-proxy --logout` (ACP `logout`) in a `logout` unit, then the
  daemon removes **only** the token file name the vendor uses
  (`.gemini/antigravity-cli/antigravity-oauth-token`) if `agy` left it, and
  marks the generation `logged_out`; conversations stay. Account removal
  (owner action) is a separate operation that archives then deletes the
  generation after an explicit confirmation.
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

**Facade identity [R1-5].** MCP server name `cowboy`; exact tool set
`ReadFile`, `ListDirectory`, `SearchFiles`, `WriteFile`, `EditFile`,
`RunCommand`, `StopCommand`. The hook allows exactly
`mcp_cowboy_ReadFile` … `mcp_cowboy_StopCommand` plus `ask_question`, and
denies everything else, including unknown names. Startup verifies `agy`'s
tool listing does not contain any other MCP tool. `agy`'s user and workspace
configuration sources are disabled: `HOME` is a fresh generation (no user
`hooks.json`, `settings.json`, skills or MCP config besides Cowboy's, written
read-only by the proxy before `agy` starts and owned by root through the
runtime store bind), and `--include-default-customizations=false`.
The hook is `hook.mjs`; a timeout, malformed input, non-zero exit or missing
Node yields deny (the hook command is wrapped so that any failure prints the
deny JSON).

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

**Mutations [R1-11, R1-12].** `WriteFile` and `EditFile` use the execution
helper's conditional write (compare the observed content hash, publish
atomically, report a conflict) instead of read-check-write. Each mutation and
process start gets a proxy-allocated operation ID persisted in the generation
home before submission; a lost reply leaves the ID `uncertain`, and any further
mutation of the same path or a new start for the same tool call is refused
until observation (re-read hash, `process/read`) resolves it. Tested with
repeated MCP calls and cold `session/load`.

**Cancellation [R1-10].** Process IDs are allocated by the proxy before
`process/start` and recorded with their ACP turn and tool call. ACP
`session/cancel`, MCP cancellation, proxy shutdown and account stop all send
`process/terminate` for every live ID of the turn; a start after cancellation
is refused; termination requests are retried until `process/exited` or the
target reports the ID unknown.

**Bounds [R1-16].** `ReadFile` ≤ 256 KiB per call (offset/length paging),
`ListDirectory` ≤ 2,000 entries, depth 1; `SearchFiles` ≤ 500 matches and
512 KiB output, 30 s; `RunCommand` output ≤ 256 KiB retained (head and tail),
default timeout 120 s, maximum 600 s; every request below the connection's
14 MiB frame. Projection: Git status ≤ 200 lines, instruction snapshot ≤ 64 KiB
total, each file ≤ 16 KiB, explicit `[truncated]` markers.

**Context suppression [R1-15].** The fresh generation home contains no user
guidance, skills or settings; `--include-default-customizations=false` removes
built-in customizations. Native acceptance compares deliberately different
runtime and target contexts (OS, cwd, guidance) across new, load, resume and
compaction, and asserts that the model reports only the target's. Repository
text in the projection is fenced as untrusted content, separate from Cowboy's
routing instructions.

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

**One active turn per account [R1-8].** The Controller reserves the account
slot atomically (durable row: account, session, turn ID) before dispatch;
the reservation survives restarts and uncertain dispatch. It is released only
after the worker reports the turn's single terminal ACP response *and* any
refusal state has been committed. On Controller start, reservations are
reconciled with live account units (daemon `list-units <key>`); an
unaccounted unit keeps the slot until it exits or is stopped. Late replies
after a terminal response are discarded.

**Refusal evidence [R1-7].** Log-watching is not accepted as authoritative
until proven. The acceptance phase with the test account records the exact
`agy` log lines and ACP shapes for a successful turn, `VALIDATION_REQUIRED`
and `UNSUPPORTED_LOCATION` (both observable without provoking enforcement),
including how a line is attributed to a turn (conversation/step IDs in the
log). Only fields that `agy` writes from the backend response (status code and
reason enum, never model text) are parsed, from the exact process's own log
file (the unit's private home makes the file unambiguous), with a cursor per
turn and a bounded reader. Until the suspension and 429 shapes are observed:
any turn that ends without a positive backend success marker in the log is
classified `unknown_refusal`, which parks the account and asks the owner;
this fails closed.

## Sign-in

- The account's Machine browser on OVH, with `accounts.machine_browser` and
  the account's `browser_enabled`; one browser identity per account.
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

## Residual risks

- Linkage of the accounts by shared egress, host fingerprint and timing.
- A hostile `ubuntu` process can drive an admitted account through the
  sandboxed entrypoint (quota use), but cannot read its credential or run code
  as it; admission, the turn cap and the state fence still apply.
- Workspace tools run with the session's full rights on the development
  Machine.
- Root on OVH can read every account's state.
