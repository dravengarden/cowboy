# Agent runtime upgrades — 2026-11-10

Status: published from `4002220c` and installed on all four Machines.

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

## Verification

Receipts are under `/srv/storage/fast0/agent/tmp/fable52-upgrade/`. Every
fixture used a loopback scripted API and fake credentials; no real model
request was made.

| Gate | Result |
| --- | --- |
| `just claude-remote-check` | 373 + 4 + 12 passed |
| Eleven native probes against 2.1.321 | As above; one baseline update |
| Claude worker conformance (all twelve phases, packaged adapter) | Accepted, 96 checks, 352 scripted requests |
| Claude managed conformance | Accepted, 7 checks |
| Claude TaskStop conformance | Accepted, four topologies |
| `plugin-check`, `provider-check`, `check-compact` at `4002220c` | Passed |

`remote_impact` also selected `codex-worker` and `session` because of
`flake.nix` changes by other tasks since the 3.20.35 release; this change does
not touch either path.

## Publication

All six releases were built, bound, signed and verified from clean source
`4002220cb4981c22cfbc32343d490b52b4e2b6f3`.

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.36 | `sha256:509fefe231b3828088b26a2da628fafde089e741c5342ba82af2d1965aa7ba1e` |
| claude-deepseek 3.1.58 | `sha256:57d6b98020040ec61aedfb2f716f651f4c4bf80133a44b4c0aa3de1139530390` |
| codex 3.5.28 | `sha256:8fa6d39728d632ce32115880a28657a104885f437ad1c6e6992afb5ded73d519` |
| codex-deepseek 3.1.58 | `sha256:04f1eb3e18db1c6eb3d33febb8f9d0a981ce3c4e7d0788245885031885624fbc` |
| grok 3.1.58 | `sha256:f67719b4696d0ee0653f01b88f8a691072853750a4141c1b882a4dee8a95ba46` |
| deepseek-harness 1.0.16 | `sha256:840ab7b33f5e70fce0bc9b1aaca37f919573fc3eff9d6a18f59a749eebc87a7f` |

| Gate | Result |
| --- | --- |
| Linux old/new generation coexistence with the resident worker | Passed for all six |
| Real macOS arm64 CLI/adapter/gateway probes on macbook-air | Passed for all six |
| Catalog readers before publication | Active (controller profile 597) and rollback (596) each read an isolated copy with all six twice: identical, 71 releases readable |
| Public package and runtime URLs | 26 URLs resolve to their bound digests |
| `provider-release-coverage` | Passed |
| Live Controller Catalog | All six `ready`, component release 3.107.0 |

## Installation

Convergence ran Falcon (canary), macbook-air, Hawk, then OVH, bounded to the
six Plugins.

| Machine | Installed |
| --- | --- |
| Falcon, Hawk | claude-deepseek 3.1.58, codex 3.5.28, codex-deepseek 3.1.58, deepseek-harness 1.0.16, grok 3.1.58 |
| macbook-air | claude-deepseek 3.1.58, codex 3.5.28, codex-deepseek 3.1.58, grok 3.1.58 |
| OVH | claude-code 3.20.36, codex 3.5.28, grok 3.1.58 |

All upgrades returned HTTP 204 and the re-read inventory reports each as
active with its predecessor retained for rollback. Running sessions keep their
generation; new sessions use the new one. The bounded run included
deepseek-harness, so macbook-air's membership also planned removing its
undeclared deepseek-harness 1.0.15; the uninstall preview refused because a
session uses it, and nothing was removed. OVH keeps its undeclared
deepseek-harness 1.0.15 by design.

A disposable session on the new generation stayed dormant: `sessions.wake`
does not start a session that never ran, and no prompt was sent to start it.
The native 2.1.321 binary maps the `fable` alias to `claude-fable-5-2`
(2.1.314 mapped it to `claude-fable-5-1`). The first prompt of a new session
is the live acceptance of Fable 5.2 on a real account.
