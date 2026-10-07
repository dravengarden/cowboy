# Claude Remote upgrade compatibility

Read this for changes to the standard Claude Plugin's CLI, Agent SDK, ACP
adapter, Mods or remote runtime. Remote Claude keeps inference, authentication
and history on the runtime Machine while the bound target owns project files
and processes. Local Claude acceptance does not establish this split.

## Inspect the candidate contract

For every Claude runtime upgrade, query current official
[tool documentation](https://code.claude.com/docs/en/tools-reference),
[Mods events](https://code.claude.com/docs/en/plugins/mods/events),
[types for the candidate build](https://code.claude.com/docs/en/plugins/mods/create#get-type-definitions-for-your-version)
and the release notes for the old-to-new version interval. Record the lookup
date, links and exact versions in the upgrade evidence; memory, registry tags
and a historical audit are not a substitute for this lookup. Batch independent
lookups and focus follow-up reading on differences and consumed behaviors.
Mods is an upstream extension surface; retain it rather than patching native
code or maintaining another model-facing tool schema. The docs do not establish
a blanket compatibility guarantee for every future CLI's tool results or
context lifecycle. In particular, a failed hook can continue into native local
execution unless its error handler denies the call.

Compare the accepted and candidate versions' generated event/result types and
actual tool definitions captured by the scripted loopback model. The generated
`.claude-plugin/types/claude-code-tools/index.d.ts` contains built-in inputs and
results; `claude-code/index.d.ts` describes Mods events and methods. Exact-build
types and observed tools take precedence when current docs describe another
version. Inspect the complete inventory, including native pass-through and
currently refused tools, rather than only the target routing list. Distinguish
removal from conditional availability by comparing the same platform, launch
flags, model/configuration and fixture capabilities; the public docs describe
tools that may not be exposed in a particular session.

Record a compact difference table: tool, old/new availability, schema/behavior
change, disposition and supporting test. Include unchanged consumed contracts
as a summary, rather than copying every schema into the report. Handle each
kind of difference before accepting the candidate:

| Difference | Required disposition |
| --- | --- |
| Added tool | Decide whether the feature belongs in this Remote lane. Implement target routing when needed, allow native handling only when it preserves target/context ownership, or explicitly retain refusal with a reason. Add coverage for that decision; discovering a tool does not require supporting it. |
| Removed, renamed or deprecated tool | Check routing, native pass-through, descriptions, launch allow/disallow lists, model instructions, task handles and tests for dependencies. Use a supported replacement or report the lost capability and block a candidate that breaks required behavior. Do not keep advertising or invoking a missing tool. Preserve compatibility for retained older session generations and their rollback artifacts. |
| Input/result or behavior change | Adapt the affected implementation and assertions. Check defaults, limits, file/read-edit semantics, permissions, cancellation, concurrency, task lifetime and context/resume behavior as relevant. An unchanged name or schema does not prove unchanged behavior. Add a regression scenario for the observed change. |

Do not forward unknown tools to `next` or broaden capabilities just to pass an
upgrade gate. If the docs/types are unavailable or conflict with observations,
retain the accepted version and report the unverified contract; do not mark the
candidate compatible. Review the surfaces Cowboy consumes rather than reverse
engineering unrelated upstream internals.

The maintained routing list is `NATIVE_TOOLS` in
`plugins/claude-code/runtime/tools.mjs`; read it rather than freezing a count
here. Today the target tools are Bash, Read, Write, Edit, Glob, Grep,
NotebookEdit and TaskStop. `context-mod.js` permits native TodoWrite and
AskUserQuestion and, only for enrolled sessions, the exact Matrix memory tools
listed in that module. Native `Agent` runs background subagents whose own tool
calls carry `agentId` through the same routing; `SendMessage` and `TaskStop`
reach native only for this session's registered agents. Unknown tools are
denied. Custom agents, agent isolation, plan files and attachments that
describe the runtime's files remain unsupported by this lane; adopting any of
them is a separate capability change.

Target project hooks (`.claude/settings.json` and `settings.local.json`, read
from the target at session start) are passed to native with `--settings`.
Native runs lifecycle and native-tool hooks; `CLAUDE_CODE_SHELL_PREFIX` routes
each registered command to `hook-proxy.mjs`, which runs it on the target. The
facade answers target tools before native hooks, so `context-mod.js` runs
PreToolUse, PostToolUse, PostToolUseFailure and PermissionRequest for them and
reproduces native's input, output folding and model-visible messages. Each
candidate CLI must be re-measured against these assumptions: the shell
prefix gets shell-form hooks as one argument with placeholders unsubstituted;
exec-form hooks bypass it; `classic.*` events (including `SubagentStart`)
carry the base hook input; facade calls never reach settings tool hooks;
PermissionRequest hooks race the host prompt; and a non-zero Bash exit is a
tool error. Bash parity is checked against
`tools/claude_shell_native_baseline.json`: re-run the native-local probe for a
candidate CLI (the shared cases are in `tools/claude_shell_cases.py`) and
re-capture native's command line, environment and snapshot generator, since
the facade reproduces them. Process lifetimes are checked against
`tools/claude_lifecycle_native_baseline.json` (cases in
`tools/claude_lifecycle_cases.py`, probe `tools/claude_lifecycle_native_probe.py`):
a command ends with its shell, a stop kills the whole tree, and the stop's
tree kill exists because the snapshot generator leaves `set -o monitor` on;
re-measure all three per CLI. PDF and file-type Reads are checked against
`tools/claude_pdf_native_baseline.json` (probe `tools/claude_pdf_native_probe.py`,
run in the dev shell for poppler): re-measure native's page limits, `pdftoppm`
arguments, binary-extension list and messages per CLI. Target skills and
commands are checked against `tools/claude_skill_native_baseline.json` (cases
in `tools/claude_skill_cases.py`, probe `tools/claude_skill_native_probe.py`).
They rely on measured native behavior to re-verify per CLI: plugin skills are
named `<plugin>:<name>` and listed only with a description; the initialize
`skills` allowlist filters them by full name; a Mods `tool.call` that changes
a Skill result drops the skill's messages, so names are projected in
`session.append` instead; `skill.prompt` sees the expanded text after native
would have run `!` commands (the mirror marks them so native does not); stored
attachments, the skill listing among them, render again on later requests;
and the attachment types `context-mod.js` drops as runtime-located
(`RUNTIME_ATTACHMENTS`) still cover native's producers. Completion notifications for target commands rely
on a plugin's `$.tool.call` of a native background Bash being notified like
the model's own, on the `<task-notification>` element shape the Mod rewrites,
and on `$.session.receive` staying unavailable (re-measure: if a later CLI
offers a native notification injection, prefer it over the waiter task);
the `*notification*` checks cover these. Session context relies on native's
instruction discovery (no AGENTS.md; CLAUDE.md, rules, imports, nested files),
on `prompt.context` rendering `instructionFiles`, on the `session_context`
attachment's `# gitStatus` section and on the `tool.call hook additional
context:` label this module removes; re-measure them per CLI (the
`*instruction*` and `*git*` checks). The packaged hook checks (`*_hook_*`/`*_hooks_*`,
`nonzero_bash_exit_is_native_tool_error`) cover these; the native baselines
are in the project hooks receipt.

Target tools are gated by native `$.tool.check` under the session's permission
mode, with asks raised to the SDK host as native `can_use_tool` requests by the
launcher. Re-verify for each CLI that `check` still agrees with native prompts
per mode (dontAsk is converted by the launcher) and that the request/response
shapes are unchanged; the `native_permission_*` and mode checks cover these.

Agent support depends on observed Mods behavior, so re-verify it for every CLI
candidate: the idle and queued (`delivery`/`queued_command`) notification
shapes, `turn.complete` agent outcomes, `next.signal` on abandoned held calls,
and that a pending Mods fetch blocks other native work (the bridge keeps each
observation to about one second). The packaged runner's `native_agent_*` and
`parent_turns_and_taskstop_progress_during_child_command` checks cover these.

Check shared CLI/SDK/ACP consumers through the parent upgrade workflow. Standard
Claude and Claude DeepSeek have different remote/authentication contracts.
Keep the builder's exact native-version guard until candidate acceptance exists;
in the isolated candidate worktree, change that guard and its emitted native
version together when needed to build the artifact under test. Never publish
that candidate or claim it accepted from a version probe alone.

## Fast feedback, then packaged acceptance

Run the cheap source gate before downloads, cross-platform builds or signing:

```bash
nix develop -c just claude-remote-check
```

It exercises routing for every supported tool, exact native pass-through,
unknown-tool refusal, authenticated bridge observation without resubmission,
error denial, file conflicts, result shapes, task cancellation and context
transforms. It uses disposable fixtures and no real Provider credentials.
It does not emulate Claude's implementation of Mods validation or timeouts.

Once the exact candidate CLI is available, use its official
[Mods validation and test commands](https://code.claude.com/docs/en/plugins/mods/create#test-the-mod)
for early feedback: `claude plugin validate <mod-directory>` and
`claude plugin test` from that directory. Use the immutable candidate executable,
an isolated fixture home and the actual Mod source; do not run whichever
`claude` happens to be on PATH. Native plugin tests need no session, sign-in or
network. Reuse relevant native tests when available; do not create a second
copy of the complete cross-Machine suite. Check the candidate's supported
commands and record absent facilities rather than assuming every version has
them. Validation and native Mod tests supplement, not replace, the source and
packaged execution gates.

For every CLI, SDK, ACP or remote-runtime candidate, build the actual immutable
adapter/native bytes through the parent builder and run:

```bash
nix develop -c just execution-worker-conformance \
  <claude-input.json> <new-receipt.json>
```

Use `provider: "claude-code"`, the exact CLI version/digest, executor
version/digest and packaged `adapter_launcher`. Read the input contract in
`src/worker_execution/tests.rs` and the current scenarios in
`tools/execution_claude_worker_conformance.py`; reuse their temporary Machines,
real worker/keeper and scripted loopback API. Do not substitute direct adapter
unit tests, production credentials or paid inference. Require `accepted: true`
and the current named scenarios, not a hard-coded historical check count:

- All routed tools execute against target bytes/processes; runtime sentinels
  stay unchanged. Cover native result validation, Unicode/CRLF, images,
  notebooks, range reads and stale-read conflicts.
- Concurrent searches, retained background output, cancellation and resume
  preserve task identity; lost receipts and transport outages never replay
  effects.
- Startup readiness makes zero model requests. Tool, title/auxiliary and
  compaction requests retain target context; cold ACP load and native resume
  preserve history and binding.
- Broken modules, bare mode, malformed native results and bridge failure deny
  execution without native local fallback.

When contract inspection finds an uncovered behavior change, add a focused
scenario to this existing runner before accepting it. The source hook tests
are fast fault isolation; only the real pinned CLI establishes that Claude
still loads, intercepts and validates the Mod correctly. See the
[Mods implementation record](../../../../docs/experiments/claude-mods-execution-2026-10-02.md).

## Keep upgrades efficient

For presets/descriptions only, with unchanged runtime inputs and routing,
reuse accepted immutable runtime artifacts through the owned builder; run the
configuration/package gates rather than rebuilding the native execution suite.
Never use that shortcut for CLI/SDK/ACP, launch environment, context or routing
changes, even when upstream calls the release a patch.

Reuse digest-verified downloads, Nix build outputs and compiled test binaries.
Batch independent source checks; run costly packaged acceptance after those
pass. A conformance receipt is reusable only for the exact tested
CLI/adapter/runtime/executor bytes and test-source revision, with all required
scenarios present. A different candidate needs new acceptance. Record duration
and distinguish source checks, artifact build and native acceptance; do not
claim faster upgrades without timings. Build/publish all declared platforms,
but keep Linux acceptance separate from macOS execution evidence.

Run the parent's remaining deterministic gates and immutable release lifecycle
once the candidate passes. Publication, installation and actual installed
generation remain separate facts. For an authorized installation, use the
existing canary-first convergence and saved operation identities; preserve
active session leases and retained generations. A failed canary stops further
rollout. Do not restart live sessions to make them use the new version.

The upgrade report should identify the dated official-doc lookup, old/new
dependency pins and artifact digests, the tool difference table and its
dispositions (including removals and behavior-only changes), source/native
gate receipts and timings, remaining unsupported surfaces, platform evidence
and authorized installation result. If nothing changed in the consumed
contract, say so with the candidate test evidence; do not manufacture adapter
changes for every upstream upgrade.
