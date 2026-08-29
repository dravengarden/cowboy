# Cowboy Machine operations

Provider login belongs to Cowboy Service scope. One encrypted credential
generation is reconciled automatically to every enrolled Machine; Machine
surfaces expose only replica and materialization health. See
[Cowboy core requirements](requirements.md) and
[Service-scoped authentication](plugin-packages.md#service-scoped-authentication).

`cowboy-machine` is an outbound-only macOS/Linux host agent. It owns machine
identity, the detached ACP broker, signed component activation, transitional
legacy-component orchestration, Provider installation and credential
projection, trusted workspace roots, and stable product-adapter tunnelling.
The Controller owns encrypted Service credential generations; a Machine holds
only its sealed replica and an installed Provider's private materialization.

For a new Git-backed session, the host fetches the advertised repository's
remote default branch and creates a worktree on the task-owned
`cowboy/<session-id>` branch at `<state-root>/worktrees/<session-id>`.
Repeating preparation for the same session reuses that path without discarding
edits; if the directory disappeared, the task branch restores it. Legacy
detached worktrees are anchored on that branch only when doing so cannot
overwrite a divergent branch. Cowboy never automatically removes the worktree
or its task branch: unpublished changes and resumable session state must survive
client, controller, and Machine restarts. Permanent session deletion does
attempt to reclaim directories named `target` in production only after the
session's transient user-systemd worker unit has stopped and been collected; a
broker-socket disconnect alone is not sufficient. Failed checks retry while the
Machine remains running; an unfinished in-memory retry is not replayed after a
Machine restart, so that target remains for a later inventory cleanup. Direct
development mode preserves the artifacts because it has no durable process-exit
proof. Cleanup is further confined to the session's Machine-owned worktree and
requires Cargo's cache markers. Source files and unmarked same-name directories
are preserved.

## Bootstrap

The initial host binary is installed out of band. Later payload updates use the
controller's signed desired-component manifest and the Machine's configured
artifact public key. Provider and Zed payloads are not resolved through the
interactive shell, Homebrew, npm global state, `~/.zed_server`, or a Nix profile.

The platform bootstrap bundle contains `cowboy`, `cowboy-machine`, and
`cowboy-machine-install`. Build all three from the repository root with
`just build-machine-bootstrap`; release packaging must keep them together.
The `cowboy` bootstrap binary exposes only `register` and `identity`, while the
other two commands install the user service/LaunchAgent, a stable launcher, a
mode-0600 one-time enrollment-token file, and the bootstrap host. The host
deletes the token file immediately after successful enrollment.

## Native macOS manager

`apps/macos-installer` is the native AppKit/SwiftUI manager around that same
bootstrap contract. It is a menu-bar accessory with no WebView, Electron,
Tauri, XPC, or privileged helper. The management window has Dashboard, Install,
Activity, Account, and Settings surfaces. Dashboard opens the installed
`top.thundersparrow.cowboy` desktop shell when present, otherwise the configured
Cowboy Service, starts or stops the exact user LaunchAgent, reports the local
and Controller Machine state, and checks or applies managed dependency updates.
Stopping a Machine or rolling a dependency used by active sessions requires an
explicit confirmation.

The manager recognizes both current Service-scoped installations and the
legacy `~/.local/state/cowboy-machine` layout. It reads the controller origin
from `service-origin` or the installed launcher, matches only
`xyz.stormbird.cowboy-machine[.<service-id>]` plists inside the current user's
`~/Library/LaunchAgents`, and invokes `/bin/launchctl` with structured
arguments. Quitting the menu app does not stop the independently owned Machine
LaunchAgent.

The process-owned task model keeps an active install or dependency update
independent from any window. Installation persists a bounded non-secret
activity history and marks a task left running across process termination as
interrupted. It invokes the bundled `cowboy register` command with structured
`Process` arguments and passes `/dev/fd/0` as `--token-file` over an anonymous
pipe; the one-time enrollment code is never placed in process arguments,
settings, activity history, or a disk file.

The Account surface uses the configured HTTPS origin's existing product and
admin endpoints. One sign-in creates both Service sessions. When the user keeps
automatic sign-in selected, the credential is stored only in a Service-origin-
scoped macOS Keychain item and is retried after an app restart or session
expiry; it never enters settings, activity history, logs, browser storage, or
process arguments. Sign-out deletes the Keychain item before ending the Service
sessions. First-time host setup stays in the browser so the host setup code
never enters this app. Auth-off Services remain usable as the synthetic local
owner even when administrator sign-in fails, while dependency mutations still
require the owner/admin session. Dependency checks read the Controller's
Machine inventory; mutations call the existing signed reconcile or bounded npm
update commands and never install directly from the macOS app.

The manager uses `SMAppService.mainApp` for its own optional Launch at Login
setting. This starts only the lightweight menu app. The separate "Run Cowboy
Machine in background" control manages the installed Machine LaunchAgent,
which remains owned by the Rust backend.

On macOS, run the project-owned gates and package the app with:

```sh
just macos-installer-test
just macos-installer-build
just macos-installer-verify
```

The build places `Cowboy Manager.app` under
`apps/macos-installer/dist/`, embeds all three Machine bootstrap commands, and
uses ad-hoc signing unless `CODE_SIGN_IDENTITY` names a real signing identity.
Copy a signed production build to `/Applications` before enabling Launch at
Login. No certificate or signing credential belongs in this repository.

On the device, prefer `cowboy register`. Create a one-time code in the UI
first, copy the command, then paste the token when the CLI asks. Cowboy
assigns the machine id. Interactive input is TTY-masked; after entry the CLI
confirms only the final four characters. The token stays off the command line
and out of shell history:

```sh
cowboy register https://cowboy.example
```

`cowboy register` requires `https://` except loopback HTTP. Workspace
defaults to `home=$HOME`. It creates an Ed25519 identity under
`~/.local/state/cowboy-machine/services/<service-id>/identity_ed25519`
(directory `0700`, key
`0600`), prepares `cowboy-machine`, and consumes the token over HTTPS. By
default the Machine stays attached to the current terminal; closing it or
pressing Ctrl-C takes the computer offline. Keep the Machine online across
logins by choosing background mode during enrollment:

```sh
cowboy register https://cowboy.example --background
```

Background mode installs and starts a Service-scoped per-user systemd unit on
Linux or LaunchAgent on macOS. Foreground mode does not write either background
service definition.
Cowboy Service stores only the public key and the token digest. The
private key never leaves the device. `cowboy identity` reprints the
OpenSSH `SHA256:…` fingerprint for later comparison. Scripts may pass
`--token-file` instead of the TTY prompt.

Machine identity deliberately uses OpenSSH SSHSIG rather than accepting
interchangeable key utilities. Cowboy checks the standard macOS, Linux,
Homebrew, and current `PATH` locations for `ssh-keygen`, canonicalizes each
candidate, rejects non-files, non-executables, and group/world-writable tools,
and probes `-Y sign` support before generating or signing. macOS normally
provides `/usr/bin/ssh-keygen`; Linux packages it as the OpenSSH client. Cowboy
does not fall back to OpenSSL, GPG, age, or an unrelated signature format.

Enrollment codes contain 256 bits of OS randomness, are stored by SHA-256
digest only, expire after 15 minutes, and are atomically consumed once. The UI
shows a live countdown and requires a fresh code after expiration.

The lower-level installer remains:

```sh
cowboy-machine-install \
  --controller-url https://cowboy.example \
  --service-id svc-0123456789abcdef0123456789abcdef \
  --machine-id macbook-air \
  --display-name 'MacBook Air' \
  --workspace cowboy=/path/to/cowboy \
  --workspace columbus=/path/to/columbus \
  --max-sessions 8 \
  --enrollment-token "$ONE_TIME_TOKEN" \
  --artifact-public-key /path/to/component-publisher.pub
```

The equivalent host options are Service-scoped. Each Cowboy Service has an
independent Machine identity, component and Plugin generations, authentication
replicas, worktrees, sockets, caches, launcher, and background unit. Registering
the same computer with another Service must never reuse or overwrite them.

```text
COWBOY_MACHINE_SERVICE_ID=svc-0123456789abcdef0123456789abcdef
```

The remaining host options are:

```text
COWBOY_MACHINE_CONTROLLER_URL=https://cowboy.example
COWBOY_MACHINE_ID=macbook-air
COWBOY_MACHINE_DISPLAY_NAME=MacBook Air
COWBOY_MACHINE_STATE_DIR=/absolute/private/state/root
COWBOY_MACHINE_WORKSPACES=cowboy=/path/to/cowboy,columbus=/path/to/columbus
COWBOY_MACHINE_ARTIFACT_PUBLIC_KEY=/path/to/component-publisher.pub
COWBOY_MACHINE_ZED_ADAPTER_SOCKET=/path/to/cowboy-managed/zed-adapter.sock
COWBOY_MACHINE_CODE_ADAPTER_SOCKET=/path/to/cowboy-managed/code-adapter.sock
COWBOY_MACHINE_MAX_SESSIONS=8
COWBOY_MACHINE_DRAINING=false
PATH=/absolute/state/root/components/commands:/usr/local/bin:/usr/bin:/bin
```

Prefer `COWBOY_MACHINE_ENROLLMENT_TOKEN_FILE`; it is consumed and removed.
The generated Ed25519 private key stays mode 0600 under the
Machine state root. Remote HTTP is rejected; loopback HTTP exists only for
hermetic tests.

## Provider usage spool status

The Machine binary has a controller-independent, read-only status command for
the durable provider-usage outbox. Run it as the Machine service user against
the same state root:

```bash
COWBOY_MACHINE_STATE_DIR=/home/draven/.local/state/cowboy-machine \
  /nix/var/nix/profiles/columbus-components/cowboy-machine/bin/cowboy-machine \
  --provider-usage-status | jq .
```

The command opens `provider-usage.sqlite3` read-only and neither starts a
broker nor contacts Cowboy. Its JSON reports total pending events, explicit
`pendingV3Events` / `v3Drained` and `pendingV4Events` / `v4Drained` downgrade
gates, and every known producer. Each producer has explicit schema 1 through 4
rows with pending count, first/last pending sequence, and the last
controller-acknowledged sequence and timestamp. Future schemas are retained as
additional rows rather than folded into an older version.

These are downgrade gates for telemetry schemas v3 and v4. First replace or
stop every gateway producing a schema unsupported by the candidate Machine and
verify its running release. Only then may that schema's `drained: true` and
pending count of zero authorize the downgrade. Record the full status for each
Machine; a null last ACK is valid only when that producer never emitted the
schema. Older supported events may be delivered by the candidate Machine, but
any pending unsupported event forbids the downgrade. Codex and Claude Code
producers are evaluated independently.

## Linux user service

The installer writes
`~/.config/systemd/user/cowboy-machine-<service-id>.service`. The
launcher prefers the active signed Machine-host generation and falls back to
the bootstrap binary. To manage it manually:

```sh
systemctl --user daemon-reload
systemctl --user enable --now cowboy-machine-<service-id>.service
```

## macOS LaunchAgent

The installer writes
`~/Library/LaunchAgents/xyz.stormbird.cowboy-machine.<service-id>.plist`
and bootstraps it in the current GUI domain. Enrollment secrets are kept only
in the one-time mode-0600 file and never enter the plist. The background
LaunchAgent sends stdout and stderr to `/dev/null`: launchd opens a file target
once, every worker and Provider descendant inherits it, and launchd supplies no
size or retention bound. Machine and session health remain visible through the
controller; unload the LaunchAgent and run its launcher in the foreground for a
bounded diagnostic capture when raw stderr is required.

macOS uses direct child-process workers rather than user-systemd units. Each
worker leads an isolated process group containing its Provider/adapter subtree.
A normal session stop drains the worker protocol first; if the exact child owner
has not exited within three seconds, Machine sends `TERM` to that process group,
then `KILL` after another two seconds. The reaper removes the PID mapping before
it may be reused, so a stale watchdog cannot signal an unrelated process.

Broker rejection is terminal for a worker (duplicate epoch, deleted session, or
protocol mismatch). Transient worker and Cowboy-to-broker failures always wait
with exponential backoff from 100 ms through 5 seconds, and only a connection
that remains healthy for ten seconds resets that backoff. A deleted session's
final worker events are consumed and acknowledged locally even though they are
not forwarded to the controller; this lets the outbox drain without reviving
the deleted session.

For emergency containment, unload the exact LaunchAgent first and verify that
no `cowboy-machine`, `cowboy-acp-worker`, or `cowboy-code-adapter` process
remains. `lsof +L1` must show no deleted Cowboy log held open before the agent is
started again. Do not delete Machine state, session worktrees, or enrollment
identity as part of this containment.

## Component publication

The controller manifest is a JSON array of `DesiredComponent`. Each artifact
signature signs a version-three, length-prefixed UTF-8 transcript using OpenSSH
namespace `cowboy-machine-v1`. The fields, in order, are:

```text
cowboy-component-v3
<kind-or-kind-slot>
<version>
<generation>
<lowercase-sha256>
<raw-or-tar_gz>
<entrypoint-or-empty>
<canonical probe JSON or empty>
<automatic boolean>
```

Each field is encoded as `<byte-length>:<value>\n`; this prevents delimiter
ambiguity and binds both the readiness command and activation policy to the
publisher signature. An automatic component must declare a bounded `probe`
(`args` and `timeout_ms`). The Machine runs it against the staged executable
before changing any active or command symlink. A timeout, spawn error, or
non-zero exit leaves the prior generation active and reports a failed update.

The Machine downloads over HTTPS, verifies the SHA-256 and Ed25519 signature,
writes a content-addressed immutable generation, and atomically switches only
that component slot. Raw artifacts are single executables. `tar_gz` artifacts
must declare a safe relative entrypoint; absolute paths, parent traversal, and
archive links are rejected. Existing ACP workers and Zed worktrees keep leases
on their old executable; unrelated slots are not restarted.

The Machine supervises the active Code Adapter and the selected Cowboy-managed
Zed adapter/server pair. A Zed adapter and server are paired only when their
component slots share the same compatibility key (normally the exact Zed
version); independently active but mismatched payloads are never launched.
Adapter generations restart independently when their
content-addressed links change. The Code Adapter revalidates every requested
root against the Machine's declared trusted workspaces before touching files or
Git, so remote sessions never fall back to the controller filesystem.
`max_sessions` and `draining` are reported by the host; Cowboy derives active
leases from its persisted session ownership and refuses new placement when the
envelope is full or draining. Existing sessions remain pinned and resumable.
Legacy Provider CLI, provider adapter, managed Node, and ACP-runtime component
activation can still relaunch the small stable host during a rolling drain.
Schema-v2 Provider installation does not reconcile those slots: it stages all
private runtime artifacts inside the Provider generation, and detached workers
stay pinned to their original generation.

### Service-owned Provider authentication

Provider login is initiated once from the Service-level Providers surface, not
from a Machine. A Provider-declared temporary executor may run on an eligible
Machine in an isolated home, but it exports only the declared portable bundle;
after the Service durably commits and redistributes that generation, Cowboy
removes the temporary executor home.

The Service encrypts one monotonic generation per Provider, seals it separately
to every enrolled Machine's public key, and reconciles online, reconnecting, and
newly enrolled Machines without another login. A Machine stores the sealed
replica even when the Provider is absent and materializes it only into the
matching installed Provider's private home. Machine inventory exposes replica
and materialization health, never login/logout or credential entry.

## Ownership

Cowboy-managed Zed payloads and state live under the Machine root. Native Zed
continues to own `~/.zed_server` and its SSH bootstrap chain. The stable tunnel
carries Cowboy adapter JSON, never Zed protobuf. Provider credential plaintext
is Service-owned; Machines hold only sealed replicas and the active Provider's
private materialized projection.
