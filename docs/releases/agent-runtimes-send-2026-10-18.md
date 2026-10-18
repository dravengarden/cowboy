# Agent runtime candidates and send delivery — 2026-10-18

Status: compatible runtime releases published from `de3b2846538f6244935e9e86e42917e016a7320b`;
fleet Provider installation is complete. Send interaction changes are not released.

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
`/srv/storage/fast0/agent/tmp/send-upgrade-20261018/`. All six signed releases
passed Linux old/new generation coexistence and real macOS CLI/ACP probes.
`release/publication.json` records ready Catalog entries and the verified bytes
of all 26 public package/runtime URLs. These receipts establish publication,
not replacement of existing session workers.

### Catalog recovery scope

The exact active Controller and accepted recovery closure both read all six
candidates twice, before and after publication. The recovery identity comes
from `current.json.recoveryRelease` and the `accepted-recovery` root, rather
than the historical `previousRelease`: the installed activator's
`checkedRollbackTarget` uses the explicit recovery release and refuses a
predecessor downgrade if it is unavailable. No incomplete transaction existed.

The historical predecessor supports Provider SDK 3.1.13 and safely omits these
3.1.14 releases, as it already omits the preceding published Provider versions.
It is not an accepted recovery target. The NixOS bootstrap initializer only
sets a missing component profile; it does not replace the existing accepted
profile during normal boot. The reader receipts distinguish availability in
active/recovery readers from historical/bootstrap readability. Do not use a
historical closure to recover this Catalog or reset the component profile.

### Fleet installation

Hawk, OVH, Falcon and macbook-air now have their declared Agent Providers at
the signed versions above. Final convergence reports no remaining Agent Provider
upgrades; Falcon's unrelated Zed upgrade remains outside this task.
OVH's first Claude staging attempt expired before activation. Its exact durable
receipt was reconciled as an activation-free staging failure before one new
operation was submitted successfully. The uncertain operation was not replayed.

Falcon and macbook-air first required the current Machine SDK. Falcon's owned
component activator committed transaction `1792338601782363545-de3b2846538f`
with worker `worker-feb2a5fcf757b8576126`. Mac's pinned Rust 1.98.1 build and real
installer integration test passed; its owned `--refresh` installed the clean
`de3b2846` bundle. Both now advertise Plugin SDK 1.12.1 / Provider SDK 3.1.14.
Mac identity and all seven workspaces are unchanged; its previous executable
bundle and launcher are retained in `recovery-send-provider-de3b2846`.
Both Machines had zero active sessions before maintenance. Existing Hawk/OVH
session workers were not recycled by these Provider installations.

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

Upstream Codex ACP 2.2.2 advertises steering, but its idle/finished-turn race
starts a detached prompt. The send candidate adds an opt-in `promptRequired`
outcome equivalent to Claude's, with the old default retained for other ACP
clients. Its initialization snapshots include that additional capability;
historical baseline fixtures stay unchanged, with the extension recorded as an
explicit expected difference.

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

### Send candidate verification

The send candidate uses Codex Plugin 3.5.2, Claude Code Plugin 3.19.46 and
component registry 3.58.0, without further native runtime pin changes.
`send-check-compact-2.log` records the passing full repository gate; the final
frontend run has 2291 passing tests. The packaged Codex ACP build passes 1510
upstream tests, with 36 upstream skips. Native Claude, Codex, managed Claude and
Codex image execution receipts are in `send-candidate/`. Firefox and Chromium
passed both themes, followed by trusted keyboard acceptance; screenshots are
under `send-desktop-acceptance-final/` and were inspected.

Current-turn admission also checks the immutable negotiated Machine protocol,
not just adapter metadata. Protocols before 34 neither advertise this action
nor admit it, including a newer worker behind an older Machine host. Worker
epoch, active turn, workspace identity and drain state are checked separately.
The additional protocol-gate tests and strict Clippy check passed. The public
session fixture passed all 10 checks, including Controller and target-Machine
restart without replay. Its temporary directory uses `/tmp` so the disposable
Controller socket remains within the Unix socket path limit.
