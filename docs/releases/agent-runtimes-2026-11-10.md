# Agent runtime upgrades — 2026-11-10

Status: source integrated; publication and installation below.

## Why

Anthropic released Claude Fable 5.2 (`claude-fable-5-2`) on 2026-11-09. Its
plan article requires Claude Code 2.1.321 or later; Cowboy pinned 2.1.314.
Lookups on 2026-11-10:
[models overview](https://platform.claude.com/docs/en/about-claude/models/overview),
[Fable 5.2](https://platform.claude.com/docs/en/models/fable-5-2/overview),
[Fable models on your plan](https://support.claude.com/en/articles/15424964-claude-fable-models-on-your-plan),
[Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
(2.1.316–2.1.321; there is no 2.1.315 or 2.1.320 section) and
[Claude Agent ACP releases](https://github.com/agentclientprotocol/claude-agent-acp/releases)
(0.100.0–0.102.0). No model request was made to check availability.

## Dependency audit

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.314 | 2.1.321 | Fable 5.2; native probes re-measured, one baseline update |
| Claude Agent ACP | 0.99.1 | 0.102.0 | Agent SDK 0.3.321 and ACP SDK 2.5.0; idle compaction disabled |

Component release 3.107.0 records provider-runtime 1.2.1. Every Plugin that
pins it advances one patch version: claude-code 3.20.36, claude-deepseek
3.1.58, codex 3.5.28, codex-deepseek 3.1.58, grok 3.1.58 and
deepseek-harness 1.0.16. Only the two Claude Plugins change behavior.

## Recommended cards

`fable-high` and `fable-max` keep their IDs and select the `fable` alias,
which 2.1.321 resolves to Fable 5.2 (1M context), the same form as the `opus`
and `sonnet` cards. ACP 0.101.1 lists a resumed session's Fable 5.2 row under
that alias too. A resumed Fable 5.1 session keeps `claude-fable-5-1`; the
existing 5.1 configuration equivalence stays for those sessions. Review and
Superagent select presets by ID, so they follow without change.

## ACP 0.100–0.102

The launcher's adapter extensions (current-turn steering, steering priority,
native cycles and configuration equivalence) use internals that are all still
present. `setSessionConfigOption` is no longer `async`; the wrapper awaits it.

ACP 0.101 sends `/compact` on its own shortly before an idle session's
one-hour prompt cache expires. Native Claude Code arms this only for attended
interactive sessions. In Cowboy it would be a paid, lossy model request that no
Cowboy turn, queue or quota decision owns, once for every idle session over
200,000 tokens. `idle-compaction.mjs` closes the adapter's own gate (its skip
reason is "Cowboy turn ownership"); a changed gate fails the launch. The CLI's
automatic compaction at the context limit is unchanged.

ACP 0.101 also starts a session without an explicit mode in Auto. Cowboy
applies the session's mode preference (default `bypassPermissions`) through its
configuration queue, and a prompt waits for that queue's fence, so the mode a
turn runs under is unchanged.

## Claude Code 2.1.321

Ten of eleven native probes match their baselines. `permissions` differs only
when the fixture's temporary directory is on a separate mount, where git adds
a filesystem-boundary line; from `/tmp` it is identical. The not-offered
`DesignSync` tool has a new description; the tools baseline records it.

| Upstream change | Disposition |
| --- | --- |
| git status keeps the first entry's index column; a failed status reads `(unavailable)` | The target `gitStatus` block follows, with a regression test |
| Background notifications and TaskStop results show only the start and end of a command over 1,000 characters | Not reproduced: the target facade still shows the whole command. Display only |
| git status refuses repository configuration that names filters it cannot switch off | Not reproduced for the target repository |
| Bash permission, sandbox, hook, skill, MCP task and shell snapshot fixes | Re-measured by the permissions, hooks, skill, shell and lifecycle probes: unchanged |
