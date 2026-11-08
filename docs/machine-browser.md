# Machine browser: owner-operated browser on a Machine

Status: design, revision 3, 2026-11-08. Nothing below is implemented. Three
Astra Max design reviews (2026-11-08: 12, 17 and 14 findings) are folded in
and cross-referenced in [Acceptance](#acceptance-before-any-real-account).
The owner decided the policy boundary the same day; see the
[credential rule](requirements.md#named-provider-accounts-isolation-acceptance)
amendment. The owner also stated strict isolation between the browsers of
different account cards as a core requirement; see
[Account isolation](#account-isolation-core-requirement).

## Problem

A named Claude sign-in runs `claude auth login` on the account's Machine
(today OVH), but the authorization page opens on the owner's own device. The
vendor therefore sees the sign-in from one network identity, timezone and
browser, and every later API call from another. The owner's device browser
shares one claude.ai cookie store across accounts, which the sign-in dialog
already warns about (`web/src/SubscriptionAuthorization.tsx`). The sign-in
page sits behind an interactive Cloudflare challenge that an automated browser
cannot pass and a person can.

The owner wants the sign-in browser to run where the credentials will be
used, operated from the phone or desktop through Cowboy, with no resident
process and no automation signature, and with nothing shared between the
browsers of different accounts.

## Goal and non-goals

Goal: an optional Plugin gives a Linux Machine a real, unmodified Chromium on
its own X display, owned per Provider account, started on demand, streamed
into the Cowboy web UI (phone and desktop) over the existing Machine
connection, and stopped when idle. The named-account sign-in flow can open the
challenge URL in that browser. Its network identity, timezone, locale and DNS
are the Machine's, because the browser is a process on the Machine.

Non-goals:

- a general remote desktop, or any network listener on the Machine;
- automating the page: no DevTools protocol, no WebDriver, no headless mode,
  no injected scripts, no extensions, no `--enable-automation`,
  no `--no-sandbox`;
- spoofing platform, user agent, GPU or screen facts. A GPU-less Linux VPS
  reports itself as one; the native login already uses that address, and a
  consistent true fingerprint is the safe one;
- any change to how the Claude CLI is launched or to the signed
  `authentication` section of `plugins/claude-code/provider.json`.
  `native_release_compatible` requires that section byte-identical for
  existing homes to carry over, so touching it would force every named
  account to sign in again.

## Boundary: Plugin versus core

CR-0 reserves communication, installation, security, state and the UI host to
core; Plugins supply bounded data and isolated Machine runtimes, never host
code (`plugin-spatiotemporal-design.md` §1.1). The split follows the Zed
precedent: the engine is a Plugin, the product is core.

| Concern | Owner |
|---|---|
| Browser runtime bytes: launcher, Xvnc, window manager, Chromium, helpers, fonts, managed policy file | `machine_browser` Plugin (signed artifact) |
| Launch graph, readiness protocol, platform and host-ABI matrix, declared executables and policy contract | Plugin contract (data) |
| Plugin kind, schema, signing, Catalog, install/upgrade/rollback/uninstall, generation leases | Core (generic Plugin lifecycle) |
| Instance manager: admission, start, attachment leases, hibernation, caps, reconciliation, confinement verification, profile snapshots | Core Machine |
| Per-account profile placement, reset, revocation wipe, durable fences | Core Machine + Controller revocation plan |
| Display stream, backpressure, binding, continuing authorization | Core wire protocol + Machine + Controller |
| Routes, owner/device/Origin checks, per-account enablement | Core Controller |
| Viewer (canvas, decoder, input, clipboard action, mobile sheet) | Core web |
| Sign-in integration | Core (Machine login session + Controller + web) |
| Resource settings | Core Device/Service configuration |
| Host sandbox policy (AppArmor) | Host configuration (Columbus `machines/ovh`), outside Cowboy |

Without the Plugin installed on a Machine, the core mechanism admits nothing:
the Machine advertises no browser capability and `open` answers
`not_available`. Stop, reset, revocation and reconciliation stay available for
anything that still exists, so disabling admission can never strand a running
unit or a profile.

## `machine_browser` Plugin kind

Release schema 6, payload schema 1, Plugin SDK 1.13+. Catalog readers before
this kind accept release schemas up to 5 and skip these envelopes. The kind is
Machine-installable, Linux-only, and carries exactly one runtime artifact per
declared platform. Like `code_intelligence`, the contract is a closed launch
graph; it has no UI, URLs, credentials or network targets.

```json
{
  "schema_version": 1,
  "id": "machine-browser",
  "version": "1.0.0",
  "display_name": "Machine browser",
  "description": "Owner-operated Chromium on the Machine for account sign-in.",
  "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
  "host_abi": {
    "interpreter": "/lib64/ld-linux-x86-64.so.2",
    "glibc_min": "2.39",
    "distributions": [{"id": "ubuntu", "version_id": "26.04"}]
  },
  "runtime": {
    "artifact": "runtime",
    "launcher": "bin/cowboy-browser-launcher",
    "browser": "opt/chromium/chrome",
    "browser_version": "151.0.7922.71",
    "display_server": "usr/bin/Xtigervnc",
    "helpers": ["usr/bin/xrandr", "usr/bin/xkbcomp", "usr/bin/xauth", "usr/bin/matchbox-window-manager"],
    "policy": {"file": "etc/cowboy-browser/policy.json", "directory": "/etc/opt/chrome/policies/managed"}
  },
  "launch": {
    "protocol": "cowboy-browser-control-v1",
    "readiness_timeout_ms": 20000
  }
}
```

- Every `runtime.*` path is a normalized relative path (no absolute path, no
  `.` or `..` components, no symlink) that the SDK resolves inside the named
  artifact at validation time, for every declared platform, exactly as the
  code-intelligence validator checks artifact correspondence and platform
  coverage. The Machine recomputes the same checks against the installed,
  content-addressed generation before every launch; a missing or non-regular
  executable fails admission, never falls back to an ambient binary.
- `host_abi` declares what the tree assumes of the host: the ELF interpreter
  path, the minimum glibc and the distributions the artifact was built for
  and tested on. The Machine advertises the browser capability only when
  the interpreter exists, `getconf GNU_LIBC_VERSION` meets the minimum and
  `/etc/os-release` matches a declared distribution; otherwise the reason is
  `host_abi`. A pristine Ubuntu 26.04 test proves that target, not Linux in
  general.
- `runtime.policy.directory` is the browser build's managed-policy directory
  (`/etc/opt/chrome/policies/managed` for Google Chrome,
  `/etc/chromium/policies/managed` for Chromium builds). It is validated
  against the build at packaging time and the launcher binds
  `runtime.policy.file` there; the pair is a signed binding like the
  executables, not a documentation hint.
- `browser_version` is the full writer version used by the
  [profile compatibility fence](#profile-compatibility-and-rollback).
- The Machine passes no shell or template arguments. The launcher receives a
  fixed argument vector: `--control-socket`, `--instance-dir`,
  `--profile-dir`, `--runtime-dir`, `--display`, `--geometry WxH`,
  `--scale N`, `--tmpfs-bytes N`. Everything else is in the launcher.
- The runtime artifact is a relocatable, symlink-free, hard-link-free tree
  within the existing limits (1 GiB compressed, 4 GiB expanded, 100 000
  entries; symbolic and hard links are rejected by the installer). It is
  assembled on a builder from the target distribution's packages (Ubuntu
  26.04 today: Chromium or Google Chrome, TigerVNC `Xtigervnc`, matchbox
  window manager, `xkbcomp`, `xrandr`, `xauth`, XKB data, fonts) by
  resolving the complete package dependency closure and excluding only what
  `host_abi` guarantees (glibc, the dynamic loader). The builder audits every
  ELF object with `ldd` against the tree plus host glibc and refuses a
  missing dependency. The `chrome-sandbox` setuid helper is removed from the
  tree; Chromium uses its namespace sandbox.
- Conformance runs the exact signed archive through the real Machine
  installer on a pristine Ubuntu 26.04 guest with the production AppArmor
  policy enforced and the stock user-namespace restriction intact, then
  opens, attaches, resizes, copies and hibernates, and runs the syscall-level
  canary probes described under [Acceptance](#acceptance-before-any-real-account).
- Chromium security releases ship as new Plugin versions through the
  ordinary per-Machine install/upgrade/rollback lifecycle, independent of
  Machine host releases.

### Plugin lifecycle integration (CR-2)

A running instance holds a generation lease on the exact installed bytes, as
Code worktrees do. Upgrade installs the new generation beside the old; new
opens use it; running instances keep their pinned bytes until their unit
exits. Uninstall with a running instance first hibernates it (explicit drain
with the owner notified), then proceeds; the generic uninstall path gains a
`machine_browser` branch for this because today it only removes the active
link. Generation retirement treats browser leases like Code leases: a
generation retires only after its last browser lease releases. After the
Plugin is uninstalled, profile snapshots remain account data; reset and
revocation still delete them.

### Launcher contract

The launcher is a small static binary in the artifact. It runs under the host
AppArmor profile described [below](#host-sandbox-policy-outside-cowboy), and
it builds the instance's isolation before anything else starts. Every step
is fixed; nothing is configurable from outside beyond the argument vector.

1. **Namespaces.** `unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWIPC)`.
   The user namespace maps the Machine user's uid and gid to themselves and
   nothing else, so no process inside is root in any sense. Mount
   propagation is made recursively private before any mount.
2. **Private mounts, in this order.** Bind the runtime tree read-only,
   `nosuid`, `nodev` at `$XDG_RUNTIME_DIR/cowboy-browser/runtime`; mount a
   tmpfs of `--tmpfs-bytes` at `$XDG_RUNTIME_DIR/cowboy-browser/profile` and
   restore the account's profile snapshot into it (see
   [Profile storage](#profile-storage-snapshots-not-live-disk)); bind the
   unit's `RuntimeDirectory` read-write at
   `$XDG_RUNTIME_DIR/cowboy-browser/instance`; mount a private tmpfs over
   `/tmp` (256 MiB) and a private tmpfs over `/dev/shm` (`--tmpfs-bytes`);
   build `/etc` as a private tmpfs scaffold containing bind mounts of exactly
   `resolv.conf`, `hosts`, `nsswitch.conf`, `ld.so.cache`, `localtime`,
   `passwd`, `group`, `ssl/` and `fonts/` from the host plus the policy file
   at `runtime.policy.directory` (so nothing has to exist on a pristine host
   and nothing else from the host's `/etc` is visible); then mask `/home`
   with an empty tmpfs. The Machine state root lives under `/home` on
   supported hosts, so masking the ancestor hides it; if a host places the
   state root elsewhere, the launcher masks that path before `/home`. All
   sources were bound before their ancestors were masked, so the bind mounts
   keep their objects.
3. **Seal.** Close every descriptor except the control socket, `chdir` to
   the instance mount point, drop every capability from the effective,
   permitted, inheritable and ambient sets and from the bounding set, and
   set `no_new_privs`. From this point no process in the instance, including
   the launcher itself, can mount, unmask or rebind anything in this mount
   namespace, because the namespace is owned by a user namespace in which
   nothing holds `CAP_SYS_ADMIN`. Chromium's zygote still creates its own
   nested user, PID and network namespaces for its sandbox: creating a user
   namespace needs no capability, the AppArmor grant is inherited, and the
   capabilities it gains apply only inside the nested namespace, which cannot
   reach this mount namespace.
4. **Display.** Write an X authorization cookie into the instance directory
   and start `Xtigervnc` on a fixed display number (collisions are impossible
   because `/tmp` and the abstract socket space are not shared, see next
   item) with `-auth <cookie> -nolisten tcp -nolisten local -rfbport -1
   -rfbunixpath <instance>/rfb.sock -rfbunixmode 0600 -SecurityTypes None
   -AcceptSetDesktopSize=0 -SendPrimary=0 -SendCutText=1 -AcceptCutText=1
   -geometry WxH -depth 24 -xkbdir <runtime xkb>` and `XKB_BINDIR` pointing
   at the runtime's `xkbcomp`. `-nolisten local` disables the X server's
   abstract Unix socket, which would otherwise live in the shared network
   namespace; the only X endpoint is the pathname socket in the private
   `/tmp`. The RFB socket is reachable from outside the namespace only
   through the bind-mounted instance directory, which is 0700.
5. **Window manager** keeps the single browser window fullscreen and
   following the screen size.
6. **Browser.** Clear the environment and set only: `PATH` and
   `LD_LIBRARY_PATH` inside the runtime, `FONTCONFIG_FILE` (generated into
   the instance directory, pointing at the runtime's fonts),
   `GSETTINGS_SCHEMA_DIR` and `LIBGL_DRIVERS_PATH` inside the runtime,
   `DISPLAY`, `XAUTHORITY`, `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
   `XDG_RUNTIME_DIR` and `TMPDIR` inside the instance directory, `TZ` from
   the host, `LANG=en_US.UTF-8`, and `SSL_CERT_FILE` for the scaffolded
   certificate bundle. No proxy variable is inherited or set. Chromium starts
   with `--user-data-dir=<profile> --no-first-run --no-default-browser-check
   --disable-sync --disable-background-networking --disable-component-update
   --disable-extensions --disable-breakpad --disable-crash-reporter
   --disable-quic --disk-cache-size=1048576 --password-store=basic
   --lang=en-US --window-position=0,0 --window-size=WxH
   --force-device-scale-factor=<scale> about:blank`.
7. **Managed policy.** The policy file is part of the signed artifact and
   turns off everything a sign-in browser must not do, enforced by the
   browser inside the isolated view: `RemoteDebuggingAllowed=false`,
   `DeveloperToolsAvailability=2`, `DownloadRestrictions=3`,
   `AllowFileSelectionDialogs=false`,
   `URLBlocklist=["file://*","chrome://*"]` with `URLAllowlist` for
   `chrome://newtab`, `ExtensionInstallBlocklist=["*"]`, `BrowserSignin=0`,
   `SyncDisabled=true`, `PasswordManagerEnabled=false`,
   `AutofillAddressEnabled=false`, `AutofillCreditCardEnabled=false`,
   `BackgroundModeEnabled=false`, `MetricsReportingEnabled=false`,
   `DefaultBrowserSettingEnabled=false`, `PromotionalTabsEnabled=false`,
   `QuicAllowed=false`, `WebRtcIpHandling=disable_non_proxied_udp`,
   `DefaultGeolocationSetting=2`, `DefaultNotificationsSetting=2`,
   `AudioCaptureAllowed=false`, `VideoCaptureAllowed=false`,
   `RestoreOnStartup=5`. Session restoration is off on purpose: a reopened
   profile starts on the new-tab page and never resumes old authenticated
   tabs or challenge URLs before the Machine's own verification and
   navigation; cookies persist through the profile snapshot, not through
   session restore. Some of these settings are observable by pages as
   ordinary denied permissions; they are choices any user could make.
8. **Control socket** (`cowboy-browser-control-v1`, newline-delimited JSON
   on a Unix socket in the instance directory): the launcher sends `ready`
   only after all three processes run, carrying the pids and start times of
   the launcher, display server, window manager and browser, the launcher's
   `/proc/self/ns/{user,mnt,ipc}` identities, its `uid_map`/`gid_map`, its
   confinement label, and the mount table it built. The Machine verifies the
   same facts from outside before accepting readiness (see
   [confinement verification](#confinement-verification)). Commands:
   `navigate {url}` (re-invokes Chromium against the running profile; the
   singleton socket in the profile hands the URL to the existing instance; no
   debugging port), `resize {width, height}` (through `xrandr`, bounded by
   the launcher and by the Machine to 320×480…`browser.max_geometry`;
   `scale` is fixed for the life of the incarnation), `attach {attachment_id}`
   and `detach {attachment_id}` (informational), `snapshot` (quiesce and
   let the Machine read a consistent profile copy; see below) and `stop`.
9. It exits when any of the three processes exits; `KillMode=control-group`
   ends the rest.

## Instance manager (core Machine)

### Identity and placement

```text
BrowserInstance = Machine + Plugin generation digest + account_id + instance_id + incarnation
Attachment      = BrowserInstance + attachment_id (one live attachment per instance)
Profile         = account_id + profile_revision (+ writer version)
```

One instance per `(Machine, account)`; `browser.max_instances` (default 1)
bounds concurrently running instances per Machine. Each instance runs in its
own user, mount and IPC namespaces, its own X server, its own `/tmp` and
`/dev/shm`, and its own systemd unit and cgroup.

### Profile storage: snapshots, not live disk

The live profile never touches the shared filesystem. During a run it lives
in the instance's size-bounded private tmpfs (`browser.profile_max_bytes`),
so the kernel enforces the storage bound and a page that fills browser
storage gets `ENOSPC` inside the browser instead of filling the volume that
holds Machine state and native credentials. The tmpfs pages count toward the
unit's `MemoryMax` and never swap.

Persistence is a Machine-written snapshot at
`provider-auth/accounts/<uuid>/browser/profile/` (0700, created parent-first
with `prepare_private_directory` at every level): on `hibernate`, on explicit
`snapshot`, and at `browser.snapshot_interval` (default 10 min) while
attached, the Machine asks the launcher to quiesce, copies the tmpfs contents
into a staging directory beside the snapshot, fsyncs, and renames it into
place; the previous snapshot is kept until the rename succeeds. A crash or
OOM loses only the changes since the last snapshot. Cookies, local storage
and Cloudflare clearance therefore persist; caches are excluded from the
snapshot. The snapshot is readable by the trusted Machine supervisor and by
the host administrator, like every other account file; no other browser
instance can reach it.

`browser-state.json` lives beside the `browser/` directory, outside the
deleted tree, and is fsynced before any mutable step: `profile_revision`,
the full `browser_version` of the last writer and the generation digest,
plus any pending reset or revocation fence with its operation id.

### Durable placement, reset and revocation

The Controller keeps a durable browser placement
`(owner, account, machine, profile_revision, last_open)` written when an open
is admitted. The account revocation plan gains `browsers` beside `logins`,
`grants` and `sessions`, captured from these placements, so a `Tab`-only
placement is revoked like any other target.

On the Machine, `revoke_browser` and `reset_browser` share one path, in this
order: write the operation fence and the next `profile_revision` into
`browser-state.json` and fsync (an admission fence that survives a crash);
stop the running unit and wait for its cgroup to be empty, escalating
`SIGTERM` to `SIGKILL` on the unit's own timeout; delete
`accounts/<uuid>/browser` with a scoped, no-follow removal; record completion
in `browser-state.json`. A restart in the middle resumes the fenced
operation from the state file and preserves its receipt; a delayed reset or
revocation carrying an older revision is a no-op. Native tombstones and every
other file under the account are untouched. Revocation completion is not
recorded while a browser placement is still pending.

### Lifecycle

```text
hibernated ──open──▶ starting ──ready──▶ detached ◀──attach/detach──▶ attached
     ▲                   │ failure                 │ grace elapsed, hibernate, deadline
     └───────────────────┴──────────── stopping ◀──┘
```

- `hibernated`: no process, no memory, snapshot on disk. This is what the
  owner's "tab" shows when nothing is running.
- `open` admits only when the Plugin is installed and its executables, policy
  binding and `host_abi` verify, the host policy is present (see
  [host policy](#host-sandbox-policy-outside-cowboy)), the account is
  enabled for the browser and not fenced, `browser.max_instances` is not
  reached, and the recorded writer version is not newer than the runtime's.
  Memory admission goes through the same compare-and-swap launch ledger as
  cold workers, converted to a byte-weighted reservation with exact refunds
  and the existing floor preserved: a browser reserves `browser.memory_max`
  bytes, a cold worker its existing reserve, so a browser and a worker can
  never both count the same headroom. The reservation is held until `ready`
  or failure, then refunded exactly.
- The unit is `systemd-run --user --collect --unit cowboy-browser-<instance>
  --slice cowboy-agents.slice -p Delegate=yes -p KillMode=control-group
  -p MemoryMax=<browser.memory_max> -p MemoryHigh=<browser.memory_high>
  -p MemorySwapMax=0 -p CPUQuota=<browser.cpu_quota> -p TasksMax=<browser.tasks_max>
  -p OOMScoreAdjust=1000 -p LimitCORE=0 -p RuntimeMaxSec=<browser.max_runtime>
  -p TimeoutStopSec=15 -p RuntimeDirectory=cowboy-browser/<instance>
  -p RuntimeDirectoryMode=0700`. The instance directory is that short
  `RuntimeDirectory` path, so every Unix socket path stays far below the
  kernel limit.
- `ready` is accepted only after [confinement verification](#confinement-verification).
- `attached` requires a live attachment lease. `detached` starts the
  `browser.hibernate_after` grace (default 120 s); expiry hibernates (which
  snapshots first). A sign-in-bound instance's deadline is
  `min(login expiry, browser.max_runtime)`.
- Every mutation (`hibernate`, `resize`, `navigate`, `stop`) carries the
  expected incarnation; attach, detach, resize and stream cleanup carry the
  `attachment_id`; reset carries the expected `profile_revision`. A command
  whose expectation no longer matches is a recorded no-op with reason
  `superseded`. Replays of the same request ID return the original receipt.
- Failure never retries with weaker isolation. A namespace or sandbox failure
  reports `sandbox_unavailable` with the host policy pointer; a confinement
  verification failure reports `confinement_unverified`; a missing host
  prerequisite reports `host_abi` or `listener_prevention_unavailable`. All
  leave the instance hibernated with the reservation refunded.

### Confinement verification

Before accepting `ready`, before every `navigate`, and before adopting any
surviving unit at restart, the Machine pins each reported pid to its reported
start time and reads from outside the namespace:

- `/proc/<pid>/ns/user`, `ns/mnt` and `ns/ipc` of the launcher, display
  server, window manager and browser: identical to the identities the
  launcher reported and different from the Machine's own; `uid_map` and
  `gid_map` contain exactly the single self-mapping;
- `/proc/<pid>/attr/current` of every pinned process: the expected AppArmor
  profile in enforce mode;
- `/proc/<pid>/mountinfo` of the browser: the profile mount point is a tmpfs
  of the expected size, the instance mount point is backed by the unit's
  `RuntimeDirectory` (matched by mount source and root, major:minor and
  inode of the backing directory), the runtime mount is read-only,
  `nosuid`, `nodev` and backed by the installed generation, `/tmp`,
  `/dev/shm`, `/etc` and `/home` are tmpfs with the expected flags, and the
  policy file is bound from the artifact at the contract's policy directory;
- `/proc/<pid>/status` of the browser's sandboxed descendants (found through
  the cgroup): `NoNewPrivs` set and capability sets empty in the instance's
  user namespace;
- the whole cgroup subtree of the unit (`cgroup.procs` recursively, because
  `Delegate=yes` lets children create sub-cgroups): no process owns a
  listening TCP socket or a UDP socket bound outside the kernel's ephemeral
  range, matched by socket inode against `/proc/<pid>/net/{tcp,tcp6,udp,udp6}`
  of each distinct network namespace found among those processes.

The socket scan repeats every 10 s while the instance runs and at every
attach, navigate and resize; a violation stops the instance with reason
`listener_detected`. The scan is detection evidence; prevention is the host
policy's socket mediation plus the browser policy (no QUIC, no non-proxied
WebRTC UDP, no remote debugging), and admission refuses when prevention is
unavailable.

### Storage during a run

The profile tmpfs is the storage bound; the instance directory, `/tmp` and
`/dev/shm` are bounded tmpfs as well, and the HTTP cache is bounded by flag.
Downloads and file dialogs are disabled by policy. Nothing the browser does
during a run can write to the shared filesystem. Snapshots are written only by
the Machine and refused when the Device disk low-watermark would be crossed,
in which case the instance keeps running and the owner is told the snapshot
is pending.

### Profile compatibility and rollback

`browser-state.json` records the full `browser_version` of the artifact that
last opened the profile, written and fsynced before the launcher restores the
snapshot into the tmpfs. `open` compares it with the runtime's
`browser_version`: an older runtime, including an older build of the same
major, refuses with `profile_newer_than_runtime` and offers Reset; a newer
runtime advances the record before launch. Artifact version and
profile-format compatibility are distinct, so the fence is the conservative
full version, not the major. The acceptance matrix includes upgrade → use →
rollback and a crash between the record and readiness.

### Restart reconciliation

The persisted instance record (`browsers/<instance>.json` under the Machine
state directory) holds account, Provider, generation digest, purpose and
login request, unit name and systemd invocation ID, boot ID, incarnation,
deadline, `profile_revision`, and the launcher's reported pids, start times
and namespace identities. At Machine start:

- a record whose boot ID differs from the current boot is cleared (the unit
  cannot have survived);
- a record with a live unit of the same invocation ID is adopted as
  `detached` only after the installation, executables and `host_abi`
  re-verify, the account is still enabled and unfenced, the deadline has not
  passed, a memory reservation is re-taken, and
  [confinement verification](#confinement-verification) passes against the
  recorded identities; otherwise the unit is stopped;
- a live `cowboy-browser-*` unit without a record is stopped;
- a sign-in-bound record is stopped at reconciliation, because the Machine's
  native logins do not survive a restart: a lost Controller connection
  cancels registered logins (`LoginConnection` drop) and native recovery
  terminates abandoned login trees. The browser follows its login's
  authority; it does not restore it.

### Sign-in binding

The Machine's `LoginSession` gains the login's account, Provider, native
generation and the `ConnectionToken` that dispatched it. When an open request
names a native login request, the Machine:

1. takes the native login lock; checks that the login is active on this
   Machine, that its account and Provider equal the browser's account and
   Provider, and that its dispatching connection is current; records the
   binding `(login request, generation, browser instance, incarnation)`;
   releases the lock;
2. starts the unit; once the unit is registered and verified it re-checks the
   same facts under the lock. If the login ended or the binding no longer
   matches, the instance is hibernated at once;
3. navigates to the `verification_url` the Machine itself emitted for that
   login (kept in `LoginSession`; the Controller never sends a URL to the
   Machine), after the pre-navigation verification;
4. hibernates the instance when the login completes, is cancelled, fails or
   expires, from the exit path that records the native login's outcome.

The Controller admits the sign-in open only for the owner of the login
operation whose receipt names that account and request, following the
continuation check in `provider_accounts/login/continuation.rs`.

Browser failures, stops, hibernation, OOM kills and storage stops never
cancel, restart, clean up or otherwise touch the native login process or the
account's credential generation. Machine restart and Controller disconnection
end native logins by existing design, independent of the browser.

## Stream channel (Machine protocol 43)

Commands and events are a closed addition to `MachineCommand` /
`MachineEvent` gated by a new minimum protocol:

- `BrowserOpen{request_id, account_id, instance_id, purpose: SignIn{login_request} | Tab, viewport{width, height, scale}}`
- `BrowserAttach{request_id, instance_id, expected_incarnation, attachment_id, stream_token, viewport}`
- `BrowserLease{instance_id, attachment_id, expires_at_ms}`
- `BrowserDetach{request_id, instance_id, attachment_id}`
- `BrowserResize{request_id, instance_id, expected_incarnation, attachment_id, width, height}`
- `BrowserNavigate{request_id, instance_id, expected_incarnation, url}` (`Tab` only)
- `BrowserHibernate{request_id, instance_id, expected_incarnation}`
- `BrowserReset{request_id, account_id, expected_profile_revision}`
- `BrowserState{instance_id, incarnation, account_id, state, reason, geometry, scale, deadline_ms, attachment_id, snapshot_at_ms}`

The display stream is not an RPC and does not use bulk lanes: lanes carry
idempotent execution requests, resend after loss and close on idle
(`src/machine_lanes.rs`), none of which fits an ordered live byte stream.
`BrowserAttach` carries a one-time `stream_token` minted by the Controller
and bound to the owner principal, device, account, instance, incarnation,
`attachment_id` and the Machine's current `ConnectionToken` identity (the
token object, not an epoch string). The Machine opens one dedicated
WebSocket, `/api/machine/browser-stream`, presenting the stream token. The
Controller accepts it only while that `ConnectionToken` is current. Only
after acceptance does the Machine connect to the instance's RFB socket. Bytes
relay both ways as binary frames of at most 64 KiB under a credit window per
direction; a slow consumer stalls its producer instead of growing a buffer,
and a frame that cannot be delivered within the credit timeout closes the
stream. Closing either side closes the RFB connection and moves the instance
to `detached`.

An attachment lease is independent of RFB traffic: the viewer renews it every
10 s through its authenticated WebSocket, the Controller forwards
`BrowserLease` renewals, and the Machine expires an attachment 30 s after the
last renewal. Expiry is monotonic: a renewal that arrives after expiry, or a
renewal carrying an older `attachment_id`, is refused and cannot resurrect
the attachment. The stream token expires after 30 s if unused; pairing must
complete within 15 s of the viewer's upgrade. A second viewer takes over only
through an explicit takeover, which mints a new `attachment_id`; the old
stream's delayed close or detach, carrying the old `attachment_id`, cannot
touch the replacement.

Continuing authorization follows the product WebSocket exactly: at pairing
the Controller validates the principal (cookie or bearer validity, session
freshness and step-up, device token validity and revocation, user disabled),
schedules the same authorization deadline the product socket schedules for a
cookie session, re-validates on every inbound input or renewal batch, and
closes both hops when the deadline passes or any check fails, even while
frames keep flowing. Both hops also close when the account is revoked or its
browser disabled, the Service gate closes, the Machine connection is lost or
replaced, or the instance incarnation changes.

Relay payloads are never logged, persisted, cached or inspected beyond
framing. Counters record bytes and frames.

## Controller routes and authority

| Route | Effect |
|---|---|
| `GET /api/provider-accounts/{account}/browser` | state, geometry, scale, deadline, snapshot time, Machine, reasons |
| `POST /api/provider-accounts/{account}/browser/open` | open as `Tab` (no URL body) |
| `POST /api/provider-accounts/{account}/logins/{request}/browser` | open bound to that sign-in; no URL in the body; owner and account checked against the login receipt |
| `POST .../browser/navigate` | `Tab` instances only, owner-supplied `https` URL, bounded |
| `POST .../browser/resize` | bounded geometry; carries the attachment |
| `POST .../browser/hibernate`, `POST .../browser/reset` | lifecycle, with expected incarnation or profile revision |
| `GET .../browser/stream` | viewer WebSocket; `takeover=true` replaces a live attachment |
| `GET /api/machine/browser-stream` | Machine upgrade with `x-cowboy-browser-stream-token` |

Route classification is explicit, with tests: the viewer routes are product
routes under `enforce_product_api` with the owner, device identity and
account-ownership checks, and the viewer `GET` upgrade runs the same Origin
check as `/ws` (the generic cookie Origin check covers `/ws` and mutations,
not arbitrary `GET` upgrades). `/api/machine/browser-stream` is a Machine
route like `/api/machine/lane`: its own token handler plus the
current-connection check, never admin-operator authentication, and never an
owner credential. Owner credentials, cookies and device tokens never reach the
Machine.

Enablement has two layers, both default off: the Service setting
`accounts.machine_browser` (gate, like `accounts.native_claude_login`) and a
per-account `browser_enabled` flag in the account registry, changed through
`accounts.update` with `{type: "set_browser_enabled"}`. Existing accounts stay
disabled until the owner enables each one; the first enablement is the test
account only. Cowboy MCP exposes the state read (`accounts.browser`) and the
lifecycle actions through `control.apply` with the same receipts as other
account operations, per the MCP discovery rule in `AGENTS.md`.

Device settings (Machine-enforced, `cowboy config`, reload class `Live`:
a changed value applies to the next `open`; running units keep the
properties they were started with; bounds enforced by the schema):

| Key | Default | Bounds | Meaning |
|---|---|---|---|
| `browser.max_instances` | 1 | 0–4 | running instances per Machine |
| `browser.memory_max` / `browser.memory_high` | 1536 MiB / 1200 MiB | 512 MiB–4 GiB | unit limits; also the admission weight |
| `browser.cpu_quota` | 200% | 50–400% | unit CPU quota |
| `browser.tasks_max` | 512 | 128–2048 | unit task limit |
| `browser.hibernate_after` | 120 s | 30 s–30 min | detached grace |
| `browser.max_runtime` | 30 min | 5 min–4 h | hard cap per run |
| `browser.profile_max_bytes` | 256 MiB | 64 MiB–1 GiB | profile tmpfs size and snapshot bound |
| `browser.snapshot_interval` | 10 min | 1 min–1 h | periodic snapshot while attached |
| `browser.max_geometry` | 2560×1600 | ≤ 4096×4096 | resize bound |

Reclaim ordering: the Controller's memory reclaim hibernates running browsers
on a Machine before it selects any session. Browser reclaim has its own
pending state: the admission fence stays raised, the stop is retried and
escalated (`SIGTERM`, then `SIGKILL` through the unit), and the step settles
only on verified unit exit followed by a fresh resource sample. A stop that
stalls does not time out into "released": it stays pending, is logged as
stuck, and blocks session selection on that Machine until the unit is gone;
the only exception is an explicit operator emergency action.
`OOMScoreAdjust=1000` is a preference that makes the browser the kernel's
likely first victim inside the shared slice; it is not relied on for
ordering.

Measured on hawk (nixpkgs Chromium 151, Xvnc 1.16, Claude sign-in page
loaded): 570–590 MiB in the cgroup, Xvnc 30 MiB of it, no TCP listeners,
RFB handshake, resize and clipboard forwarding verified over the Unix socket.
The defaults above are provisional until the mixed-load measurement in
[Acceptance](#acceptance-before-any-real-account) exists.

## Viewer (core web)

The viewer is core code in the main bundle, same origin, no iframe: a canvas,
an RFB decoder running in a Worker (the CSP already allows `worker-src 'self'
blob:` and `wss://<host>`), pointer and keyboard translation, and a small
toolbar. Vendoring the noVNC RFB core (MPL-2.0) as a dependency is
acceptable; its application UI, `resizeSession` and clipboard automation are
not used. The viewer always shows the label of the account whose browser it
displays.

- **Resize authority.** The viewer is the only size authority: it measures
  its container in CSS pixels, requests `width = css × scale` and
  `height = css × scale` through `POST .../browser/resize` (debounced,
  bounded), and renders the framebuffer scaled to fit. The server never
  resizes on its own and `AcceptSetDesktopSize` is off, so the noVNC
  CSS-pixel resize race cannot occur.
- **Scale and device handoff.** `scale` is chosen at open (1 on pointer
  devices, 2 on touch devices) and is immutable for the incarnation because
  it is a Chromium launch flag. A device that attaches with a different
  preferred scale sees the framebuffer scaled to fit and an explicit
  "Restart for this screen" action, which hibernates (snapshotting) and
  reopens with the new scale (new incarnation). For a sign-in-bound instance
  the Machine re-navigates to the stored verification URL after the restart;
  the page state before the restart is lost, which the action says.
- **Mobile sheet.** Full-height sheet with the bottom workbench: Keyboard
  (focuses a hidden input with autocorrect, autocapitalize and smart
  punctuation off; composition events are sent as text), Paste (local
  clipboard to remote via `ClientCutText`), Use remote clipboard as code,
  Hibernate, Close. Tap focuses the remote, two-finger drag scrolls, long
  press is right click. The sheet resizes to `visualViewport` so the keyboard
  never covers the focused field. Backgrounding the app stops lease renewal,
  which detaches within 30 s; returning re-attaches to the same instance
  while the grace period keeps it alive.
- **Remote clipboard.** Xvnc forwards only explicit copies (`SendPrimary=0`).
  A received cut text is held in viewer memory for the open attachment and
  shown as "Remote clipboard: n characters". The authorization code field is
  filled only by the explicit "Use as authorization code" action, never
  automatically, never over an edited value; the existing code bounds apply
  (`provider_account::login_code`); the held text is dropped on attachment,
  incarnation or login-request change and when the viewer closes.
- **Passkeys** are not transported by RFB; the sign-in guidance names the
  email or one-time-code path.

Desktop mounts the viewer in the account panel ("Browser") and inside the
sign-in dialog; the cross-device route
`/#subscription-sign-in/<account>/<request>` shows the same viewer, so a
sign-in started on the desktop continues on the phone against the same
running instance.

## Host sandbox policy (outside Cowboy)

Ubuntu restricts unprivileged user namespaces
(`kernel.apparmor_restrict_unprivileged_userns=1`), so neither the launcher's
isolation nor Chromium's sandbox can start from the Machine's user service
without an AppArmor profile that grants `userns`. That profile is host
configuration, kept and receipted in Columbus `machines/ovh`, and it confines
rather than exempts. The sysctl and the ordinary denial for every other
process stay exactly as they are.

- It attaches to the Plugin's launcher executable through a glob over the
  Machine state root and runtime digest directory (derived from
  `runtime.launcher`), and is inherited by every child, including Chromium
  and its nested sandboxes.
- It grants `userns` and, inside the instance's user namespace only, the
  mount and capability operations the launcher needs before it seals; the
  profile never grants anything against host paths.
- It allows read and map of the runtime mount point, read of the scaffolded
  `/etc`, read-write of the profile and instance mount points, `/tmp` and
  `/dev/shm` inside the namespace, and the X and control sockets inside the
  instance directory.
- It denies the rest of the Machine user's home and the Machine state root
  with audit logging, so an attempt is evidence. Because the launcher masks
  those paths inside the namespace and nothing inside can unmask them, the
  deny rule is a second layer and needs no per-account exceptions: the only
  account visible to the browser is the one bound at the profile mount
  point, and the profile cannot tell accounts apart, so it does not try to.
- It denies `listen` and `accept` for `inet` and `inet6` sockets. The
  Machine checks that the host's AppArmor exposes socket-permission
  mediation (`/sys/kernel/security/apparmor/features/network`) and refuses
  admission with `listener_prevention_unavailable` otherwise.

The Machine does not weaken anything when the policy is absent: the launcher
cannot create its namespaces and reports `sandbox_unavailable`. The policy is
validated first on the disposable Ubuntu 26.04 KVM guest already used for OVH
SSH tests (`machines/ovh/tests/openssh_kvm.py` pattern), with synthetic
account directories and canary files, before it is installed on OVH.

## Account isolation (core requirement)

The browsers of different account cards share no state. This is what makes
several accounts on one Machine safe to operate, and it is a requirement of
the design, not a side effect:

| State | Isolation |
|---|---|
| Cookies, local storage, IndexedDB, service workers, HTTP cache, GPU and shader caches, Chromium singleton lock, `Local State` | per-account profile: a private tmpfs during the run, restored from and snapshotted to the account's own directory by the Machine; no other account's data exists in the instance's view |
| `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `TMPDIR`, NSS database | inside the per-run instance directory (tmpfs), destroyed with the unit |
| `/tmp`, X display socket, X lock files | private tmpfs per instance; the X server's abstract socket is disabled; each instance has its own `Xtigervnc`, so X selections, window state and input never cross |
| `/dev/shm`, SysV and POSIX IPC | private tmpfs and private IPC namespace per instance |
| Abstract Unix sockets | none are created: the X server's is disabled and Chromium's singleton uses a pathname socket inside the profile; the two-instance acceptance test checks the abstract namespace |
| Control and RFB sockets | per instance, inside the 0700 instance directory |
| Process tree, memory, CPU accounting | one systemd unit and cgroup per instance; `max_instances` may allow several at once, each in its own namespaces |
| Remote clipboard | held per attachment in the viewer, never shared between accounts, dropped on attachment, incarnation or login-request change; the owner's device clipboard is only ever pasted by an explicit action |
| Runtime files (browser, fonts, policy) | shared read-only artifact; they carry no state |
| Sign-in binding | a sign-in opens only in the browser of the same account as the login, checked under the login lock; the viewer always names the account whose browser it shows |

What stays shared is what is shared for the native CLIs already: the
Machine's egress address, timezone and locale, and the runtime version. The
design does not manufacture per-account device differences; separate,
persistent, honestly distinct profiles are the isolation, and the account
label in the viewer is the guard against the owner signing one account into
another account's browser.

## Security and privacy

- While the owner is attached, Cowboy forwards their keystrokes, pointer
  events and explicitly copied clipboard text to the Machine-local browser,
  and the browser's display back. The Controller and the web client handle
  that transit; they do not persist it, index it, or extract credentials from
  it, and the only thing the viewer examines is the clipboard text the owner
  chooses to use as the authorization code. This is the owner's
  remote-display path to their own browser on their own Machine. The policy
  text in `requirements.md` records it as the owner's accepted interpretation,
  not as established vendor permission.
- Credentials: the native login's OAuth files stay Machine-local and
  native-owned exactly as today. The browser's cookies live in the account's
  profile snapshot on the Machine and in the instance's private tmpfs while
  it runs. No other browser instance can reach them; the trusted Machine
  supervisor and the host administrator can, as they can reach every other
  account file (`requirements.md`, trust boundary). The authorization code
  travels only through the existing owner-submitted path.
- Same-user processes on the Machine (other workers) could reach the RFB
  socket through the host path of the instance directory. The existing
  requirement already states that directory separation is not a security
  boundary against host filesystem access; agent tools execute on their bound
  development host, not the OVH Machine. This is recorded, not claimed
  solved.
- No automation surface exists: no debugging port (and the policy forbids
  enabling one), no WebDriver, no injected code; input is delivered through
  the X server, so pages see ordinary trusted events from a person.

## Test-time prohibitions for the existing accounts

During every test, `Claude · Lynx` (legacy `default`) and `Claude · Lark`
(named) keep `browser_enabled=false`. No step opens, reads, hashes, migrates,
resets or wipes their homes; changes their authentication contract; bypasses
the named-Claude portable-credential fence; enables any credential export;
broadens the AppArmor or userns permissions or relaxes the sysctl; adds
`--no-sandbox`; or exposes a remote-display port to make a test pass.

## Acceptance before any real account

Each row cites the round-1 (R1-n), round-2 (R2-n) or round-3 (R3-n) finding
it closes.

| Evidence | Findings |
|---|---|
| Listener evidence throughout a run: cgroup-subtree socket scan across every network namespace at ready, navigate, resize, reconnect and every 10 s; a short-lived listener attempt from inside the namespace is refused by the host policy, and admission refuses on a host without socket mediation; X auth enforced; private `/tmp`; no abstract X socket; QUIC and non-proxied WebRTC UDP off | R1-1, R2-17, R3-9 |
| Policy text accepted and accurate about transit and about who can read the profile; Controller stream path has no persistence or logging of payloads (code review + test) | R1-2, R2-7, R3-14 |
| Syscall-level canaries from inside a running instance (a conformance helper exec'd in the namespace, not the browser UI): `open`, `openat`, `stat`, `rename` and `mount` against another synthetic account's profile, native credentials and the Machine state root fail; the mount table shows only the bound profile; AppArmor audit shows the denials; `file://` and dialog UI tests are additional, not sufficient | R1-3, R2-1, R3-3 |
| Nested sandbox evidence on pristine Ubuntu 26.04 with the stock sysctl: Chromium's zygote creates its user, PID and network namespaces under the inherited enforcing profile; `uid_map` is the single self-mapping; capability sets of the launcher and all descendants are empty after sealing; a descendant cannot mount, unmask or rebind | R3-1 |
| Two synthetic accounts running at once (`browser.max_instances=2`): no shared path, X display, `/tmp`, `/dev/shm`, IPC object, abstract socket or clipboard between them; a cookie set in one is absent from the other across hibernate and reopen; a sign-in open against the other account's login is refused | core isolation, R3-5 |
| Mount sequence on a pristine host without Chrome or policy directories installed: `/etc` scaffold, policy file bound at the contract's directory, bind-before-mask ordering; effective-policy evidence from the browser (remote debugging flag refused, download blocked, `file://` blocked, QUIC off) in conformance | R2-12, R3-4, R3-7 |
| Stream binding and authorization: token single-use, wrong `ConnectionToken`, wrong incarnation, wrong attachment, revoked owner, expired session or step-up during continuous traffic, revoked device, disabled account, gate closed, Origin mismatch all close both hops; takeover detaches only the old attachment; old-connection rejection after Machine reconnection | R1-4, R2-4, R2-6, R2-9, R3-8 |
| Race and fencing tests: login completes between reservation and unit registration; Machine restart with a live unit (same and different boot) re-verifies confinement before adoption; delayed hibernate/resize/reset/detach for a superseded incarnation, attachment or profile revision; browser OOM during login leaves the native login and generation untouched; cross-account open refused | R1-5, R2-3, R2-4, R2-5 |
| Durable fences: crash injection after the fence is written, after the unit is stopped, and after deletion but before completion, for both reset and revocation; restart resumes the operation, the revision never goes backwards, a delayed reset cannot delete a newly created profile | R2-2, R3-2 |
| Mixed-load measurement on the target Machine with simultaneous worker and browser admissions: byte-weighted reservations never over-admit, exact refunds, memory peak, pressure stall, OOM events, session continuity; reclaim hibernates the browser before any session; a browser stop stalled beyond 60 s keeps the fence and blocks session selection instead of settling | R1-6, R2-11, R3-10 |
| Abnormal-exit cleanup: SIGKILL and OOM of the unit, and a deliberately crashing child, with synthetic secrets in the instance directory; no core dump, no swap (`MemorySwapMax=0`), instance directory removed; the last snapshot survives; profile deleted by reset and by revocation with a running browser | R1-7 |
| Attachment lease: a viewer that disappears without Close is detached within 30 s and the instance hibernates after the grace; unpaired streams time out; stale and post-expiry renewals cannot resurrect an attachment | R1-8, R2-8 |
| Clipboard: PRIMARY copies never arrive; explicit action required; edited code never overwritten; held text cleared on attachment change | R1-9 |
| Resize authority and scale handoff: phone and desktop geometries, DPR 2 on touch, bounded sizes, desktop→phone handoff through "Restart for this screen", pointer mapping verified | R1-10, R2-14 |
| Physical iPhone over the overlay: full sign-in flow including keyboard, IME, paste, Mail switch and return, email-code path, desktop handoff | R1-11 |
| Enablement limited to the test account; authentication contract, worker generation, artifact, host-policy and lifecycle receipts retained; the separate native-owned lifecycle acceptance remains in force | R1-12 |
| Exact signed archive installed through the real Machine installer on pristine Ubuntu 26.04: archive limits, link rejection, relocation, `ldd` audit, `host_abi` check (and refusal on a host that fails it), enforced AppArmor, confinement verification from outside | R2-12, R2-13, R3-13 |
| Storage: a page filling IndexedDB faster than any polling interval hits the tmpfs bound and the Machine's own state writes keep succeeding; snapshot refused at the disk low-watermark; reset works afterwards | R2-10, R3-6 |
| Session restore: reopening a snapshot shows the new-tab page and issues no request to a previously open page (local test server) after clean hibernation and after a crash | R3-11 |
| Plugin lifecycle: upgrade and uninstall with a running instance (drain, pinned bytes, retirement after lease release); rollback to an older build of the same major refuses and reset recovers; a crash between the writer record and readiness leaves a consistent record | R2-15, R2-16, R3-12 |

Only after this matrix passes does the test account (`Claude · Test`) sign in
through the Machine browser on OVH.

## Rollout

1. Core: Plugin kind, Machine instance manager, protocol 43 stream, routes,
   viewer, sign-in hook, settings; gate off. Controller and web release.
2. Plugin: runtime tree built on the builder, conformance on the KVM guest
   with the AppArmor policy, signed and published to the Catalog.
3. Host: AppArmor policy installed on OVH with a receipt.
4. Machine: OVH Machine release through the normal maintenance transaction.
5. Install the Plugin on OVH; enable `accounts.machine_browser`; enable the
   browser for `Claude · Test` only; run the acceptance matrix; sign in.
6. Decide per existing account whether and when to enable the browser.

## Alternatives considered

- m1k1o/neko container: sound architecture (real X server, real browser,
  XTEST input) but Docker and WebRTC/UDP are not admitted on OVH, and its
  default Chromium ships `--no-sandbox` with the warning hidden by policy,
  WebGL disabled, guest mode, session-only cookies and two force-installed
  extensions, all visible browser modifications.
- Exposing an OVH port to the phone: needs Stormbird ACL, DNS and TLS changes
  on a public host; the Machine connection already reaches the Controller.
- DevTools screencast of a headless Chrome: automation signature.
- Browser on hawk with a proxy to OVH egress: egress consistency is not
  guaranteed and it adds a Stormbird dependency; retained as the fallback if
  the OVH policy cannot be installed.
- Changing the CLI environment so it uses a localhost callback: modifies the
  signed authentication section and forces every named account to sign in
  again.
- Per-account AppArmor profiles: host policy cannot follow dynamically created
  account identifiers; the namespace bind gives the same guarantee with one
  static profile.
- Live profile on disk with polling-based size checks: cannot bound growth
  between samples; the tmpfs profile with Machine-written snapshots gives a
  kernel-enforced bound at the cost of losing changes since the last
  snapshot on a crash.
- Freezing the browser and squeezing its cgroup into swap for hibernation:
  more state to reason about than stopping the process, and in conflict with
  `MemorySwapMax=0`; stopping with a persistent snapshot costs a 3–5 s cold
  start, which is acceptable for a sign-in.

## Open questions

- Whether a second display stream for a `Tab` instance should ever be opened
  by an agent (for example, "show me the page") or stays owner-only. The
  first release is owner-only.
- Whether Codex's device-code approval should be offered in the same browser.
  It needs no browser on the Machine; offering it is a UI choice.
