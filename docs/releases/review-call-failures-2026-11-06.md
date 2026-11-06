# Review call failure audit — 2026-11-06

Read-only inspection of the Service's retained call ledger and child lifecycle
events, for `review` and `design_review` calls created through 05:05 UTC. The
414 retained calls were 356 completed, 52 failed and 6 cancelled when inspected.
This is retained history, not an all-time denominator. Missing historical model
metadata is not evidence that a Claude call used Fable.

| Failure                     | Count | Evidence and disposition                                                                                                                                                                                                                                                                                        |
| --------------------------- | ----: | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `review_incomplete`         |    14 | Several retained results contain source findings but call missing Go/browser/dependencies incomplete. The static-review contract was fixed in `eafe78d52`; missing required inputs still legitimately fail. `997cc85d7` requires stopping on a missing base commit. Do not relabel historical results as clean. |
| `unsupported_input`         |     9 | Submodules, escaping symlinks and unsupported context entries. `0c5f48976` now carries clean submodule commit pointers, not their contents; dirty submodules remain refused. This change corrects the stale blanket submodule hint.                                                                             |
| `snapshot_unavailable`      |     8 | Retained detail is `source_unavailable`. The root must resolve to an authorized Git worktree. `7f38ec242` accepts symlinked source roots and isolates base fetches; absent or unauthorized roots still fail.                                                                                                    |
| `configuration_unavailable` |     6 | Five children's lifecycle events say memory-headroom admission timed out; one reports missing native Codex authentication configuration. `d67ed13cf` distinguishes startup failures from unsupported presets. This change also retains the startup detail in the call error.                                    |
| `child_crashed`             |     6 | Four Claude failures include `Target Bash is unavailable` during execution binding startup; see the reserved sandbox directory diagnosis in `execution-environments.md`. These precede the current execution runtime. Two Codex failures are separate historical child crashes.                                 |
| `structured_output_invalid` |     5 | Retained output exists but fails the review result contract. EndTurn alone is not a successful review. Preserve findings/text for inspection; do not launch another review automatically or interpret arbitrary text as approval.                                                                               |
| `configuration_unconfirmed` |     3 | Two historical Codex failures and the Fable continuation below. Exact observed model/effort evidence remains required.                                                                                                                                                                                          |
| `context_unreadable`        |     1 | A requested file was absent from the snapshot. Caller-side path validation and the existing actionable input hint apply.                                                                                                                                                                                        |

## Fable continuation root cause

There are 25 calls identifiable as Fable from their preset or recorded native
configuration: 22 completed and 3 failed. The failures are one old execution
binding crash, one submodule snapshot refusal, and one configuration timeout.

`call-d4edcf13a448186b117ce531` completed on Claude Code 3.20.26 with
`model=claude-fable-5-1[1m]`, `effort=max`. `call-25ce19915a836c91ec8938d0`
continued the same child/native conversation. The resumed worker advertised
`claude-fable-5-1`, `max`; its model catalog no longer contained the suffixed
spelling. Generic preference reconciliation retired the unlisted spelling, but
the call correctly retained its original intent. Exact string confirmation could
therefore never succeed. It failed after approximately 60.6 seconds, before a
new review prompt/turn was recorded. This was not a Fable inference failure,
exhausted context, or a reason to lower effort. Usage from the successful
preceding turn must not be attributed to the failed continuation; an absent call
usage field alone does not prove zero cost.

The pinned native CLI is 2.1.314 and ACP adapter 0.99.1. Current official
[model documentation](https://code.claude.com/docs/en/model-config#extended-context),
checked 2026-11-06, states that Fable 5.1 has native 1M context without the
suffix. The plugin now annotates these two exact spellings as equivalent using
ACP option metadata. The Controller consumes this provider-owned declaration
only from a connected worker and only when it is bound to that option's actual
current value. It records the real observed spelling. No generic model-name
heuristic, different version, effort downgrade, permissions change, or automatic
paid retry is introduced. Old plugin generations remain unchanged.

## Verification and rollout

Regression coverage includes fresh/recovered option responses, notifications,
stale metadata after a model switch, another model/version, lower effort,
unknown metadata schema, missing observations, and the actual ACP Rust codec.
The private CLI, ACP, SDK and tools remain pinned; this changes configuration
evidence only. Release verification and activation receipts are recorded after
the final artifacts pass their gates.
