# Session extension Plugins and component release 3.53.0 — 2026-10-16

Matrix memory and Cowboy review guidance became Plugins: `matrix` is a
`memory_backend` and `review` a `call_profile`, both data-only release-schema-5
releases (`docs/session-extension-plugins.md`). Every Agent Provider was rebuilt
on component release 3.53.0 (plugin-sdk 1.12.0, plugin-contract 1.11.0,
memory-client 0.2.0) so that it reads the Machine's per-session descriptor.

All releases were built, signed and verified from
`392a2c8013e1cfe8dd989ab5a903824857dff743`. The publisher ran at `fe8933de`,
which changes only `tools/publish-plugin-release.ts` to admit schema 5.

| Plugin          | Previous | Released | Kind           |
| --------------- | -------- | -------- | -------------- |
| codex           | 3.4.5    | 3.4.7    | agent_provider |
| claude-code     | 3.19.34  | 3.19.36  | agent_provider |
| claude-deepseek | 3.1.34   | 3.1.35   | agent_provider |
| codex-deepseek  | 3.1.33   | 3.1.34   | agent_provider |
| gemini          | 3.1.33   | 3.1.34   | agent_provider |
| grok            | 3.1.35   | 3.1.36   | agent_provider |
| matrix          | —        | 1.0.0    | memory_backend |
| review          | —        | 1.0.0    | call_profile   |

claude-code 3.19.36 had already been published at 2026-10-15T23:24:47Z from a
build of `d566e3ee`. No Plugin source changed between that revision and
`392a2c80`, and the rebuild reproduced its artifact digest
(`sha256:fa0dcc0f…a6ba`) and package digest exactly, so it was verified rather
than published again.

## Acceptance

| Gate                                    | Result                                                                                                            |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `codex-worker`                          | Accepted                                                                                                          |
| `claude-worker`, all phases but context | Accepted, 83 checks                                                                                               |
| `claude-worker`, context phase          | Fails identically at `a16f2d64`, the revision accepted on 2026-10-11; see below                                   |
| `session`                               | Accepted after `b359cb0e` (the Machine creates its state directory before publishing file-buffer policy)          |
| `keeper`, `claude-managed`, `task-stop` | Accepted                                                                                                          |
| `execution-rust-tests`                  | Accepted                                                                                                          |
| Linux worker coexistence (6 Providers)  | Passed against the production worker, each with its previous Catalog release                                      |
| macOS execution (6 Providers)           | Passed                                                                                                            |
| Catalog readers                         | Accepted before publication                                                                                       |
| Publication                             | 8 releases advertised `ready`, 28 public artifacts served with matching digests, provider-release-coverage passed |

### claude-worker fixture changes (`392a2c80`)

Since `af5512a9` native Write publishes through the target's `apply` helper, not
`fs/writeFile`. The suite's lost-completion fixture still matched only
`fs/writeFile`, so it never fired. The relay now loses the helper's final output
read instead. A completion not recovered within the apply timeout is settled by
the target's digest, and the later change makes that digest stale. The probe
therefore accepts either success or "modified since read", and still requires
the later change to survive. A hard-linked name is replaced by a new inode, as
`docs/remote-file-buffers.md` documents, so both aliased edits now apply, each
to its own name.

### Known issue: target gitStatus missing

Claude 2.1.296 attaches its `session_context` only when it has something to
announce. When the runtime directory is not a repository it announces nothing,
so `targetSessionContext` never runs and the target's `# gitStatus` block is
absent. The same check fails at `a16f2d64` today, so this is not a Cowboy
regression in this batch. It also affects production sessions running on a
non-repository runtime directory. Fixing it needs the Claude Mod to supply the
attachment itself; that work is out of scope here.

## Deployment

| Component    | Where | Result                                                                                                                   |
| ------------ | ----- | ------------------------------------------------------------------------------------------------------------------------ |
| Web          | Hawk  | `succeeded` at `392a2c80`                                                                                                |
| Controller   | Hawk  | `succeeded` at `392a2c80`; `cowboy.service` active                                                                       |
| Machine host | OVH   | Accepted at `392a2c80` (`/var/lib/columbus/ovh-machine-392a2c8013e1-20261016T042244Z`); 32 retained workers kept running |
| codex 3.4.7  | OVH   | Converged, receipt `applied`                                                                                             |
| matrix 1.0.0 | OVH   | Installed, receipt `applied`, inventory `active`                                                                         |
| review 1.0.0 | OVH   | Installed, receipt `applied`, inventory `active`                                                                         |
| claude-code  | OVH   | Already 3.19.36                                                                                                          |
| Machine      | Hawk  | Not deployed: needs a `cowboy-workers` pin advance                                                                       |

The Hawk writer-host release refuses `retained-worker-interface-compatible`.
Since the pin `7bf125ce`, `Cargo.toml`, `Cargo.lock`, `runtime_wire.rs`,
`machine_protocol.rs` and the plugin SDK have changed. The changes come from the
rustls security update, Agent API protocol 30, session hibernation and this
batch. Advancing the pin replaces every live Hawk worker
(`docs/machine-worker-pin-advance-runbook.md`) and needs its own explicit go.
Until then, Hawk-run sessions get no session extension descriptor.
