# Machine browser: owner-operated browser on a Machine

Status: design, 2026-11-08. Nothing below is implemented. The first design
review (Astra Max, 2026-11-08) raised twelve findings; every one is folded in
and listed in [Acceptance](#acceptance-before-any-real-account). The owner
decided the policy boundary on the same day; see the
[credential rule](requirements.md#named-provider-accounts-isolation-acceptance)
amendment.

## Problem

A named Claude sign-in runs `claude auth login` on the account's Machine
(today OVH), but the authorization page opens on the owner's own device.
The vendor therefore sees the sign-in from one network identity, timezone
and browser, and every later API call from another. The owner's device
browser also shares one claude.ai cookie store across accounts, which the
sign-in dialog already warns about (`web/src/SubscriptionAuthorization.tsx`).
The sign-in page is behind an interactive Cloudflare challenge, which an
automated browser cannot pass and a person can.

The owner wants the sign-in browser to run where the credentials will be used,
operated from the phone or desktop through Cowboy, without a resident process
and without any automation signature.

## Goal and non-goals

Goal: an optional Plugin gives a Linux Machine a real, unmodified Chromium on
its own X display, owned per Provider account, started on demand, streamed
into the Cowboy web UI (phone and desktop) over the existing Machine
connection, and stopped when idle. The named-account sign-in flow can open
the challenge URL in that browser. Its network identity, timezone, locale and
DNS are the Machine's, because the browser is simply a process on the Machine.

Non-goals:

- a general remote desktop, or any listener on the Machine;
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
precedent: the engine is a Plugin, the Code product is core.

| Concern | Owner |
|---|---|
| Browser runtime bytes: Xvnc, window manager, Chromium, fonts, launcher | `machine_browser` Plugin (signed artifact) |
| Launch graph, readiness, platform matrix, declared executable paths | Plugin contract (data) |
| Plugin kind, schema, signing, Catalog, install/upgrade/rollback/uninstall | Core (generic Plugin lifecycle, unchanged) |
| Instance manager: start, viewer lease, hibernate, caps, reconciliation | Core Machine |
| Per-account profile placement and wipe | Core Machine (account directory) |
| Stream channel, backpressure, binding, revocation | Core wire protocol + Machine + Controller |
| Routes, owner/device/Origin checks, per-account enablement | Core Controller |
| Viewer (canvas, decoder, input, clipboard action, mobile sheet) | Core web |
| Sign-in integration | Core (Machine login session + Controller + web) |
| Resource settings | Core Device/Service configuration |
| Host sandbox policy (AppArmor) | Host configuration (Columbus `machines/ovh`), outside Cowboy |

Without the Plugin installed on a Machine, every core piece is inert: the
Machine advertises no browser capability, routes answer `not_available`, and
the UI shows nothing.

## `machine_browser` Plugin kind

Release schema 6, payload schema 1, Plugin SDK 1.13+. Catalog readers before
this kind accept release schemas up to 5 and skip these envelopes. The kind is
Machine-installable, Linux-only, and carries one runtime artifact per declared
platform. Like `code_intelligence`, the contract is a closed launch graph; it
has no UI, URLs, credentials or network targets.

```json
{
  "schema_version": 1,
  "id": "machine-browser",
  "version": "1.0.0",
  "display_name": "Machine browser",
  "description": "Owner-operated Chromium on the Machine for account sign-in.",
  "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
  "display": {"kind": "xvnc_rfb_unix_v1"},
  "browser": {"kind": "chromium"},
  "launch": {
    "command": {"kind": "runtime_path", "path": "bin/cowboy-browser-launcher"},
    "arguments": [
      {"kind": "literal", "value": "--instance-dir"}, {"kind": "instance_directory"},
      {"kind": "literal", "value": "--profile-dir"}, {"kind": "profile_directory"},
      {"kind": "literal", "value": "--control-socket"}, {"kind": "socket"}
    ]
  },
  "readiness": {"kind": "control_socket_ready", "timeout_ms": 20000},
  "executables": {"browser": "opt/chromium/chrome"}
}
```

- `instance_directory` is a fresh 0700 tmpfs directory per run (X auth
  cookie, X and RFB sockets, control socket, temporary files). It is removed
  when the run ends.
- `profile_directory` is the persistent per-account profile described below.
- `executables.browser` is the runtime-relative path of the sandboxed browser
  binary. Cowboy does not use it; the host policy documentation does, so the
  AppArmor attachment glob can be derived from the Plugin instead of guessed.
- The runtime artifact is a relocatable, symlink-free tree within the
  existing limits (1 GiB compressed, 4 GiB expanded). A Nix closure is not a
  portable Plugin artifact (`architecture/14-zed-code-provider.md`); the
  tree is assembled on a builder from distribution packages (Chromium or
  Google Chrome, TigerVNC `Xvnc`, matchbox window manager, fonts) with a
  launcher that sets `LD_LIBRARY_PATH`, `FONTCONFIG_FILE`, `XKB` and
  `GSETTINGS` paths inside the tree. The builder's conformance test runs the
  tree on a pristine Ubuntu 26.04 guest with the production AppArmor policy.
- Chromium security releases ship as new Plugin versions through the ordinary
  per-Machine install/upgrade/rollback lifecycle, independent of Machine host
  releases.

### Launcher contract

The launcher owns three processes and one control socket. It:

1. creates an X authorization cookie in the instance directory and starts
   `Xvnc` with `-auth <cookie> -nolisten tcp -rfbport -1
   -rfbunixpath <instance>/rfb.sock -rfbunixmode 0600 -SecurityTypes None
   -AcceptSetDesktopSize=0 -SendPrimary=0 -SendCutText=1 -AcceptCutText=1
   -geometry WxH -depth 24`. No TCP listener exists in any mode; the Unix
   socket is readable only by the Machine user; resizing is a launcher
   operation, not a client request;
2. starts the window manager, which keeps the single browser window
   fullscreen and following the screen size;
3. starts Chromium with `--user-data-dir=<profile> --no-first-run
   --no-default-browser-check --disable-sync --disable-background-networking
   --disable-component-update --disable-extensions --disable-breakpad
   --disable-crash-reporter --disk-cache-size=1048576 --password-store=basic
   --lang=en-US --window-position=0,0 --window-size=WxH
   --force-device-scale-factor=<scale> about:blank`, with the environment
   cleared except `PATH`, locale and TLS variables, `HOME` and `XDG_*`
   pointing into the instance directory, `TMPDIR` inside it, and `TZ` from
   the host. No proxy variables are inherited or set;
4. verifies after start, before any navigation, that no process in its
   cgroup owns a listening TCP or UDP socket, by matching the unit's
   `/proc/<pid>/fd` socket inodes against `/proc/net/{tcp,tcp6,udp,udp6}`.
   A listener fails the start;
5. serves a newline-delimited JSON control socket for the Machine:
   `navigate {url}` (opens the URL by re-invoking Chromium against the running
   profile, which hands the URL to the existing instance through its singleton
   socket; no debugging port), `resize {width,height}` (through `xrandr` on
   the display, bounded by the launcher to 320×480…2560×1600), `attach`
   and `detach` (whether a viewer is connected, for its own idle accounting),
   and `stop`;
6. exits when any of the three processes exits. The unit's `KillMode=control-group`
   ends the rest.

## Instance manager (core Machine)

### Identity and placement

```text
BrowserInstance = Machine + Plugin generation digest + account_id + instance_id + incarnation
```

One instance per `(Machine, account)`; `browser.max_instances` (default 1)
bounds concurrent running instances per Machine and the owner-side cap is the
owner's enabled accounts. The profile lives in the account's own directory,
`provider-auth/accounts/<uuid>/browser/profile`, created with
`prepare_private_directory` (0700, no symlink following), so it has the same
owner and lifecycle as the native credentials: an account revocation wipe
removes it, and `accounts/<other>` is never readable through Cowboy paths. The
profile is intentionally persistent: the vendor sees one device over time
instead of a new device at every sign-in, Cloudflare clearance survives, and
nothing in the profile is Cowboy data. "Reset browser profile" deletes it
explicitly. A profile larger than `browser.profile_max_bytes` refuses to start
until reset.

### Lifecycle

```text
hibernated ──open──▶ starting ──ready──▶ attached ◀──attach/detach──▶ detached
     ▲                   │ failure                         │ grace elapsed, hibernate, cap
     └───────────────────┴──────────────── stopping ◀──────┘
```

- `hibernated`: no process, no memory, profile on disk. This is the state the
  owner's "tab" shows when nothing is running.
- `open` admits only when the Plugin is installed, the Machine is Linux,
  `browser.max_instances` is not reached, the account is enabled for the
  browser, and slice memory headroom is at least `browser.memory_max` plus
  the existing cold-launch floor. Admission is a reservation in the Machine's
  resource ledger, not a one-time check.
- The unit is `systemd-run --user --collect --unit cowboy-browser-<instance>
  --slice cowboy-agents.slice -p Delegate=yes -p KillMode=control-group
  -p MemoryMax=<browser.memory_max> -p MemoryHigh=<browser.memory_high>
  -p MemorySwapMax=0 -p CPUQuota=<browser.cpu_quota> -p TasksMax=<browser.tasks_max>
  -p OOMScoreAdjust=1000 -p LimitCORE=0 -p RuntimeMaxSec=<browser.max_runtime>
  -p RuntimeDirectory=cowboy-browser/<instance> -p RuntimeDirectoryMode=0700`.
  The browser is the first thing the kernel and Cowboy's memory reclaim
  sacrifice: it never swaps, never dumps core, and ranks before any session
  in the Controller's reclaim order.
- `attached` requires a live viewer lease (below). `detached` starts the
  `browser.hibernate_after` grace (default 120 s); expiry hibernates. The
  launcher's own disconnection timer is not used for policy because it
  measures the Machine's Unix connection, not the owner.
- Hibernate and stop are idempotent per instance and incarnation. A stop for
  an older incarnation is a no-op.
- At Machine start, persisted instance records (`browsers/<instance>.json`
  under the Machine state directory: account, generation, unit name,
  incarnation, deadline) are reconciled against `systemctl --user list-units
  'cowboy-browser-*'`: a record with a live unit is adopted as `detached`, a
  live unit without a record is stopped, a record without a unit is cleared.
- Failure never retries with weaker isolation. A sandbox failure (Chromium
  cannot create its user namespace) reports `sandbox_unavailable` with the
  host policy pointer and leaves the instance hibernated.

### Sign-in binding

When the open request names a native login request, the Machine:

1. takes the native login lock, checks that the login is active and owned by
   this Machine, records the binding `(login request, browser instance)` and
   releases the lock;
2. starts the unit; once the unit is registered it re-checks the login under
   the lock. If the login ended meanwhile the instance is hibernated at once;
3. navigates to the `verification_url` the Machine itself emitted for that
   login (kept in the Machine's `LoginSession`; the Controller never sends a
   URL to the Machine);
4. hibernates the instance when the login completes, is cancelled, fails or
   expires, from the same exit path that records the native login's outcome.
   The deadline of a sign-in-bound instance is `min(login expiry, cap)`.

The inverse never holds: a browser failure, stop, hibernate, OOM kill or
Machine restart does not cancel, restart, clean up or otherwise touch the
native login process or the account's credential generation.

## Stream channel (Machine protocol 43)

Commands and events are a closed addition to `MachineCommand` / `MachineEvent`
gated by a new minimum protocol:

- `BrowserOpen{request_id, account_id, instance_id, purpose: SignIn{login_request} | Tab, viewport}`
- `BrowserAttach{request_id, instance_id, incarnation, stream_token, viewport}`
- `BrowserDetach{request_id, instance_id, incarnation}`
- `BrowserResize{request_id, instance_id, width, height, scale}`
- `BrowserHibernate{request_id, instance_id}` / `BrowserReset{request_id, account_id}`
- `BrowserState{instance_id, incarnation, account_id, state, reason, geometry, deadline_ms}`

The display stream is not an RPC and does not use bulk lanes: lanes carry
idempotent execution requests, resend after loss and close on idle
(`src/machine_lanes.rs`), none of which fits an ordered live byte stream.
Instead `BrowserAttach` carries a one-time `stream_token` minted by the
Controller and bound to the owner session, account, instance, incarnation and
the Machine connection epoch. The Machine opens one dedicated WebSocket,
`/api/machine/browser-stream`, presenting that token, connects to the
instance's RFB Unix socket only after the Controller accepts the stream, and
relays bytes both ways as binary frames of at most 64 KiB with a credit-based
window per direction. Closing either side closes the Unix connection and
moves the instance to `detached`. A revoked owner session, a disabled account,
a Machine epoch change or an instance incarnation change closes the stream;
the token is single-use and expires in 30 s if unused. The Controller pairs
exactly one viewer with one Machine stream per instance; a second viewer
takes over only through an explicit takeover request, which detaches the
first.

Relay payloads are never logged, persisted, cached or inspected beyond
framing. Counters record bytes and frames.

## Controller routes and authority

All routes sit under `enforce_product_api` with the owner, device and Origin
checks used by the existing login routes and WebSocket upgrades.

| Route | Effect |
|---|---|
| `GET /api/provider-accounts/{account}/browser` | state, geometry, deadline, Machine, reasons |
| `POST /api/provider-accounts/{account}/browser/open` | open as `Tab` (no URL body; navigation is a separate call) |
| `POST /api/provider-accounts/{account}/logins/{request}/browser` | open bound to that sign-in; no URL in the body |
| `POST .../browser/navigate` | `Tab` instances only, owner-supplied `https` URL, bounded |
| `POST .../browser/resize` | bounded geometry and scale |
| `POST .../browser/hibernate`, `POST .../browser/reset` | lifecycle |
| `GET .../browser/stream` | viewer WebSocket; `takeover: true` to replace a live viewer |
| `GET /api/machine/browser-stream` | Machine upgrade with `x-cowboy-browser-stream-token` |

Enablement has two layers, both default off: the Service setting
`accounts.machine_browser` (gate, like `accounts.native_claude_login`) and a
per-account `browser_enabled` flag in the account registry, changed through
`accounts.update` with `{type: "set_browser_enabled"}`. Existing accounts stay
disabled until the owner enables each one; the first enablement is the test
account only. Cowboy MCP exposes the state read (`accounts.browser`) and the
lifecycle actions through `control.apply` with the same receipts as other
account operations, per the MCP discovery rule in `AGENTS.md`.

Device settings (Machine-enforced, `cowboy config`):

| Key | Default | Meaning |
|---|---|---|
| `browser.max_instances` | 1 | running instances per Machine |
| `browser.memory_max` / `browser.memory_high` | 1536 MiB / 1200 MiB | unit limits |
| `browser.cpu_quota` | 200% | unit CPU quota |
| `browser.tasks_max` | 512 | unit task limit |
| `browser.hibernate_after` | 120 s | detached grace |
| `browser.max_runtime` | 30 min | hard cap per run |
| `browser.profile_max_bytes` | 512 MiB | refuse start above this |
| `browser.max_geometry` | 2560×1600 | resize bound |

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
not used.

- **Resize authority.** The viewer is the only size authority: it measures
  its container in CSS pixels, chooses `scale` (1 on pointer devices, 2 on
  touch devices), requests `width = css × scale`, `height = css × scale`
  through `POST .../browser/resize` (debounced, bounded), and renders the
  framebuffer scaled to fit. The server never resizes on its own and
  `AcceptSetDesktopSize` is off, so the noVNC CSS-pixel resize race cannot
  occur. A phone therefore gets a crisp mobile-width layout with
  `--force-device-scale-factor=2`; a desktop gets a 1:1 window. Switching
  devices re-issues one resize.
- **Mobile sheet.** Full-height sheet with the bottom workbench: Keyboard
  (focuses a hidden input with autocorrect, autocapitalize and smart
  punctuation off; composition events are sent as text), Paste (local
  clipboard to remote via `ClientCutText`), Use remote clipboard as code,
  Hibernate, Close. Tap focuses the remote, two-finger drag scrolls, long press
  is right click. The sheet resizes to `visualViewport` so the keyboard never
  covers the focused field. Backgrounding the app detaches; returning
  re-attaches to the same instance (the grace period keeps it alive).
- **Remote clipboard.** Xvnc forwards only explicit copies (`SendPrimary=0`).
  A received cut text is held in memory for the open viewer and shown as
  "Remote clipboard: n characters". The authorization code field is filled
  only by the explicit "Use as authorization code" action, never
  automatically, never over an edited value; the existing code bounds apply
  (`provider_account::login_code`); the held text is dropped on instance,
  incarnation or login-request change and when the viewer closes.
- **Passkeys** are not transported by RFB; the sign-in guidance names the
  email or one-time-code path. The viewer never captures or replays
  credentials; typed input is forwarded as key events only while the viewer
  is attached.

Desktop mounts the viewer in the account panel ("Browser") and inside the
sign-in dialog; the cross-device route `/#subscription-sign-in/<account>/<request>`
shows the same viewer, so a sign-in started on the desktop continues on the
phone against the same running instance.

## Host sandbox policy (outside Cowboy)

Ubuntu restricts unprivileged user namespaces
(`kernel.apparmor_restrict_unprivileged_userns=1`), so Chromium's sandbox
cannot start from the Machine's user service without an AppArmor profile that
grants `userns`. That profile is host configuration, kept and receipted in
Columbus `machines/ovh`, and it confines rather than exempts:

- attaches to the Plugin's browser executable through a glob over the
  Machine state root and runtime digest directory (derived from
  `executables.browser`);
- grants `userns` and the namespaced capabilities the zygote needs, nothing
  on the host;
- allows read and map of the Plugin runtime tree, read of system
  configuration, fonts and certificates, and read-write only of the instance
  directory, `/dev/shm`, and the account's `browser/` directory through a glob
  that cannot match `providers/*/native`, `.claude`, `.credentials.json` or
  any other account's files;
- denies everything else under the Machine user's home, including every
  credential path, with audit logging, so an attempt is evidence.

The Machine does not weaken anything when the policy is absent: Chromium
fails to start and the instance reports `sandbox_unavailable`. The policy is
validated first on the disposable Ubuntu 26.04 KVM guest already used for
OVH SSH tests (`machines/ovh/tests/openssh_kvm.py` pattern), with synthetic
account directories and canary files, before it is installed on OVH.

## Security and privacy

- What crosses the Controller is a display of the owner's own browser and the
  owner's own input while they are attached. The Controller is already
  trusted with every session's conversation and with the owner's product
  session; it stores none of the stream. This is the owner's remote-display
  path to their own browser on their own Machine, not third-party credential
  collection. The policy text is in `requirements.md`.
- Credentials: the native login's OAuth files stay Machine-local and
  native-owned exactly as today; the browser's cookies stay in the
  account-owned profile on the Machine, protected by the same directory and
  AppArmor rules, wiped with the account. The Controller never sees a cookie,
  token or password as data; the authorization code travels only through the
  existing owner-submitted path.
- Same-user processes on the Machine (other workers) could reach the RFB
  socket. The existing requirement already states that directory separation
  is not a security boundary against host filesystem access; agent tools
  execute on their bound development host, not the OVH Machine; the AppArmor
  policy additionally confines the browser itself. This is recorded, not
  claimed solved.
- No automation surface exists: no debugging port, no WebDriver, no injected
  code; input is delivered through the X server, so pages see ordinary
  trusted events from a person.

## Acceptance before any real account

Each row cites the review finding it closes.

| Evidence | Finding |
|---|---|
| Listener proof while running, before navigation: Unix sockets only, `ss -tlnup` empty for the unit, X auth enforced (`xhost` denied without cookie) | 1 |
| Policy boundary written and accepted in `requirements.md`; Controller stream path has no persistence or logging of payloads (code review + test) | 2 |
| Synthetic account homes with canaries: AppArmor denies `file://` reads and dialog writes outside the allowed subtrees; audit log shows the denial | 3 |
| Stream binding tests: token single-use, wrong Machine epoch, wrong instance incarnation, revoked owner, disabled account, Origin mismatch all refuse; takeover detaches the first viewer | 4 |
| Race tests: login completes between reservation and unit registration; Machine restart with live unit; stop of stale incarnation; browser OOM during login leaves the native login and generation untouched | 5 |
| Mixed-load measurement on the target Machine: memory peak, pressure stall, OOM events and session continuity with the browser running beside the normal session load; defaults adjusted from the data | 6 |
| Abnormal-exit cleanup: kill -9 and OOM of the unit with synthetic secrets in the instance directory; no core dump, no swap (`MemorySwapMax=0`), instance directory removed | 7 |
| Viewer lease: detached instance hibernates after the grace even when the Machine's Unix connection stayed open; unpaired streams time out | 8 |
| Clipboard: PRIMARY copies never arrive; explicit action required; edited code never overwritten; held text cleared on instance change | 9 |
| Resize authority: phone and desktop geometries, DPR 2 on touch, bounded sizes, device handoff | 10 |
| Physical iPhone: full sign-in flow over the overlay including keyboard, paste, Mail switch and return, email-code path | 11 |
| Cross-account, cancellation, stale stop, takeover, crash/restart and OOM with synthetic accounts; enablement limited to the test account; authentication contract, worker generation and lifecycle receipts retained | 12 |

Only after this matrix passes does the test account (`Claude · Test`) sign in
through the Machine browser on OVH. The existing accounts are never enabled
during the test; their homes are not read, hashed or opened by any step.

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
  extensions, all of which are visible browser modifications.
- Exposing an OVH port to the phone: needs Stormbird ACL, DNS and TLS changes
  on a public host; the Machine connection already reaches the Controller.
- DevTools screencast of a headless Chrome: automation signature.
- Browser on hawk with a proxy to OVH egress: egress consistency is not
  guaranteed and it adds a Stormbird dependency; retained as the fallback if
  the OVH policy cannot be installed.
- Changing the CLI environment so it uses a localhost callback: modifies the
  signed authentication section and forces every named account to sign in
  again.
- Freezing the browser and squeezing its cgroup into swap for hibernation:
  more state to reason about than stopping the process, and in conflict with
  `MemorySwapMax=0`; stopping with a persistent profile costs a 3–5 s cold
  start, which is acceptable for a sign-in.

## Open questions

- Whether a second display stream for a `Tab` instance should ever be
  opened by an agent (for example, "show me the page") or stays owner-only.
  The first release is owner-only.
- Whether Codex's device-code approval should be offered in the same
  browser. It needs no browser on the Machine; offering it is a UI choice.
