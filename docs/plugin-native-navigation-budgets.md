# Private whole-query navigation budgets

The Zed `1.14.0` source candidate pairs adapter `1.14.0` with private server
`1.1.0`, retaining the exact upstream revision and third-party pins. It adds a
separate private protobuf request/response (tags 1002/1003, protocol 1), not a
new core grant or a generic Plugin executor. Production navigation remains
closed. Building this candidate does not publish or install it.

The [candidate record](releases/native-navigation-budgets-candidate-2026-09-18.md)
identifies the exact static pair, source and acceptance evidence. It is separate
from the earlier single-input candidate and from any production rollout.

The subsequent adapter-only `1.14.1` [single-use Open fix](plugin-native-open-once.md)
retains this exact server. Ordinary acquisition uncertainty now also fences new
navigation and handoff; neither a missing share nor a failed registration can
trigger implicit close/reopen.

## Two-phase acquisition

Only the five navigation query kinds are admitted. The input uses the pinned
typed LSP query codec, limited to 8 KiB, without caller-selected server or query
identity. An empty input is an effect-free capability probe. An old native pair
cannot substitute synchronization support, health or the upstream query route.

The server captures the original buffer, exact native vector, file object and
worktree. It rechecks that the same native peer still owns that source before
dispatch, every target open and final conversion. It selects at most four
registered, capable language servers in that source language scope before
dispatch, from an existing registration table limited to 32 entries. Selection
does not invoke manifest discovery; a lost registered participant refuses before
dispatch instead of disappearing from the result. Every response must succeed;
an LSP error, including `content modified`, refuses the whole query. It never
omits an error and returns empty/partial success. Each LSP request has a five-
second timeout, with a twenty-second native observation budget for the whole
operation. One private navigation handler at a time is admitted per LSP store.

All results are collected and validated before opening any target: at most 256
locations (duplicates count) and 32 distinct relative target paths across all
servers. Only bounded local file URIs within the original worktree are allowed.
External paths, archive and network schemes, residual parent components, NULs
and reversed ranges refuse. Parsed paths must remain within the original root.
Acquisition uses that original worktree's `ProjectPath` directly, never
the upstream invisible-worktree discovery or archive extraction route.

Each distinct path opens once in the query. Source version/file/worktree checks
run again before each open and final conversion. Exact target UTF-16 ranges must
fit their actual native buffers; the query refuses instead of clipping them.
The complete result is validated before any result is shared or returned. The
adapter still checks original target IDs, content, epochs and worktree, retains
its pins, and explicitly registers newly retained targets.

The [single-input budgets](plugin-native-input-bounds.md) remain independent:
8 KiB LSP headers, 2 MiB LSP bodies and 4 MiB raw/decoded text per file. Together
the path and file limits bound this query's potential new target text to 128 MiB
before native text-buffer overhead. This is not a bound on process RSS, already
retained buffers/history, decoding scratch, background LSP effects or filesystem
worktree scanning. Cancellation/timeout of observation cannot prove that every
background native load has ended.

## Refusal is not recovery

The private response distinguishes Supported, Complete and Refused. Refused has
one closed reason (budget, source, language server, target or deadline) and no
result payload. Adapter decoding rejects mismatched outcome/payload shapes,
unknown reasons, duplicate server results and invalid identities. No fallback
or retry is added.

After Execute has been attempted, even a typed refusal retains the existing
Unknown state and admission fence. LSP execution itself can have side effects;
a later target-load/range failure can follow earlier target acquisition. A
refusal is therefore not proof of no effect or restored state. Original-ID
observation never repeats the query. Native saved-query recovery, independently
authorized restoration and close acknowledgements remain separate work.

## Acceptance boundary

Required evidence includes native multi-server aggregate planner tests, malformed
target and source tests, real native typed refusal with no partial result, all
five nonempty queries against the final static pair, private transport and
Unknown/no-replay gates, signed temporary lifecycle, connected v5 and browser
ownership regressions. Synthetic LSP answers and fixture teardown are not
production language semantics, recovery, supported-device acceptance or a
Machine maintenance receipt. The intended deployed consumer and exact signed
native generation still require independent acceptance before cutover.
