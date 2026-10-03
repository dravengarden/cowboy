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

This release updates
installer tooling and stages bootstrap/launcher configuration with `--no-start`;
it does not activate a new resident Machine or worker generation.

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
