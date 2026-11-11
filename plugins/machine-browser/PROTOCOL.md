# `cowboy-browser-control-v1` and the launcher contract as implemented

This is the wire contract between the Machine (`components/machine/src/machine_browser/launcher.rs`)
and `cowboy-browser-launcher` 1.0.5, the static supervisor shipped in the
runtime artifact at `usr/libexec/cowboy-browser-launcher`. It records what the
launcher does, in the order it does it, so the Machine, the host `AppArmor`
policy and the conformance run can be checked against one text. The design
intent is in `docs/machine-browser.md` ("Launcher contract").

## Invocation

```text
cowboy-browser-launcher --probe
cowboy-browser-launcher run <flags>
```

`--probe` prints one line, `{"protocol":"cowboy-browser-control-v1","version":"1.0.5"}`,
and exits 0. It is the artifact probe (`{"args":["--probe"],"timeout_ms":5000}`).

`run` takes a fixed flag vector; every value is validated before anything
forks, and an invalid vector exits 2 with the usage text on stderr.

| Flag | Required | Meaning |
|---|---|---|
| `--control-dir DIR` | yes | Host directory that holds `control.sock` and `rfb.sock`: the unit's `RuntimeDirectory`. Absolute, normalized. |
| `--runtime-dir DIR` | yes | Extracted artifact root containing `usr/` and `opt/`. |
| `--staging-dir DIR` | yes | The account's `browser/staging` root; snapshot copies go to `gen-<n>` below it. Opened as an `O_PATH` descriptor before anything is masked. |
| `--snapshot-dir DIR` | no | Published generation to restore into the profile (read-only). Opened as an `O_PATH` descriptor before masking. |
| `--display :N` | yes | X display number, 0–9999. |
| `--geometry WxH` | yes | Framebuffer, 320x480 ≤ WxH ≤ 4096x4096. |
| `--scale N` | yes | 1 or 2; Chromium's `--force-device-scale-factor`. |
| `--profile-bytes N` | yes | Size of the profile tmpfs, of `/dev/shm`, and the byte bound of every snapshot copy (1 MiB – 64 GiB). |
| `--instance-bytes N` | yes | Size of the instance tmpfs (`HOME`, `XDG_*`, logs). |
| `--tmp-bytes N` | no | Size of the private `/tmp`; default 268435456 (256 MiB). |
| `--policy-file REL` | yes | Artifact-relative managed policy file (`usr/share/cowboy-browser/policy.json`). |
| `--policy-directory ABS` | yes | The browser build's managed-policy directory (`/etc/opt/chrome/policies/managed`). |
| `--browser REL`, `--display-server REL`, `--window-manager REL`, `--xrandr REL`, `--xkbcomp REL`, `--xauth REL` | yes | Artifact-relative executables. They must exist as regular files under `--runtime-dir`; `xkbcomp` is only verified (Xtigervnc runs it at its compiled path `/usr/bin/xkbcomp`). |
| `--sealed-profile NAME` | no | `AppArmor` profile every child transitions to (`cowboy-browser-sealed`). Absent: no transition (conformance without `AppArmor`). |
| `--cgroup-dir DIR` | no | The unit's cgroup directory. Absent: derived from `/proc/self/cgroup` (`0::/...`) under `/sys/fs/cgroup`. |
| `--mask PATH` | no, repeatable | Extra directories hidden behind an empty read-only tmpfs (for example the Machine state root). A missing path is skipped. |

Relative flags accept only normal components (no `/` prefix, `.`, `..`,
trailing `/`); absolute flags must be normalized too.

Exit status of the unit: `0` after a `stop` or a `SIGTERM`-initiated stop;
the browser's own exit code (or 128 + signal) when the browser exited
unexpectedly; `1` for an internal failure; `2` for a missing host
prerequisite; `3` for a sandbox failure (namespaces, mounts, `AppArmor`
transition).

## Processes

```text
supervisor (MainPID, host namespaces)
└─ namespace relay (unshare: user, mount, pid, ipc; writes the self-only maps)
   └─ init (PID 1 of the instance; mounts, control socket, children, snapshots)
      ├─ display/  cgroup: xauth (once), Xtigervnc, matchbox-window-manager, xrandr helpers
      └─ browser/  cgroup: chrome and its descendants, navigation helpers
```

