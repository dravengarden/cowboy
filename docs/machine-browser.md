# Machine browser: owner-operated browser on a Machine

Status: design, revision 6, 2026-11-08. Implementation is in progress; the
"Implementation notes" section records where the code refines this text.
Five Astra Max design-review rounds (2026-11-08: 12, 17, 14, 13 and 9 findings)
are folded in and cross-referenced in
[Acceptance](#acceptance-before-any-real-account); the fifth round judged
the remaining medium items resolvable during implementation review behind
the disabled gates, and they are addressed below. The owner decided the
policy boundary the same day; see the
[credential rule](requirements.md#named-provider-accounts-isolation-acceptance)
amendment. The owner also stated strict isolation between the browsers of
different account cards as a core requirement; see
[Account isolation](#account-isolation-core-requirement).
On 2026-11-09 the owner required Provider logins (Claude, Codex, Grok and any
other) to be fully independent of the browser: the sign-in binding was
removed, and a login's verification link may only be shown in the account's
ordinary tab browser as an optional convenience; see
[Login independence](#login-independence).
The same day the owner required every Provider usage card to offer a
browser, including the legacy default cards (Claude · Lynx, DeepSeek · Mole,
OpenAI · Pika, Grok · Fox, ...), each with its own strictly isolated
identity; see [Legacy default cards](#legacy-default-cards).

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
connection, and stopped when idle. A named-account sign-in dialog may offer
to show its verification link in that account's browser; the person completes
the page and copies anything it shows back into the dialog. Its network
identity, timezone, locale and DNS are the Machine's, because the browser is
a process on the Machine.

Non-goals:

- a general remote desktop, or any network listener on the Machine;
- automating the page: no DevTools protocol, no WebDriver, no headless mode,
  no injected scripts, no extensions, no `--enable-automation`,
  no `--no-sandbox`;
- spoofing platform, user agent, GPU or screen facts. A GPU-less Linux VPS
  reports itself as one; the native login already uses that address, and a
  consistent true fingerprint is the safe one;
- any coupling of a Provider login to the browser: no login waits for,
  binds to, reads from or is stopped by a browser, and no browser state
  changes how a login behaves;
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
| Browser runtime bytes: launcher, Xvnc, window manager, Chromium, helpers, fonts, glibc, managed policy file | `machine_browser` Plugin (signed artifact) |
| Launch graph, readiness protocol, host requirements, declared executables and policy contract | Plugin contract (data) |
| Plugin kind, schema, signing, Catalog, install/upgrade/rollback/uninstall, generation leases | Core (generic Plugin lifecycle) |
| Instance manager: admission, start, attachment leases, hibernation, caps, reconciliation, confinement verification, snapshot publication | Core Machine |
| Per-account profile snapshots, reset, revocation wipe, durable fences, staging budget | Core Machine + Controller revocation plan |
| Display stream, backpressure, binding, continuing authorization | Core wire protocol + Machine + Controller |
| Routes, owner/device/Origin checks, per-account enablement | Core Controller |
| Viewer (canvas, decoder, input, clipboard action, mobile sheet) | Core web |
| Optional "open the verification link in the account's browser" entry | Core web (ordinary open and navigate routes only) |
| Resource settings | Core Device/Service configuration |
| Host sandbox policy (AppArmor bootstrap and sealed profiles) | Host configuration (Columbus `machines/ovh`), outside Cowboy |

Without the Plugin installed on a Machine, the core mechanism admits nothing:
the Machine advertises no browser capability and `open` answers
`not_available`. Stop, reset, revocation and reconciliation stay available for
anything that still exists, so disabling admission can never strand a running
unit or a profile.

## `machine_browser` Plugin kind

Release schema 7, payload schema 1, Plugin SDK 1.14+. Catalog readers before
this kind accept release schemas up to 6 and skip these envelopes. The kind is
Machine-installable, Linux-only, and carries exactly one runtime artifact per
declared platform. Like `code_intelligence`, the contract is a closed launch
graph; it has no UI, URLs, credentials or network targets.

```json
{
  "schema_version": 1,
  "id": "machine-browser",
  "version": "1.0.5",
  "display_name": "Machine browser",
  "description": "Owner-operated Chromium on the Machine for account sign-in.",
  "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
  "host_requirements": {
    "kernel_min": "5.15",
    "merged_usr": true,
    "mount_points": ["/opt", "/mnt"],
    "apparmor_features": ["network_v9", "network_v9/af_unix", "mount", "namespaces/userns_create"],
    "built_from": {"id": "ubuntu", "version_id": "26.04"}
  },
  "runtime": {
    "artifact": "runtime",
    "launcher": "usr/libexec/cowboy-browser-launcher",
    "browser": "opt/chromium/chrome",
    "browser_version": "151.0.7922.71",
    "display_server": "usr/bin/Xtigervnc",
    "helpers": ["usr/bin/xrandr", "usr/bin/xkbcomp", "usr/bin/xauth", "usr/bin/matchbox-window-manager"],
    "policy": {"file": "usr/share/cowboy-browser/policy.json", "directory": "/etc/opt/chrome/policies/managed"}
  },
  "launch": {
    "protocol": "cowboy-browser-control-v1",
    "readiness_timeout_ms": 20000
  }
}
```

- The artifact is the instance's whole userland. Its `usr/` and `opt/` are
  bound over `/usr` and `/opt` inside the instance, and it ships glibc and
  the dynamic loader, so the ELF interpreter path `/lib64/ld-linux-x86-64.so.2`
  resolves through the host's merged-usr symlink into the artifact. Nothing
  from the host's userland is used; `LD_LIBRARY_PATH` is not needed; compiled-in
  paths such as `/usr/bin/xkbcomp`, `/usr/share/X11/xkb` and
  `/usr/share/fonts` resolve inside the artifact. The spike on hawk
  (2026-11-08) confirmed the opposite approach fails: Ubuntu's `Xtigervnc`
  finds `xkbcomp` only at its compiled path, and `LD_LIBRARY_PATH` cannot
  reach that.
- `host_requirements` are what the host must still provide: the kernel,
  merged-usr symlinks (`/lib64`, `/lib`, `/bin` → `usr/…`), the `/opt` and
  `/mnt` mount points, and an AppArmor with socket-permission, `unix`
  address, mount and user-namespace mediation. The Machine advertises the
  browser capability only when all of them hold; otherwise the reason is
  `host_requirements`. `built_from` is provenance, not a host requirement.
- Every `runtime.*` path is a normalized relative path (no absolute path, no
  `.` or `..` components, no symlink) that the SDK resolves inside the named
  artifact at validation time, for every declared platform, exactly as the
  code-intelligence validator checks artifact correspondence and platform
  coverage. The Machine recomputes the same checks against the installed,
  content-addressed generation before every launch; a missing or non-regular
  executable fails admission, never falls back to an ambient binary.
- `runtime.policy.directory` is the browser build's managed-policy directory
  (`/etc/opt/chrome/policies/managed` for Google Chrome,
  `/etc/chromium/policies/managed` for Chromium builds). It is validated
  against the build at packaging time and the launcher binds
  `runtime.policy.file` there; the pair is a signed binding like the
  executables, not a documentation hint.
- `browser_version` is the full writer version used by the
  [profile compatibility fence](#profile-compatibility-and-rollback).
- The Machine passes no shell or template arguments. The launcher receives a
  fixed argument vector: `--control-socket-dir`, `--snapshot-dir`
  (read-only source, may be absent), `--staging-dir`, `--runtime-dir`,
  `--display`, `--geometry WxH`, `--scale N`, `--profile-bytes N`,
  `--instance-bytes N`. Everything else is in the launcher.
- The runtime artifact is a relocatable, symlink-free, hard-link-free tree
  within the existing limits (1 GiB compressed, 4 GiB expanded, 100 000
  entries; symbolic and hard links are rejected by the installer; the
  builder replaces package symlinks with copies). It is assembled on a
  builder from the `built_from` distribution's packages by resolving the
  complete dependency closure of the browser, `Xtigervnc`, the window
  manager, the helpers, XKB data, fonts and glibc, and excluding only
  packages that are not runtime files. The builder audits every ELF object
  with `ldd` against the tree alone and refuses a missing dependency. The
  `chrome-sandbox` setuid helper and the Qt theme shims are removed; Chromium
  uses its namespace sandbox. Measured spike tree: 967 MiB uncompressed
  before symlink replacement.
- Conformance runs the exact signed archive through the real Machine
  installer on a pristine Ubuntu 26.04 guest with the production AppArmor
  policy enforced and the stock user-namespace restriction intact, then
  opens, attaches, resizes, copies, snapshots and hibernates, and runs the
  probes described under [Acceptance](#acceptance-before-any-real-account).
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
link. The drain stops every instance gracefully with reason `reclaim`,
refuses new opens until the uninstall finishes, and proceeds only once every
unit's exit is confirmed; otherwise the uninstall step stays pending
(`slot_fenced`) and nothing is unlinked. The drain runs only for a step the
journal would execute at that moment (owner, admission, execution lease,
installation CAS, slot fences and the exact installed target all hold), and
the lease is re-checked before every individual stop, after that stop's own
waits (a running copy, the account lifecycle lock) and immediately before it
changes the instance's phase, so a stale or orphaned uninstall stops no
browser. An incomplete restart inventory also keeps the drain pending. It needs no journal entry of its own: it changes
no Plugin state, a repeated drain stops only what still runs, and a
hibernated browser reopens. Starts and drains share one gate: a drain refuses
new starts at once and waits until every start already admitted (including
one still waiting for memory, which gives up) has settled into a registered
instance, so no open can launch a unit after the drain completed. Generation retirement treats browser leases like Code leases: a
generation retires only after its last browser lease releases. The lease is
the `generation_digest` the instance launched from (recorded in its instance
record): registered instances (starting, running, stopping, adopted
survivors), opens that resolved the runtime but have not registered yet, and
units retained until their exit is confirmed all pin their generation. An
installed browser Plugin also keeps its active and rollback (retained
previous) generations; an uninstalled one keeps only pinned generations, and
its dangling rollback link is removed. Retirement asks the registered browser
manager for its pins and runs under the manager's pin lock; an open pins its
generation under that lock only while the launcher still exists, so a start
never launches from a retired generation (it answers `not_available`). Pins
are unknown, and nothing is retired, until the restart inventory completed
and while a unit of unknown generation may still run. The Controller holds no
references for browser generations: the Machine retires them during the
periodic uninstalled-generation sweep (and on a per-Plugin retention request
naming the browser Plugin), skipping a Plugin whose uninstall may still
compensate. After the
Plugin is uninstalled, profile snapshots remain account data; reset and
revocation still delete them.

### Launcher contract

The launcher is a small static binary in the artifact. It is the supervisor
of one instance: it builds the isolation, starts the three display and
browser processes under a sealed policy, serves the control socket, performs
snapshot copies on the Machine's request, and reaps everything as PID 1 of
the instance. It runs under the host's `bootstrap` AppArmor profile; the
processes it starts run under the `sealed` profile (see
[host policy](#host-sandbox-policy-outside-cowboy)). Every step is fixed;
nothing is configurable from outside beyond the argument vector.

1. **Namespaces.** `unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID |
   CLONE_NEWIPC)`; `setgroups` is denied before the single self-mapping of
   the Machine user's uid and gid is written, so no process inside is root
   in any sense. The launcher forks once so that it is PID 1 of the new PID
   namespace and owns reaping. Mount propagation is made recursively private
   before any mount. Processes of other instances are invisible and cannot
   be signalled from inside; the Machine still sees real pids through the
   unit's cgroup.
2. **Private mounts, in this order.** Mount a private tmpfs over `/mnt` and
   create the alias directories in it. Bind the runtime tree read-only,
   `nosuid`, `nodev` at `/mnt/runtime`, then bind its `usr/` over `/usr` and
   `opt/` over `/opt` (read-only, `nosuid`, `nodev`): from here the instance's
   userland is the artifact. Mount a tmpfs of `--profile-bytes` at
   `/mnt/profile` and a tmpfs of `--instance-bytes` at `/mnt/instance`
   (`HOME`, `XDG_*`, `TMPDIR`, logs); bind the unit's `RuntimeDirectory`,
   which holds only the control and RFB sockets, at `/mnt/sockets`. Before
   anything is masked, open `O_PATH | O_DIRECTORY` descriptors on the
   account's `browser/staging` directory and, if `--snapshot-dir` is given,
   on that snapshot generation; later binds use these descriptors as their
   source (`open_tree` on the descriptor, then `move_mount` onto the alias),
   so the source stays reachable after its pathname is masked, and AppArmor
   mediates the resolved source path against the bootstrap profile's exact
   patterns. If a snapshot was given, bind it read-only at `/mnt/snapshot`,
   restore it into `/mnt/profile` with the no-follow copier described under
   [snapshots](#profile-storage-tmpfs-and-published-snapshots) (the pages are
   allocated by the launcher inside the unit's cgroup, so they are charged to
   the unit), then unmount `/mnt/snapshot` and close its descriptor. Mount a
   private tmpfs over `/tmp` (256 MiB) and over `/dev/shm` (`--profile-bytes`);
   mount a fresh `proc` for the new PID namespace. Build `/etc` as a private
   tmpfs scaffold containing: bind mounts of exactly `hosts`, `localtime`,
   `passwd`, `group`, `machine-id` and `ssl/` from the host; a generated
   `resolv.conf` with the host's nameservers plus `options use-vc` (every DNS
   query goes over TCP, see the sealed profile); the artifact's own
   `nsswitch.conf` (`hosts: files dns`, matching the shipped NSS modules
   exactly); and the policy file at `runtime.policy.directory`. Nothing has
   to exist on a pristine host and nothing else from the host's `/etc` is
   visible. Then mask `/home`, the Machine user's `/run/user/<uid>` and, if
   it lies elsewhere, the Machine state root with empty tmpfs. Every source
   was bound before its ancestor was masked, so the bind mounts keep their
   objects; afterwards no unselected account directory, no other instance's
   `RuntimeDirectory`, no native credential and no Machine state is reachable
   by any path, in this namespace or in any namespace a descendant could
   create.
3. **Seal the children, not the mounts.** For each child the launcher
   forks, closes every descriptor except the ones the child needs, `chdir`s
   into `/mnt/instance`, writes itself into the child's sub-cgroup
   (`display/` or `browser/`), performs the authorized **immediate**
   AppArmor transition to the `sealed` profile (`aa_change_profile`, which
   the bootstrap profile permits; this is not an on-exec transition), clears
   the bounding set while it still holds `CAP_SETPCAP`, clears the ambient,
   inheritable, permitted and effective sets, and only then sets
   `no_new_privs` and execs, so the sealed label is already in place when
   `no_new_privs` starts restricting transitions. The sealed profile has no
   mount, pivot, umount or remount permission, so a descendant that creates
   its own user and mount namespace gains nothing it can bind: the kernel
   still asks the profile. Chromium's zygote keeps working: creating a user
   namespace needs no capability, the sealed profile grants `userns` and the
   namespaced `chroot` and `/proc` reads its sandbox uses, and the
   capabilities it gains apply only inside the nested namespace.
   The launcher's namespace init keeps its capabilities in the instance's
   user namespace under the `bootstrap` profile; it needs them to mount and
   unmount the snapshot and staging aliases and to make the socket export
   read-only. It talks to the children only through the X protocol and the
   fixed control protocol, never executes anything they write, and is itself
   validated by the Machine (see
   [confinement verification](#confinement-verification)).
4. **Display.** Write an X authorization cookie into `/mnt/instance` and
   start `Xtigervnc` on a fixed display number with `-auth <cookie>
   -nolisten tcp -nolisten local -rfbport -1 -rfbunixpath /mnt/sockets/rfb.sock
   -rfbunixmode 0600 -SecurityTypes None -AcceptSetDesktopSize=0
   -SendPrimary=0 -SendCutText=1 -AcceptCutText=1 -AcceptTouchEvents=1
   -geometry WxH -depth 24` (the last flag exists only in the runtime's
   patched build; see [Native touch](#native-touch)).
   `-nolisten local` disables the X server's abstract socket; the only X
   endpoint is the pathname socket in the private `/tmp`; collisions are
   impossible because `/tmp` and the abstract address space are both
   unavailable to other instances (private `/tmp`, abstract addresses denied
   by the sealed profile). As soon as the RFB socket exists, the launcher
   remounts `/mnt/sockets` read-only (bind remount in the instance's mount
   namespace): the listening socket keeps accepting connections, but no
   sealed process can unlink it, recreate it or place a regular file on the
   host's `/run/user` storage. The RFB socket is reachable from outside the
   namespace only through the unit's `RuntimeDirectory`, which is 0700.
5. **Window manager** keeps the single browser window fullscreen and
   following the screen size.
6. **Browser.** Environment: `PATH=/usr/bin`, `FONTCONFIG_FILE` (generated
   into `/mnt/instance`, pointing at `/usr/share/fonts`),
   `GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas`, `DISPLAY`,
   `XAUTHORITY`, `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
   `XDG_RUNTIME_DIR` and `TMPDIR` inside `/mnt/instance`, `TZ` from the host,
   `LANG=en_US.UTF-8`, `SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt`.
   No proxy variable is inherited or set. Chromium starts with
   `--user-data-dir=/mnt/profile --no-first-run --no-default-browser-check
   --disable-sync --disable-background-networking --disable-component-update
   --disable-extensions --disable-breakpad --disable-crash-reporter
   --disable-quic --disk-cache-size=1048576 --password-store=basic
   --lang=en-US --window-position=0,0 --window-size=WxH
   --force-device-scale-factor=<scale> chrome://newtab`.
7. **Managed policy.** The policy file is part of the signed artifact and
   turns off everything a sign-in browser must not do, enforced by the
   browser inside the isolated view: `RemoteDebuggingAllowed=false`,
   `DeveloperToolsAvailability=2`, `DownloadRestrictions=3`,
   `AllowFileSelectionDialogs=false`,
   `URLBlocklist=["file://*","chrome://*"]` with `URLAllowlist` for
   `chrome://newtab` (and the `new-tab-page` hosts it renders), `ExtensionInstallBlocklist=["*"]`, `BrowserSignin=0`,
   `SyncDisabled=true`, `PasswordManagerEnabled=false`,
   `AutofillAddressEnabled=false`, `AutofillCreditCardEnabled=false`,
   `BackgroundModeEnabled=false`, `MetricsReportingEnabled=false`,
   `DefaultBrowserSettingEnabled=false`, `PromotionalTabsEnabled=false`,
   `QuicAllowed=false`, `WebRtcIPHandling=disable_non_proxied_udp`,
   `BuiltInDnsClientEnabled=false`, `DnsOverHttpsMode=off`,
   `DefaultGeolocationSetting=2`, `DefaultNotificationsSetting=2`,
   `AudioCaptureAllowed=false`, `VideoCaptureAllowed=false`,
   `RestoreOnStartup=5`. The browser starts on Chromium's own new-tab page,
   whose `NTPShortcuts` tiles are the sign-in pages: Claude, ChatGPT · Codex,
   Grok, DeepSeek and Google sign-in. Shortcuts are browser UI only; no page
   can read them, and opening one is an ordinary typed-address navigation.
   With the built-in resolver off, Chromium resolves
   names through the artifact's glibc, which the scaffolded `resolv.conf`
   forces onto TCP, so the browser needs no UDP socket at all and the sealed
   profile denies them. Session restoration is off on purpose: a reopened
   profile starts on the new-tab page and never resumes old authenticated
   tabs or challenge URLs before the Machine's own verification and
   navigation; cookies persist through the profile snapshot, not through
   session restore. Some of these settings are observable by pages as
   ordinary denied permissions; they are choices any user could make.
8. **Control socket** (`cowboy-browser-control-v1`, newline-delimited JSON
   on `/mnt/sockets/control.sock`): the launcher sends `ready` only after all
   three processes run, carrying the pids and start times (host view, read
   through the unit's cgroup by the Machine) of itself, the display server,
   the window manager and the browser, its `/proc/self/ns/{user,mnt,pid,ipc}`
   identities, its `uid_map`/`gid_map`, both profile labels, and the mount
   table it built. The Machine verifies the same facts from outside before
   accepting readiness. Commands: `navigate {url}` (re-invokes Chromium
   against the running profile; the singleton socket in the profile hands the
   URL to the existing instance; no debugging port), `resize {width, height}`
   (through `xrandr`, bounded by the launcher and by the Machine to
   320×480…`browser.max_width`×`browser.max_height`; `scale` is fixed for
   the life of the incarnation), `attach {attachment_id}` and
   `detach {attachment_id}` (informational), `snapshot {generation}`
   (described below), `status` (the readiness facts again, for adoption
   after a Machine restart) and `stop {graceful_ms, snapshot_generation?}`:
   the optional generation asks for the stop-time copy of the profile into
   that staging generation after the browser processes have exited, so the
   primary snapshot path needs no second round trip.
9. **Supervisor and PID 1.** The process systemd starts is a thin outer
   supervisor: it stays the unit's `MainPID`, forwards `SIGTERM` to the
   namespace init, waits for it and exits with its status. The namespace init
   (PID 1 inside) owns the mounts, the control socket, the children and the
   snapshot copies, and it stays alive through every copy and publication
   until `stop` completes or the unit's deadline kills it;
   `KillMode=control-group` ends anything left. A graceful `stop` sends
   `SIGTERM` to the **browser main process only**, waits (bounded by the
   unit's stop timeout) for every process in `browser/` to exit so that
   Chromium's storage services have flushed, copies the profile, stops the
   display, then exits. An interrupted shutdown is classified as
   crash-consistent or unsaved, never as flushed.

## Instance manager (core Machine)

### Identity and placement

```text
BrowserInstance = Machine + Plugin generation digest + account_id + instance_id + incarnation
Attachment      = BrowserInstance + attachment_id (one live attachment per instance)
Snapshot        = account_id + snapshot generation (+ profile_revision, writer version)
```

One instance per `(Machine, account)`; `browser.max_instances` (default 1)
bounds concurrently running instances per Machine. Each instance runs in its
own user, mount, PID and IPC namespaces, its own X server, its own `/tmp`,
`/dev/shm`, profile and instance tmpfs, and its own systemd unit and cgroup
with two sub-cgroups, `display/` and `browser/`, created by the launcher so
the Machine can freeze the browser alone.

### Profile storage: tmpfs and published snapshots

The live profile never touches the shared filesystem. During a run it lives
in the instance's size-bounded private tmpfs (`browser.profile_max_bytes`),
so the kernel enforces the storage bound and a page that fills browser
storage gets `ENOSPC` inside the browser instead of filling the volume that
holds Machine state and native credentials. Tmpfs pages are charged to the
cgroup that allocates them: the launcher restores the snapshot from inside
the unit, and Chromium writes from inside the unit, so every profile,
`/tmp` and `/dev/shm` page counts against the unit's `MemoryMax` and never
swaps. With swap disabled these pages compete with Chromium's own memory, so
the unit's OOM kill can precede tmpfs `ENOSPC`; the defaults below leave
headroom for a full profile plus the measured browser footprint and are
confirmed by the restore and full-profile measurements in
[Acceptance](#acceptance-before-any-real-account). An open admitted on a
tight Device gets a smaller `MemoryMax` (down to `browser.memory_reserve`,
see [admission](#lifecycle)); such a browser has less room for a full
profile and is OOM-killed inside its own unit sooner, losing at most the
state since its last published generation.

Persistence is a **published snapshot** under
`provider-auth/accounts/<uuid>/browser/`:

```text
accounts/<uuid>/
  browser-state.json    control record, outside the deleted tree: profile_revision,
                        writer browser_version, generation digest, current snapshot
                        generation, pending reset/revocation fence and operation id
  browser/
    snapshots/gen-<n>/  immutable, complete snapshot generations
    staging/gen-<n>/    being written; never read as a snapshot
```

`browser-state.json` is always replaced atomically: write
`browser-state.json.tmp`, fsync it, rename over the old file, fsync the
account directory. Reset and revocation delete only `browser/`; the control
record survives them, like the native wipe keeps its fences and tombstones.

A snapshot is taken only at a **coherent barrier**:

- at every graceful stop (hibernate, deadline, reclaim, uninstall drain):
  the launcher sends `SIGTERM` to the browser main process, waits for every
  process in the `browser/` sub-cgroup to exit (bounded by the unit's stop
  timeout) so that Chromium's storage services have flushed and closed their
  databases, and only then copies the tmpfs; this is the primary path and the
  only one that captures in-memory browser state. If the wait expires, the
  remaining processes are killed and the copy is classified as
  crash-consistent, not flushed;
- optionally while attached, every `browser.snapshot_interval`
  (default 10 min) and on an explicit `BrowserSnapshot`: the Machine freezes
  the `browser/` sub-cgroup (`cgroup.freeze`), waits for `cgroup.events` to
  report `frozen 1`, and only then asks the launcher to copy; it thaws on
  every normal, error and cancellation path. A frozen copy is
  crash-consistent (SQLite WAL and journal files and LevelDB manifests and
  logs are copied together and recover exactly as after a crash); it does
  not include state Chromium has not yet written, and the loss window is
  since the last **published** generation, not a fixed interval, because
  periodic snapshots run only while attached and may be refused or fail.
  Every Chromium process, including navigation helpers, is placed in
  `browser/` before it executes and cannot leave it (the sealed profile has
  no write access to the cgroup tree), so freezing the sub-cgroup stops every
  profile writer.

Freezer recovery: if the Machine restarts while `browser/` is frozen, the
instance is adopted only after the Machine resolves the interrupted
snapshot (the launcher reports whether a copy is in progress; an in-progress
copy is allowed to finish or is cancelled) and thaws the sub-cgroup; a
staging directory a live launcher may still be writing is never deleted.

The copy itself is done by the launcher, inside the unit: the Machine
reserves the staging budget (below), creates `staging/gen-<n>` and the
launcher binds it at `/mnt/publish` through the staging descriptor it
retained before masking, for the duration of the copy only, then unmounts
it. The copier opens every entry descriptor-relative with
`O_NOFOLLOW`, copies only regular files and directories (sockets, FIFOs,
symlinks, the singleton link and lock files, and the cache directories are
skipped by name), preserves holes with `SEEK_HOLE`, and stops at the
reserved logical-byte, entry-count and depth limits. The Machine then
validates the staging tree with the same rules before publication:
regular files and directories only, bounded bytes, entries and depth, no
links. It treats the tree as untrusted input and never follows anything
into the host filesystem. Publication is: fsync every copied file and
directory, fsync `staging/`, rename `staging/gen-<n>` to
`snapshots/gen-<n>`, fsync `snapshots/`, publish `browser-state.json`
atomically with the new current generation, then remove older generations
beyond the retained one. A generation that exists under `snapshots/` but is
not named by `browser-state.json` is an orphan from a crash between the
rename and the pointer update; reconciliation at Machine start and before
every admission removes orphans and abandoned staging, using the state file
as the only authority. Restoration reads a snapshot generation only through
the read-only alias and with the same no-follow copier.

Staging budget: the Machine keeps one byte and inode reservation ledger for
all browser staging on the Machine, admits a snapshot only when the
reservation fits under the Device disk low-watermark with the previous
generation retained, allows at most one staging and one published
generation per account, and reconciles abandoned staging directories at
start and before every new admission. Publication reports how far it got: a
failure before the rename removes the staging generation before its budget is
released; a failure after the rename (the directory or record fsync, or the
record write) reconciles the renamed generation against the durable state
file first (it is published exactly when the file names it, otherwise it is
an orphan and is removed), and keeps the budget reserved, retried before the
account's next admission, while that reconciliation fails. A prune of the
superseded generation that fails leaves the publication standing. A snapshot that cannot be admitted is
refused, never queued.

Periodic and explicit snapshots run as a cancellable copy job: the account
lifecycle lock is held only to admit the generation and to publish it, so
supervision (leases, socket scans, deadlines) and resets or revocations
never wait for a copy. Every stop cancels a running job; a graceful stop
first lets it finish while still enforcing the deadline and the confinement
checks, and becomes a forced stop if one of them fails meanwhile. A staging generation stays reserved in the ledger as
long as a copier may write it: a copy the launcher refused or finished is
removed (scoped, no-follow) before its budget is released; a cancelled or
timed-out copy keeps its reservation until the unit's exit is confirmed. A
removal that fails keeps the budget reserved and is retried before the
account's next admission. A copy binds `/mnt/publish` in the instance's mount
namespace, which [confinement verification](#confinement-verification)
rejects; so no copy runs while a navigation verifies and navigates: new copies
are refused while it does, and a running copy is awaited within its own bound
(the browser is frozen meanwhile) and cancelled if it overruns. The verifier
stays strict instead of tolerating a transient publish mount.

Snapshot failure is subordinate to mandatory stops: a graceful stop attempts
the snapshot at most twice within the stop deadline; a stop forced by a
confinement failure, deadline, reclaim, revocation or `RuntimeMaxSec` proceeds
without a snapshot, keeps the last published generation, and reports
`unsaved_state_lost`. An explicit `hibernate` whose snapshot fails reports
the failure rather than success. Pending persistence never extends an
instance's authority to run.

### Durable placement, reset and revocation

The Controller keeps a durable browser placement
`(owner, account, machine, profile_revision, last_open)`. It is committed
before the open is dispatched, in one transaction under the owner lock that
also re-checks that the account exists, is available and has its browser
enabled (for a [legacy default card](#legacy-default-cards): that the key is
the owner's key of that card and the card's enablement row exists); a failed
write fails the open. The account revocation plan gains
`browsers` beside `logins`, `grants` and `sessions`, captured from these
placements under the same lock, so a `Tab`-only placement is revoked like any
other target: a revocation that begins after the commit captures the
placement, and one that began before makes the commit refuse the open. The
placement is kept whatever the open's outcome (an unanswered or timed-out
open may still have started a browser); a new one carries profile revision 0
until the Machine reports its revision. Only recorded revocation evidence for
that Machine removes it. Within the Controller, an account's open dispatch
and its revocation dispatch share one lifecycle lock, so a `revoke_browser`
is never sent ahead of an open committed before the revocation. If the
account's browser was disabled while the open was in flight, the Controller
hibernates the instance the open reported and fails the open.

On the Machine, lifecycle mutations and snapshots of one account are
serialized by one per-account lifecycle lock. `revoke_browser` and
`reset_browser` share one path, in this order: take the lock; publish
`browser-state.json` with the operation fence and the next
`profile_revision` (an admission fence that survives a crash); cancel any
in-flight snapshot (the launcher's copy is interrupted and its staging
directory abandoned, or the unit is killed, which has the same effect);
stop the running unit and wait for its cgroup to be empty, escalating
`SIGTERM` to `SIGKILL` on the unit's own timeout; delete
`accounts/<uuid>/browser` (snapshots and staging) with a scoped, no-follow
removal while `browser-state.json` stays in place; publish
`browser-state.json` again with the completion, which also clears the
recorded writer `browser_version` and generation digest (the version fence
guarded the deleted data). If systemd's stop and `SIGKILL` escalation do not
collect the unit, the Machine writes `cgroup.kill` and waits for the cgroup to
empty; when even that cannot be confirmed the instance stays registered, the
profile is not deleted, the operation answers `failed` with the fence still
raised, and a background retry repeats the termination and completes the
fenced operation once the exit is confirmed. Every `systemctl` and
`systemd-run` call is bounded (killed after 10 s) and the systemd phase of a
termination as a whole is bounded, so a stalled user manager reaches the
`cgroup.kill` fallback on the retained cgroup path instead of blocking the
account's command queue. A termination is bound to the systemd invocation ID
it stopped: a unit of the same name with another invocation is never stopped
or killed and counts as the original's exit. When a retry confirms the exit,
the Machine reports a terminal `hibernated` observation for the retained
incarnation (its stop reason and `unsaved_state_lost`), then completes each
fenced operation the unit blocked and reports the completion as the account's
`hibernated` observation with the operation's reason (`owner_request` for a
reset, `revoked` for a revocation) and the advanced `profile_revision`; a
reset's earlier `failed` receipt is replaced by that completion. A snapshot job publishes only if the
`(incarnation, profile_revision)` it captured still match under the lock;
otherwise its staging is discarded. A restart in the middle resumes the
fenced operation from `state.json` and preserves its receipt; a delayed
reset or revocation carrying an older revision is a no-op. Native tombstones
and every other file under the account are untouched. Revocation completion
is not recorded while a browser placement is still pending.

### Lifecycle

```text
hibernated ──open──▶ starting ──ready──▶ detached ◀──attach/detach──▶ attached
     ▲                   │ failure                 │ grace elapsed, hibernate, deadline
     └───────────────────┴──────────── stopping ◀──┘
```

- `hibernated`: no process, no memory, published snapshot on disk. This is
  what the owner's "tab" shows when nothing is running.
- `open` admits only when the Plugin is installed and its executables, policy
  binding and `host_requirements` verify, the host policy is present, the
  account is enabled for the browser and not fenced, `browser.max_instances`
  is not reached, and the recorded writer version is not newer than the
  runtime's. The `max_instances` slot is reserved atomically before any
  asynchronous step (memory wait, unit start) and released only when the
  unit's exit is confirmed. Memory admission goes through the same launch
  ledger as cold workers (a byte-weighted reservation, checked and taken in
  one step under the ledger lock, with exact refunds and the existing floor
  preserved), so a browser and a worker can never both count the same
  headroom. An open is admitted with a **memory budget**: the largest
  amount up to `browser.memory_max` that the Device headroom (the tightest
  of host `MemAvailable` and every finite cgroup limit from
  `cowboy-agents.slice` up, less usage net of `inactive_file`, and zero
  while a stalled scope has less than a quarter of its limit left) covers
  beside every outstanding reservation and the 512 MiB floor, rounded down
  to whole MiB. The open is refused (`Reason::Memory`) when not even
  `browser.memory_reserve` (default 768 MiB, about the resident working
  set; a live Chromium, Xvnc and window manager measure about 640 MiB)
  fits within a 15 s wait; the refusal logs the measured headroom, the
  outstanding reservations, the requested minimum and maximum and the floor
  at INFO. `active_file` is not credited: it is recently referenced cache
  whose reclaim would refault. The budget becomes the unit's `MemoryMax`,
  and its `MemoryHigh` is the lesser of `browser.memory_high` and the
  budget less an eighth (at least 64 MiB); with ample headroom these are
  the configured 2 GiB and 1.5 GiB. The browser therefore can never take
  shared memory beyond what admission granted: growth past its budget is
  throttled and then OOM-killed inside its own cgroup, before any shared
  ancestor's `memory.high` throttles a session, whether or not
  `sessions.memory_reclaim` is enabled. `OOMScoreAdjust=1000` additionally
  makes it the first victim if a shared ancestor still runs out. The
  budget is held with the instance's `max_instances` slot for the
  instance's whole life and released with it, exactly once, only when the
  unit's exit is confirmed (directly, by a termination retry, or with a
  failed start's retained slot). Once the unit's cgroup is known the
  reservation is charged to it: the ledger then counts only the part of the
  budget the unit has not used yet (budget less its `memory.current` net
  of `inactive_file`), because the used part is already deducted from every
  headroom sample. A running unit's `MemoryMax` is never raised; a larger
  budget needs a new open. The schema rejects
  `memory_reserve > memory_high` and `memory_high > memory_max`.
- The unit is `systemd-run --user --collect --unit cowboy-browser-<instance>
  --slice cowboy-agents.slice -p Delegate=yes -p KillMode=control-group
  -p MemoryMax=<budget> -p MemoryHigh=<min(browser.memory_high, budget - max(budget/8, 64 MiB))>
  -p MemorySwapMax=0 -p CPUQuota=<browser.cpu_quota> -p TasksMax=<browser.tasks_max>
  -p OOMScoreAdjust=1000 -p LimitCORE=0 -p RuntimeMaxSec=<browser.max_runtime>
  -p TimeoutStopSec=20 -p RuntimeDirectory=cowboy-browser/<instance>
  -p RuntimeDirectoryMode=0700`. The `RuntimeDirectory` holds only the two
  sockets; it is not a storage location, and the Machine stops the instance
  if anything else appears in it.
- `ready` is accepted only after [confinement verification](#confinement-verification).
- `attached` requires a live attachment lease. `detached` starts the
  `browser.hibernate_after` grace (default 120 s); expiry hibernates
  (graceful stop with snapshot). The grace stop waits for a running copy and
  the account lifecycle lock, so it re-checks, under the instance's state
  lock and atomically with the change to `stopping`, that the instance is
  still `detached` in the same detached interval and that interval still
  exceeds the grace; an attach meanwhile abandons the idle stop. The
  deadline is not idleness and stops unconditionally.
  Lease expiry detaches only after
  re-checking, under the instance's state lock, that the instance is still
  `attached` with the same attachment still expired: a stop, renewal or
  takeover that happened meanwhile wins, and `stopping` never returns to
  `detached`. An instance's deadline is `browser.max_runtime` from its
  start; nothing outside the browser shortens or extends it.
- Every mutation (`hibernate`, `resize`, `navigate`, `snapshot`, `stop`)
  carries the expected incarnation; attach, detach, resize and stream cleanup
  carry the `attachment_id`; reset carries the expected `profile_revision`.
  A command whose expectation no longer matches is a recorded no-op with
  reason `superseded`. Replays of the same request ID return the original
  receipt. An attach re-checks the instance's phase where it installs the
  attachment, after its asynchronous resize: a stop (hibernate, reset,
  revocation) that began meanwhile wins, the new stream is torn down and the
  attach answers `busy` with the stopping state. A resize re-checks its
  attachment once it holds the control connection (it may have waited behind
  a snapshot or navigation): unless that attachment is still the current,
  unexpired one of an attached instance, it answers `superseded` and leaves
  the geometry to the viewer that took over.
- Failure never retries with weaker isolation. A namespace or sandbox failure
  reports `sandbox_unavailable` with the host policy pointer; a confinement
  verification failure reports `confinement_unverified`; a missing host
  prerequisite reports `host_requirements`. All leave the instance
  hibernated with the reservation refunded. A failed start's unit is
  terminated with the same confirmation as any stop; an unconfirmed exit
  keeps the instance slot reserved and the account's fences raised, and
  refuses (`busy`) every open of that account, or of any account for a unit
  of unknown ownership, or with the same unit name, until the exit is
  confirmed.

### Confinement verification

Before accepting `ready`, before every `navigate`, and before adopting any
surviving unit at restart, the Machine pins each reported pid to its reported
start time and reads from outside the namespace:

- the unit's root cgroup holds exactly the launcher's supervision tree: the
  supervisor (the unit's `MainPID`, single-entry `NSpid`), the namespace
  relay (its child, single-entry `NSpid`, in the instance's user, mount and
  IPC namespaces but its parent's pid namespace, with `pid_for_children`
  equal to the instance's), and the init (the relay's child, `NSpid` ending
  in 1); any other root-cgroup process must be a child of the init inside
  the instance pid namespace (the snapshot copier) under the bootstrap
  profile;
- `/proc/<pid>/ns/user`, `ns/mnt`, `ns/pid` and `ns/ipc` of the launcher,
  display server, window manager and browser: identical to the identities
  the launcher reported and different from the Machine's own; `uid_map` and
  `gid_map` of that **outer instance namespace** contain exactly the single
  self-mapping;
- `/proc/<pid>/attr/current`: the launcher under the `bootstrap` profile,
  the display server, window manager and browser under the `sealed` profile,
  all in enforce mode;
- `/proc/<pid>/status` of the display server, window manager and browser
  main process: `NoNewPrivs` set and every capability set empty. Chromium's
  sandboxed descendants are identified by namespace ancestry: their `NSpid`
  carries a pid in the instance's PID namespace (they live inside it or in a
  namespace nested in it) and they sit in the unit's `browser/` sub-cgroup,
  which the sealed profile cannot leave; they are checked for `NoNewPrivs`
  and for the sealed label, not for empty capability bitmaps, because a
  nested user namespace legitimately grants capabilities that apply only
  inside it;
- `/proc/<pid>/mountinfo` of the browser: `/usr` and `/opt` backed by the
  installed generation, read-only, `nosuid`, `nodev`; `/mnt/profile` and
  `/mnt/instance` tmpfs of the expected sizes; `/mnt/sockets` backed by the
  unit's `RuntimeDirectory` (matched by mount source and root, major:minor
  and inode of the backing directory); `/tmp`, `/dev/shm`, `/etc`, `/home`
  and `/run/user/<uid>` tmpfs with the expected flags; `/home`,
  `/run/user/<uid>` and the Machine state root (as the kernel resolves it)
  each covered by an empty `ro,nosuid,nodev,noexec` 4 KiB mask with nothing
  but masks mounted below; `/mnt/snapshot` and `/mnt/publish` absent;
  `<policy-directory>/policy.json` bound read-only from exactly the
  artifact's policy file (same device and mount-root path, not merely the
  same device);
- the whole cgroup subtree of the unit (`cgroup.procs` recursively, because
  `Delegate=yes` lets children create sub-cgroups): no process owns a
  listening TCP socket or **any** UDP socket, matched by socket inode
  against `/proc/<pid>/net/{tcp,tcp6,udp,udp6}` of each distinct network
  namespace found among those processes, and no process owns a reachable
  abstract Unix socket (`/proc/<pid>/net/unix` entries with an `@` path in
  any state but connected: Chromium's `SO_PASSCRED` socketpairs are
  autobound to abstract names by the kernel and stay connected, refusing
  every third party). The scan fails closed: only a process whose `/proc`
  entry vanished is skipped; unreadable descriptors or namespace links of a
  live process, or a namespace whose tables no live member can read, are a
  violation;
- `/proc/<pid>/mountinfo` also shows `/mnt/sockets` read-only, and the
  `RuntimeDirectory` on the host contains exactly the two sockets.

The socket scan repeats every 10 s while the instance runs and at every
attach, navigate and resize; a violation stops the instance with reason
`listener_detected`. The scan is detection evidence; prevention is the
sealed profile (below: no UDP at all, no TCP bind/listen/accept, no bind,
listen or connect on abstract addresses) plus the browser policy (no QUIC, no non-proxied WebRTC UDP, no
built-in resolver, no remote debugging), and admission refuses when
prevention is unavailable.

### Storage during a run

The profile and instance tmpfs are the storage bounds; `/tmp` and `/dev/shm`
are bounded tmpfs as well; the HTTP cache is bounded by flag; downloads and
file dialogs are disabled by policy. The only host-visible writable location
is the sockets directory, which may hold the two sockets and nothing else.
Snapshots are written only through the staging budget. Nothing the browser
does during a run can write to the shared filesystem.

### Resource reporting

Each periodic host resource report carries an optional `browser` object:
`running` (registered instances, starting and stopping included),
`max_instances`, `occupied_slots` (the capacity slots admission checks
against `max_instances`: running instances plus starts not yet registered
and units retained until their exit is confirmed), `memory_bytes` (sum of
each running instance's cgroup `memory.current`) and `budget_bytes` (sum of
their admitted budgets, each unit's `MemoryMax`). It holds counts and bytes
only, never an account, instance or URL. The report reads in-memory state
and the cgroup files only: no systemd call, no lifecycle or control lock a
stop holds, and no Plugin store read, whose installation journal writers
hold across fsync. Whether a browser runtime exists comes from the last
capability observation (the hello and each 30 s capability watch tick). A
Machine whose last observation found no runtime and that runs no instance
omits the object, as do older Machines; an installed runtime with nothing
running reports `running: 0`. `occupied_slots` is optional; readers of a
report without it fall back to `running`.

The About page's Machine card shows it as a **Browsers** meter (running of
`max_instances`; free slots and the warning tone follow `occupied_slots`, so
a slot admission would refuse is never offered as free) and a **Browser
memory** meter against the summed budget. Browser units run in
`cowboy-agents.slice`, so their memory is already part of the card's Agent
memory and already deducted from its Headroom estimate; the card says so
rather than adding the two.

### Profile compatibility and rollback

`browser-state.json` records the full `browser_version` of the artifact that
last opened the profile, published before the launcher restores the
snapshot. `open` compares it with the runtime's `browser_version`: an older
runtime, including an older build of the same major, refuses with
`profile_newer_than_runtime` and offers Reset; a newer runtime advances the
record before launch. Artifact version and profile-format compatibility are
distinct, so the fence is the conservative full version, not the major. The
acceptance matrix includes upgrade → use → rollback and a crash between the
record and readiness.

### Restart reconciliation

The persisted instance record (`browsers/<instance>.json` under the Machine
state directory) holds account, Provider, generation digest, purpose
(always `tab`), unit name and systemd invocation ID, boot ID, incarnation,
deadline, `profile_revision`, and the launcher's reported pids, start times
and namespace identities. At Machine start:

- the unit inventory is one fenced phase: before any record is read an
  inventory-wide fence holds every account's reset and revocation completion
  and every open, and it is lowered only once a pass has accounted for every
  live unit invocation, that is registered it as an adopted instance or
  retained it until its exit is confirmed. A termination retry that confirms
  an exit meanwhile therefore completes no fence of an account the inventory
  has not reached;
- a record whose boot ID differs from the current boot is cleared (the unit
  cannot have survived);
- a record whose invocation is gone (the unit was collected, or its name now
  runs another invocation) is cleared; a replacement invocation is not that
  record's and is classified like any unrecorded one;
- a record with a live unit of the same invocation ID is adopted as
  `detached` only after the installation, executables and
  `host_requirements` re-verify, the account is still enabled and unfenced,
  the deadline has not passed, a memory reservation is re-taken, and
  [confinement verification](#confinement-verification) passes against the
  recorded identities; otherwise the unit is stopped;
- every listed `cowboy-browser-*` unit is observed for its live invocation;
  one that no registered instance or retained termination is bound to
  (unrecorded, or a replacement) is stopped, bound to that invocation, and
  if its exit is unconfirmed it is retained with a capacity slot and every
  account's fences held. An unobservable unit nobody claims is stopped
  whatever runs under its name; one that is claimed leaves the inventory
  incomplete;
- every stop here uses the confirmed termination above: a record is cleared
  only once its unit's exit is confirmed, and a pending reset or revocation
  of its account (of every account, for an unrecorded unit) is completed only
  then;
- a recorded unit that cannot be observed (`systemctl show` fails) is not
  treated as gone: it keeps its record, a capacity slot, its account's fences
  and admission held, and a retry bound to its recorded invocation stops it.
  When the unit listing fails (or a claimed unit cannot be classified), the
  inventory fence stays raised and the whole pass is retried: records that
  became readable meanwhile are reconciled like any other, never exempted,
  and only invocations the manager has registered or retained are skipped;
- abandoned staging directories are reconciled against the staging ledger,
  but only for an account no unresolved unit may write: while the inventory
  fence is raised, or a unit of that account (or of unknown ownership) is
  retained with an unconfirmed exit, its staging is left in place (orphan
  published generations are still removed), because a surviving launcher may
  still be copying into a generation the restarted ledger does not know. The
  reconciliation that follows the inventory's completion or the confirmed
  exit removes it;
- a record an earlier Machine wrote under the retired sign-in purpose is
  stopped (reason `not_available`) instead of adopted.

### Login independence

Provider logins (native Claude, Codex, Grok and every other Provider, native
or Service-owned) have no dependency on the Machine browser, in either
direction:

- the Machine's login registry (`machine_cli/login_sessions.rs`) holds only
  what a login itself needs (cancellation, code input, candidate cleanup); it
  names no browser instance, and the browser manager has no access to it;
- no login path calls into the browser manager: starting a login, emitting
  its challenge (with its verification URL) and ending it leave every browser
  instance untouched, and the presence, absence or state of the browser
  Plugin never changes how a login behaves;
- browser failures, stops, hibernation, OOM kills and storage stops never
  cancel, restart, clean up or otherwise touch a login process or the
  account's credential generation;
- the Controller admits browser opens on browser rules alone (gate,
  per-account enablement, placement, capability), never on login state.

The only meeting point is in the web UI, and it is optional: a named-account
sign-in dialog whose challenge carries a verification URL may offer
"Open in this account's Machine browser" (see [Viewer](#viewer-core-web)).
It opens the account's ordinary `Tab` browser pinned to the login's Machine
and navigates it to that URL through the ordinary routes. The person
completes the page there and copies any code back into the dialog
themselves; nothing reads the remote page or fills the dialog.

Wire compatibility: protocol 45 keeps `Purpose::SignIn` and the reasons
`login_ended` and `login_mismatch` decodable. A Machine refuses an `Open`
carrying `SignIn` with `not_available` and touches no instance; the
Controller never sends one.

## Stream channel (Machine protocol 45)

Commands and events are a closed addition to `MachineCommand` /
`MachineEvent` gated by a new minimum protocol:

- `BrowserOpen{request_id, account_id, instance_id, purpose: Tab, viewport{width, height, scale}}`
  (`SignIn{login_request}` is retired: decodable, refused with
  `not_available`)
- `BrowserAttach{request_id, instance_id, expected_incarnation, attachment_id, stream_token, viewport}`
- `BrowserLease{instance_id, attachment_id, expires_at_ms}`
- `BrowserDetach{request_id, instance_id, attachment_id}`
- `BrowserResize{request_id, instance_id, expected_incarnation, attachment_id, width, height}`
- `BrowserNavigate{request_id, instance_id, expected_incarnation, url}`
- `BrowserSnapshot{request_id, instance_id, expected_incarnation}`
- `BrowserHibernate{request_id, instance_id, expected_incarnation}`
- `BrowserReset{request_id, account_id, expected_profile_revision}`
- `BrowserRevoke{request_id, account_id, operation_id}` (account revocation;
  unconditional, idempotent by operation id, same deletion path as reset)
- `BrowserQuery{request_id, account_id}` (current running or hibernated state,
  for the Controller's state read)
- `BrowserState{instance_id, incarnation, account_id, state, reason, geometry, scale, deadline_ms, attachment_id, snapshot_generation, snapshot_at_ms, profile_revision, unsaved_state_lost}`

Reasons beyond the lifecycle ones: `capacity` (`browser.max_instances`
reached), `memory` (no headroom for the reservation), `busy` (a lifecycle
operation of that account is in progress). `login_ended` and
`login_mismatch` belong to the retired sign-in purpose and stay decodable
only for older Machines.

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
replaced, or the instance incarnation changes. Account admission (Service
gate, account availability, browser enablement), the attachment still being
the account's registered viewer and the Machine connection are re-checked at
pairing, on every lease renewal or resize and on the 25 s heartbeat; the
Service gate is also checked on every relayed binary frame. Disabling or
revoking the account cancels its viewer directly. Attachment admission is
serialized per account on the Controller: the takeover decision, the
viewer's registration and the Machine's attach happen under one per-account
lock, so a second viewer sees the first (even one still attaching) and needs
an explicit takeover. The viewer is registered before the final admission
check, so a disable that check does not yet see still cancels it.

Relay payloads are never logged, persisted, cached or inspected beyond
framing. Counters record bytes and frames.

## Controller routes and authority

| Route | Effect |
|---|---|
| `GET /api/provider-accounts/{account}/browser` | state, geometry, scale, deadline, snapshot generation and time, Machine, reasons; `machine_id=` keeps the read on that Machine; `choose_machine` (no placement and no native credentials anywhere: the first open must name a Machine) and `capable_machines` (`[{machine_id}]`, Machines whose current connection speaks the browser protocol and reported an available browser capability) |
| `POST /api/provider-accounts/{account}/browser/open` | open as `Tab` (no URL body); an optional `machine_id` opens only on that Machine: `browser_not_placed` unless the account has a placement or native credentials there (an account with neither anywhere, never signed in, may name its first Machine), `browser_machine_unavailable` when it is not connected or cannot run browsers (never another Machine) |
| `POST .../browser/navigate` | owner-supplied `https` URL, bounded; names `machine_id` and `instance`; an optional `attachment_id` navigates only while that attachment is current |
| `POST .../browser/resize` | bounded geometry; carries the attachment, `machine_id` and `instance` |
| `POST .../browser/snapshot`, `POST .../browser/hibernate` | lifecycle of the named `machine_id` and `instance` (incarnation included) |
| `POST .../browser/reset` | deletes the profile on the named `machine_id`, with that Machine's expected profile revision |
| `GET .../browser/stream` | viewer WebSocket for the named `machine_id`, `instance_id`, `incarnation`; `takeover=true` replaces a live attachment |
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
account only. A legacy default card has its own per-card enablement instead
(`{type: "set_legacy_browser_enabled", card, enabled}` through the same
`accounts.update` writer, also off by default); see
[Legacy default cards](#legacy-default-cards). Owner-defined
[shared browser profiles](#shared-browser-profiles) change through the same
writer and never replace a member's own enablement. `GET /api/provider-accounts` reports the gate as
`machine_browser: "enabled" | "not_enabled"` so account cards can explain a
closed gate without a per-account read. Cowboy MCP exposes the state read (`accounts.browser`) and the
lifecycle actions through `control.apply` with the same receipts as other
account operations, per the MCP discovery rule in `AGENTS.md`.

Device settings (Machine-enforced, `cowboy config`, reload class `Live`:
a changed value applies to the next `open`; running units keep the
properties they were started with; bounds enforced by the schema):

| Key | Default | Bounds | Meaning |
|---|---|---|---|
| `browser.max_instances` | 1 | 0–4 | running instances per Machine |
| `browser.memory_max` / `browser.memory_high` | 2048 MiB / 1536 MiB | 1–6 GiB | upper bounds of the unit limits (the admitted budget and the `MemoryHigh` derived from it), including every tmpfs page; `memory_high <= memory_max` |
| `browser.memory_reserve` | 768 MiB | 256 MiB–6 GiB, `<= memory_high` | smallest admitted budget: an open that cannot reserve this much is refused |
| `browser.cpu_quota_percent` | 200 | 50–400 | unit CPU quota in percent of one CPU |
| `browser.tasks_max` | 512 | 128–2048 | unit task limit |
| `browser.hibernate_after` | 120 s | 30 s–30 min | detached grace |
| `browser.max_runtime` | 30 min | 5 min–4 h | hard cap per run |
| `browser.profile_max_bytes` | 256 MiB | 64 MiB–1 GiB | profile tmpfs size, `/dev/shm` size and snapshot bound |
| `browser.instance_max_bytes` | 128 MiB | 32–512 MiB | instance tmpfs size |
| `browser.snapshot_interval` | 10 min | 1 min–1 h, or off | periodic frozen snapshot while attached |
| `browser.staging_max_bytes` | 1 GiB | 256 MiB–8 GiB | Machine-wide snapshot staging budget |
| `browser.max_width` / `browser.max_height` | 2560 / 1600 | 320–4096 / 480–4096 | resize bound |

Reclaim ordering: the Controller's memory reclaim hibernates running browsers
on a Machine before it selects any session. Browser reclaim has its own
pending state: the admission fence stays raised, the stop is retried and
escalated (`SIGTERM`, then `SIGKILL` through the unit), and the step settles
only on verified unit exit followed by a fresh resource sample: the targeted
incarnation reported `hibernated` (or the Machine's authoritative answer that
no instance runs, or a newer incarnation), then a resource report taken after
that exit was first seen. `stopping` is not an exit; the instance still holds
its memory while it copies its snapshot. When a background termination retry
confirms the exit, the Machine reports a terminal `hibernated` for that same
instance and incarnation, which replaces the cached `stopping` and settles the
step. The step is captured as one instance on one Machine: the hibernate is
sent only to that instance on that Machine's connection, and only that
Machine's own observation of the account counts as exit evidence; the same
account's browser on another Machine neither settles nor retargets it. A stop that
stalls does not time out into "released": it stays pending, is logged as
stuck, and blocks session selection on that Machine until the unit is gone;
the only exception is an explicit operator emergency action. While the step's
observation is unresolved (not yet an exit: for example a Machine that
reconnected mid-stop and answers `stopping` with no instance, for which no
terminal event follows), the Controller asks that Machine directly
(`BrowserQuery`, at most once per Machine and account every 10 s, each bounded
by the query timeout) and settles on its authoritative answer.
A Hibernate can be lost when the Machine reconnects after the step was
captured but before the command arrives (it is refused for the old
connection, or the connection ends under it). When an authoritative query
answer on the Machine's verified current connection reports exactly the
captured instance (same Machine, `instance_id` and incarnation) still
running, and the last Hibernate went to an earlier connection or has
finished, the Controller sends Hibernate again to exactly that instance on
that connection: at most 3 times per step, at most once per query interval.
The step's `RELEASE_TIMEOUT` still runs from the original decision, and the
Device decision lock stays held throughout.
A reconnect clears the Controller's browser observations for that Machine,
and the placement refresh re-learns them asynchronously, so a surviving
browser is briefly neither running nor stopping in the cache. Each
connection therefore starts with its Machine's inventory unresolved: once
the durable placements on that Machine load, each stays unresolved until an
authoritative answer (a query, a confirmed reset) arrives for it on that
same connection. While any is unresolved (or the placements have not
loaded, including when loading them failed: fail closed) session selection
on that Machine waits, querying each unresolved placement at the same
bounded rate, and after the last one resolves it again needs a resource
report taken after that. A Machine that cannot run browsers has nothing to
resolve.
A browser that is already stopping when reclaim reaches session selection
(an owner request, recovery, an earlier Controller) also holds memory, so
any current `stopping` observation on the Device's Machine, including one
naming no instance, is unresolved browser work: session selection waits,
querying each such browser at the same bounded rate, until none remains
stopping and a resource report taken after that has arrived. Past
`RELEASE_TIMEOUT` such a wait is logged as stuck, never treated as released.
`OOMScoreAdjust=1000` is a preference that makes the browser the kernel's
likely first victim inside the shared slice; it is not relied on for
ordering.

Measured on hawk (nixpkgs Chromium 151, Xvnc 1.16, Claude sign-in page
loaded, profile on disk): 570–590 MiB in the cgroup, Xvnc 30 MiB of it, no
TCP listeners, RFB handshake, resize and clipboard forwarding verified over
the Unix socket. The tmpfs profile adds its pages to that figure; the
defaults above are provisional until the restore, full-profile and
mixed-load measurements in [Acceptance](#acceptance-before-any-real-account)
exist.

## Viewer (core web)

The viewer is core code in the main bundle, same origin, no iframe: a canvas,
an RFB decoder running in a Worker (the CSP already allows `worker-src 'self'
blob:` and `wss://<host>`), pointer and keyboard translation, and
browser-style chrome around it. Vendoring the noVNC RFB core (MPL-2.0) as a
dependency is acceptable; its application UI, `resizeSession` and clipboard
automation are not used. The chrome always shows the label of the account
whose browser it displays and the Machine it runs on.

- **Sign-in pages live in the browser.** The panel offers no quick links:
  the browser starts on Chromium's own new-tab page, whose policy
  `NTPShortcuts` tiles are Claude, ChatGPT · Codex, Grok, DeepSeek and Google
  sign-in (see "Managed policy"). The navigate command still accepts an
  `attachment_id`: the Machine checks it under the control lock a takeover
  holds while installing its attachment, so a navigation fenced to one
  viewer never reaches a replacement viewer's page.
- **Shared browsers look different.** Account and usage card entries of a
  shared browser carry the group icon, the secondary accent and an
  accessible name "Shared browser: <group>"; the identity line reads
  "<group> · shared with <members>" with the same icon; the empty page is
  titled "Shared browser · <group>"; the reset, hibernate and restart
  confirmations name the group and every member they affect. Private browsers keep their
  look.
- **Browser chrome.** The panel adds only Close and one identity line
  (account label, Machine and state): a slim top row on Desktop, the very
  bottom of the bottom area on a phone. The remote page fills the frame and the remote Chromium
  shows its own address bar and tabs, so the panel adds no address field of
  its own. Back, Forward and Reload send the
  standard Chromium key chords through the attached RFB session (Alt+Left,
  Alt+Right, F5; every pressed key is released in reverse order); they and
  the touch keyboard are enabled only while this panel's viewer is
  connected, and act only on the viewer mounted for the shown account,
  Machine and instance. The "More" menu holds Hibernate, Save snapshot,
  Restart for this screen (on a scale mismatch), Stop viewing, Reset
  profile, Paste from this device…, Copy remote clipboard to this device,
  Refresh state and Close; inapplicable entries are disabled. Without an
  attached display the frame shows a browser-like empty page naming the
  state and Machine with one primary action (Open browser, the Machine
  chooser, or Take over), or the admission reason. Errors and notices float
  over the frame as dismissible banners, so the frame and its requested
  framebuffer never move; a refusal that frees up by itself (no memory
  headroom, capacity, another operation in progress) says so and offers
  Retry.
- **Desktop panel.** A large centered modal (`min(96vw, 1960px)` x
  `min(94vh, 1180px)`, rounded, over a dimmed backdrop; the page underneath
  stays as it was). Maximize fills the whole window (100vw x 100vh, no
  corners) and the choice is remembered on the device. A backdrop click
  never closes it. One 40 px header row holds the identity and state (with
  the shared-browser icon and "<group> · shared with <members>"), Back,
  Forward and Reload, More (paste, hibernate, restart, reset, take over),
  Maximize/Restore and Close; the frame fills the rest and
  picks its fixed screen mode as below, re-requesting it when the modal is
  resized or maximized. With focus in the remote frame every key, Esc and
  browser shortcuts included, goes to the remote browser: the global Desktop
  key router stands aside before any leader (Space, the workspace prefix,
  the modal leader) can claim a key there, its keyups included; surfaces
  below the modal (Usage, Settings) stand aside while it is open. Esc closes
  the modal from Cowboy's chrome, the empty page and Machine chooser
  included, never from the remote viewer. The release chord, registered as
  `DESKTOP_SHORTCUTS.browserRelease` (Command-Shift-Esc on macOS,
  Alt-Shift-Q elsewhere, where Ctrl-Shift-Esc and Alt-Esc belong to the
  system; IME-safe), moves focus from the frame to the header and is
  named in the Close tooltip. It is a dialog labelled by the identity,
  traps focus and restores it on close; its menus speak the Desktop menu
  grammar (`j`/`k`/`l`/Enter/Esc). Not running, the frame shows the same
  page as the phone: the state (or "Shared browser · <group>") and Open
  browser.

- **Screen modes and resize authority.** The remote screen is never sized
  to the device. As in neko, it runs one of a few fixed, ordinary modes at
  scale 1 and the viewer scales the framebuffer to fit and letterboxes it
  (noVNC `scaleViewport`). Pointer devices get the largest of 1280x800,
  1440x900 and 1920x1080 that fits the frame at DPR 1 (never less than
  1280x800). Touch devices get 720x1280 held upright and 1280x720 on their
  side: 720 px is below the common 768 px tablet breakpoint, so sites lay
  out for a phone, a 9:16 screen nearly matches a phone's frame (about 393
  by 690 CSS px between the status bar and the toolbar), so little of it is
  letterbox, and the narrower mode keeps text larger than neko's 800x1600.
  Touch versus mouse follows neko's rule (touch points, and neither
  `(pointer: fine)` nor `(hover: hover)`), with the surface profile as the
  fallback where media queries are missing; orientation comes from the
  Screen Orientation API (else `window.orientation`, else the window's
  shape), never from the frame. The viewer is the only size authority: it
  re-requests a mode, debounced, only when the mode changes (a rotation, or
  a Desktop window resize crossing into another mode) as a
  `{type:"resize"}` text frame on the attached stream; touch input coming or
  going (an iPad trackpad) resizes the same attachment at once and never
  reattaches, which would race the Machine's detach acknowledgement. The
  soft keyboard and the panel chrome never resize the remote. Every mode lies within the
  wire bounds (320x480 to 4096x4096). The server never resizes on its own
  and `AcceptSetDesktopSize` is off, so the noVNC CSS-pixel resize race
  cannot occur. `POST .../browser/resize` stays for callers without a
  stream (the MCP operation).
- **Scale and device handoff.** `scale` is chosen at open and is immutable
  for the incarnation because it is a Chromium launch flag. Every viewer now
  opens at scale 1, so the remote looks like an ordinary screen; an
  incarnation an older client opened at scale 2 is still scaled to fit and
  offers an explicit "Restart for this screen" action, which hibernates
  (snapshotting) and reopens at scale 1 (new incarnation). The restart stays on the
  captured Machine: it reopens only after that Machine confirms the
  captured instance stopped (the hibernate's answer, else reads pinned to
  that Machine, about 30 s at most: `hibernated` for that instance and
  incarnation, or no instance running), and the open names that Machine
  (`machine_id`), so it never starts on another Machine with a different
  profile. A newer instance there (`browser_instance_changed`) or no
  confirmation (`browser_restart_unconfirmed`) reopens nothing. The page
  state since the last snapshot may be lost, which the action says.
- **Attachment.** The stream socket carries the device proof as a
  subprotocol and renews the lease every 10 s while open; the Machine drops
  an attachment whose lease lapses for 30 s. Attaching while another device
  holds the display is an explicit, confirmed takeover. A refused or
  unanswered attachment ends the viewer with the stream's close code
  (`browser_attach_refused`, `browser_attach_failed`) and a notice naming
  the state; re-attaching is a user action, never automatic. An open whose
  answer is a live browser carrying a reason did not take effect: the panel
  shows the failure and never mounts the viewer for it, and an open that
  answers `attached` never takes the display over by itself.
- **Target binding.** The viewer attaches to exactly the Machine and
  instance (`instance_id` and `incarnation`) it was given: the open's answer
  (an open answering `starting` attaches once that same instance on that
  Machine runs) or the state the panel shows. Hibernate, snapshot and
  restart name the same Machine and instance, and reset names the shown
  Machine with that Machine's profile revision (from its state, else its
  placement). While the panel shows a live browser, is attached or awaits a
  starting open, its state reads carry that Machine, so a newer browser of
  the same account on another Machine never replaces what it shows. Each
  read is tagged with its generation, pinned Machine and lifecycle epoch;
  an answer that is no longer the newest read, whose pin no longer matches
  the panel (an open mounted the viewer meanwhile), or that was issued
  before the latest lifecycle transition (an applied command result, a
  viewer mounted or ended, an awaited open settled) is discarded. An answer
  naming no instance never ends a viewer whose instance was established
  after that read was issued (a restart's stopping-time read cannot close
  the replacement viewer). Status polling keeps one read in flight: the
  next poll is scheduled only after the previous read settles, so reads
  slower than the poll period still apply. While the viewer is
  mounted, the shown state and every instance command come from the
  viewer's Machine and instance: a read for another Machine or an older
  incarnation never replaces it, and a newer instance on the viewer's
  Machine ends the viewer. A confirmation (takeover, hibernate, restart,
  reset) captures the Machine and instance, or the Machine and profile
  revision for a reset, shown when it opens and sends exactly that; it
  closes without acting once the panel shows another target, and a
  confirmed takeover never attaches to a browser that replaced the confirmed
  one. A
  `browser_instance_changed` refusal tells the owner to check the current
  state; nothing is retried against another browser.
- **Outcomes.** Snapshot and reset answer HTTP 200 with the Machine's state;
  the viewer reports success only for a snapshot without a reason and a
  reset hibernated with `owner_request`, and otherwise names the reason
  (for example `storage_exceeded`, `busy`). A stopped browser and every
  reset are reported without an instance (`instance_id: ""`,
  `incarnation: 0`), which is valid only as `hibernated`.
- **Mobile sheet.** An edge-to-edge full-screen cover with no top row:
  the frame starts right below the status bar (safe-area top inset) and
  fills everything down to the bottom area, which holds one row of six
  46 px targets, Close, Back, Forward, Reload, Keyboard and More, with one
  ellipsized 16 px identity line (account, Machine, state) below it at the
  very bottom (2 px top padding, then the safe-area bottom inset less
  `--kb-inset`, at least 4 px: 68 px plus the home indicator). Nothing overlays the framebuffer but
  transient banners. The bottom area clears the side safe areas and sits
  above the home indicator and the app shell's keyboard inset, so the
  software keyboard never covers it. Touches reach the remote Chromium as
  native multi-touch contacts ([Native touch](#native-touch)); against a
  display server without it, tap clicks, two-finger drag scrolls and long
  press is right click. Backgrounding the app suspends lease renewal,
  which detaches within 30 s; returning shows the detached state and the
  Open browser action while the grace period keeps the instance alive.
- **Touch keyboard.** As in neko, a transparent full-size `<textarea>`
  lies over the framebuffer (16 px so iOS never zooms; autocapitalize,
  autocomplete and autocorrect off, no spellcheck, `inputmode="text"`). It
  ignores pointers, so taps, long presses and scrolls still reach noVNC
  below, and it is never focused by default: the toolbar Keyboard button
  focuses or blurs it synchronously inside the tap and prevents the
  button's pointer and mouse down so focus stays. While it is focused, taps
  on the page do not move focus. Text is the text the OS committed, never
  inferred from key events: it is sent on `input` when no composition is
  running and on `compositionend`, never during a composition, and the field
  is reset after every send. The field is never rewritten under a live
  composition (iOS WebKit would strand the IME, PITFALLS #110): blurring it
  mid-composition (Hide keyboard, More) keeps the marked text, and the
  commit WebKit delivers after the blur is sent once, only to the session
  the composition began in while it is still live; teardown drops it. The
  idle field holds one untransmitted zero-width sentinel, so a keyboard
  that reports Backspace only as an edit (Android Gboard: key code 229, then
  `deleteContentBackward`) deletes the sentinel, which is one remote
  Backspace. Short printable ASCII (at most 32
  characters) is pressed as keysyms (Xvnc adds the Shift a capital or
  shifted symbol needs); non-ASCII text such as an IME result, emoji and
  longer insertions go through the remote clipboard and Ctrl+V. Backspace,
  Enter, Tab, Escape, Delete, Home, End and the arrows come from `keydown`
  (prevented; IME key code 229 and composing keys are ignored; the edit of
  a key already sent is not sent again where the OS ignores the prevention).
  This avoids
  neko #724, where guacamole.js guesses characters from key events and
  loses capitals, shifted symbols and text after a composition on iOS. The
  overlay feeds only the session of the viewer that owns it, which is
  mounted for exactly one account, Machine and instance.
- **Paste from this device.** Safari and Firefox cannot sync a clipboard by
  themselves, so the menu entry opens a small sheet with a text field,
  prefilled from `navigator.clipboard.readText()` only when the browser
  allows it inside the tap and empty for a manual paste otherwise. Send
  sets the remote clipboard (`ClientCutText`) and presses Ctrl+V on exactly
  the viewer that was bound when the sheet opened; the sheet closes without
  sending once another viewer is shown.
- **Remote clipboard.** Xvnc forwards only explicit copies (`SendPrimary=0`).
  A received cut text is held in viewer memory for the open attachment and
  offered as the explicit "Copy remote clipboard (n characters) to this
  device" menu action. It is never wired into any dialog or login: a sign-in
  code field is filled only by the person pasting it. The held text is
  dropped on attachment or incarnation change and when the viewer closes.
- **Passkeys** are not transported by RFB; use the email or one-time-code
  path inside the browser.
- **First Machine.** When the state read reports `choose_machine` (never
  signed in, no placement), the tab panel replaces "Open browser" with a
  Machine chooser listing only `capable_machines` (display names from the
  Machine list) and opens with that `machine_id`; with none offered it says
  so and opens nothing. The chosen Machine becomes the placement, so later
  opens need no choice. A panel pinned to one Machine (the verification-link
  entry below) never offers the chooser.
- **Entries.** Every Provider usage card offers the panel, named accounts
  and legacy default cards alike: the account management card ("Browser…",
  beside the enable toggle; for a legacy card on the Service-account row of
  its first Provider) and every usage card ("Browser", Mobile Info sheet and
  Desktop usage panel). A usage-card entry opens only that card's ordinary
  tab browser and never enables anything. With the gate closed, the account
  disabled or its browser not enabled, the entry is disabled with a one-line
  reason and, for the account-level cases, a link to account management. A
  legacy card opens through its own browser key
  ([Legacy default cards](#legacy-default-cards)); the shared `default`
  identity itself is refused (`browser_key_required`), and a card the Service
  no longer collects offers no entry.
- **Isolation in the panel.** A panel is bound to one account: its model is
  keyed by that account, switching accounts starts from an empty model (no
  read, viewer target, pinned Machine or remote clipboard), and every
  request result and viewer callback is tagged with the account that issued
  it, so a late answer for another account is discarded. Each entry mounts
  its own sheet keyed by the account id; two cards never share a viewer.
- **Verification link (optional).** A named-account sign-in dialog of any
  Provider (Claude, Codex, Grok, ...) whose challenge carries an `https`
  verification URL may offer "Open in this account's Machine browser". The
  entry appears only when the account's browser read on the login's Machine
  is admitted (gate on, account available, browser enabled), that Machine is
  among `capable_machines`, and an open pinned there is the account's own
  (its placement there, no placement yet, or a first-Machine choice). It
  opens the account's ordinary tab panel pinned to that Machine (opening or
  attaching as the usage card would), and once that exact instance runs on
  that Machine it calls `POST .../browser/navigate` with the URL once. The
  person completes the page and copies any code back themselves. The entry
  never names the login to the Controller or the Machine, the login works
  identically without it, and nothing flows from the browser into the
  dialog.

The cross-device route `/#subscription-sign-in/<account>/<request>` resumes
the sign-in dialog on another device; its verification-link entry opens the
same account's tab browser there.

## Native touch

A touch viewer used to reach the remote Chromium as an emulated mouse, which
is itself a tell: Chromium on an X display without typed input devices
reports `navigator.maxTouchPoints == 0` and matches neither `(pointer: …)`
nor `(hover: …)`, which no real device does, and pages cannot pinch, swipe
or long-press natively. The runtime's display server therefore carries a
small patch (`plugins/machine-browser/runtime/native-touch.patch`) and the
viewer speaks one RFB extension.

- **Devices.** With `-AcceptTouchEvents=1` the core pointer is typed
  `XI_MOUSE` and an XI2 direct-touch slave, "TigerVNC touch"
  (`XI_TOUCHSCREEN`, 10 contacts, absolute MT axes), is attached beside it.
  Chromium then reports 10 touch points, coarse and fine pointers and hover,
  like any touchscreen laptop. The typing applies to every viewer, phone or
  Desktop, so the page sees the same machine either way. The devices are
  in-process X devices: no `/dev/input`, `uinput` or other host interface is
  involved.
- **Wire.** The viewer adds pseudo-encoding `-1883` to its SetEncodings. A
  server with the feature on answers with server message `192` (u8 type, u8
  version 1, u8 contacts 10) at most once per connection, even if the
  encoding is dropped and requested again. The viewer keeps the advertised
  count (at most 10; 0 or an unknown version leaves native touch off) and
  ignores fingers beyond it: they send no begin and never reach noVNC. Only after that may the client send
  message `193`, the KasmVNC layout: u8 type, u8 state (0 begin, 1 update,
  2 end), u32 id, u16 x, u16 y, big endian, framebuffer pixels. A stock
  server, or the patched one without the parameter, never answers, so a
  client never sends `193` to a server that would treat it as an unknown
  message and disconnect. A client that never requests the encoding sees a
  byte-for-byte stock protocol.
- **Gating and bounds.** `AcceptTouchEvents` defaults off and is not in
  `AllowOverride`, so no X client can turn it on; the launcher passes it for
  every instance. Contacts are slots of one pool shared by every screen of
  the process, because all of them feed the one XI2 device (Xvnc has a
  single screen; the pool stays process-wide for the Xorg module's case): at
  most 10 in total, so ids from different clients never collide. A duplicate
  begin, an update or end of an unknown id and a begin beyond the bound are
  ignored; coordinates are clamped to the framebuffer; any other state is a
  protocol error for that connection. Every contact a connection leaves
  active is ended when it closes, so a dropped viewer never leaves a finger
  down (neko issue 719) and the next attachment starts clean. Without pointer
  permission (access rights, or `AcceptPointerEvents` turned off through the
  VNC extension) nothing new is accepted but active contacts still end, at
  their last accepted positions: at once when the parameter changes,
  otherwise on the client's next touch message.
- **Viewer.** `web/src/machineBrowser/nativeTouch.ts` hooks noVNC 1.7's
  SetEncodings writer (only for the viewer's own sockets) and consumes the
  acknowledgement before noVNC's message loop, which would fail on an
  unknown message. Until the acknowledgement nothing changes: noVNC's
  gesture-to-mouse emulation stays the fallback. Native touch takes over only
  between touch sequences: one noVNC already began (its touchstart reached
  noVNC) runs to its end there, or its emulated button would stay pressed.
  After that, Pointer Events of
  type `touch` (one contact per `pointerId`, captured to the frame) are
  mapped through noVNC's CSS scaling to framebuffer pixels and sent; a
  contact may not begin in the letterbox, and moves outside it clamp. The
  frame's capture-phase listeners prevent and stop the touch events, so
  noVNC's gesture handler never runs and the browser synthesizes no
  compatibility mouse events or clicks. Window blur, `pagehide` and a hidden
  page end every active contact; a closed stream sends nothing (the server
  ends the contacts). Mouse and pen input stay with noVNC, so Desktop
  behaviour is unchanged. A contact focuses the canvas exactly when
  noVNC's own tap would (the touch keyboard's focus rule holds).

### Building the patched display server

The artifact is otherwise Ubuntu's bytes; `Xtigervnc` becomes a rebuild of
Ubuntu's own source with the patch added. Options considered:

| Option | Verdict |
|---|---|
| Commit a prebuilt binary, or upload one built at lock time (Docker) and pin its URL and SHA-256 | Rejected: the bytes cannot be reviewed or reproduced from what is in Git, and the release would depend on one builder's container |
| Build with Nix | Rejected: a different toolchain and glibc than the artifact's userland, and store paths in the binary |
| Rebuild at artifact time from pinned sources in a pinned build root, and require the recorded output digest | **Chosen** |

`display-server-lock.sh` (Docker, lock time only, metadata only) finds the
Launchpad build of the exact `tigervnc-standalone-server` version the
runtime lock ships and pins its `.buildinfo` by SHA-256. Its
`Installed-Build-Depends` is the complete build root; every one of those
packages is pinned by `snapshot.ubuntu.com` path at the build's
`Build-Date` instant (including `-proposed`, which Launchpad builds the
development series against), size and SHA-256, beside the `.dsc`, orig and
debian tarballs. `display-server.ts` (also run by `build.ts`, no Docker)
downloads those bytes, verifies the `.dsc` checksums and that the build root
equals the `.buildinfo` exactly, unpacks every package without maintainer
scripts (recreating only the alternatives the build calls and the dpkg
vendor link), applies the package's quilt series with `native-touch.patch`
appended (`--fuzz=0`), and runs `debian/rules build-arch` at the
`.buildinfo`'s `Build-Path` in an unprivileged bubblewrap sandbox without
network, with `SOURCE_DATE_EPOCH` from the changelog and the recorded
build environment. It then repeats what debhelper does to the shipped file
(`dh_dwz`'s multifile pass, `dh_strip` with a seeded build id and a
debuglink) and installs the result only if it hashes to
`display-server.lock.json`'s `output`. `--record` writes that digest after a
review; nothing else accepts an unrecorded binary.

`display-server.ts --verify-stock` is the build root's own proof: without
the patch the same pipeline reproduces Ubuntu's shipped `Xtigervnc` byte for
byte (`e59b12d2…`), so the only difference in the artifact is the reviewed
patch. The patch carries `Cowboy-Reviewed-For: tigervnc <version>`; when
`lock.sh` moves to another tigervnc version, the build refuses until
`display-server-lock.sh` is rerun, the patch is reviewed against the new
source and its line moves, and the new output is recorded. The builder
needs `bwrap` with unprivileged user namespaces and about 300 MB of cached
downloads; the build takes about a minute.

Verification: `runtime/touch_probe.py`, run by `conformance.sh` through the
real launcher, checks negotiation, stock-client compatibility (no
acknowledgement, pointer unaffected, unnegotiated `193` disconnects only that
client), an invalid state, clamping, and then, against a loopback page in
the instance's Chromium, the device capabilities, touchstart/move/end with
mapped coordinates, a two-finger pinch, contacts ended when their connection
closes, no stuck contact after reconnecting, and the ten-contact bound. The
same display server without the flag is checked to answer like stock, and
with the flag `AcceptPointerEvents` is turned off through the VNC extension
while ten contacts are down: all ten end at once (XI2 events observed with
`xinput`), nothing new is accepted, and all ten slots are free once pointer
input is allowed again. The viewer's tests run its hooks against noVNC's own
`Websock` and pin the `rfb.js` internals they rely on.

## Host sandbox policy (outside Cowboy)

Ubuntu restricts unprivileged user namespaces
(`kernel.apparmor_restrict_unprivileged_userns=1`), so neither the launcher's
isolation nor Chromium's sandbox can start from the Machine's user service
without AppArmor policy that grants `userns`. That policy is host
configuration, kept and receipted in Columbus `machines/ovh`. It is two
profiles, and it confines rather than exempts. The sysctl and the ordinary
denial for every other process stay exactly as they are.

The profiles are named `cowboy-browser-bootstrap` and `cowboy-browser-sealed`;
the Machine checks for exactly these names in enforce mode.

**`bootstrap`** attaches to the Plugin's launcher executable through a glob
over the Machine state root and runtime digest directory (derived from
`runtime.launcher`). It grants `userns`; separate `mount` rules for each
bind, tmpfs and `proc` mount of step 2 with its exact resolved source
pattern and destination alias (read-only bind of the installed generation,
tmpfs at the fixed alias paths, bind of `accounts/*/browser/snapshots/gen-*`
at `/mnt/snapshot`, bind of `accounts/*/browser/staging/gen-*` at
`/mnt/publish`, the `/etc` scaffold binds); separate `remount` rules for the
read-only, `nosuid`, `nodev` flags on the artifact binds and on
`/mnt/sockets` (a remount carries no source pathname, so it is matched on
the destination and flags alone); `umount` rules for `/mnt/snapshot` and
`/mnt/publish`; read of the runtime and of the snapshot alias; read-write of
`/mnt/instance`, `/mnt/profile`, `/mnt/publish` and `/mnt/sockets`; write to
the unit's own cgroup tree for placing children; and `change_profile ->
sealed` plus `signal` and `ptrace (read)` toward its own children for
supervision. It denies everything else under the Machine user's home and the
Machine state root with audit logging. The bootstrap profile can bind an
alias only from the patterns above; it cannot, for example, bind another
unit's `RuntimeDirectory` or another account's native directory, and the
launcher is the only process that ever runs under it.

**`sealed`** is reached only by `change_profile` from `bootstrap` and is
inherited by every descendant, including Chromium's nested sandboxes. It has
no `mount`, `umount`, `remount` or `pivot_root` permission at all, so a
descendant that creates a new user and mount namespace cannot bind or mask
anything there either. It grants `userns` (for the zygote), read and map of
`/usr/**`, `/opt/**` and the scaffolded `/etc`, read-write of
`/mnt/profile/**`, `/mnt/instance/**`, `/tmp/**` and `/dev/shm/**`, creation
of exactly `/mnt/sockets/rfb.sock` (only possible before the launcher
remounts that export read-only) and the X socket in `/tmp/.X11-unix/`, the
namespaced `sys_chroot` capability and `/proc` reads Chromium's sandbox
uses, and `network inet stream (connect send receive)` and
`network inet6 stream (connect send receive)`. It denies every `dgram`
socket (`deny network inet dgram`, `deny network inet6 dgram`: the browser
resolves names over TCP and has no other UDP use, so no UDP channel,
auto-bound or otherwise, can exist), grants `bind`, `listen` and `accept`
on no `inet`/`inet6` stream socket, grants no bind, listen or connect on an
abstract Unix address (only send and receive between sealed peers on the
five-hex-digit names the kernel autobinds to connected socketpairs; these
refusals are implicit because, on Ubuntu 26.04's parser, the equivalent
explicit `deny` rules refuse far more than written, see the Columbus
`machines/ovh/docs/machine-browser-policy.md`), denies `/mnt/snapshot/**`, `/mnt/publish/**`,
`/home/**`, the Machine state root and any write to the cgroup tree with
audit logging, and denies `ptrace` and `signal` toward anything outside its
own instance.

The Machine checks that the host's AppArmor exposes the features these
profiles rely on (`/sys/kernel/security/apparmor/features`: network
permissions, `unix` addresses, mount, user namespaces) and that both profiles
are loaded in enforce mode, and refuses admission with
`host_requirements` otherwise. It never weakens anything when the policy is
absent: the launcher cannot create its namespaces and reports
`sandbox_unavailable`. The policy is validated first on the disposable Ubuntu
26.04 KVM guest already used for OVH SSH tests
(`machines/ovh/tests/openssh_kvm.py` pattern), with synthetic account
directories and canary files, before it is installed on OVH.

## Account isolation (core requirement)

Isolation is per **browser identity**: an account card, or an owner-defined
[share](#shared-browser-profiles) of cards of pairwise distinct account-usage
families. The browsers of different browser identities share no profile,
display, temporary, shared-memory, IPC or socket state, and nothing a page
can store in one identity's browser can be read by another's. Two accounts of
the same family (two Claude accounts, two OpenAI accounts) are never one
browser identity. This is what makes several accounts on one Machine safe to
operate, and it is a requirement of the design, not a side effect. In the
table, "account" means browser identity:

| State | Isolation |
|---|---|
| Cookies, local storage, IndexedDB, service workers, HTTP cache, GPU and shader caches, Chromium singleton lock, `Local State` | per-account profile: a private tmpfs during the run, restored from and published to the account's own snapshot directory; no other account's data exists in the instance's view, and the sealed profile cannot mount anything into it |
| `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `TMPDIR`, NSS database | the per-run instance tmpfs, destroyed with the unit |
| `/tmp`, X display socket, X lock files | private tmpfs per instance; the X server's abstract socket is disabled and abstract addresses are denied by the sealed profile; each instance has its own `Xtigervnc`, so X selections, window state and input never cross |
| `/dev/shm`, SysV and POSIX IPC | private tmpfs and private IPC namespace per instance |
| Processes and signals | private PID namespace per instance with its own `/proc`; the sealed profile also denies `ptrace` and `signal` outside the instance |
| Abstract Unix sockets, UDP sockets of any kind, TCP listeners | denied by the sealed profile (DNS goes over TCP through the artifact's glibc); the Machine's recursive socket scan is the evidence |
| Control and RFB sockets | per instance, inside the unit's 0700 `RuntimeDirectory`, read-only to every sealed process once created; reachable only through that host path (the same-user boundary below) |
| Memory and CPU accounting | one systemd unit and cgroup per instance; `max_instances` may allow several at once, each in its own namespaces |
| Remote clipboard | held per attachment in the viewer, never shared between accounts, dropped on attachment or incarnation change, never wired into a login; the owner's device clipboard is only ever pasted by an explicit action |
| Runtime files (browser, fonts, policy, glibc) | shared read-only artifact; they carry no state |
| Legacy default cards | each card has its own derived browser key, so its profile, placement, instance, unit and viewer are those of a distinct account; the key never equals a registry account, another card's key or the legacy default's native credential directory ([Legacy default cards](#legacy-default-cards)) |
| Shared browser profiles | only by the owner's explicit choice, only between members of distinct families, and only through the share's own derived key; members' own profiles are never copied into it or deleted by it ([Shared browser profiles](#shared-browser-profiles)) |
| Logins | no login is bound to a browser; a sign-in dialog's optional verification-link entry opens only the browser of the dialog's own account, on the login's Machine, through the ordinary open and navigate routes; the viewer always names the account whose browser it shows |

What stays shared is what is shared for the native CLIs already: the
Machine's egress address, timezone and locale, the runtime version, the
host network namespace, and the Machine user identity, under which the
trusted Machine supervisor and any same-user host process can reach the
sockets directory and the snapshot files (the trust boundary recorded in
`requirements.md`). The design does not manufacture per-account device
differences; separate, persistent, honestly distinct profiles are the
isolation. It prevents automatic inheritance of one account's browser state
by another; it cannot stop the owner from typing one account's upstream credentials into
another account's browser, which is why the viewer always names the account.

## Legacy default cards

Every Provider usage card offers a Machine browser, including the cards that
show the Service-collected legacy default account of an account-usage
provider (Claude · Lynx for `anthropic`, DeepSeek · Mole for `deepseek`,
OpenAI · Pika for `openai`, Grok · Fox for `xai`, and any other).

- **One card, one browser.** The usage page shows one legacy card per
  account-usage provider, so the browser identity is per card, not per
  Provider. The `claude-deepseek`, `codex-deepseek` and `deepseek-harness`
  defaults share one usage card (account-usage provider `deepseek`, the
  "Mole" card), which therefore has exactly one browser; any other card, a
  named account of the same Provider, and another owner's DeepSeek card each
  have their own. Should the usage page ever show one card per Provider, each
  would need its own card id and therefore its own key.
- **Key.** `legacy_browsers::account(owner, card)`: the first 16 bytes of
  `sha256("cowboy-legacy-browser-account-v1\0" ‖ owner ‖ "\0" ‖ card)`
  formatted as a UUID with version 8 and the RFC variant. It is stable, a
  valid Machine `ProviderAccountId` and a single directory name, never
  `default`, and never a registry identity (registry identities are version 4,
  so the key spaces are disjoint). The browser routes, the viewer stream and
  Cowboy MCP (`accounts.browser`, `accounts.browser_*`) accept it exactly like
  a named account id; `accounts.list` (`GET /api/provider-accounts`) reports
  each card as `legacy_browsers: [{card, account_id, providers,
  browser_enabled, available}]`. The literal `default` names no single card
  and is refused (`browser_key_required`).
- **Machine directory.** The card's browser lives under
  `provider-auth/accounts/<key>/` (`browser-state.json`, `browser/`), the same
  layout as a named account. The legacy default's native credentials stay
  under `provider-auth/providers/<provider>/`; no browser path reads or writes
  them, and the Machine prepares only the account directory, never a
  `providers/` credential scope, so a card key never appears as a credential
  account.
- **Enablement.** Off by default, per owner and card, in
  `provider_legacy_browsers (owner_user_id, card_id)` (migrations
  `0081_provider_legacy_browsers.sql` and sqlite `0055`; a row means
  enabled). It changes only through `accounts.update`
  `{type: "set_legacy_browser_enabled", card, enabled}`, under the owner lock
  with the registry's revision and operation receipt, without changing any
  registry account. Enabling needs a card the Service collects now; disabling
  is always accepted, ends that card's viewers and hibernates its running
  instances (the snapshot is kept). The account management row shows the
  "Browser…" panel and the enable toggle on the Service-account row of the
  card's first Provider; usage cards show the disabled entry with its reason
  and a link to account management while it is off. The Service gate
  `accounts.machine_browser` still applies.
- **Machine placement.** The rule of named accounts, with the card's
  credential Machines in place of native generations: the existing
  placement, else a Machine holding the card's default credentials (an
  enrolled Machine with an active installation of one of the card's
  Providers whose Service credentials are ready; the Service distributes them
  to every enrolled Machine), else, only when the card has neither anywhere,
  the first-Machine chooser.
- **Reset and revocation.** Reset deletes only the card's own `browser/`
  tree on the named Machine. Legacy cards are not revocable through the
  account registry; a named account's revocation plan captures placements by
  its own id and can never capture a card's key, and nothing a card does
  touches a named account's placements. A key whose card is no longer
  collected but still has a placement can be read, stopped and reset, never
  opened.

## Shared browser profiles

An owner may let several cards use one browser profile, for example Claude ·
Lynx, OpenAI · Pika and Grok · Fox, which all sign in with the same Google
account, so that the owner signs in to Google once. By default nothing is
shared: every card keeps its own browser exactly as above.

- **Share.** Owner-scoped: an id (UUID v4), a label, its creation time and a
  revision, in `provider_browser_shares`; membership in
  `provider_browser_share_members (owner, member, share_id, family)`
  (migrations `0082_provider_browser_shares.sql` and sqlite `0056`). A member
  is a browser identity: a named account (`{account_id}`, stored
  `account:<id>`) or a legacy card (`{card}`, stored `card:<card>`). A member
  belongs to at most one share (primary key `(owner, member)`).
- **Family rule (hard).** Families come only from a closed, versioned table
  in Cowboy code (`store::provider_accounts::browser_families`, version 1,
  pinned by a test), never from the mutable Provider catalog: named-account
  Providers `claude-code` → `anthropic`, `codex` → `openai`, `grok` → `xai`,
  `claude-deepseek`, `codex-deepseek` and `deepseek-harness` → `deepseek`;
  legacy cards `anthropic`, `openai`, `xai` and `deepseek` by their id. A
  named account's Provider never changes, so neither does its family. A
  Provider or card outside the table has no family and never joins
  (`browser_share_family_unknown`). The table changes only with a reviewed
  Cowboy change that bumps its version. A share holds at
  most one member per family. This is enforced three times: the unique key
  `(owner, share_id, family)`, the mutation (`browser_share_family_conflict`),
  and resolution, which recomputes every member's and binding's family before
  a share key is used and marks the share blocked when two members would have
  one family, a member's family changed or is unknown, or a binding refuses
  the member: a blocked share is never opened or attached (rather than
  shared), and its admission code is reported; stopping and resetting (the
  remedy) stay possible.
  A member, even one leaving or not in any share, must be the owner's own
  identity (`account_not_found` otherwise).
  So Claude · Lark can never be linked with Claude · Lynx, and two OpenAI
  accounts never share.
- **Family binding (hard).** The profile may keep a member's cookies after it
  leaves, so each family of a share's profile stays bound to the first member
  of that family that joined (`provider_browser_share_bindings`, recorded at
  join). Leaving and revocation keep the binding; another account of that
  family is refused (`browser_share_family_bound`) until the profile is fresh:
  a reset of the share that its Machine confirmed (`owner_request`) while the
  share has no placement on another Machine renews the bindings to the current
  members, and deleting the share removes them (a new share is a new key with
  an empty profile). The bound member itself may rejoin. A confirmed reset
  releases only the bindings of identities that had already left when the
  reset began (recorded with its fence); an identity revoked meanwhile stays
  bound. Bindings record the identity and its family from the table. As a
  defence against a future table change, a binding or member whose recorded
  family no longer equals the table's blocks the share for every member
  (`browser_share_family_drift`) until a confirmed reset re-binds the current
  members, or the share is deleted.
- **Key.** `browser_shares::share_key(owner, share_id)`: the first 16 bytes
  of `sha256("cowboy-browser-share-v1\0" ‖ owner ‖ "\0" ‖ share_id)` as a
  version 8 UUID. Its domain differs from the card keys', so it never equals
  a registry account, a card key or `default`. The Machine keys the share's
  profile, instance, fences and snapshots by it like any account key and
  needs no change.
- **Resolution.** One function (`shares::resolve_key`, used by
  `owned_account`) maps the identity a route names to the key it uses now:
  the share's key for a member, its own key otherwise. Every browser path
  goes through it: open, the state read and `accounts.browser`, attach and
  the viewer's continuing checks, resize, navigate, snapshot, hibernate,
  reset and placements. Observations, memory reclaim, reconnect inventory and
  revocation are keyed by the key itself. Routes are addressed by the
  member's own id; a share key in a route is refused
  (`browser_member_required`). Responses report the instance under the
  member's id (the share key never reaches the owner's views), and commands
  naming that instance are translated back to the key. A state read and a
  reset answer carry `share: {share_id, label, members}`.
- **Authorization** is the member's: its own enablement (the account's
  `browser_enabled` and availability, or the card's enablement row) and the
  Service gate. A member with its own browser disabled can neither open nor
  attach, even while the share runs for another member. The share has no
  admission and no credentials. The share's placement is admitted under the
  owner lock only while the share exists, is not being deleted, the opening
  member is still in it and that member's own browser is admitted
  (`browser_share_changed` otherwise). All members open the same single
  instance; the usual one-viewer takeover applies, and the viewer stays
  recorded as the member whose card attached. A viewer whose member left the
  share, or whose share was deleted, ends at its next check.
- **Placement.** A share has one placement, like an account: once its
  browser lives on a Machine it opens only there, whichever member opens it.
  The admission re-reads the share's placements under the owner lock, so of
  two concurrent first opens on different Machines only one is placed (the
  other gets `browser_not_placed`).
  Its first Machine follows the opening member's credential Machines (else the
  first-Machine chooser). Members' own placements are never reused.
- **Profiles.** A new share starts with a fresh empty profile under its key.
  Cookies are never copied between keys. Members' own profiles are retained
  untouched and are what the member uses again after leaving the share.
- **Leaving, disabling, revoking.** Removing a member, disabling its browser
  or account, or revoking a named account never resets, revokes, hibernates or
  deletes the share's profile: other members depend on it. Each ends the
  viewers that member's card attached (to its own browser or the share's) and
  hibernates only the member's own key. Revocation captures and revokes only
  the account's own placements, and drops its membership and advances the
  share's membership revision in the same transaction (a reset captured
  before is refused). Joining a share also ends the member's viewers and hibernates
  its own browser, which its card no longer reaches. An open whose member
  lost its access while the open was in flight fails without stopping the
  share's browser; the recheck and the cleanup of the member's own browser
  run under the owner's share fence, so a later leave-and-reopen never has
  its now-authorized instance stopped.
- **Reset fence (durable).** A share's reset is recorded on the share
  (`pending_reset`: operation id, Machine, expected profile revision, the
  bindings it may release) under the owner lock, at the membership revision
  the caller observed, before it is dispatched. Until its terminal outcome is
  known, joining, leaving or moving a member of that share is refused
  (`browser_share_reset_pending`), also after a Controller restart; only
  revocation (never refused) and deleting the share pass. The Machine keys a
  reset by its request id (the recorded operation id) and keeps its own
  durable fence for it, so only a terminal answer clears the Controller's:
  `owner_request` (carried out) or a plain `hibernated` (that operation
  already completed) proves it applied, `superseded` (no fence was raised:
  the profile revision moved, possibly by this very reset after a later
  operation replaced its remembered completion) proves it runs no further,
  not that it never ran; `failed` (the Machine retained its fence because
  the unit's exit is unconfirmed), `busy` (another operation holds it), any
  other answer and no answer keep the fence. Only the request's own
  `owner_request` renews the bindings, in the same transaction; any other
  settlement clears the fence without renewing. An unsettled reset (a
  timeout, for example queued behind a slow open, or a retained fence) is
  settled by the reconciler after a grace beyond the request timeout, with
  backoff and after a Controller restart: it sends the same operation again
  (same request id, key and expected revision). The Machine caches only
  settled reset and revocation answers by request id; a `failed` or `busy`
  one is never replayed from its receipt cache, so the retry reaches the
  durable operation state and settles once the Machine recovers.
- **Reset** from a member resets the share's profile, the one the member uses;
  the state read lists the affected members and the viewer's reset
  confirmation names them. Every state read and lifecycle answer carries
  `browser_identity` (`own`, or `share:<share_id>:<membership revision>`), and
  a reset must send it back as `expected_browser`: under a per-owner fence
  that also serializes membership changes, the reset is refused
  (`browser_identity_changed`) when the card joined, left or moved since, so
  it never deletes a profile other than the one its caller saw. The web
  confirmation captures the identity with the target and closes when it
  changes; every panel (account card, account settings, sign-in link) takes
  the sharing metadata from one source and starts over when the card's
  browser identity changes, in the inventory or in a state read. Against a
  Service with shared profiles, Reset needs a successful read that names the
  identity, and the inventory's sharing warning stays until one arrives. Because other members depend on that profile, a
  reset also needs the member's own browser access (available and enabled);
  stopping the share's browser stays possible for any member, as for every
  browser.
- **Deleting a share** removes its memberships at once (members fall back to
  their own keys) and, when the share was ever placed, marks it deleting: its
  viewers end and every placement of its key is revoked through the Machine's
  ordinary `revoke_browser` (stable operation id per owner and share, after
  any in-flight open of that key). Each Machine's confirmation removes that
  placement under the owner lock; after the last one the share is forgotten.
  A pending deletion is retried with backoff (5 s doubling to 5 min, the same
  Machine operation id) while its Machine stays connected, immediately when
  the Machine reconnects, and after a Controller restart (the deleting share
  is durable), so a timed-out revoke or a reboot converges without a human
  step. A deleting share admits no member and no open.
- **Effects.** Viewer endings are owner-scoped (owner and member). A replayed
  `accounts.update` receipt reruns no browser effect, and a deferred
  hibernate checks, for each target under the owner's share fence held
  through that target's command, that the change that caused it still holds
  (the member is still in that share, the browser still disabled).
- **API.** Through `accounts.update`, with the registry revision and receipt:
  `{type: "create_browser_share", label}` (the answer carries `share_id`),
  `{type: "rename_browser_share", share_id, label}`,
  `{type: "delete_browser_share", share_id}` and
  `{type: "set_browser_share", member: {account_id} | {card}, share_id}`
  (`null` leaves). Joining a card needs a card the Service collects.
  `accounts.list` reports `browser_shares: [{share_id, label, created_ms,
  revision, deleting, members: [{member, family, account_id}],
  bound: [{family, account_id}]}]` (renaming does not change the membership
  revision),
  `browser_share_families` (the static table: `{version, providers, cards}`)
  and each legacy card's
  `share_id`; a named account's share is the one listing it as a member.
- **Web.** Account settings has a "Shared browser" section to create, rename
  and delete shares and add or remove members; a candidate whose family is
  already present is shown disabled with the reason. The section appears only
  when the Service's inventory lists `browser_shares` (an older Service offers
  no share mutation). Groups are named like accounts: a new group is
  offered the first animal name (`GROUP_NAMES`) no group or account uses,
  taken when the name field is left empty. A shared card's Browser entry
  says "Group <label> · with <other members>", and the reset confirmation lists the members it affects. Identity-less
  resets are offered only after a loaded inventory proved the Service has no
  shared profiles (never while it loads or failed). A read's identity is
  observed before any incarnation staleness filtering, so a move from a
  share's browser to the card's own re-binds the panel; a lifecycle answer
  naming another identity re-binds it too and leaves Reset waiting for a
  state read that names the identity. The inventory's and the reads' view
  feed one binding per sheet: the first binds, agreement from either source
  never remounts, another identity re-binds once. A restart is confirmed
  for the identity shown; a hibernate answer or confirmation read naming
  another one stops it before the reopen and re-binds, and the reopen
  itself carries `expected_browser`: the Service checks it under the
  owner's share fence, held until the placement is admitted, and refuses a
  changed identity (`browser_identity_changed`). A re-bind aborts
  every request and continuation of the former panel, and the sheet's
  one-shot link navigation and auto-open are consumed above the re-bound
  panel, so they never repeat. A shared reset the Machine carried out
  renews the family bindings and advances the registry revision: the
  Service announces `provider_accounts` to every open client, and the
  panel's own client re-reads its inventory and settings at once.

## Security and privacy

- While the owner is attached, Cowboy forwards their keystrokes, pointer
  events and explicitly copied clipboard text to the Machine-local browser,
  and the browser's display back. The Controller and the web client handle
  that transit; they do not persist it, index it, or extract credentials from
  it, and the viewer examines nothing beyond framing and the clipboard text
  the owner explicitly copies to the device. This is the owner's
  remote-display path to their own browser on their own Machine. The policy
  text in `requirements.md` records it as the owner's accepted interpretation,
  not as established vendor permission.
- Credentials: the native login's OAuth files stay Machine-local and
  native-owned exactly as today. The browser's cookies live in the account's
  published snapshot on the Machine and in the instance's private tmpfs while
  it runs. No other browser instance can reach them; the trusted Machine
  supervisor and the host administrator can, as they can reach every other
  account file (`requirements.md`, trust boundary). An authorization code
  travels only through the existing owner-submitted path: the person pastes
  it into the sign-in dialog.
- Same-user processes on the Machine (other workers) could reach the RFB
  socket through the host path of the sockets directory. The existing
  requirement already states that directory separation is not a security
  boundary against host filesystem access; agent tools execute on their bound
  development host, not the OVH Machine. This is recorded, not claimed
  solved.
- No automation surface exists: no debugging port (and the policy forbids
  enabling one), no WebDriver, no injected code; input is delivered through
  the X server, so pages see ordinary trusted events from a person.

## Test-time prohibitions for the existing accounts

During every test, `Claude · Lynx` (legacy `default`, card `anthropic`: no
`provider_legacy_browsers` row) and `Claude · Lark` (named) keep their
browsers disabled. No step opens, reads, hashes, migrates,
resets or wipes their homes; changes their authentication contract; bypasses
the named-Claude portable-credential fence; enables any credential export;
broadens the AppArmor or userns permissions or relaxes the sysctl; adds
`--no-sandbox`; or exposes a remote-display port to make a test pass.

## Acceptance before any real account

Each row cites the round-1 (R1-n), round-2 (R2-n), round-3 (R3-n), round-4
(R4-n) or round-5 (R5-n) finding it closes.

| Evidence | Findings |
|---|---|
| Socket evidence throughout a run: recursive cgroup scan across every network namespace at ready, navigate, resize, reconnect and every 10 s for TCP listeners, UDP sockets of any kind and abstract Unix sockets; from inside the sealed profile a TCP `listen`, any UDP `socket()`/`sendto` and an abstract `bind`/`connect` are all refused; name resolution still works over TCP (`getent hosts` and a page load inside the sealed profile); admission refuses on a host without the AppArmor features; X auth enforced; private `/tmp`; no abstract X socket; QUIC and non-proxied WebRTC UDP off, with the effective `WebRtcIPHandling`, `BuiltInDnsClientEnabled` and `DnsOverHttpsMode` values read back | R1-1, R2-17, R3-9, R4-12, R5-6 |
| Socket export immutability: after the RFB socket exists, a sealed helper's `unlink`, recreate and regular-file write attempts in `/mnt/sockets` fail; the host `RuntimeDirectory` holds exactly the two sockets throughout; the Machine still connects | R5-2 |
| Control record durability: `browser-state.json` survives reset and revocation and crash injection at every step of both; it is replaced only by temp-file, fsync, rename and parent fsync; an orphan generation left by a crash between rename and pointer update is removed by reconciliation | R5-1, R5-4 |
| Supervisor and namespace init: the unit's `MainPID` survives the PID-namespace fork, `SIGTERM` to the unit reaches the init, the init outlives a graceful stop's copy and publication, and the cgroup is empty afterwards | R5-7 |
| Transition order: the final sealed label, empty outer capability sets and `NoNewPrivs` are verified together on every child on Ubuntu 26.04 with the stock sysctl, and Chromium's namespace sandbox is active (`chrome://sandbox` equivalent evidence from the conformance helper) | R5-5 |
| Freezer recovery: Machine failure injected before the copy, during the copy and before the thaw leaves no frozen browser and no deleted live staging after reconciliation | R5-9 |
| Staging bind after masking: a real post-ready snapshot binds the staging generation through the retained descriptor and publishes; AppArmor audit shows the resolved source matched | R5-3 |
| Graceful shutdown: state written just before `hibernate` is present in the snapshot; a shutdown that overruns the stop timeout is recorded as crash-consistent, not flushed | R5-8 |
| Policy text accepted and accurate about transit, host visibility and the remaining shared channels; Controller stream path has no persistence or logging of payloads (code review + test) | R1-2, R2-7, R3-14, R4-13 |
| Syscall-level canaries from inside a running instance (a conformance helper exec'd under the sealed profile, not the browser UI): `open`, `openat`, `stat`, `rename`, `mount`, `umount` and `pivot_root` against another synthetic account's snapshot, native credentials, the Machine state root and another instance's `RuntimeDirectory` fail, including from a freshly created descendant user+mount namespace that tries to bind one of those onto `/mnt/profile` or `/mnt/sockets`; the mount table shows only the bound aliases; AppArmor audit shows the denials; `file://` and dialog UI tests are additional, not sufficient | R1-3, R2-1, R3-1, R3-3, R4-1 |
| Nested sandbox evidence on pristine Ubuntu 26.04 with the stock sysctl: Chromium's zygote creates its user, PID and network namespaces under the inherited sealed profile; the outer instance namespace's `uid_map` is the single self-mapping; capability sets of the display server, window manager and browser main process are empty with `NoNewPrivs`; sandboxed descendants are identified by namespace ancestry and carry the sealed label | R3-1, R4-10 |
| Two synthetic accounts running at once (`browser.max_instances=2`): no shared path, X display, `/tmp`, `/dev/shm`, IPC object, PID visibility, signal delivery, abstract socket, UDP channel or clipboard between them; a cookie set in one is absent from the other across hibernate and reopen; a sign-in dialog's verification-link entry opens only its own account's browser | core isolation, R3-5, R4-11, R4-12 |
| Mount sequence on a pristine host without Chrome or policy directories installed: `/mnt` aliases, `/usr` and `/opt` from the artifact, `/etc` scaffold, policy file bound at the contract's directory, `/proc` for the PID namespace, bind-before-mask ordering, `/run/user/<uid>` masked; effective-policy evidence from the browser (remote debugging flag refused, download blocked, `file://` blocked, QUIC off) in conformance | R2-12, R3-4, R3-7 |
| Hermetic runtime: `Xtigervnc` finds `xkbcomp` and XKB data, Chromium finds fonts and GSettings, all from the artifact, with no `LD_LIBRARY_PATH`; `ldd` audit of every ELF object against the tree alone; `host_requirements` check and refusal on a host that fails it | R2-13, R3-13 |
| Stream binding and authorization: token single-use, wrong `ConnectionToken`, wrong incarnation, wrong attachment, revoked owner, expired session or step-up during continuous traffic, revoked device, disabled account, gate closed, Origin mismatch all close both hops; takeover detaches only the old attachment; old-connection rejection after Machine reconnection | R1-4, R2-4, R2-6, R2-9, R3-8 |
| Race and fencing tests: a login's start, challenge and end touch no browser instance (`a_login_lifecycle_never_touches_a_browser_instance`); a retired `SignIn` open is refused and touches no instance; Machine restart with a live unit (same and different boot) re-verifies confinement before adoption; delayed hibernate/resize/snapshot/reset/detach for a superseded incarnation, attachment or profile revision; browser OOM during login leaves the native login and generation untouched; cross-account open refused | R1-5, R2-3, R2-4, R2-5 |
| Durable fences and snapshot serialization: crash injection after the fence is written, after the unit is stopped, after deletion but before completion, and during an in-flight snapshot copy and publication, for both reset and revocation; restart resumes the operation, the revision never goes backwards, a stale snapshot never publishes over a reset or replacement profile, abandoned staging is reconciled | R2-2, R3-2, R4-4, R5-1 |
| Snapshot coherence: frozen-copy snapshots under concurrent Chromium writes recover cleanly (SQLite WAL and LevelDB open without error, cookies intact); graceful-stop snapshots capture state written just before the stop; publication is atomic (generation directory plus fsynced `state.json`), the previous generation survives a crash at every point, and the retained generation is the one `state.json` names | R4-2, R4-6 |
| Snapshot contents as untrusted input: a profile containing absolute and relative symlinks, FIFOs, sockets, hard links, sparse files, deep trees and oversized files is copied without following, hanging or expanding beyond the bound, and restored the same way; the Machine's validation rejects the hostile staging tree | R4-3 |
| Restoration and memory accounting: restoring a full-size snapshot charges the unit's cgroup (not the supervisor), the sum of browser plus profile, `/tmp` and `/dev/shm` tmpfs stays below `MemoryMax` under the defaults, no swap is used, and the loss window after a crash equals the state since the last published generation | R4-7 |
| Storage: a page filling IndexedDB faster than any polling interval hits the profile tmpfs bound; a process filling `HOME` or `TMPDIR` hits the instance tmpfs bound; `/run/user/<uid>` free space and unrelated runtime-directory operations are unaffected; the Machine's own state writes keep succeeding; snapshot refused at the staging budget and at the disk low-watermark; concurrent snapshots of two accounts near the watermark never over-commit | R2-10, R3-6, R4-5, R4-9 |
| Snapshot failure versus stops: a stop forced by a confinement failure, deadline, reclaim or `RuntimeMaxSec` completes without a snapshot and reports `unsaved_state_lost`; an explicit hibernate whose snapshot fails reports failure; pending persistence never keeps a unit alive past its authority | R4-8 |
| Mixed-load measurement on the target Machine with simultaneous worker and browser admissions: byte-weighted reservations never over-admit, exact refunds, memory peak, pressure stall, OOM events, session continuity; reclaim hibernates the browser before any session; a browser stop stalled beyond 60 s keeps the fence and blocks session selection instead of settling | R1-6, R2-11, R3-10 |
| Abnormal-exit cleanup: SIGKILL and OOM of the unit, and a deliberately crashing child, with synthetic secrets in the instance tmpfs; no core dump, no swap (`MemorySwapMax=0`), the tmpfs is gone with the unit, the sockets directory removed; the last published generation survives; profile deleted by reset and by revocation with a running browser | R1-7 |
| Attachment lease: a viewer that disappears without Close is detached within 30 s and the instance hibernates after the grace; unpaired streams time out; stale and post-expiry renewals cannot resurrect an attachment | R1-8, R2-8 |
| Clipboard: PRIMARY copies never arrive; explicit action required; nothing is filled into a sign-in dialog; held text cleared on attachment change | R1-9 |
| Resize authority and scale handoff: phone and desktop geometries, DPR 2 on touch, bounded sizes, desktop→phone handoff through "Restart for this screen", pointer mapping verified | R1-10, R2-14 |
| Native touch: the conformance touch checks pass on the exact archive, `--verify-stock` reproduces Ubuntu's binary, and a physical iPhone pinches, scrolls and taps a page natively with no stuck contact after backgrounding and returning | native touch |
| Physical iPhone over the overlay: verification link opened in the account browser, sign-in page completed with keyboard, IME, paste, Mail switch and return, email-code path; code copied back by hand; desktop handoff | R1-11 |
| Enablement limited to the test account; authentication contract, worker generation, artifact, host-policy and lifecycle receipts retained; the separate native-owned lifecycle acceptance remains in force | R1-12 |
| Exact signed archive installed through the real Machine installer on pristine Ubuntu 26.04: archive limits, link rejection, relocation, enforced AppArmor profiles, confinement verification from outside | R2-12, R2-13 |
| Session restore: reopening a snapshot shows the new-tab page and issues no request to a previously open page (local test server) after clean hibernation and after a crash | R3-11 |
| Plugin lifecycle: upgrade and uninstall with a running instance (drain, pinned bytes, retirement after lease release); rollback to an older build of the same major refuses and reset recovers; a crash between the writer record and readiness leaves a consistent record | R2-15, R2-16, R3-12 |

Only after this matrix passes does the test account (`Claude · Test`) use its
Machine browser on OVH (for example to complete a sign-in page opened through
the verification-link entry).

## Rollout

1. Core: Plugin kind, Machine instance manager, protocol 45 stream, routes,
   viewer, settings; gate off. Controller and web release.
2. Plugin: runtime tree built on the builder, conformance on the KVM guest
   with the AppArmor policy, signed and published to the Catalog.
3. Host: AppArmor policy installed on OVH with a receipt.
4. Machine: OVH Machine release through the normal maintenance transaction.
5. Install the Plugin on OVH; enable `accounts.machine_browser`; enable the
   browser for `Claude · Test` only; run the acceptance matrix.
6. Decide per existing account whether and when to enable the browser.

## Implementation notes

Where the Machine code (`components/machine/src/machine_browser/`) refines
the text above:

- The launch ledger is `components/machine/src/launch_ledger.rs`: one
  byte-weighted reservation set, checked and changed under one lock, shared
  by cold worker launches (512 MiB each, until ready) and browser instances
  (their admitted budget, `reserve_within(memory_reserve, memory_max)`),
  with a 512 MiB floor and exact refunds on drop. A browser's reservation
  rides on its `CapacityGuard`, so every path that releases the
  `max_instances` slot on confirmed exit releases the budget with it.
  `Reservation::charge_to` credits the unit cgroup's usage net of
  `inactive_file` against the budget.
- Restart reconciliation reserves a budget for every unit that may still be
  alive, before deciding its fate: a recorded invocation (whether adopted,
  refused at adoption, ending, of the retired sign-in purpose, or
  unobservable) and an unrecorded or replacement invocation. The budget is
  the `MemoryMax` systemd reports for the unit, or `browser.memory_max` when
  it reports none or cannot be observed. Without headroom for it the budget
  is force-reserved with a WARN: its pages are already resident, every later
  admission (browser or worker) must see what it may still grow into, and
  stopping it would only lose unsaved state for accounting. The cgroup
  usage credit applies only when the unit's cgroup is known (recorded, or
  reported by systemd); otherwise the full budget is held. That one
  `CapacityGuard` moves into the adopted instance, or back out of a refused
  adoption into the stop and, when the exit is unconfirmed, its termination
  retry; it is released once, only on confirmed exit. A failed start
  (including a confinement verification failure after the budget was
  admitted) likewise keeps its slot and budget until its stop is confirmed.
- Host pids are never taken from the launcher. The Machine enumerates the
  unit's cgroup tree and matches the launcher's namespace-local pids through
  `/proc/<pid>/status` `NSpid` and `PPid`; the supervisor must be the unit's
  `MainPID`, the relay its child and the init the relay's child (see
  Confinement verification).
- Nested-sandbox ancestry is established through `NSpid` and `browser/`
  membership (above) rather than `NS_GET_PARENT`, which needs raw `ioctl`
  and the crate forbids `unsafe`.
- The stop-time copy travels on `stop {snapshot_generation}`; the launcher
  replies `graceful` (flushed) or not (crash-consistent), and the Machine
  publishes either while marking `unsaved_state_lost` for the latter.
- Restart adoption of a `Tab` instance reconnects to the control socket,
  asks `status`, waits for an in-progress copy, thaws `browser/`, re-takes
  the memory reservation and re-runs confinement verification against the
  recorded unit; records of the retired sign-in purpose are stopped. A
  fenced account is never
  adopted; the fence is completed first.
- A running browser is always treated as potentially dirty: a frozen copy
  never clears that, so a forced stop reports `unsaved_state_lost` unless a
  flushed stop-time copy was published.
- An `Open` carrying the retired `SignIn` purpose answers `hibernated` with
  `not_available` and touches no instance. An owner `navigate` the launcher
  rejects answers `failed` without stopping the instance; one the launcher
  never answers may still land, so the instance stops instead of being
  reused, and a navigation waiting behind it never reaches the launcher.
- Browser commands are dispatched in arrival order; an account's `Open`,
  `Hibernate`, `Reset`, `Revoke` and `Query` run strictly one after another
  in that order (leases, attachments, resizes and navigation do not wait
  behind them), so an open and a revocation sent in that order apply in that
  order.
- Browser-control `Query` for an account with a registered instance answers
  that instance's state (`stopping` while its exit is unconfirmed). Without
  one it never reports absence while a browser may still run for the
  account: a retained incarnation of the account whose exit is unconfirmed
  (a restart-retained record, or a failed start) answers `stopping` with that
  `InstanceRef` and the reason it is being stopped; a unit of unknown
  ownership or incarnation whose exit is unconfirmed, or an incomplete
  restart inventory, answers `stopping` with `instance_id: ""`,
  `incarnation: 0` and reason `busy`. Both carry the account's profile
  revision and current snapshot. A command naming the retained incarnation
  answers the same `stopping` observation. Otherwise `Query`, and every
  `Reset`/`Revoke` answer, is a `hibernated` observation with
  `instance_id: ""` and `incarnation: 0`. A failed start whose exit is
  confirmed later reports that incarnation's terminal `hibernated`.
- `BrowserAttach` replaces any live attachment (takeover) and the old
  stream's close cannot touch the replacement because the attachment id
  differs; lease renewals are bounded to 30 s from now whatever the
  Controller sends.
- Controller (`components/server/src/server/provider_accounts/browser*`):
  the viewer WebSocket requires the `cowboy-browser-v1` subprotocol and the
  `machine_id`, `instance_id`, `incarnation`, `width`, `height`, `scale`
  query (plus `takeover=true` to replace a live attachment); the server
  sends `{type:"state",state}` first and on every
  resize, the product authentication snapshot messages exactly as `/ws`, and
  `{type:"error",code}`; the viewer sends `{type:"lease"}` every 10 s and
  `{type:"resize",width,height}`; everything binary is RFB relayed verbatim.
  `GET .../browser` returns `{machine_id, state, placements, admission}`,
  where `admission` is the refusal code (`machine_browser_not_enabled`,
  `browser_not_enabled`, `account_unavailable`) or null. Opens choose the
  account's placement Machine, else a Machine holding its native generation
  (for a legacy card, its default credentials); an open naming `machine_id`
  uses only that Machine.
  Authority is split: open, attach, navigate, resize and snapshot need launch
  admission; the state read and hibernate need only ownership, and reset
  needs ownership of an account that is not being revoked (`account_revoking`),
  so closing either gate never strands a running instance or a retained
  profile. Withdrawn admission on a live stream sends `{type:"error",code}`
  with the refusal code before closing.
  The Controller's browser cache is keyed by Machine and account (one
  account may have browsers on several Machines) and scoped to the Machine
  connection that reported it. One Machine's report never replaces another's.
  Every instance-scoped request (navigate, resize, snapshot, hibernate) and
  the viewer attach name the Machine and the `InstanceRef` the owner was
  shown (the state read or the open's answer); the Controller forwards them
  only while that Machine's current connection reports exactly that
  instance and incarnation, and otherwise refuses with
  `browser_instance_changed` (409), or `browser_not_running` when that
  Machine reports nothing or the instance is not running for an attach. It
  never substitutes another browser of the account. A reset names its
  Machine; it needs the account's placement there (`browser_not_placed`)
  and its expected profile revision is checked against that Machine's
  profile only, since revisions are per Machine and often equal. Disablement
  and revocation end viewers and hibernate running instances on every
  Machine. The state read prefers a running instance, then a stopping one,
  then the newest answer; with `machine_id` naming a Machine that holds the
  account's placement or observation it reports that Machine instead, so
  the panel keeps reading the browser it shows (and the one an open is
  starting) while it is live. A Machine that answers a query with
  `stopping` keeps the account's browser live: for a restart-retained unit
  whose exit is unconfirmed it names that instance, and while a unit of
  unknown owner is unconfirmed or its startup inventory is pending it names
  no instance (`instance_id: ""`, `incarnation: 0`, reason `busy`). Neither
  is read as absent: placements stay, memory reclaim neither counts it
  exited (on the same or a later connection) nor hibernates it again, an
  answer naming no instance never hides it outside a query, and no instance
  command or attachment is ever sent for an empty instance.
  Per connection: a new connection (a restarted Machine may have discarded its
  instances) drops everything learned before and ends those viewers, the
  connection's teardown does the same, and every durable placement on the
  reconnected Machine is queried again. A state read without a running
  instance on the current connection (a `stopping` one included) asks the
  Machine (`BrowserQuery`), whose
  answer, including "no instance", replaces the cached state; a confirmed
  reset or revocation does the same.
  The Machine-side revocation operation id is `revoke-<sha256(operation)[..32]>`,
  so a retried reconciliation replays the Machine's receipt.

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
  account identifiers; the namespace bind under a sealed profile gives the
  same guarantee with two static profiles.
- A relocatable tree over the host's `/usr` with `LD_LIBRARY_PATH`: fails on
  compiled-in helper paths (`xkbcomp`) and ties the artifact to the host's
  glibc; binding the artifact as the instance's `/usr` removes both problems.
- Live profile on disk with polling-based size checks: cannot bound growth
  between samples; the tmpfs profile with published snapshots gives a
  kernel-enforced bound at the cost of losing changes since the last
  published generation on a crash.
- Freezing the browser and squeezing its cgroup into swap for hibernation:
  more state to reason about than stopping the process, and in conflict with
  `MemorySwapMax=0`; stopping with a published snapshot costs a 3–5 s cold
  start, which is acceptable for a sign-in.

## Open questions

- Whether a second display stream for a `Tab` instance should ever be opened
  by an agent (for example, "show me the page") or stays owner-only. The
  first release is owner-only.
- Whether Codex's device-code approval should be offered in the same browser.
  It needs no browser on the Machine; offering it is a UI choice.
