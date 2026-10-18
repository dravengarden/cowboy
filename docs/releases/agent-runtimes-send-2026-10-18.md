# Agent runtime candidates and send delivery — 2026-10-18

Status: candidate work in progress; no publication or installation is accepted
by this document. Upgrade the compatible Provider runtimes before releasing
the send interaction changes.

## Dependency audit

The repository's dependency auditor queried npm on 2026-10-18. Candidate
manifests and platform archives use exact registry integrity values.

| Dependency | Accepted pin | Candidate | Decision |
| --- | --- | --- | --- |
| Claude Code | 2.1.296 | 2.1.302 | Test native tool, permission, Mods and execution contracts |
| Claude Agent ACP | 0.89.1 | 0.93.0 | Test with SDK 0.3.300 and ACP SDK 1.9.0 |
| Codex | 0.160.1 | 0.163.0 | Retain 0.160.1: outside the adapter's declared range |
| Codex ACP | 2.2.2 | 2.2.2 | Current; declares native `^0.160.1` |
| Gemini CLI | 0.63.0 | 0.64.0 | Test packaged lifecycle |
| Grok | 1.0.50 | 1.0.62 | Test packaged lifecycle |

Both DeepSeek variants consume the corresponding shared CLI and adapter pins.
Their gateways retain Columbus `27352e3400e5b55c6c88d6e7ac80f788e12dfb53`:
freshly fetched Columbus main has no later change to either gateway directory.
Each affected Plugin needs its own new version and immutable release receipt;
shared runtime component changes also version their dependency closure.

Authoritative sources inspected:

- [Claude tools](https://code.claude.com/docs/en/tools-reference),
  [Mods events](https://code.claude.com/docs/en/plugins/mods/events), and
  [native changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md).
- [Claude ACP releases](https://github.com/agentclientprotocol/claude-agent-acp/releases)
  and its exact `v0.93.0` source.
- [Codex ACP package](https://registry.npmjs.org/@agentclientprotocol/codex-acp/2.2.2).
- [Gemini releases](https://github.com/google-gemini/gemini-cli/releases).

Claude 2.1.302 changes fork-agent defaults and image Read admission; 2.1.301
changes binary-file detection and permission-check interruption reporting.
Native probe results, dispositions and packaged acceptance remain required.

### Native compatibility findings

The 2.1.302 loopback probes passed lifecycle, hooks, skills, context, Mods and
native-behavior checks. Tool inventory changes introduce no newly offered tool:
fork Agent remains outside this remote lane's supported agent types. Cron and
DesignSync remain governed by their existing dispositions.

File probes show that BOM-marked UTF-16LE now reads as decoded text and both
Edit and Write refuse invalid UTF-8 and NUL-bearing files, including replacement
with plain UTF-8 text. The remote adapter preserves that refusal and the original
bytes. Encoding warnings travel as tool context without changing file content
metadata. Native binary-extension refusals now also cover XLSM, Parquet, EPUB
and HEIC. These are measured changes, not relaxed acceptance expectations.

The range helper adds an optional UTF-8 observation and routes encoding-sensitive
files to the full reader. New adapters use bounded full reads with older helpers;
retained Machines remain compatible but do not acquire the optimized range path
until their helper is upgraded. A Provider installation alone does not replace
the Machine worker or keeper.

Base revision `59ba724e` includes the app-shell scrolling fix `a15d9a82` without
an updated component digest. This release registers those existing bytes as
app-shell 1.1.30; it adds no further scrolling behavior.

### Verification before publication

`just check-compact` passed, including Rust, frontend, PostgreSQL and release
build checks. The final Claude source gate also passed after extending the
native binary-file cases and adding retained-helper coverage. The isolated
native worker receipts record 92 Claude checks (including the enclosing worker
checks), 34 Codex checks, 12 keeper checks, 7 managed Claude checks and 10
public-session checks. Native TaskStop parity also passed, including its
unforwarded negative control. The extra binary extensions receive a separate
packaged PDF/file-type phase after the complete worker run.

Receipts and native measurements are under
`/srv/storage/fast0/agent/tmp/send-upgrade-20261018/`. Platform probes,
generation coexistence, Catalog reader acceptance and production installation
must still complete; source tests do not establish publication.

## Send design constraints

Normal submission is queue-first: idle dispatches, busy appends, manually
paused stays held. Queue position is independent of cancellation. Putting a
message first must never imply stopping current work.

The current MCP `queue.submit` already exposes optional `front` and `force`;
only `force` requests interruption. `queue.send` promotes an existing item;
`queue.force_send` interrupts. An observed interruption therefore needs evidence
from the actual operation and its parameters, rather than assuming all MCP
submission interrupts.

Use explicit delivery choices shared by CLI, MCP and UI: after the current
turn (default), supplement the current turn when supported, and stop then send.
Stopping is a one-shot explicit choice, never a sticky default. Preserve the
message identity through editing, withdrawal and promotion. Report submission,
runtime acceptance and failure separately; an accepted control request is not
proof that the agent received or acted on the message.

[Codex App Server](https://learn.chatgpt.com/docs/app-server) has native
`turn/steer`, fenced by `expectedTurnId`, distinct from `turn/interrupt`.
Its presence in native Codex does not prove that an installed ACP adapter
exposes it. Discover capabilities from the exact running adapter.

The pinned Codex ACP 2.2.2 does advertise steering, but an idle/finished-turn
race currently starts a detached prompt. Before Cowboy enables current-turn
delivery, its adapter needs an opt-in `promptRequired` outcome equivalent to
Claude's, with the old default retained for other ACP clients.

Claude ACP 0.93.0 advertises `_meta.steering.supported` and implements
`_session/steering`. Its normal delivery uses native priority `now`, while an
open permission/elicitation uses `later`. Therefore its steering must not be
labelled as an unconditional promise to wait for the current tool to finish.
Its `idleBehavior: promptRequired` option leaves a finished-turn race with the
host, allowing ordinary queue admission rather than an unobserved detached turn.

Unknown, timed-out or disconnected delivery must retain its original identity
and outcome uncertainty. Never automatically resubmit as a new turn after an
ambiguous steering result. Unsupported delivery must be visible and must not
silently fall back to interrupting.
