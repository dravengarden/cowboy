# Agent runtime upgrades — 2026-11-17

Status: component release 3.109.0 in source. Publication and installation are
recorded below once they complete.

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
