# Agent runtime upgrades — 2026-11-22

Status: component releases 3.137.0 and 3.138.0 published from Cowboy main and
installed on Hawk and OVH; Codex 3.6.28 adds predicted prompts. Falcon and
macbook-air wait for a Machine update (Plugin SDK 1.19.0 and 1.18.0; the
releases need 1.20.0).

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
| agy | 1.5.9 | 1.5.9 (1.5.14 withdrawn in 3.138.0) | The install manifest is `manifests/linux_amd64.json`; `linux_x64.json` answers 404. 1.5.14 ends its turns without the stream barrier the backend evidence needs; see below |
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
still present. That was not enough:

1. Antigravity 0.2.22 (3.137.0) on OVH: the account sandbox refused the runtime
   (`runtime_rejected`), because its allow-list
   (Columbus `machines/ovh/account-sandbox/agy-sha512.allow`) lists only the
   1.5.9 executable. OVH went back to 0.2.21.
2. With the 1.5.14 executable admitted (allow-list file only, daemon not
   restarted), one analysis prompt on the Husky account succeeded natively:
   four backend successes and no refusal in the unit's log. But no
   `Stream goroutine exited for <session>, sending completion signal` barrier
   followed, so the turn settled `unknown_refusal` and parked the account.
   (1.5.10 keeps the session stream open after a turn to push autonomous
   replies.) OVH went back to 0.2.21 and the allow-list to 1.5.9 only, so 0.2.22
   fails closed at startup instead of parking an account.

Component release 3.138.0 pins 1.5.9 again (Antigravity 0.2.23, the other
Providers a patch each, their runtime bytes unchanged). 1.5.14 needs the
proxy's evidence to recognise its turn end, a real-account acceptance and the
allow-list entry together.

## Collector checks

| Check | Result |
| --- | --- |
| `just codex-refresh-guard-probe` (0.171.0) | A fake token endpoint got two refresh requests; the collector's refusal URL got none and `auth.json` was unchanged (as 0.169.1) |
| `just grok-billing-capture` (1.0.92) | Request differs from 1.0.77 only in client version and user agent; all 31 projection cases and the 401 answer unchanged; `auth.json` not rewritten |

Both tools are new in this change, so the next pin change re-measures the same
behavior; the capture first reproduced the committed 1.0.77 fixture exactly.

## Verification

Receipts are under `/srv/storage/fast0/agent/tmp/agent-upgrade-2026-11-22/`.
Every fixture used a loopback scripted API and fake credentials unless stated.

| Gate | Result |
| --- | --- |
| `provider-check`, `check-compact` at `69e901460` (3.137.0) and `0f881f3b2` (3.138.0) | Passed |
| `just claude-remote-check` | 386 passed |
| Eleven native probes against 2.1.330 | As above |
| Claude worker conformance (all phases, packaged adapter) | Accepted, 96 checks |
| Claude managed / TaskStop conformance | Accepted (7 checks; four topologies) |
| Codex worker / turn / image / child-stop | Accepted (34 / 9 / 13 checks / 4 cases) |
| Codex native hooks research | Identical to 0.169.1 |
| codex-acp upstream suite with the seven deltas | Type checks and 2,614 tests passed (39 skipped upstream) |
| Old/new coexistence with the resident worker | Passed for the six non-Antigravity Providers against the installed releases (claude-code 3.20.63, claude-deepseek 3.1.80, codex 3.6.22, codex-deepseek 3.1.80, grok 3.2.21, deepseek-harness 1.0.38) |
| Catalog readers before publication | Active (`42gs643j`) and rollback (`l48cffn1`) Controllers each read an isolated copy twice, identical, for both releases |
| Public artifact URLs | 29 per release resolve to their bound digests |
| `provider-release-coverage` | Passed, isolated and live |

