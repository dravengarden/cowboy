# Agent runtime upgrades — 2026-11-17

Status: published from `f472723d`; installed on all four Machines.

## Dependency audit

Registry and release data were read from npm and GitHub on 2026-11-17 with
`.agents/skills/release-cowboy-plugin/scripts/audit-dependencies.ts all`.

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.321 | 2.1.326 | Native probes re-measured; three baselines updated, one wrapper change |
| Claude Agent ACP | 0.102.0 | 0.107.3 | Agent SDK 0.3.325, ACP SDK 2.7.0; launcher follows the v1 move and the owner-only foreground map |
| Codex ACP / Codex | 2.5.0 / 0.165.0 | 2.7.0 / 0.166.0 | See [the Codex record](codex-runtime-2026-11-17.md) |
| Grok | 1.0.70 | 1.0.77 | npm `latest`; 1.0.78–1.0.88 are `alpha` only |
| dsh | 0.2.1-rc.5 | 0.2.2-rc.1 | npm `latest`; `next` (rc.3) is not adopted |
| Claude/Codex DeepSeek gateways | — | unchanged | No Columbus commits since their pinned revisions |

Component release 3.109.0 records provider-runtime 1.2.2. Every Plugin that
pins it advances one patch version: claude-code 3.20.41, claude-deepseek
3.1.60, codex 3.5.30, codex-deepseek 3.1.60, grok 3.1.60 and deepseek-harness
1.0.18. Recommended presets and new-session defaults are unchanged.

