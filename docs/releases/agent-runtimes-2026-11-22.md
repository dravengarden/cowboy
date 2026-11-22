# Agent runtime upgrades — 2026-11-22

Status: source and component release 3.137.0 on the task branch; verification,
publication and installation below.

## Dependency audit

Registry and release data were read from npm, GitHub and the vendors' release
pages on 2026-11-22 with
`.agents/skills/release-cowboy-plugin/scripts/audit-dependencies.ts all`.

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.326 | 2.1.330 | npm `latest`; native probes re-measured, see below |
| Claude Agent ACP | 0.107.3 | 0.109.1 | Agent SDK 0.3.330, ACP SDK 2.9.1; the launcher's extension points are unchanged |
| Codex ACP / Codex | 2.7.0 / 0.169.1 | 2.9.0 / 0.171.0 | ACP 2.9.0 declares `@openai/codex ^0.171.0`; the `native-codex-version` delta is dropped |
| Grok | 1.0.77 | 1.0.92 | npm `latest` and `x.ai/cli/stable` |
| agy | 1.5.9 | 1.5.14 | The install manifest is `manifests/linux_amd64.json`; `linux_x64.json` answers 404 |
| dsh | 0.2.2-rc.1 | unchanged | `latest` is 0.2.2-rc.5; not part of this upgrade |
| Claude/Codex DeepSeek gateways | — | unchanged | No change requested |

Component release 3.137.0 records provider-runtime 1.3.21. Every Plugin that
pins it advances one patch version: antigravity 0.2.22, claude-code 3.20.67,
claude-deepseek 3.1.84, codex 3.6.26, codex-deepseek 3.1.84, deepseek-harness
1.0.42 and grok 3.2.25. Recommended presets and new-session defaults are
unchanged.

Sources:
[Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
(2.1.327–2.1.330),
[Claude Agent ACP releases](https://github.com/agentclientprotocol/claude-agent-acp/releases)
(0.108.0–0.109.1),
[Codex ACP releases](https://github.com/agentclientprotocol/codex-acp/releases)
(2.8.0, 2.9.0),
[Codex 0.170.0](https://github.com/openai/codex/releases/tag/rust-v0.170.0),
[Codex 0.171.0](https://github.com/openai/codex/releases/tag/rust-v0.171.0),
[Grok Build changelog](https://x.ai/changelog/build) (through 1.0.89; 1.0.90–1.0.92
have no published notes),
[agy releases](https://github.com/google-antigravity/antigravity-cli/releases)
(1.5.10–1.5.14). No model request was made.

## Claude Agent ACP 0.108–0.109

The launcher keeps importing `dist/v1/acp-agent.js` with
`CLAUDE_AGENT_ACP_PROTOCOL=v1`. The steering, foreground, autonomous-cycle and
configuration methods it extends are byte-identical, and `idle-compaction.js`
is unchanged.

| Upstream change | Disposition |
| --- | --- |
| 0.108: MCP servers, 0.109: settings, sent to the CLI over stdin | Only without `CLAUDE_CODE_EXECUTABLE`; Cowboy always sets it, so both stay on `--mcp-config`/`--settings`, which the configured CLI already rewrites |
| 0.108.1: a v1 turn of a `subagents` client ends at its result; requests naming a child session answer -32602 | Cowboy only cancels a child; the worker gates cover the turn end |
| 0.109: config options ordered model, effort, fast, ultracode, mode, agent; Fast mode in the `speed` category | Cowboy applies options by ID; `speed` decodes as an unknown category |
| 0.109: `prompt_suggestions_update` only for clients advertising `session.promptSuggestions` | Cowboy does not advertise it, so no prediction request is made per turn |
| 0.108: foreground Bash output for clients advertising `_meta.terminal_output_delta` | Not advertised; Bash results keep their content form |
| 0.109.1: MCP tool titles read `server: tool` | Display only |

## Codex ACP 2.8–2.9 and Codex 0.170–0.171

Seven deltas are kept and rebased; their decisions:

| Delta | Decision |
| --- | --- |
| `native-launch-arguments` | Kept: upstream passes no caller `-c` options |
| `jsonl-frame-reader` | Kept: upstream still drops non-JSON lines without a size bound |
| `steer-prompt-required` | Kept: upstream still advertises only `steering.supported`; the AIR fixtures regain `idleBehavior` beside upstream's new `auth.status` |
| `server-overloaded-failure` | Kept: non-AIR clients still fail only on `usageLimitExceeded` |
| `managed-read-only-profile` | Kept: rebased onto upstream's expanded MCP overrides (MCP-over-ACP) |
| `native-image-profile` | Kept |
| `configured-mcp-startup-wait` | Kept: rebased beside upstream's pre-start MCP failures |
| `native-codex-version` | Dropped: its drop condition is met |

The Cowboy tests of two deltas mock `GetAccountResponse.userId`, required since
0.171.0. Type checks and all 2,614 enabled upstream tests pass with the series
(39 remain skipped upstream). The bundled model catalog of 0.171.0 equals
0.169.1's, so every Recommended preset stays valid.

## Claude Code 2.1.330

Six of eleven native probes match their baselines (file and shell differ only
in probe metadata that the baselines carry). The others:

| Probe | Change | Disposition |
| --- | --- | --- |
| hooks | zsh guard adds `NO_EQUALS` (2.1.328: `=cmd` no longer expands) | The target Bash wrapper follows; baseline updated |
| context | The environment block ends with "In `curl`, send untrusted values with `--form-string`, not `-F`." | The remote context reproduces it; baseline updated |
| tools | Descriptions or schemas of CronCreate, DesignSync, ListAgents, ReportFindings (optional `severity`) and SendMessage; no tool added or removed | Dispositions unchanged (forbidden, not offered, runtime); baseline updated |
| pdf | A heavy PNG is recompressed to a different JPEG | Native still processes target images itself; baseline updated |
| mods | Subagent token counts in a task notification | Baseline updated |

## Grok 1.0.92 and agy 1.5.14

Grok's unauthenticated ACP `initialize` is unchanged apart from a model
description. Its new `features.shared_daemon` applies only to the leader route,
which `--no-leader` keeps off.

agy's `acp --help` and unauthenticated `initialize` are identical to 1.5.9's
apart from the version, and the log formats the backend evidence reads are all
still present.

## Verification

To be completed with the gate results.
