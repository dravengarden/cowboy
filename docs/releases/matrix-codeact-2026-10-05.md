# Matrix TS/JS CodeAct release — 2026-10-05

Enrolled standard Codex and Claude sessions can compose shared memory operations
in TypeScript or JavaScript through Matrix 0.3.0 on OVH. Matrix compiles permissive
input, exposes a strict SDK and runs each program in a fresh V8 sandbox. A lazy
shell can process bounded memory results in private scratch files. Target project
commands retain Cowboy's existing execution binding.

Claude's signed 3.4.7 generation permits the three new exact MCP names:
`memory_read`, `memory_execute` and `memory_receipt`. Its four existing Matrix
tools remain available. Unknown suffixes remain denied. Codex 3.3.2 discovers
the tools dynamically and did not need a new release. New Claude sessions use
the installed routing; retained sessions keep their previous generation.

## Accepted artifacts and installation

Source revision `7727fdd47637d6923cf3758c6233a7196c37556c` passed the full
`just check-compact` gate, including concurrent main changes. The release
preserves the concurrent Claude Recommended preset update. Native CLI 2.1.287,
ACP 0.84.0 and Node 24.21.0 remain pinned. The dependency audit identified newer
CLI/ACP candidates; this release does not upgrade them.

| Provider | Installed version | Signed release digest |
| --- | --- | --- |
| Claude Code | 3.4.7 | `sha256:b726429401c865dded1996a965a0e3b9ad65ae699602d39cf87a08047fccfbe1` |
| Codex | 3.3.2, retained | `sha256:a833d5b617922fc67938bd5ddf9a43bf9dd928caee2a3242a869014357c3e08f` |

The canonical [release skill](../../.agents/skills/release-cowboy-plugin/SKILL.md)
was used for signed publication and normal Operator installation. Linux x86_64
and actual macOS arm64 dependency probes passed. A real Linux worker initialized
the new generation, coexisted with 3.4.5 and drained its descendants on stop.
Active, next-transaction recovery and cold Controller readers accepted the
release. Complete prospective Catalog and actual host-policy reads passed for
both distinct reader binaries, including the independently published Cardea
fixture. The production publisher signature was independently verified. All
five public package/runtime URLs were fetched and their digests matched.

Normal Operator operation `ovh-matrix-codeact-claude-3-4-7-20261005` completed
with Applied revision
`installation-dc03653a8706a669660ee131d13100ab1dac4413db52dcce5943cc0ef56e20e8`.
The first observation reached its 90-second timeout after the Machine had
applied the release. Reconciliation reused that operation ID and recovered the
terminal receipt. Final inventory showed both standard Providers active with
current credential replicas and materializations. Claude 3.4.5 remains the
rollback generation. No Provider credentials are included in this evidence.

Matrix was activated independently at product revision
`67300627e1a5922058f6eeb308d8545b5be737ec`, with storage schema 2 and a compatible
0.2.3 rollback. Its private `mymemory` instance made and pushed a checkpoint after
the live acceptance calls. Only Matrix services restarted. This task did not
activate a Cowboy Controller, resident Machine, Web, iOS or Columbus release.

## Native and Remote acceptance

The [machine-readable evidence](../acceptance-results/matrix-codeact-2026-10-05.json)
retains each gate's limits separately:

- Six direct native checks used the exact packaged Codex and Claude clients:
  HTTP MCP discovery, shared recall, TS/JS compilation, batch reads, lazy shell,
  atomic commit and completed-turn capture.
- Codex Remote passed 25 checks, including real target execution, transport
  interruption, cancellation, backpressure, native cold resume, ACP cold load,
  Matrix scope/capture and CodeAct.
- Claude Remote passed 36 checks, including native target tools, actual
  compaction, cold resume, ACP cold load, context ownership, interruption,
  concurrent reads, closed failure behavior and Matrix CodeAct.
- The actual Claude CLI validated the generated Mod manifest and hooks. It
  reported only absent description/author metadata. Its `plugin test` command
  found no native `*.test.ts`/`*.test.tsx` suite and exited 1; this is not recorded
  as a passing native Mod unit suite. Source routing and packaged native/Remote
  gates supplied the behavioral coverage.

The fixture now carries Codex's exact `codex-code-mode-host` companion and uses
`SYSTEMCTL_FORCE_BUS=1` for the contained Matrix supervisor in its PID namespace.
That [documented systemd option](https://github.com/systemd/systemd/blob/main/docs/ENVIRONMENT.md)
selects the authenticated user bus; guest
isolation and production policy were not relaxed to pass the tests.

The [official Claude tool reference](https://code.claude.com/docs/en/tools-reference),
[Mods events](https://code.claude.com/docs/en/plugins/mods/events) and
[creation/validation contract](https://code.claude.com/docs/en/plugins/mods/create)
were consulted on October 5. The consumed contract comparison is:

| Surface | Old to new | Disposition and evidence |
| --- | --- | --- |
| Native built-ins and Mods event types | Same exact CLI 2.1.287 bytes and existing capabilities | Retained target routing, validation, scripted native inventory and full Remote gate |
| Existing four Matrix MCP tools | Retained | Shared recall/capture and compatibility checks |
| `memory_read`, `memory_execute`, `memory_receipt` | Added to the exact enrolled allowlist | Source routing tests and native/Remote CodeAct calls |
| Unknown Matrix tool names | Still unavailable | Explicit refusal coverage; no prefix-wide forwarding |

The scripted model fixtures make native clients issue real tool calls without
subscription inference. They do not establish real-model task quality or a new
Cowboy-managed cross-host production canary. Separate production MCP calls used
both enrolled Provider identities on OVH and verified shared reads, correction,
atomic writes, aborts, receipts, replay, scope exclusion and forgetting. All
synthetic facts were forgotten. Matrix API restart and a Git restore using the
compatible rollback reader passed. See the independent
[Matrix acceptance record](https://github.com/dravengarden/matrix/blob/main/docs/acceptance-codeact-2026-10-05.md).

## Operational baseline and follow-up

Matrix passed 75 tests locally and on OVH, including 11 contained runtime tests,
and strict TypeScript SDK checks with four negative assertions. Small sequential
production loopback samples measured p95 of 12.1 ms for one query, 210.9 ms for TS
search plus batch read and 445.7 ms for JS with shell. These perform different
work and exclude inference and cross-host latency; they are not an SLA or a
demonstration of token savings.

Private metrics retain 35 days and distinguish product/retrieval versions,
latency, bytes, calls, cache hits and feedback. At acceptance, new-policy
production recall had two events and zero usefulness judgments. Review between
October 12 and 19 after collecting representative useful/missed/stale/incorrect
labels. Exclude synthetic acceptance traffic. Grok, semantic embeddings and
claims of superiority over native memory remain outside this release.
