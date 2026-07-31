# Cowboy Machine operations

`cowboy-machine` is an outbound-only macOS/Linux host agent. It owns machine
identity, the detached ACP broker, signed component activation, provider login
orchestration, trusted workspace roots, and stable product-adapter tunnelling.
Cowboy remains the controller and never receives provider credentials.

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
signature signs these exact UTF-8 bytes using OpenSSH namespace
`cowboy-machine-v1`:

```text
cowboy-component-v2
<kind-or-kind-slot>
<version>
<generation>
<lowercase-sha256>
<raw-or-tar_gz>
<entrypoint-or-empty>
```

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

Codex device login and Claude browser login can be initiated from Machines.
Gemini does not expose a stable login/status command, so complete its official
CLI setup once on the execution Machine and press Refresh. Cowboy recognizes
OAuth credential presence or configured API-key/Vertex metadata without
reading, storing, or transmitting secret contents.

## Ownership

Cowboy-managed Zed payloads and state live under the Machine root. Native Zed
continues to own `~/.zed_server` and its SSH bootstrap chain. The stable tunnel
carries Cowboy adapter JSON, never Zed protobuf. Codex, Claude, and Gemini auth
state remains in each official CLI's own state root on that Machine.