- The supervisor binds `<control-dir>/control.sock` (mode 0600) **before**
  any isolation step, blocks `SIGTERM`/`SIGINT`/`SIGHUP`/`SIGCHLD`, and forks
  the namespace relay. It forwards every termination signal to its child and
  exits with the child's status. The relay does the same for the init. A
  failure at any stage, including `unshare` itself, is therefore reported
  over the socket as an `error` event (the failing process waits up to 5 s
  for a client) before the unit exits.
- The relay writes `deny` to `/proc/self/setgroups`, then `gid gid 1` to
  `gid_map` and `uid uid 1` to `uid_map` (the Machine user's ids), then forks
  the init. The init sets `PR_SET_PDEATHSIG=SIGKILL` so the instance dies
  with its relay.
- The init derives the unit cgroup, creates `display/` and `browser/` below
  it, and places every child there before sealing it. The copier forked for a
  `snapshot` command stays in the unit's own cgroup, like the init.
- `SIGTERM` to the init runs the graceful stop below with a 10 s grace and no
  snapshot, then exits 0.
- The init reaps every child, including processes reparented to it. The
  browser exiting unexpectedly produces an `exited` event, the display is
  shut down and the init exits with the browser's status. The display server
  or window manager exiting unexpectedly produces an `exited` event and exit
  status 1.

## Mount table (step 2), in order

All mounts happen in the init, after `mount --make-rprivate /`. `ro` binds
are a bind followed by a `MS_REMOUNT|MS_BIND|MS_RDONLY|MS_NOSUID|MS_NODEV`
remount that keeps the flags the kernel locks (`noexec`, atime). Binds whose
source may be masked later use the retained descriptor through
`open_tree(OPEN_TREE_CLONE|AT_EMPTY_PATH)` + `move_mount`.

| Target | Mount | Flags / options |
|---|---|---|
| `/mnt` | tmpfs | `nosuid,nodev,noexec,size=1m,mode=0755`; alias directories `runtime profile instance sockets snapshot publish` created inside |
| `/mnt/runtime` | bind of `--runtime-dir` | `ro,nosuid,nodev` |
| `/usr` | bind of `/mnt/runtime/usr` | `ro,nosuid,nodev` |
| `/opt` | bind of `/mnt/runtime/opt` | `ro,nosuid,nodev` |
| `/mnt/profile` | tmpfs | `nosuid,nodev,size=<profile-bytes>,mode=0700` |
| `/mnt/instance` | tmpfs | `nosuid,nodev,size=<instance-bytes>,mode=0700` |
| `/mnt/sockets` | bind of `--control-dir` | rw until the RFB socket exists, then remounted `ro,nosuid,nodev` |
| `/mnt/snapshot` | bind of the `--snapshot-dir` descriptor | `ro,nosuid,nodev`; the copier restores it into `/mnt/profile`; unmounted (`MNT_DETACH`) afterwards and the descriptor closed |
| `/tmp` | tmpfs | `nosuid,nodev,size=<tmp-bytes>,mode=1777` |
| `/dev/shm` | tmpfs | `nosuid,nodev,size=<profile-bytes>,mode=1777` |
| `/proc` | proc | `nosuid,nodev,noexec` (the new pid namespace) |
| `/etc` | tmpfs | `nosuid,nodev,noexec,size=1m,mode=0755`, populated then remounted `ro` |
| `/etc/{hosts,localtime,passwd,group,machine-id}` | bind of the host file descriptor (symlinks followed at open) | `ro,nosuid,nodev`; each only when the host has it |
| `/etc/ssl` | bind of the host directory descriptor | `ro,nosuid,nodev`; only when the host has it |
| `/etc/resolv.conf` | regular file | the host's `nameserver` lines and `options use-vc` |
| `/etc/nsswitch.conf` | regular file | copy of the artifact's `usr/share/cowboy-browser/nsswitch.conf` |
| `<policy-directory>/policy.json` | bind of `/mnt/runtime/<policy-file>` | `ro,nosuid,nodev` |
| `/home`, `/run/user/<uid>`, every `--mask` | tmpfs | `ro,nosuid,nodev,noexec,size=4096,mode=0755` (empty) |
| `/mnt/publish` | bind of `<staging>/gen-<n>` through the staging descriptor | rw, only for the duration of one snapshot copy, then `MNT_DETACH` |

Nothing uses `LD_LIBRARY_PATH`; executables are invoked by their artifact
paths, which are the real `/usr` and `/opt` paths inside the instance.

## Children (steps 3–6)

Every child is forked by the init and, before `execve`: restores the default
signal mask; `dup2`s `/dev/null` onto stdin and its log file
(`/mnt/instance/log/<name>.log`) onto stdout and stderr (every other
descriptor is `CLOEXEC`); `chdir /mnt/instance`; writes its pid into its
sub-cgroup's `cgroup.procs`; when `--sealed-profile` is set, writes
`changeprofile <name>` to `/proc/self/attr/apparmor/current` (fallback
`/proc/self/attr/current`); drops every capability from the bounding set
(including numbers above the ones rustix names, up to
`/proc/sys/kernel/cap_last_cap`); clears the ambient set; clears the
effective, permitted and inheritable sets; sets `PR_SET_NO_NEW_PRIVS`. A
failure before exec is reported back through a `CLOEXEC` pipe; an `AppArmor`
transition failure is classified `sandbox`, every other start failure `host`.

1. `xauth -q -f /mnt/instance/xauth add :N . <32 hex chars from getrandom>`
   in `display/`; the launcher pre-creates the 0600 authority file.
2. `Xtigervnc :N -auth /mnt/instance/xauth -nolisten tcp -nolisten local -rfbport -1 -rfbunixpath /mnt/sockets/rfb.sock -rfbunixmode 0600 -SecurityTypes None -AcceptSetDesktopSize=0 -SendPrimary=0 -SendCutText=1 -AcceptCutText=1 -AcceptTouchEvents=1 -geometry WxH -depth 24`
   in `display/`. The init waits (15 s) for both `/mnt/sockets/rfb.sock` and
   `/tmp/.X11-unix/XN`, then remounts `/mnt/sockets` read-only. The server
   compiles its keymap by running `xkbcomp` through `/bin/sh`, which the
   artifact provides (`dash`); the compiled keymap lands in the private
   `/tmp`.
3. `matchbox-window-manager -use_titlebar no -use_cursor yes` in `display/`.
4. Browser in `browser/`:
   `--user-data-dir=/mnt/profile --no-first-run --no-default-browser-check --disable-sync --disable-background-networking --disable-component-update --disable-extensions --disable-breakpad --disable-crash-reporter --disable-quic --disk-cache-size=1048576 --password-store=basic --lang=en-US --window-position=0,0 --window-size=W,H --force-device-scale-factor=<scale> about:blank`.

Environment of the display server, window manager and `xauth`/`xrandr`
helpers:
`PATH=/usr/bin HOME=/mnt/instance/home TMPDIR=/tmp XDG_RUNTIME_DIR=/mnt/instance/run DISPLAY=:N XAUTHORITY=/mnt/instance/xauth FONTCONFIG_FILE=/mnt/instance/fonts.conf LANG=en_US.UTF-8 TZ=<tz>`.

Environment of the browser and navigation helpers, exactly:
`PATH=/usr/bin FONTCONFIG_FILE=/mnt/instance/fonts.conf GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas DISPLAY=:N XAUTHORITY=/mnt/instance/xauth HOME=/mnt/instance/home XDG_CONFIG_HOME=/mnt/instance/home/.config XDG_CACHE_HOME=/mnt/instance/home/.cache XDG_RUNTIME_DIR=/mnt/instance/run TMPDIR=/tmp TZ=<tz> LANG=en_US.UTF-8 SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt`.

`<tz>` is the host's `TZ` variable when set and printable; else the zone
name `/etc/localtime` links to (the part after `zoneinfo/`); else the
contents of `/etc/timezone`; else `:/etc/localtime` when that file exists
(it is bound into the scaffold); else `UTC`. `fonts.conf` names
`/usr/share/fonts` and the cache directory `/mnt/instance/fontconfig`.

## Transport

- Unix stream socket `<control-dir>/control.sock`, mode 0600, created by the
  supervisor; the same inode is `/mnt/sockets/control.sock` inside.
- Newline-delimited JSON objects, UTF-8, one per line, at most 262144 bytes
  per line in either direction. A client that exceeds the bound is dropped.
- One client at a time. When the client disconnects the launcher accepts the
  next one; messages produced while no client was connected (`ready`, a
  `snapshot_done` for a copy whose requester left, `error`) are delivered
  to the next client in order. `ready` is sent once; a reconnecting Machine
  uses `status`.
- Client lines are commands and carry an `id` (a JSON integer, or a string of
  at most 128 bytes) that the reply echoes. A line that is not a valid
  command gets `{"id":<id>,"ok":false,"error":"...","kind":"internal"}` when
  it had an id and is logged otherwise.
- Launcher lines without `id` are events.

## Launcher → client

### `ready` and the `status` reply

```json
{"event":"ready",
 "pids":{"supervisor":4242,"init":1,"display_server":7,"window_manager":9,"browser":12},
 "start_times":{"init":987654,"display_server":987701,"window_manager":987733,"browser":987760},
 "namespaces":{"user":"user:[4026532641]","mnt":"mnt:[4026532642]","pid":"pid:[4026532644]","ipc":"ipc:[4026532643]"},
 "uid_map":"1000 1000 1","gid_map":"1000 1000 1",
 "labels":{"init":"cowboy-browser-bootstrap (enforce)","sealed":"cowboy-browser-sealed (enforce)"},
 "mounts":[{"target":"/usr","source":"/dev/nvme0n1p2[/state/plugins/.../usr]","fstype":"ext4","options":"ro,nosuid,nodev,relatime,errors=remount-ro"}],
 "snapshot_in_progress":false}
```

- `pids.supervisor` is the supervisor's pid in its own (host) pid namespace;
  every other pid is as the init sees it (the init is 1). A process that has
  exited is reported as 0.
- `start_times` are field 22 of `/proc/<pid>/stat` (clock ticks since boot),
  identical from every pid namespace, so the Machine can pin host pids to
  them.
- `namespaces` are `readlink("/proc/self/ns/*")` of the init.
- `uid_map` / `gid_map` are the init's maps with padding collapsed to single
  spaces (one triple per line).
- `labels.init` is the init's `/proc/self/attr/current`, trimmed; on a kernel
  without `AppArmor` (the read fails) it is `"unconfined"`. `labels.sealed`
  is the label read from the browser process when `--sealed-profile` was
  given, `null` otherwise.
- `mounts` lists the entries of the init's `/proc/self/mountinfo` whose
  target is one of the mount points in the table above (plus every masked
  path), in mount-id order. `source` is the mountinfo source, with the bound
  subtree appended as `[/path]` when the mount root is not `/`. `options` is
  the per-mount option string followed by the superblock options other than
  `rw`/`ro` (so tmpfs `size=`/`mode=` are visible).
- `snapshot_in_progress` is true while a forked copier is running.

The `status` reply is the same object with `"id"` and `"ok":true` instead of
`"event"`.

### Other events

| Event | Shape | When |
|---|---|---|
| `exited` | `{"event":"exited","process":"browser"\|"display_server"\|"window_manager","code":<int or null>}` | the process ended while no stop was in progress; `code` is the exit code, or 128 + signal |
| `snapshot_done` | `{"event":"snapshot_done","generation":n,"bytes":b,"entries":e}` | a `snapshot` copy finished after its requester disconnected (the next client learns the outcome) |
| `error` | `{"event":"error","kind":"sandbox"\|"host"\|"internal","message":"..."}` | a fatal failure before readiness, or a failed detached copy (`kind` `internal`, message prefixed `snapshot gen-n failed:`) |

`kind`: `sandbox` for namespace creation, id maps, every mount and remount
and the `AppArmor` transition; `host` for missing host prerequisites
(cgroup directory, staging or snapshot directory, runtime files, a helper or
the display server failing to start or to open its sockets, `xauth`);
`internal` for everything else.

## Client → launcher

Every command is an object with `id` and `command`. Replies are
`{"id":..,"ok":true,...}` or `{"id":..,"ok":false,"error":"...","kind":"..."}`.

| Command | Request | Reply | Semantics |
|---|---|---|---|
| `status` | `{"id":1,"command":"status"}` | the status object above | current facts |
| `attach` | `{"id":2,"command":"attach","attachment_id":"a1"}` | `{"id":2,"ok":true}` | bookkeeping only |
| `detach` | `{"id":3,"command":"detach","attachment_id":"a1"}` | `{"id":3,"ok":true}` | bookkeeping only |
| `navigate` | `{"id":4,"command":"navigate","url":"https://..."}` | `{"id":4,"ok":true}` | `url` must be `http://` or `https://`, ≤ 2048 bytes, printable ASCII without quotes or backslashes. The browser executable is re-invoked in `browser/` with the browser's environment and `--user-data-dir=/mnt/profile <url>`; Chromium's singleton hands the URL to the running instance. The reply waits (20 s) for the helper to exit 0. |
| `resize` | `{"id":5,"command":"resize","width":1024,"height":700}` | `{"id":5,"ok":true}` | bounds 320x480..4096x4096. `xrandr --query` names the connected output; when the output has no `WxH` mode, `xrandr --newmode WxH 0 W 0 0 0 H 0 0 0` and `xrandr --addmode <output> WxH`; then `xrandr --output <output> --mode WxH`. All helpers run sealed in `display/`. |
| `snapshot` | `{"id":6,"command":"snapshot","generation":7}` | `{"id":6,"ok":true,"bytes":b,"entries":e}` | binds `<staging>/gen-7` (must exist and be owned by the Machine user) at `/mnt/publish` through the retained staging descriptor, forks a copier that copies `/mnt/profile` into it with the rules below (fsync of every file and directory), unmounts `/mnt/publish`, replies. Refused (`internal`) while another copy runs or the instance is stopping; a missing generation directory is `host`. The Machine freezes `browser/` around this command for a crash-consistent copy. |
| `stop` | `{"id":8,"command":"stop","graceful_ms":5000}` or `{"id":8,"command":"stop","graceful_ms":5000,"snapshot_generation":9}` | `{"id":8,"ok":true,"graceful":true}`, or with `"snapshot":{"bytes":b,"entries":e}`, or with `"snapshot_error":"..."` | `SIGTERM` to the browser main process only; wait until the `browser/` cgroup is empty or `graceful_ms` (capped at 600000) elapses, then `SIGKILL` what is left (`cgroup.kill`); `graceful` is whether the cgroup drained before the deadline. When `snapshot_generation` is present the profile is then copied into `<staging>/gen-<n>` exactly like `snapshot` (inline, before anything else), whether or not the wait was graceful; the Machine classifies the copy as flushed or crash-consistent from `graceful`. A copy failure still completes the stop and is reported as `snapshot_error`. After the reply the window manager and display server receive `SIGTERM`, leftovers are killed after 2 s, and the init exits 0. This is the primary snapshot path for every graceful hibernate. |

An in-flight `snapshot` copy is cancelled by `stop` (its requester, if still
connected, gets an `internal` error reply).

## Snapshot copies (restore and publish)

The copier opens every entry descriptor-relative with `O_NOFOLLOW`, copies
only regular files (mode 0600) and directories (mode 0700), re-checks inode
and device after opening, preserves holes with `SEEK_DATA`/`SEEK_HOLE`
(`copy_file_range` with a read/write fallback), and stops with an error when
the copy would exceed `--profile-bytes` logical bytes, 100000 entries or
depth 32. Symlinks, sockets, FIFOs and device nodes are never copied. Skipped
by basename at any depth: directories `Cache`, `Code Cache`, `GPUCache`,
`ShaderCache`, `GrShaderCache`, `DawnCache`, `DawnGraphiteCache`,
`DawnWebGPUCache`, `GraphiteDawnCache`, `blob_storage`, `Crashpad`; files
`SingletonLock`, `SingletonSocket`, `SingletonCookie`, `lockfile`. Everything
else is copied, including `*-journal`, `*-wal`, LevelDB `MANIFEST-*`, `LOG`,
`*.ldb` and `*.log`. Restoration (`--snapshot-dir` into `/mnt/profile`)
applies the same rules without fsync.

## Notes for the host `AppArmor` policy

- The bootstrap profile must allow the launcher to create and bind
  `control.sock` in the unit's `RuntimeDirectory` through its host path
  (the supervisor binds it before the `/mnt/sockets` alias exists) and to
  `accept` on it from the init.
- Beyond the design's enumerated mounts, the launcher remounts the `/etc`
  tmpfs read-only after populating it, binds the five host `/etc` files and
  `/etc/ssl` from descriptors (the resolved host paths are what `AppArmor`
  sees), and mounts every mask as a read-only tmpfs.
- The sealed children keep `/mnt/instance/log/<name>.log` open for writing.