Sources:
[Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
(2.1.322–2.1.326),
[Claude Agent ACP releases](https://github.com/agentclientprotocol/claude-agent-acp/releases)
(0.103.0–0.107.3). No model request was made.

## Claude Agent ACP 0.103–0.107

ACP 0.103 serves draft ACP v2 from a separate agent and moves the v1
`ClaudeAcpAgent` to `dist/v1/acp-agent.js`; its package exports only `lib.js`.
Cowboy speaks ACP v1, so the launcher imports the v1 agent from its new path and
sets `CLAUDE_AGENT_ACP_PROTOCOL=v1`: a client that negotiated v2 would otherwise
bypass the steering, native-cycle, configuration and idle-compaction
extensions. The steering, turn queue, sign-out respawn and idle compaction
internals those extensions use are unchanged.

ACP 0.103 also keeps only the owner in `foregroundWork` (a turn, or the
autonomous-cycle marker) instead of a work record. The native-cycle adapter
attached its cycle name to that record, so a background result no longer
announced a steerable cycle; the packaged worker gate caught it. Names now live
in a per-agent map that follows the same start, takeover and end rules.

| Upstream change | Disposition |
| --- | --- |
| 0.104–0.106: ACP auth status, `session/status`, `session/set_title`, `session/options`, initial config options | Additive; Cowboy keeps applying modes through its configuration queue |
| 0.106: idle compaction honours the client's settings option | Cowboy's gate still closes it |
| 0.107: rate-limit and retry causes in errors, 1M context assumed until reported | Display only |
| 0.107.1–0.107.3: v2 turn fixes, stream read after setup/restart, unlisted model IDs passed through | v1 path unaffected or improved; worker gate passes |

## Claude Code 2.1.326

Eight of eleven native probes match their baselines. The others:

| Probe | Change | Disposition |
| --- | --- | --- |
| hooks | Native's command wrapper turns extglob off through `\builtin shopt`, falling back to `\builtin setopt NO_EXTENDED_GLOB NO_BARE_GLOB_QUAL NO_BRACE_CCL NO_MAGIC_EQUAL_SUBST` | The target Bash wrapper uses the same guard for bash and zsh; baseline updated |
| native-behavior | Bash description adds `.claude/settings.local.json` to the secrets example | Baseline updated |
| tools | Descriptions of Agent, Bash, ExitWorktree, Monitor; schemas of ExitWorktree and Workflow | Baseline updated; no disposition changes (Workflow stays not offered) |

Other upstream changes relevant to the Remote lane (Code Mode, sandbox prompt
defaults, permission rule fixes) are opt-in or apply to native-local tools and
are covered by the unchanged permissions, shell, lifecycle and skill probes.

## Grok 1.0.77 and dsh 0.2.2-rc.1

Both builds probe `--version` without credentials. dsh 0.2.2 requires Node
24.9 or later (the runtime ships 24.21.0) and still resolves the `acp`, `web`
and `web-deepseek-api` configuration blocks and the provider IDs Cowboy's
launch patch writes. Its new LiveShare bundle is opt-in.

## Verification

Receipts are under `/srv/storage/fast0/agent/tmp/agent-deps-2026-11-17/` and,
for Codex, `/srv/storage/fast0/agent/tmp/codex-deps-2026-11-17/`. Every fixture
used a loopback scripted API and fake credentials.

| Gate | Result |
| --- | --- |
| `just claude-remote-check` | 380 + 4 + 12 passed |
| Eleven native probes against 2.1.326 | As above |
| Claude worker conformance (all phases, packaged adapter) | Accepted, 96 checks |
| Claude managed conformance | Accepted, 7 checks |
| Claude TaskStop conformance | Accepted, four topologies |
| Codex worker / turn / child-stop / hooks / image | See the Codex record |

## Publication

All six releases were built, bound, signed (`cowboy-first-party-v1`) and
verified from clean source `f472723d9d6c4ad1e06283c2f8f49f0b7a00c552`.

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.41 | `sha256:7ae26ef5709c60615934ab4b11edf7fb2eb40699be57743ed0b8321da18fc3a4` |
| claude-deepseek 3.1.60 | `sha256:d57eed27f800ba8fc284b26d22d11f42157d9b9d66c355f1478771d0c2402496` |
| codex 3.5.30 | `sha256:4c862c3dfe75dcf1db9e093825df903345d2c43d5f34f90ed68e5756af957441` |
| codex-deepseek 3.1.60 | `sha256:30b4a27c9a70a13703f3b62759c4ded50a07dac40038649cf4ea744a9794fe36` |
| grok 3.1.60 | `sha256:b06ff29f82670e88876a566192873088152e6be3a571a4013ef250f22c8d5b1b` |
| deepseek-harness 1.0.18 | `sha256:0339a338dd49c5e2308bf17025965bc894b29664057373eb8738e176ef78674e` |

| Gate | Result |
| --- | --- |
| `provider-check`, `check-compact` at `f472723d` | Passed |
| Linux old/new generation coexistence with the resident worker | Passed for all six (previous: the installed 3.20.39/3.1.58/3.5.28/1.0.16 releases) |
| Real macOS arm64 CLI/adapter/gateway probes on macbook-air | Passed for all six |
| Catalog readers before publication | Active (controller profile 618) and rollback (617) each read an isolated copy with all six twice: identical, 70 releases readable |
| Public package and runtime URLs | 26 URLs resolve to their bound digests |
| `provider-release-coverage` | Passed |
| Live Controller Catalog | All six `ready`, component release 3.109.0 |

## Installation

| Machine | Result |
| --- | --- |
| Hawk | claude-deepseek 3.1.60, codex 3.5.30, codex-deepseek 3.1.60, deepseek-harness 1.0.18, grok 3.1.60 |
| OVH | claude-code 3.20.41, codex 3.5.30, grok 3.1.60 |
| macbook-air | claude-deepseek 3.1.60, codex 3.5.30, codex-deepseek 3.1.60, grok 3.1.60, after its Machine update |
| Falcon | claude-deepseek 3.1.60, codex 3.5.30, codex-deepseek 3.1.60, deepseek-harness 1.0.18, grok 3.1.60, after its Machine update |

Falcon and macbook-air first refused with `plugin_sdk_unsupported`: their
Machines reported Plugin SDK 1.14.0, and component release 3.108.0 moved every
Plugin to 1.15.0. Neither had an active session.

- macbook-air: `just build-machine-bootstrap` at `778fd509` on the Mac, then
  `cowboy-machine-install --refresh` with its existing Service, state
  directory, capacity, admission and seven workspaces. Installed hashes match
  the build, the launcher is unchanged; backups carry the suffix
  `pre-agent-deps-20261117`. It then reported SDK 1.15.0.
- Falcon: its activator refuses a release behind freshly fetched `origin/main`,
  so the Machine release was built on Falcon at `79d5abd5` and activated with
  `cowboy-machine-activate` (receipt `succeeded`). It reports SDK 1.16.0.
  Main had briefly been unbuildable: component release 3.110.0 moved
  plugin-sdk to 1.16.0 without refreshing the fixed-output Cargo vendor hash,
  fixed in `236bc84e` (`23273d10` had committed a placeholder by mistake).

On OVH the Machine's control connection was replaced during the codex
installation. `ovh-codex-3-5-30-converge` returned an unknown outcome; two
`reconcile-install` calls under the same identity recorded the Machine's
`Unknown/Staging` (`authorization_ended`) receipt and resolved the staging
failure without replaying it. `ovh-codex-3-5-30-after-staging-resolution` then
applied 3.5.30. The grok request had not been sent and converged under its
original identity.

OVH keeps its undeclared deepseek-harness 1.0.15, and macbook-air's undeclared
1.0.15 was left out of the bounded run so convergence would not remove it.
OVH's claude-code inventory reports `materialization_state=failed` for the
legacy default account ("Provider runtime credentials are incomplete"), as it
did before this release; named-account sessions are unaffected.
