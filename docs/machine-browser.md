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
| Browser runtime bytes: launcher, Xvnc, window manager, Chromium, helpers, fonts, glibc, managed policy file | `machine_browser` Plugin (signed artifact) |
| Launch graph, readiness protocol, host requirements, declared executables and policy contract | Plugin contract (data) |
| Plugin kind, schema, signing, Catalog, install/upgrade/rollback/uninstall, generation leases | Core (generic Plugin lifecycle) |
| Instance manager: admission, start, attachment leases, hibernation, caps, reconciliation, confinement verification, snapshot publication | Core Machine |
| Per-account profile snapshots, reset, revocation wipe, durable fences, staging budget | Core Machine + Controller revocation plan |
| Display stream, backpressure, binding, continuing authorization | Core wire protocol + Machine + Controller |
| Routes, owner/device/Origin checks, per-account enablement | Core Controller |
| Viewer (canvas, decoder, input, clipboard action, mobile sheet) | Core web |
| Sign-in integration | Core (Machine login session + Controller + web) |
| Resource settings | Core Device/Service configuration |
| Host sandbox policy (AppArmor bootstrap and sealed profiles) | Host configuration (Columbus `machines/ovh`), outside Cowboy |

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
  "host_requirements": {
    "kernel_min": "5.15",
    "merged_usr": true,
    "mount_points": ["/opt", "/mnt"],
    "apparmor_features": ["network_v9", "unix", "mount", "namespaces/userns"],
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
link. Generation retirement treats browser leases like Code leases: a
generation retires only after its last browser lease releases. After the
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
   -SendPrimary=0 -SendCutText=1 -AcceptCutText=1 -geometry WxH -depth 24`.
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
   `QuicAllowed=false`, `WebRtcIPHandling=disable_non_proxied_udp`,
   `BuiltInDnsClientEnabled=false`, `DnsOverHttpsMode=off`,
   `DefaultGeolocationSetting=2`, `DefaultNotificationsSetting=2`,
   `AudioCaptureAllowed=false`, `VideoCaptureAllowed=false`,
   `RestoreOnStartup=5`. With the built-in resolver off, Chromium resolves
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
[Acceptance](#acceptance-before-any-real-account).

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
start and before every new admission. A snapshot that cannot be admitted is
refused, never queued.

Snapshot failure is subordinate to mandatory stops: a graceful stop attempts
the snapshot at most twice within the stop deadline; a stop forced by login
termination, deadline, reclaim, revocation or `RuntimeMaxSec` proceeds
without a snapshot, keeps the last published generation, and reports
`unsaved_state_lost`. An explicit `hibernate` whose snapshot fails reports
the failure rather than success. Pending persistence never extends an
instance's authority to run.

### Durable placement, reset and revocation

The Controller keeps a durable browser placement
`(owner, account, machine, profile_revision, last_open)` written when an open
is admitted. The account revocation plan gains `browsers` beside `logins`,
`grants` and `sessions`, captured from these placements, so a `Tab`-only
placement is revoked like any other target.

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
`browser-state.json` again with the completion. A snapshot job publishes only if the
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
  runtime's. Memory admission goes through the same compare-and-swap launch
  ledger as cold workers, converted to a byte-weighted reservation with
  exact refunds and the existing floor preserved: a browser reserves
  `browser.memory_max` bytes, a cold worker its existing reserve, so a
  browser and a worker can never both count the same headroom. The
  reservation is held until `ready` or failure, then refunded exactly.
- The unit is `systemd-run --user --collect --unit cowboy-browser-<instance>
  --slice cowboy-agents.slice -p Delegate=yes -p KillMode=control-group
  -p MemoryMax=<browser.memory_max> -p MemoryHigh=<browser.memory_high>
  -p MemorySwapMax=0 -p CPUQuota=<browser.cpu_quota> -p TasksMax=<browser.tasks_max>
  -p OOMScoreAdjust=1000 -p LimitCORE=0 -p RuntimeMaxSec=<browser.max_runtime>
  -p TimeoutStopSec=20 -p RuntimeDirectory=cowboy-browser/<instance>
  -p RuntimeDirectoryMode=0700`. The `RuntimeDirectory` holds only the two
  sockets; it is not a storage location, and the Machine stops the instance
  if anything else appears in it.
- `ready` is accepted only after [confinement verification](#confinement-verification).
- `attached` requires a live attachment lease. `detached` starts the
  `browser.hibernate_after` grace (default 120 s); expiry hibernates
  (graceful stop with snapshot). A sign-in-bound instance's deadline is
  `min(login expiry, browser.max_runtime)`.
- Every mutation (`hibernate`, `resize`, `navigate`, `snapshot`, `stop`)
  carries the expected incarnation; attach, detach, resize and stream cleanup
  carry the `attachment_id`; reset carries the expected `profile_revision`.
  A command whose expectation no longer matches is a recorded no-op with
  reason `superseded`. Replays of the same request ID return the original
  receipt.
- Failure never retries with weaker isolation. A namespace or sandbox failure
  reports `sandbox_unavailable` with the host policy pointer; a confinement
  verification failure reports `confinement_unverified`; a missing host
  prerequisite reports `host_requirements`. All leave the instance
  hibernated with the reservation refunded.

### Confinement verification

Before accepting `ready`, before every `navigate`, and before adopting any
surviving unit at restart, the Machine pins each reported pid to its reported
start time and reads from outside the namespace:

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
  and `/run/user/<uid>` tmpfs with the expected flags; `/mnt/snapshot` and
  `/mnt/publish` absent; the policy file bound from the artifact at the
  contract's policy directory;
- the whole cgroup subtree of the unit (`cgroup.procs` recursively, because
  `Delegate=yes` lets children create sub-cgroups): no process owns a
  listening TCP socket or **any** UDP socket, matched by socket inode
  against `/proc/<pid>/net/{tcp,tcp6,udp,udp6}` of each distinct network
  namespace found among those processes, and no process owns an abstract
  Unix socket (`/proc/<pid>/net/unix` entries with an `@` path);
- `/proc/<pid>/mountinfo` also shows `/mnt/sockets` read-only, and the
  `RuntimeDirectory` on the host contains exactly the two sockets.

The socket scan repeats every 10 s while the instance runs and at every
attach, navigate and resize; a violation stops the instance with reason
`listener_detected`. The scan is detection evidence; prevention is the
sealed profile (below: no UDP at all, no TCP bind/listen/accept, no abstract
addresses) plus the browser policy (no QUIC, no non-proxied WebRTC UDP, no
built-in resolver, no remote debugging), and admission refuses when
prevention is unavailable.

### Storage during a run

The profile and instance tmpfs are the storage bounds; `/tmp` and `/dev/shm`
are bounded tmpfs as well; the HTTP cache is bounded by flag; downloads and
file dialogs are disabled by policy. The only host-visible writable location
is the sockets directory, which may hold the two sockets and nothing else.
Snapshots are written only through the staging budget. Nothing the browser
does during a run can write to the shared filesystem.

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
state directory) holds account, Provider, generation digest, purpose and
login request, unit name and systemd invocation ID, boot ID, incarnation,
deadline, `profile_revision`, and the launcher's reported pids, start times
and namespace identities. At Machine start:

- a record whose boot ID differs from the current boot is cleared (the unit
  cannot have survived);
- a record with a live unit of the same invocation ID is adopted as
  `detached` only after the installation, executables and
  `host_requirements` re-verify, the account is still enabled and unfenced,
  the deadline has not passed, a memory reservation is re-taken, and
  [confinement verification](#confinement-verification) passes against the
  recorded identities; otherwise the unit is stopped;
- a live `cowboy-browser-*` unit without a record is stopped;
- abandoned staging directories are reconciled against the staging ledger;
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
operation of that account is in progress) and `login_mismatch` (the named
login is not that account's active native login).

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
| `GET /api/provider-accounts/{account}/browser` | state, geometry, scale, deadline, snapshot generation and time, Machine, reasons |
| `POST /api/provider-accounts/{account}/browser/open` | open as `Tab` (no URL body) |
| `POST /api/provider-accounts/{account}/logins/{request}/browser` | open bound to that sign-in; no URL in the body; owner and account checked against the login receipt |
| `POST .../browser/navigate` | `Tab` instances only, owner-supplied `https` URL, bounded |
| `POST .../browser/resize` | bounded geometry; carries the attachment |
| `POST .../browser/snapshot`, `POST .../browser/hibernate`, `POST .../browser/reset` | lifecycle, with expected incarnation or profile revision |
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
| `browser.memory_max` / `browser.memory_high` | 2048 MiB / 1536 MiB | 1–6 GiB | unit limits, including every tmpfs page; also the admission weight |
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
only on verified unit exit followed by a fresh resource sample. A stop that
stalls does not time out into "released": it stays pending, is logged as
stuck, and blocks session selection on that Machine until the unit is gone;
the only exception is an explicit operator emergency action.
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
auto-bound or otherwise, can exist), denies `bind`, `listen` and `accept`
on every `inet`/`inet6` stream socket, denies every abstract Unix address
(`deny unix addr=@**`), denies `/mnt/snapshot/**`, `/mnt/publish/**`,
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

The browsers of different account cards share no profile, display, temporary,
shared-memory, IPC or socket state, and nothing a page can store in one
account's browser can be read by another's. This is what makes several
accounts on one Machine safe to operate, and it is a requirement of the
design, not a side effect:

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
| Remote clipboard | held per attachment in the viewer, never shared between accounts, dropped on attachment, incarnation or login-request change; the owner's device clipboard is only ever pasted by an explicit action |
| Runtime files (browser, fonts, policy, glibc) | shared read-only artifact; they carry no state |
| Sign-in binding | a sign-in opens only in the browser of the same account as the login, checked under the login lock; the viewer always names the account whose browser it shows |

What stays shared is what is shared for the native CLIs already: the
Machine's egress address, timezone and locale, the runtime version, the
host network namespace, and the Machine user identity, under which the
trusted Machine supervisor and any same-user host process can reach the
sockets directory and the snapshot files (the trust boundary recorded in
`requirements.md`). The design does not manufacture per-account device
differences; separate, persistent, honestly distinct profiles are the
isolation. It prevents automatic inheritance of one account's browser state
by another and binds a sign-in to the account that owns the browser; it
cannot stop the owner from typing one account's upstream credentials into
another account's browser, which is why the viewer always names the account.

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
  published snapshot on the Machine and in the instance's private tmpfs while
  it runs. No other browser instance can reach them; the trusted Machine
  supervisor and the host administrator can, as they can reach every other
  account file (`requirements.md`, trust boundary). The authorization code
  travels only through the existing owner-submitted path.
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

During every test, `Claude · Lynx` (legacy `default`) and `Claude · Lark`
(named) keep `browser_enabled=false`. No step opens, reads, hashes, migrates,
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
| Two synthetic accounts running at once (`browser.max_instances=2`): no shared path, X display, `/tmp`, `/dev/shm`, IPC object, PID visibility, signal delivery, abstract socket, UDP channel or clipboard between them; a cookie set in one is absent from the other across hibernate and reopen; a sign-in open against the other account's login is refused | core isolation, R3-5, R4-11, R4-12 |
| Mount sequence on a pristine host without Chrome or policy directories installed: `/mnt` aliases, `/usr` and `/opt` from the artifact, `/etc` scaffold, policy file bound at the contract's directory, `/proc` for the PID namespace, bind-before-mask ordering, `/run/user/<uid>` masked; effective-policy evidence from the browser (remote debugging flag refused, download blocked, `file://` blocked, QUIC off) in conformance | R2-12, R3-4, R3-7 |
| Hermetic runtime: `Xtigervnc` finds `xkbcomp` and XKB data, Chromium finds fonts and GSettings, all from the artifact, with no `LD_LIBRARY_PATH`; `ldd` audit of every ELF object against the tree alone; `host_requirements` check and refusal on a host that fails it | R2-13, R3-13 |
| Stream binding and authorization: token single-use, wrong `ConnectionToken`, wrong incarnation, wrong attachment, revoked owner, expired session or step-up during continuous traffic, revoked device, disabled account, gate closed, Origin mismatch all close both hops; takeover detaches only the old attachment; old-connection rejection after Machine reconnection | R1-4, R2-4, R2-6, R2-9, R3-8 |
| Race and fencing tests: login completes between reservation and unit registration; Machine restart with a live unit (same and different boot) re-verifies confinement before adoption; delayed hibernate/resize/snapshot/reset/detach for a superseded incarnation, attachment or profile revision; browser OOM during login leaves the native login and generation untouched; cross-account open refused | R1-5, R2-3, R2-4, R2-5 |
| Durable fences and snapshot serialization: crash injection after the fence is written, after the unit is stopped, after deletion but before completion, and during an in-flight snapshot copy and publication, for both reset and revocation; restart resumes the operation, the revision never goes backwards, a stale snapshot never publishes over a reset or replacement profile, abandoned staging is reconciled | R2-2, R3-2, R4-4, R5-1 |
| Snapshot coherence: frozen-copy snapshots under concurrent Chromium writes recover cleanly (SQLite WAL and LevelDB open without error, cookies intact); graceful-stop snapshots capture state written just before the stop; publication is atomic (generation directory plus fsynced `state.json`), the previous generation survives a crash at every point, and the retained generation is the one `state.json` names | R4-2, R4-6 |
| Snapshot contents as untrusted input: a profile containing absolute and relative symlinks, FIFOs, sockets, hard links, sparse files, deep trees and oversized files is copied without following, hanging or expanding beyond the bound, and restored the same way; the Machine's validation rejects the hostile staging tree | R4-3 |
| Restoration and memory accounting: restoring a full-size snapshot charges the unit's cgroup (not the supervisor), the sum of browser plus profile, `/tmp` and `/dev/shm` tmpfs stays below `MemoryMax` under the defaults, no swap is used, and the loss window after a crash equals the state since the last published generation | R4-7 |
| Storage: a page filling IndexedDB faster than any polling interval hits the profile tmpfs bound; a process filling `HOME` or `TMPDIR` hits the instance tmpfs bound; `/run/user/<uid>` free space and unrelated runtime-directory operations are unaffected; the Machine's own state writes keep succeeding; snapshot refused at the staging budget and at the disk low-watermark; concurrent snapshots of two accounts near the watermark never over-commit | R2-10, R3-6, R4-5, R4-9 |
| Snapshot failure versus stops: a stop forced by login termination, deadline, reclaim or `RuntimeMaxSec` completes without a snapshot and reports `unsaved_state_lost`; an explicit hibernate whose snapshot fails reports failure; pending persistence never keeps a unit alive past its authority | R4-8 |
| Mixed-load measurement on the target Machine with simultaneous worker and browser admissions: byte-weighted reservations never over-admit, exact refunds, memory peak, pressure stall, OOM events, session continuity; reclaim hibernates the browser before any session; a browser stop stalled beyond 60 s keeps the fence and blocks session selection instead of settling | R1-6, R2-11, R3-10 |
| Abnormal-exit cleanup: SIGKILL and OOM of the unit, and a deliberately crashing child, with synthetic secrets in the instance tmpfs; no core dump, no swap (`MemorySwapMax=0`), the tmpfs is gone with the unit, the sockets directory removed; the last published generation survives; profile deleted by reset and by revocation with a running browser | R1-7 |
| Attachment lease: a viewer that disappears without Close is detached within 30 s and the instance hibernates after the grace; unpaired streams time out; stale and post-expiry renewals cannot resurrect an attachment | R1-8, R2-8 |
| Clipboard: PRIMARY copies never arrive; explicit action required; edited code never overwritten; held text cleared on attachment change | R1-9 |
| Resize authority and scale handoff: phone and desktop geometries, DPR 2 on touch, bounded sizes, desktop→phone handoff through "Restart for this screen", pointer mapping verified | R1-10, R2-14 |
| Physical iPhone over the overlay: full sign-in flow including keyboard, IME, paste, Mail switch and return, email-code path, desktop handoff | R1-11 |
| Enablement limited to the test account; authentication contract, worker generation, artifact, host-policy and lifecycle receipts retained; the separate native-owned lifecycle acceptance remains in force | R1-12 |
| Exact signed archive installed through the real Machine installer on pristine Ubuntu 26.04: archive limits, link rejection, relocation, enforced AppArmor profiles, confinement verification from outside | R2-12, R2-13 |
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

## Implementation notes

Where the Machine code (`components/machine/src/machine_browser/`) refines
the text above:

- The launch ledger is `components/machine/src/launch_ledger.rs`: one
  byte-weighted compare-and-swap reservation shared by cold worker launches
  (512 MiB each) and browser opens (`browser.memory_max`), with a 512 MiB
  floor and exact refunds on drop.
- Host pids are never taken from the launcher. The Machine enumerates the
  unit's cgroup tree and matches the launcher's namespace-local pids through
  `/proc/<pid>/status` `NSpid`; the supervisor must be the unit's `MainPID`
  and the init the only other process in the unit's root cgroup.
- Nested-sandbox ancestry is established through `NSpid` and `browser/`
  membership (above) rather than `NS_GET_PARENT`, which needs raw `ioctl`
  and the crate forbids `unsafe`.
- The stop-time copy travels on `stop {snapshot_generation}`; the launcher
  replies `graceful` (flushed) or not (crash-consistent), and the Machine
  publishes either while marking `unsaved_state_lost` for the latter.
- Restart adoption of a `Tab` instance reconnects to the control socket,
  asks `status`, waits for an in-progress copy, thaws `browser/`, re-takes
  the memory reservation and re-runs confinement verification against the
  recorded unit; sign-in instances are stopped. A fenced account is never
  adopted; the fence is completed first.
- `BrowserAttach` replaces any live attachment (takeover) and the old
  stream's close cannot touch the replacement because the attachment id
  differs; lease renewals are bounded to 30 s from now whatever the
  Controller sends.
- Controller (`components/server/src/server/provider_accounts/browser*`):
  the viewer WebSocket requires the `cowboy-browser-v1` subprotocol and the
  `width`, `height`, `scale` query (plus `takeover=true` to replace a live
  attachment); the server sends `{type:"state",state}` first and on every
  resize, the product authentication snapshot messages exactly as `/ws`, and
  `{type:"error",code}`; the viewer sends `{type:"lease"}` every 10 s and
  `{type:"resize",width,height}`; everything binary is RFB relayed verbatim.
  `GET .../browser` returns `{machine_id, state, placements, admission}`,
  where `admission` is the refusal code (`machine_browser_not_enabled`,
  `browser_not_enabled`, `account_unavailable`) or null. Opens choose the
  account's placement Machine, else a Machine holding its native generation.
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
