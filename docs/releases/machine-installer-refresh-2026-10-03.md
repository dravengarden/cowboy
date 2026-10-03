# Enrolled Machine installer refresh — 2026-10-03

Implementation revision: `ca7a73c2` (following `a300db49` and `b00a709a`),
published to Cowboy `origin/main`.

`cowboy-machine-install` now accepts `--plugin-operation-admission` and
`--refresh`. Refresh replaces enrollment with checks against the Controller's
Service identity, the saved origin or matching legacy launcher, the existing
private-key file and the enrolled Machine id. It preserves the private key and
enrollment-token file. Configuration remains explicit: callers supply the
workspace list and desired launcher options again.

Legacy refresh reuses the existing launcher, service label and socket paths.
The bootstrap bundle includes the ACP worker, built separately with default
features; both the CLI build recipe and native Manager packaging include it.
Signed commands keep precedence over bootstrap fallback binaries. Executables
and launchers are replaced through same-directory atomic renames. The installer
initializes Rustls before its Service identity request.

Verification:

- Linux: 10 installer unit tests pass with default and Machine-host feature
  sets; the real CLI integration test passes, including successful legacy
  refresh and rejection of a different Service before writes.
- Linux: Rust formatting and clippy pass; native Manager packaging scripts
  pass shell syntax checks.
- macOS aarch64: the real CLI integration test passes using pinned Rust 1.98.1.
  It also verifies that refresh produces only the existing legacy LaunchAgent.
- macOS library unit-test compilation remains blocked by existing uses of
  Linux-only `rustix::fs::mknodat`/`mkfifoat` in unrelated Controller and Machine
  test modules. The independent CLI integration test avoids those modules.

The initial delivery updated installer tooling and staged bootstrap/launcher
configuration with `--no-start`. The production activation below completes that
delivery by switching the resident Mac Machine and its runtime bundle.

## macbook-air installation receipt

All five bootstrap executables were built in the clean isolated worktree
`/Users/dravenchen/worktrees/cowboy/installer-refresh-20261003` at `ca7a73c2`.
The installer was installed at
`/Users/dravenchen/.local/bin/cowboy-machine-install` with SHA-256
`d5ed5aa4993a81a1de65985da080a8c0cde41e2adbfd3f3af4fce79b9199fe3b`.

The installed executable successfully ran `--refresh --no-start` against the
real Service `svc-4e4d5154f3df9aa109d7d841dd925fd7`, retaining the existing state
root and all seven workspace declarations. The real HTTPS identity lookup
matched the expected Service. Refresh created its missing `service-origin`
binding from the validated legacy launcher.

After refresh:

- Machine PID remains `25020`.
- Public identity fingerprint remains
  `SHA256:XRGsnK0WbsuJBc8R9Sz5sj43hDlOd7Vrj4aqCK6wZZg`.
- The three resident component-command executable hashes are unchanged.
- The launcher remains `~/.local/bin/cowboy-machine-launch`; SHA-256 is
  `bf56221126400fa47ee818b5411d7407e2a4892581f329062d1338911fe042aa`.
- The plist remains
  `~/Library/LaunchAgents/xyz.stormbird.cowboy-machine.plist`; SHA-256 is
  `9fb1aeb3f63ed48d37180926b01d52ac48fe6fda65259906c2e1df04d4d054f1`.
  `plutil -lint` and launcher shell syntax checks pass. No Service-scoped
  duplicate plist was created.

The old installer, launcher and plist are retained beside their replacements
with `.pre-ca7a73c2` suffixes. The resident Machine was not restarted.

## Immutable Nix artifact

`nix build .#cowboy-machine-release --no-link --print-out-paths` succeeded:

`/nix/store/34xsxkmh577ha2jfjnll4q2pw294iyfc-cowboy-machine-release`

Its source receipt records clean revision
`ca7a73c20a112a696f3f5f5298631c862a8b3c50` and worker generation
`worker-eed1d8105af00846771d`. The packaged installer help exposes both new
options. The artifact was built and verified, without activating a resident
Machine generation on Hawk or Falcon for this installer-only change.

## Completed macbook-air production activation

Source revision: `1c181e9415e1654f34ea110164fa7fd28b68f95c`, freshly fetched
from `origin/main` and built in the same clean isolated macOS worktree using
pinned Rust 1.98.1. The real installer CLI integration test passes at this
revision. The complete host, installer, Code Adapter and ACP-worker bundle
builds successfully.

Before activation, the Controller reported `ready`, zero active Mac sessions,
and zero Plugin session leases. macbook-air had no signed active/rollback
component pointers; three ordinary executable overrides from September were
masking its newly staged bootstrap. The exact legacy LaunchAgent was unloaded,
those three overrides were archived, and the native Machine installer performed
`--refresh` without `--no-start`, bootstrapping the same legacy LaunchAgent with
the new complete bootstrap bundle. The installer's redundant bootout of the
already-unloaded unit emitted launchd error 5; its subsequent bootstrap
succeeded. No other Machine or Controller was restarted.

Activation evidence:

- Resident Machine PID changed from `25020` to `62458`; its executable is now
  `~/.local/state/cowboy-machine/bootstrap/cowboy-machine`.
- The Code Adapter runs from the same bootstrap bundle under the new Machine.
- Controller inventory reports macbook-air connected, schedulable and `ready`,
  with Plugin SDK `1.11.0` (previously `1.8.1`).
- The Machine identity fingerprint and all seven workspace declarations remain
  unchanged. The remote `operator projects --machine macbook-air` read returns
  HTTP 200 and all seven projects.
- LaunchAgent and shell syntax checks pass. Each installed bootstrap executable
  is byte-identical to its clean-worktree build output.
- Controller `/healthz` and `/version` return HTTP 200; the Controller remains
  active with PID `1998362`.

Installed macOS SHA-256 digests:

| Executable | SHA-256 |
| --- | --- |
| Machine | `cac394d7f0f1b2ee8255b4fc6e070a16328dac23ae728169555705d7eca09f57` |
| ACP worker | `931db6dc0391801464f8f28d8e5d9b0bd4ff32d89d25366a4ce0d9da08ebb8f6` |
| Code Adapter | `f4f9fd06910a719cc792c2b69be23757733d9458c28b32cfd36d8160bbb49f29` |

Previous bootstrap bytes are retained in
`~/.local/state/cowboy-machine/bootstrap.pre-activation-1c181e94`; the old
ordinary overrides are retained in `legacy-commands.pre-activation-1c181e94`
under the same state root. Installer, launcher and plist predecessors have
`.pre-activation-1c181e94` suffixes. The historical September overrides are
archives, not an accepted recovery floor for newer installation journals.

The corresponding clean Linux Nix artifact was also built and checked:
`/nix/store/cqsimjy2cp85m9pnzdsbcn888mkwdd2n-cowboy-machine-release`, with source
revision `1c181e94` and worker generation `worker-eed1d8105af00846771d`. It was
not applied to Hawk or Falcon; this activation targets macbook-air.
