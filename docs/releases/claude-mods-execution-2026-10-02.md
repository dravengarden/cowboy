# Claude native tool Mods, 2026-10-02

Claude Code Plugin **3.3.0** is installed and active on OVH. Source revision
`9cb8fa07fed5c8339b40f7f795f06964fd65d9f7` is published on Cowboy `main`.
The [production receipt](claude-mods-execution-2026-10-02.json) binds the exact
release, installation, recovery and verification evidence.

Remote sessions now route native Bash, Read, Write, Edit, Glob, Grep,
NotebookEdit and TaskStop through Claude Mods and Cowboy's existing execution
binding. The execution MCP server and alias schemas are removed. The agent
uses its native tools while files and commands run on the bound target Machine.
Claude inference, subscription authentication and history remain on OVH.
Native CLI 2.1.287, ACP 0.84.0, Agent SDK 0.3.284 and the Plugin's component
release pin 3.35.0 are unchanged.

The private Mod bridge observes long-running requests without another model
turn or replaying their effects. It denies tool execution on bridge errors or
malformed results. Same-file ordering, stale-read checks, task handles and
concurrent native searches survive resume. Native compaction file snapshots
are suppressed so runtime-local files do not replace target project context.
The [implementation record](../experiments/claude-mods-execution-2026-10-02.md)
explains the native timeout, exception fallback and compaction behavior tested.

## Verification

- `plugin-check`, the complete `just check` and final `provider-check` passed,
  including 19 focused native tool tests.
- The final immutable packaged CLI/ACP passed all 30 execution-worker checks,
  covering target edits and conflicts, images, notebooks, concurrency,
  background cancellation, a 35-second transport outage, compaction, cold
  resume, actual ACP startup/load and failures without native fallback.
- Actual old/new detached workers passed coexistence and descendant drain.
  Active, next-transaction recovery, previous and cold Controller Catalog
  readers accepted the release. All five public artifact URLs matched SHA-256,
  covering 308,006,129 bytes. Production signing was independently verified.
- Postflight reports the exact installed generation, current authentication
  replica/materialization and retained auth generation 51. Sibling Plugin
  installations, Controller, OVH Machine and both existing worker process
  identities/start times are unchanged. `/healthz` reports `ok`.

## Installation receipts

The first operation, `ovh-claude-3-3-0-mods-20261002`, exceeded its observation
deadline and ended with a durable `Unknown/Staging/Expired` Machine receipt.
It never reached activation. Same-ID Operator reconciliation first recorded
that exact receipt, then independently verified the unchanged installation
target and recorded staging resolution
`install-staging-bbdb8ab0aab60564839fe18c0c1d2511a1894cc57836c51fca18b76d521384ee`.
The original uncertain receipt remains intact.

After that resolution released the slot, the public adapter archive was
transferred to a temporary file and imported through the Machine's owned
`--cache-runtime-artifact` command, which verified its digest without installing
it. A fresh authorized operation,
`ovh-claude-3-3-0-mods-cached-20261002`, applied the same signed release. Its
initial response also exceeded the 90-second observation deadline; a temporary
Machine disconnect delayed confirmation. Same-ID reconciliation then read the
Applied receipt and completed normal authentication finalization. No installation
was replayed, and no slot fence remains.

Release digest:
`sha256:82f07338f73ed62cbf7a1c239019a82757a34f324093114f40eb87bd515017c3`.
Installation revision:
`installation-29e1b6af54c6449b0201659a06f6959bd7b17d2c776f64c735c3ceea432f087c`.
No Controller, Machine, Web or NixOS component activation occurred.

## Scope and limits

New bound remote Claude sessions use 3.3.0; existing sessions retain their
generation and placement. Local sessions retain the upstream ACP path.
Native tool schemas still occupy context. Removing the extra execution MCP
surface does not establish a measured token, turn or cross-host latency saving.
Acceptance uses scripted loopback model responses, not subscription inference.
macOS artifacts are built and published but native execution was tested on Linux.

This is tool/context routing, not process-wide filesystem isolation. Native
subagents, project hooks/skills, implicit file attachments, plan files and PDF
extraction remain outside this execution lane. Whole-file transfer, startup
guidance snapshots and non-atomic conflicts against unrelated writers remain.
The app-shell 1.1.18/component matrix 3.36.0 record included in source only fixes
the missing digest entry for previously shipped Web update controls; it changes
neither this Plugin's dependency pin nor Web behavior.
