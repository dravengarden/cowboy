# Agent runtime upgrades — 2026-10-30

Status: published from `113edddb` and installed on all four Machines.

## Dependency audit

The repository auditor queried npm on 2026-10-30.

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.312 | 2.1.314 | Native probes re-measured; two baseline updates, no plugin change |
| Claude Agent ACP | 0.99.0 | 0.99.1 | Bug fixes; same Agent SDK 0.3.312 |
| Codex | 0.164.0 | 0.165.0 | ACP 2.5.0 declares `^0.165.0`; 0.166.0 is outside that range |
| Codex ACP | 2.3.0 | 2.5.0 | Owned patch rebased (see `plugins/codex/runtime/README.md`) |
| DeepSeek Harness (dsh) | 0.2.1-rc.2 | 0.2.1-rc.5 | Packaged lifecycle |
| Grok | 1.0.70 | 1.0.70 | Current |
| Gemini CLI | 0.64.0 | 0.64.0 | Gemini was sunset on 2026-10-20 (removed from every Machine, `plugins.disabled`); not upgraded |

Component release 3.71.0 records provider-runtime 1.1.24:

| Plugin | Previous published | Released |
| --- | --- | --- |
| claude-code | 3.20.18 | 3.20.20 |
| claude-deepseek | 3.1.45 | 3.1.47 |
| codex | 3.5.11 | 3.5.13 |
| codex-deepseek | 3.1.46 | 3.1.48 |
| grok | 3.1.46 | 3.1.48 |
| gemini | 3.1.44 | 3.1.46 (sunset; carried only by the shared component closure, published but not advertised or installed) |
| deepseek-harness | 1.0.3 | 1.0.5 |

These also carry main's unpublished 3.70.0 changes, including Provider SDK
3.1.16 and the codex-deepseek model catalog shipped in its gateway artifact.

Sources: [Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
(2.1.313–2.1.314),
[Claude Agent ACP 0.99.1](https://github.com/agentclientprotocol/claude-agent-acp/releases/tag/v0.99.1),
[Codex ACP 2.4.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.4.0),
[Codex ACP 2.5.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.5.0),
[Codex 0.165.0](https://github.com/openai/codex/releases/tag/rust-v0.165.0).

## Claude Code 2.1.314

Nine of eleven native probes match their baselines. Two differ:

| Observation | Disposition |
| --- | --- |
| The shell-prefixed native command also unaliases `which` before the command | Native-local only; the target Bash facade builds its own command line and its shell probe is unchanged. Hooks baseline updated |
| The not-offered `DesignSync` tool has a new description | Remains not offered. Tools baseline updated |

The collector's usage request names Claude Code 2.1.314 in its user agent.

## Codex ACP 2.5.0

Upstream now parses the steering `idleBehavior: "promptRequired"` opt-in
itself (#593). The rebased patch keeps advertising it in `initialize`, the
ambiguous-error rule for a lost steer reply, the managed-profile
reconfiguration refusals and the managed title suppression. New standard
subagent sessions apply only to clients that declare
`clientCapabilities.subagents`; Cowboy does not. Prompt suggestions are
AIR-only.

## Verification

Receipts are under `/srv/storage/fast0/agent/tmp/plugin-upgrades-20261030/`.

| Gate | Result |
| --- | --- |
| Codex ACP typecheck and upstream tests with the patch | 2,222 passed, 40 skipped upstream |
| `just claude-remote-check` | 359 + 4 passed |
| Claude worker conformance (all twelve phases, packaged adapter) | Accepted, 96 checks |
| Claude TaskStop conformance | Accepted, four topologies |
| Codex worker conformance | Accepted, 34 checks |
| Codex turn / child-stop / hooks / image | Accepted (9 checks / passed / 10 hooks / 13 checks) |
| `just check-compact` at `113edddb` | Passed |

The Codex turn and hooks probes ran under a user namespace whose PID 1 sits
outside the probe's network namespace, so the probe can compare the two.

## Publication

All seven releases were built, signed and verified from clean source
`113edddbc0cd7542c50de5ffd74246d90acff413`. The final Claude and Codex
adapter archives are byte-identical to the accepted candidates.

| Gate | Result |
| --- | --- |
| Linux old/new generation coexistence with the resident worker | Passed for all seven |
| Real macOS arm64 CLI/adapter/gateway probes on macbook-air | Passed for all seven |
| Catalog readers before and after publication | Active and rollback read all seven twice. The accepted-recovery reader (`1e6b4e42`, Provider SDK 3.1.15) and the bootstrap-only reader skip them safely |
| Public package and runtime URLs | 29 URLs resolve to their bound digests |
| `provider-release-coverage` | Passed |

## Installation

Convergence ran Falcon (canary), macbook-air, Hawk, then OVH.

| Machine | Installed |
| --- | --- |
| Falcon, Hawk | claude-deepseek 3.1.47, codex 3.5.13, codex-deepseek 3.1.48, deepseek-harness 1.0.5, grok 3.1.48 |
| macbook-air | claude-deepseek 3.1.47, codex 3.5.13, codex-deepseek 3.1.48, grok 3.1.48 |
| OVH | claude-code 3.20.20, codex 3.5.13, grok 3.1.48 |

macbook-air first refused every upgrade with `plugin_sdk_unsupported`: its
Machine predated Plugin SDK 1.12.3. Its pinned Rust 1.98.1 bootstrap build at
`113edddb` and an owned `cowboy-machine-install --refresh` with the existing
Service, state directory, capacity, admission and seven workspaces replaced the
bootstrap Machine and worker (installed hashes match the build). The launcher
only gained the DeepSeek Harness fallback line; the previous launcher and
bootstrap directory are retained as `*.pre-plugin-upgrades-20261030` and
`recovery-plugin-upgrades-20261030`. It had no live worker. The same
convergence then applied all four upgrades.

All 17 upgrades returned HTTP 204 and a final dry run reports nothing remaining
on any Machine. No live session was recycled.
