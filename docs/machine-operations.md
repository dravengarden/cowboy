# Cowboy Machine operations

`cowboy-machine` is an outbound-only macOS/Linux host agent. It owns machine
identity, the detached ACP broker, signed component activation, provider login
orchestration, trusted workspace roots, and stable product-adapter tunnelling.
Cowboy remains the controller and never receives provider credentials.

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

Build or download `cowboy-machine` and `cowboy-machine-install`, then run the
installer once. It installs a user service/LaunchAgent, a stable launcher, a
mode-0600 one-time enrollment-token file, and the bootstrap host. The host
deletes the token file immediately after successful enrollment:

```sh
cowboy-machine-install \
  --controller-url https://cowboy.example \
  --machine-id macbook-air \
  --display-name 'MacBook Air' \
  --workspace cowboy=/path/to/cowboy \
  --workspace columbus=/path/to/columbus \
  --max-sessions 8 \
  --enrollment-token "$ONE_TIME_TOKEN" \
  --artifact-public-key /path/to/component-publisher.pub
```

The equivalent host options are:

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

The installer writes `~/.config/systemd/user/cowboy-machine.service`. The
launcher prefers the active signed Machine-host generation and falls back to
the bootstrap binary. To manage it manually:

```sh
systemctl --user daemon-reload
systemctl --user enable --now cowboy-machine.service
```

## macOS LaunchAgent

The installer writes `~/Library/LaunchAgents/xyz.stormbird.cowboy-machine.plist`
and bootstraps it in the current GUI domain. Enrollment secrets are kept only
in the one-time mode-0600 file and never enter the plist.

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
Provider CLI, provider adapter, managed Node, and ACP-runtime activation causes
the small stable host process to be relaunched by launchd/systemd so its worker
launch environment is rebuilt atomically. Detached worker services and their
session generations remain alive and are adopted by the new broker.

Codex and Grok Build device login plus Claude browser login can be initiated
from Machines. Grok credentials remain in the official CLI's `~/.grok/auth.json`
or configured `GROK_HOME`/`GROK_AUTH_PATH`; Cowboy never copies token material.
Gemini exposes two explicit Cowboy flows: an API key is submitted directly to
the target Machine and stored in its CLI-owned `~/.gemini/.env` with mode 0600;
Standard/Enterprise Google Login is enabled only when that Machine has a
`GOOGLE_CLOUD_PROJECT`. Consumer Google Login credentials without a project are
retired and never make the provider ready. A controller refuses Gemini login
commands from older Machine hosts that cannot advertise these semantics.

## Ownership

Cowboy-managed Zed payloads and state live under the Machine root. Native Zed
continues to own `~/.zed_server` and its SSH bootstrap chain. The stable tunnel
carries Cowboy adapter JSON, never Zed protobuf. Codex, Claude, Gemini, and Grok auth
state remains in each official CLI's own state root on that Machine.