The `session` suite the remote map selects for the Codex manifest was not run:
its fixture rebuilds the Codex manifest with an empty environment and a fixture
agent, so neither release changes what it exercises. 3.138.0 changes no runtime
bytes of the six other Providers (their runtime artifacts equal 3.137.0's) and
Antigravity's equal 0.2.21's.

## Publication

| Plugin | 3.137.0 | 3.138.0 |
| --- | --- | --- |
| claude-code | 3.20.67 `sha256:1c83a947…` | 3.20.68 `sha256:f8cc9652…` |
| claude-deepseek | 3.1.84 `sha256:35ba7d92…` | 3.1.85 `sha256:e9cce53e…` |
| codex | 3.6.26 `sha256:8dc600d9…` | 3.6.27 `sha256:fc6a4834…` |
| codex-deepseek | 3.1.84 `sha256:34e5f1ba…` | 3.1.85 `sha256:647fec68…` |
| grok | 3.2.25 `sha256:18aa7122…` | 3.2.26 `sha256:85d0dc81…` |
| antigravity | 0.2.22 `sha256:81d82549…` (agy 1.5.14, fails closed) | 0.2.23 `sha256:643136d2…` |
| deepseek-harness | 1.0.42 `sha256:3985cc86…` | 1.0.43 `sha256:9f4a6e19…` |

## Installation and real-account smoke

`cowboy operator converge` (canary Hawk, then OVH) applied every planned
upgrade; Antigravity, not declared for OVH, was installed by exact release.

| Machine | Installed |
| --- | --- |
| Hawk | claude-deepseek 3.1.85, codex 3.6.27, codex-deepseek 3.1.85, deepseek-harness 1.0.43, grok 3.2.26 |
| OVH | antigravity 0.2.23, claude-code 3.20.68, codex 3.6.27, grok 3.2.26 |
| Falcon, macbook-air | Refused with `plugin_sdk_unsupported` (SDK 1.19.0 / 1.18.0; 1.20.0 required); unchanged |

One managed analysis call per Provider read a file and answered: Claude Code
3.20.67 (Sonnet) and Codex 3.6.26 (Sol 6.2 Medium) completed. Antigravity is
described above; after the rollback the same call on 0.2.21 completed. The Husky
account's control was left in `unknown_refusal` for its owner to clear.

## Codex 3.6.28: predicted next prompt

The Web has offered a predicted next prompt published as
`session_info_update._meta.promptSuggestions` since 2026-10-31 (Desktop ghost
text that Tab fills, a Mobile chip; never sent, live only), but both adapters
predict only for JetBrains AIR clients, so it stayed dormant. Cowboy cannot
claim to be AIR, and its worker's ACP schema (agent-client-protocol 3.6) has no
`prompt_suggestions_update`.

The new `cowboy-prompt-suggestions` codex-acp delta takes the prediction when
the Provider sets `COWBOY_CODEX_PROMPT_SUGGESTIONS=1`: after a completed turn
outside plan mode, one `thread/sample` (high effort) on an ephemeral fork of the
session's thread, whose context is a cache hit, publishes one line or clears it.
A managed call never predicts; `CODEX_ACP_PROMPT_SUGGESTIONS=false` still turns
it off. Claude's prediction stays dormant: claude-agent-acp 0.109 sends it only
as the standard update, and its native generator did not run offline with an
API key, so it waits for the worker to read the standard form.

| Gate | Result |
| --- | --- |
| codex-acp delta tests (`cowboy-prompt-suggestions`, upstream prompt suggestions) | 16 passed; the series' type checks and upstream suite pass in the runtime build |
| `provider-check`, `check-compact` at `7671baa41` | Passed |
| Codex worker / image conformance on the 3.6.28 runtime | Accepted (34 / 13 checks); their harness does not set the opt-in, so turns are unchanged |
| Catalog readers | Active (`mhd8kxx0`) and rollback (`42gs643j`) Controllers read an isolated copy twice: readable |
| Public artifact URLs, live coverage | Resolve to their digests; passed |

Codex 3.6.28 (`sha256:f0c2c5e1abc00ea6ee0d9e9ca627a4bbc87bb4ec3bc0896254a39646761424bf`)
is installed on Hawk and OVH by `cowboy operator converge --plugin codex`, and a
managed analysis call on it completed (managed calls never predict). An ordinary
session's first prediction appears after its next completed turn; it costs one
cached `thread/sample` per completed turn.

## agy 1.5.14 again: a barrier request (components 3.139.0)

Why 1.5.14 left no barrier, from both official executables, read without
running a model (their pclntab maps each loaded string to its Go function):

- The barrier line is still logged by
  `backend.(*ServerBackend).StreamConversationUpdates.func3.1` in both. 1.5.14
  moved the ACP server to `acp/server` and added
  `(*Server).outOfTurnCallbacks` ("Conversation stream of session %s lost
  between turns; the next prompt reopens it") and a turn end decided on the
  stream's IDLE state in `(*Server).promptTurn`: the stream now outlives the
  turn, so the line marks a session's stream end, not a turn's. This is agy
  1.5.10's "a background task or subagent that wakes the agent reaches the
  editor immediately".
- Every other evidence line is unchanged: the success and request-failure
  lines in `code_assist_client/http_helpers.go`, the retry and
  `generating and executing` wrap in `oneharness/internal/core/run.go`, and the
  agent executor (moved from `gemini_coder/framework/executor` to
  `oneharness/internal/agentexecutor`) still reports through
  `errorreport.ReportErrorContext` from the same functions.
- Both log `ACP Request received: <method>` for every request they read,
  before handling it, an unknown extension method included (answered
  `-32601`).

cowboy-antigravity-acp 1.0.19 settles a turn on a request only the proxy
sends after the turn's reply, `_cowboy/evidence_barrier/<random nonce>`
([evidence contract](../../components/provider-runtime/packages/antigravity-acp/README.md#backend-evidence)).
Nothing in Cowboy wakes `agy` between turns: built-in tools are refused (so no
native background task), subagents are the proxy's own sessions and stop with
their turn, and facade commands never reach `agy`. An out-of-turn backend
request would belong to no turn's window, as before.

`just agy-barrier-probe` runs an executable unauthenticated, in a closed HOME
and without network, through the production `AgyLog`:

| Executable | SHA-512 (the allow-list entry) | Barrier line | Barrier reply |
| --- | --- | --- | --- |
| agy 1.5.9 | `800d5ee8…42f3349ea` | `server.go:988] ACP Request received: _cowboy/evidence_barrier/<nonce>` | `-32601` |
| agy 1.5.14 | `02915d29…8baf38f4` | `server.go:213] ACP Request received: _cowboy/evidence_barrier/<nonce>` | `-32601` |

Component release 3.139.0 records provider-runtime 1.3.23 (agy 1.5.14,
cowboy-antigravity-acp 1.0.19); Antigravity 0.2.24 pins it and the other six
Providers advance one patch with unchanged runtime bytes.
