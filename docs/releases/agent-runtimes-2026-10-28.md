# Agent runtime upgrades — 2026-10-28

Status: published from `46ee8d9a` and installed on all four Machines.

## Dependency audit

The repository auditor queried npm on 2026-10-28. Pins use exact registry
integrity values; Codex platform archives use their version-suffixed packages.

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.302 | 2.1.312 | Native probes re-measured; three behavior changes adopted |
| Claude Agent ACP | 0.93.0 | 0.93.0 | Retained: 0.94+ moves to ACP SDK 2 under Cowboy's prototype hooks; separate change |
| Codex | 0.160.1 | 0.164.0 | ACP 2.3.0 declares `^0.164.0`; 0.165.0 is outside that range |
| Codex ACP | 2.2.2 | 2.3.0 | Owned patch rebased (see `plugins/codex/runtime/README.md`) |
| Grok | 1.0.62 | 1.0.70 | Packaged lifecycle |
| Gemini CLI | 0.64.0 | 0.64.0 | Gemini is disabled for this Service; not upgraded |

Both DeepSeek variants consume the corresponding shared CLI and adapter pins;
their gateways retain Columbus `27352e3400e5b55c6c88d6e7ac80f788e12dfb53`.
Component release 3.61.0 records provider-runtime 1.1.16 and the six dependent
Agent Plugins:

| Plugin | Previous published | Released |
| --- | --- | --- |
| claude-code | 3.20.5 | 3.20.6 |
| claude-deepseek | 3.1.37 | 3.1.39 |
| codex | 3.5.4 | 3.5.5 |
| codex-deepseek | 3.1.36 | 3.1.38 |
| grok | 3.1.38 | 3.1.40 |
| gemini | 3.1.36 | 3.1.38 (source only; not published) |

Authoritative sources inspected on 2026-10-28:

- [Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
  for 2.1.303–2.1.312, and the exact 2.1.312 binary for undocumented defaults.
- [Codex ACP v2.3.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.3.0)
  and its exact source from `v2.2.2`.
- [Codex 0.164.0](https://github.com/openai/codex/releases/tag/rust-v0.164.0).

## Claude Code 2.1.312

All eleven native probes ran against the exact binary. PDF, skill,
permissions, context and hooks match their baselines; shell and file results
are unchanged. Differences and dispositions:

| Observation | Disposition | Coverage |
| --- | --- | --- |
| 2.1.304 enables tool search by default behind a custom `ANTHROPIC_BASE_URL`; the default was already on for first-party API sessions | claude-deepseek sets `ENABLE_TOOL_SEARCH=false`, keeping its gateway traffic unchanged. Standard Claude production is unaffected. Fixture environments set it too, so baselines keep measuring the full inventory | `plugin_runtime_conformance_test.py` |
| New `Monitor` tool streams a command from the runtime's own processes | Forbidden in the remote lane; target commands notify through background Bash | `claude-remote-tools-baseline.test.mjs` |
| Bash and AskUserQuestion descriptions changed | Target tools keep native descriptions; shell parity cases are unchanged | tools baseline |
| A timed-out Bash command leads with the output read so far, adds a TaskStop sentence, and says when no output was read | The remote Bash reproduces the measured text, including trimming | `claude-remote-shell.test.mjs`, new lifecycle `timeout_moves_output` case |
| Background notifications carry a preceding `tools-used-before-summary` reminder | Mods baseline refreshed; notification projection unchanged | `claude-remote-mods-baseline.test.mjs` |

Native first-party tool search was confirmed in both 2.1.302 and 2.1.312 by
the CLI's mode selection, which differs only in classifying a custom base URL.

## Codex 0.164.0 and ACP 2.3.0

The ACP 2.3 rebase is described in `plugins/codex/runtime/README.md`. Codex
0.164.0 now stops a CodeAct child's process tree on interrupt, as it already
did for direct commands. The child-stop probe records that change; no product
code relied on the retained tree.

## Verification

Receipts and logs are under
`/srv/storage/fast0/agent/tmp/native-bump-20261028/`.

| Gate | Result |
| --- | --- |
| `just claude-remote-check` | Passed |
| Claude worker conformance (all twelve phases, packaged adapter) | Accepted, 95 checks |
| Claude TaskStop conformance | Accepted |
| Claude managed conformance | Accepted, 7 checks (test now resolves its harness from the crate root) |
| Codex ACP typecheck and upstream tests | 1,937 passed, 36 skipped upstream |
| Codex turn / child-stop / hooks native probes | Accepted (9 checks / 4 cases / 10 hooks) |

The Codex launch test's prompt-length bound now admits the published Review
1.0.6 guidance (854 characters), which had already exceeded the old bound on
`main`.

## Publication

All six releases were built, signed and verified from clean source
`46ee8d9ae88c8ec6bc796437efe7cdec79c9c89c`. The final Claude and Codex
adapter archives are byte-identical to the accepted candidates above.

| Gate | Result |
| --- | --- |
| Linux old/new generation coexistence with the resident worker | Passed for all six |
| Real macOS arm64 CLI/adapter/gateway probes on macbook-air | Passed for all six |
| Catalog readers before and after publication | Active, rollback (`previousRelease`) and accepted-recovery read all six twice; the bootstrap-only reader skips them safely |
| Public package and runtime URLs | 26 URLs resolve to their bound digests |
| `provider-release-coverage` | Covered for all six |

The Controller receipt has no explicit `recoveryRelease`, so the rollback
reader is its `previousRelease`; the accepted-recovery root was read as well.
Gemini is disabled for this Service and is not advertised, but its 3.1.38
release keeps component-release coverage complete for later Controller builds.

## Installation

Convergence ran Falcon (canary, no active sessions), macbook-air, Hawk, then
OVH. All fifteen upgrades returned HTTP 204 with nothing remaining:

| Machine | Installed |
| --- | --- |
| Falcon, macbook-air, Hawk | claude-deepseek 3.1.39, codex 3.5.5, codex-deepseek 3.1.38, grok 3.1.40 |
| OVH | claude-code 3.20.6, codex 3.5.5, grok 3.1.40 |

No live session was recycled. Idle sessions move through the existing idle
update policy or `session-update --when-idle`. Claude 3.20.0 sessions keep their
runtime because their native session contract changed; named-account sessions
pinned to 3.20.1 need a verified login transition before they can move.

## Remaining dependency findings

`cargo deny` still reports two advisories outside this change:

- RUSTSEC-2026-0387 (crypto-bigint 0.5 timing) through p256 0.13 and
  web-push-native 0.5. No 0.5 fix exists; it needs a p256 migration.
- RUSTSEC-2026-0386 (tokio `AsyncFd`) in the Zed adapter's own lock. Updating
  it changes the Zed Plugin source and needs its own Zed release.
