# Claude Remote upgrade compatibility

Read this for changes to the standard Claude Plugin's CLI, Agent SDK, ACP
adapter, Mods or remote runtime. Remote Claude keeps inference, authentication
and history on the runtime Machine while the bound target owns project files
and processes. Local Claude acceptance does not establish this split.

## Inspect the candidate contract

Use the documented [Mods events](https://code.claude.com/docs/en/plugins/mods/events)
and [types for the candidate build](https://code.claude.com/docs/en/plugins/mods/create#read-the-types-for-your-build).
Mods is an upstream extension surface; retain it rather than patching native
code or maintaining another model-facing tool schema. The docs do not establish
a blanket compatibility guarantee for every future CLI's tool results or
context lifecycle. In particular, a failed hook can continue into native local
execution unless its error handler denies the call.

Compare the accepted and candidate versions, generated event/result types and
actual tool definitions captured by the scripted loopback model. Review the
surfaces Cowboy consumes, rather than investigating every upstream internal
change. Record added/removed tools and changes to input/result shapes, Mods
events, timeout/error handling and context rendering. Classify each newly
exposed tool as target-routed, deliberately native, or refused. Do not forward
unknown tools to `next` or broaden capabilities just to pass an upgrade gate.

The maintained routing list is `NATIVE_TOOLS` in
`plugins/claude-code/runtime/tools.mjs`; read it rather than freezing a count
here. Today the target tools are Bash, Read, Write, Edit, Glob, Grep,
NotebookEdit and TaskStop. `context-mod.js` permits native TodoWrite and
AskUserQuestion and, only for enrolled sessions, the four exact Matrix memory
tools. Unknown tools are denied. Native agents, project hooks/skills, implicit
file attachments, plan files and PDF extraction remain unsupported by this
lane; adopting any of them is a separate capability change.

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

The upgrade report should identify the old/new dependency pins and artifact
digests, tool/Mods contract differences and their disposition, source/native
gate receipts and timings, remaining unsupported surfaces, platform evidence
and authorized installation result. If nothing changed in the consumed
contract, say so with the candidate test evidence; do not manufacture adapter
changes for every upstream upgrade.
