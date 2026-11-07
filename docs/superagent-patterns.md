# Super-agent patterns: scoped Markdown and continuous refinement

Cowboy supervision uses a dedicated, visible Astra Max or Fable Max session to
assess completed turns and ask the original worker to finish already-authorized
work. Native agents keep owning execution, goals, plans, compaction and history.
No session participates merely because these files exist.

## Setup

1. Keep profiles and patterns in their owning repositories. A private library or
   an existing plugin can supply owner-global patterns. Use one explicit chain:
   **owner global → workspace → project**. Suger and Columbus have separate
   workspace procedures; an external project retains its own delivery rules.
2. Copy the relevant examples into those source repositories and adjust their
   canonical guidance references. Each source root is explicit; a UI folder or
   filesystem parent does not imply inheritance.
3. Build an import bundle with
   `python3 tools/superagent_bundle.py --source ROOT PROFILE --source ROOT PROFILE`.
   The helper includes only declared profile, pattern and guidance files and
   refuses paths escaping a source root. Review the JSON and import it in
   **Settings → Agent tools → Super-agent**. Import replaces the library
   atomically and validates existing bindings. It does not change the source
   Markdown files.
4. Create an ordinary dedicated session, select Astra Max or Fable Max, and bind
   the idle session to a project/profile in Settings. It retains its native
   context, account and execution Machine. Coordinator and worker sessions must
   currently execute on the same Machine.
5. In a worker's **Tools → Super-agent**, select the project, describe its
   already-requested task scope, and choose Off, Observe, Coordinate or Inherit.
   Selecting a project starts that worker Off. Coordinate requires a nonempty
   task scope and a complete pattern catalog.
6. Start with Observe and inspect the coordinator's visible transcript. Enable
   Coordinate when its judgments are useful. Each human task allows two
   follow-ups by default, configurable from one to five.
7. Enable **Propose pattern improvements** to retain Markdown candidates.
   Download candidates from recent activity, review their evidence and scope,
   update the owning procedure, and import the revised library.

A project's mode is a **default**, so explicit session overrides remain in
effect. Inherit restores that default; Off excludes the worker from scoped reads
and new follow-ups. Removing project supervision unenrolls its workers and
releases the coordinator role without deleting any session. Unbinding a busy
coordinator is refused.

The mode and task-scope fields do not grant publication authority. Existing user
instructions and project guidance still decide whether push, deploy or other
external effects are authorized. Reuse existing authorization; do not ask
repeatedly for work already authorized. A research task remains research.

## Native access boundary

The coordinator's Cowboy MCP surface allows scoped catalog/document/context
reads, follow-ups, proposals, help and its own receipts. General Cowboy control,
Calls, preference editing and unrelated session reads are refused. An Off
coordinator keeps this role until explicitly unbound.

**Native filesystem, shell, other MCP servers and historical native context
retain their ordinary session permissions.** This release requires a trusted,
dedicated coordinator with appropriate native access. It is not a hostile-agent
sandbox, and Off cannot erase information already present in native context. The
coordinator is instructed to leave project execution to its workers.
Provider-enforced tool isolation remains a separate signed Provider/worker
maintenance boundary.

UI configuration requires the existing product edit permission. Agent
configuration uses the same writer and preserves the execution-Machine control
boundary: a remote agent cannot alter another Machine's supervision indirectly
through a shared source or project setting. Bound coordinators cannot broaden
their own role.

Ordinary Operator agents retain the existing owner-authorized settings control;
enrollment is an explicit settings write, not an inferred task permission.
The mandate is descriptive and never authorizes a push or deploy. Limiting every
ordinary agent to self-enrollment would remove the requested AI project/global
configuration capability. Dedicated coordinators cannot use that control.

## Files and resolution

Examples:

- [Global profile](examples/superagent-patterns/global.md)
- [Suger workspace](examples/superagent-patterns/suger.md)
- [Columbus workspace](examples/superagent-patterns/columbus.md)
- [Cowboy project](examples/superagent-patterns/cowboy.md)
- [Follow-through supervisor](examples/superagent-patterns/patterns/follow-through.md)
- [Suger procedure](examples/superagent-patterns/patterns/suger-delivery.md)
- [Columbus procedure](examples/superagent-patterns/patterns/columbus-delivery.md)
- [Cowboy procedure](examples/superagent-patterns/patterns/cowboy-delivery.md)

Both file types use `+++` TOML frontmatter followed by Markdown. The pinned TOML
parser rejects duplicate and unknown fields, including `x-*`. Native `SKILL.md`
metadata is unchanged; these are reference documents.
Profile frontmatter selects the library; its Markdown body explains that
selection. Put operational preferences in the selected patterns or guidance,
whose bodies the coordinator can load.

| Profile field | Contract                                                               |
| ------------- | ---------------------------------------------------------------------- |
| `schema`      | `cowboy.pattern-profile/v1`                                            |
| `id`, `scope` | Stable ID; `global`, `workspace` or `project`                          |
| `extends`     | One parent ID; omit for the global root                                |
| `guidance`    | Canonical guidance paths relative to this source root                  |
| `patterns`    | New pattern IDs mapped to files relative to this profile               |
| `replace`     | Inherited ID mapped to exact `from = "profile/id@revision"` and `file` |
| `disable`     | Inherited pattern IDs removed from this selection                      |

Workspace/project profiles require canonical guidance. Scope must become more
specific at each inheritance step; cycles, missing parents and implicit
overrides are invalid. Resolve the parent, disable explicit IDs, add new IDs,
and apply exact whole-pattern replacements. Never merge paragraphs. A Cowboy
project replacement of `delivery.procedure` retains the global supervisor.

| Pattern field    | Contract                                       |
| ---------------- | ---------------------------------------------- |
| `schema`         | `cowboy.pattern/v1`                            |
| `id`, `revision` | Stable ID and positive integer revision        |
| `role`           | `supervisor` or `procedure`                    |
| `summary`        | Discovery text, at most 240 bytes              |
| `events`         | Supervisor-only; currently `turn.completed`    |
| `uses`           | Required procedure IDs; at most eight          |
| `effects`        | Supervisor-only; currently `session.follow_up` |

A procedure cannot launch work by itself. A global supervisor can require a
procedure supplied by a child profile; incomplete catalogs allow observation but
cannot enable Coordinate. Disabling knowledge never disables an AGENTS, native
or host rule.

Import accepts at most 32 sources, 64 files per source, 32 KiB per file and 512
KiB of combined file text. The bundler and UI also bound the JSON envelope.
Reject oversized guidance instead of truncating its stop conditions. Sources are
content snapshots; discovery never executes commands or fetches arbitrary paths.
Source revision labels are informational Git provenance claims, not proof of a
clean or approved checkout. Exact content hashes, including inherited guidance,
define the effective catalog. A change invalidates pending admission even if its
author reused a human-readable revision label.

Maintain pattern revisions when editing their meaning. An inherited replacement
whose `from` no longer matches must be revalidated explicitly. Current worker
guidance remains authoritative over an imported snapshot; the follow-up never
asks a worker to treat old bundled guidance as permission.

## Persistence and MCP

UI-edited participation, model selection, learning and imported source content
use database preferences under scope `superagent`, following
[configuration ownership](configuration.md#where-a-setting-belongs). The writer
uses revision comparison and durable operation receipts. HTTP and MCP share
validation and persistence. No SQL migration or second policy file is needed.

Load `cowboy_read({help:"superagent"})` on demand. Ordinary authorized agents
configure via `preferences.get("superagent")` and `preferences.update`, with
`expected_revision`, a stable operation ID and
`changes: {configuration: {sources, projects, sessions}}`.

Each supplied configuration field replaces its whole map/list; omitted fields
are retained. For example, changing `sessions` need not resend `sources` or
`projects`. MCP preference reads and receipts omit source bodies and expose
`source_catalog` metadata instead, keeping Markdown out of routine settings
traffic. The library editor retains the complete editable snapshot; session
Tools requests a compact snapshot and writes only changed fields.
Large library imports use Settings or the authenticated `cowboy api` HTTP route.
MCP action arguments are limited to 32 KiB, within the outer 256 KiB request
envelope; that envelope is not the usable import size. Routine project/session
edits do not need to carry the library. The HTTP preferences route accepts up to
1 MiB, while source-library limits and the editor's bundle limit still apply.

For CLI import, first read `GET /api/preferences/superagent`. Preserve its
revision and dataset ID, and submit `PUT /api/preferences?compact=true` with
`{operation,dataset,scope:"superagent",expected_revision,changes:{configuration:{sources}}}`
using `cowboy api PUT '/api/preferences?compact=true' --data - < request.json`.
Use a private request file, a stable operation ID and the exact observed dataset;
do not replace projects or sessions when only importing a library. After an
uncertain response, inspect current state and reuse the identical request if
needed. Compact HTTP responses omit source content; verify the imported profile
IDs and hashes in the MCP `preferences.get("superagent")` response's
`preferences.source_catalog` field.
The owner-wide activity/export view is GUI-only: its historical candidates can
span projects, while coordinator MCP remains scoped to participation and its own
receipts. This exception does not limit the shared settings writer.

The scoped coordinator reads:

- `cowboy.superagent.catalog()`: IDs, summaries, content hashes, guidance keys,
  settings revision, learning/follow-up settings and explicitly participating
  sessions.
- `cowboy.superagent.document(id, {offset, chars})`: selected pattern/guidance
  text, at most 4,000 characters per page.
- `cowboy.superagent.context(session_id, {offset, chars})`: bounded recent
  evidence, mode, mandate and exact evidence/catalog markers.

Actions:

- `superagent.follow_up`: session ID, evidence marker, catalog digest, pattern
  ID, bounded reason and expected settings revision.
- `superagent.propose`: session ID, evidence marker, expected revision and at
  most 12 KiB of Markdown. This records a candidate; it cannot adopt it.
- `cowboy_receipt({domain:"superagent", operation})`: the caller's outcome. An
  unknown outcome does not permit another attempt under a new ID.

## Evaluation and delivery

Turn completion, existing status/preference notifications, confirmed model
options and shared usage observations trigger a debounced scan. No timer
periodically wakes a model. Before reserving an evaluation, the
Controller checks explicit enrollment, ownership, current mode, idle endpoints,
human queue priority, pause/stop/Trash/attention state, native thread evidence,
the installed signed Provider preset and cached account quota.

The coordinator must still select the configured Max preset. Quota uses only the
Service's shared sample: available, explicitly non-stale, observed within ten
minutes, future reset times and every reported window below 95% usage. Checking
all reported windows is conservative and can hold work because of an unrelated
model bucket. Missing evidence holds evaluation; there is no silent downgrade,
account switch or extra quota probe.

The most recent real human prompt in the current native thread defines a causal
budget. Automatic follow-ups, managed-call notices and messages from another
agent do not reset it. Missing human history or a cancelled/incomplete turn
holds work. One completed turn gets one assessment; further automated turns
consume the same budget. Evaluation is bounded to the follow-up limit plus one
final assessment. Learning is bounded to one candidate per human task.

Reservations use the existing durable Agent API operation ledger. Duplicate
events cannot allocate another attempt for the same completed turn. A crash
reservation without an outcome is unknown and blocks automatic replay. The
Controller rechecks settings, evidence, role/model and endpoint state at native
handoff. It sends only to an already-live worker, without reviving, recovering,
ensuring a new worker or putting failed sends into the user queue. Late failure
callbacks cannot clear a newer dispatch.

Request admission precedes causal-slot reservation, so a request-admission
failure cannot consume a slot. A synchronous `not_dispatched` result proves
zero handoff and can atomically reopen that slot; unknown reservations cannot.
Internal `sa-` operation IDs are reserved against caller-supplied operation IDs.

A reservation can consume an attempt even if delivery is held or lost. This
release favors an inspectable missing wake over replaying an uncertain effect.
Recent activity distinguishes submitted evaluation/follow-up from final
delivery; neither means the native task completed. Verify fresh worker evidence.
A `handed_off` receipt cannot prove native acceptance or a Git/deployment
effect succeeded. Late returned prompts also record a durable session error
with their message ID; they never enter the user queue. Activity supports cursor
paging, including tied timestamps, so older candidates remain exportable.

The evidence reader exposes at most the latest 80 transcript events and paged text.
When the human anchor has left the hot tail, it scans durable history backwards
in bounded pages with a five-second deadline, retaining only the anchor and
latest turn boundary. Missing or changing evidence holds work. It does not copy
full history into model context or subscribe to external events. Model
reasoning assesses whether the task still has a valid next step; deterministic
admission checks cannot prove that a proposed push or deploy is authorized. The
original worker must verify that authority and the real outcome.

## Refining patterns

Use observation → candidate → validation → source revision → adoption →
feedback. The native session remains the task; this is a knowledge refinement
process.

1. Keep a small observation: intended result, actual result, scope and evidence
   references. Repeated retries of one incident are one episode.
2. Check existing AGENTS, skills, scripts and patterns. Improve the existing
   owner when possible. A deterministic failure belongs in a check/helper when
   it can be enforced there.
3. Draft the narrowest useful change, with applicability, exclusions, required
   evidence, completion checks, stop conditions and counterexamples.
4. Validate against successful cases plus stale SHA, research-only, missing
   authority, paused session and unknown receipt cases as relevant.
5. Review/export the candidate, edit the source Markdown, then import the
   updated library. Candidates and worker worktree edits cannot activate
   themselves.

Suger already has session-analysis and delivery workflows; reuse those owners
for retrospective work instead of adding another scanner. Company tooling
references stay in the personal workspace where required. Global promotion needs
explicit scope review and generalization; do not copy private transcripts,
company snippets or evidence URLs into a shared library.

Automatic adoption, version-specific suspension, historical clustering,
cross-project dependency subscriptions and per-pattern session selection remain
future work. They must preserve explicit scope, evidence, budgets and existing
authority. Normal matching execution needs no repeated approval when that
authority already exists.

## Context efficiency and native coverage

Reuse the same native coordinator session. Keep tool definitions and role
instructions stable; load discovery first, then only the selected supervisor,
procedure and required guidance. Append bounded changed evidence rather than
copying full transcripts or rebuilding a system prompt at every wake. Technical
investigation stays with the worker.

Native skills already provide progressive disclosure and reusable tool
sequencing. Native runtimes own compaction; Cowboy adds no compactor, transcript
mirror, task engine or model keepalive. Changed policy must invalidate stale
instructions even when that costs cache reuse. Do not infer real Provider cache
hits from byte limits or native resume behavior.

Measure total/new/cached tokens, useful interventions, incorrect interventions
and completion quality in an opted-in pilot. This implementation makes no
measured token-saving or unattended-success claim.

Cowboy's residual gap is cross-session participation, scoped evidence and
intervention admission. No Columbus core runtime is added. Codex and Claude
share these Markdown/MCP contracts; each keeps its native lifecycle. Stronger
native tool isolation is currently a gap with the trusted-session boundary
above. Delete local lifecycle/discovery glue when native surfaces provide
equivalent permissions, freshness, receipts and visibility.

## Verification and research provenance

Deterministic tests cover strict profile resolution, content changes, source
bounds, owner/Machine/Off/pause scope, cancelled turns, causal budgets,
concurrent reservations, persistence across reopening and late failed dispatch.
Browser fixtures exercise actual participation controls, inheritance, reload,
revision conflicts and retries with the original operation ID. Local tests,
deployment receipts and real unattended outcome evidence are separate claims.

The research inspected DeepSeek's
[Super-agent prototype](https://github.com/deepseek-ai/dsh-superagent-prototype)
and native [skills](https://learn.chatgpt.com/docs/build-skills) /
[MCP skill composition](https://developers.openai.com/plugins/concepts/skills).
The transferable ideas are a persistent coordinator, explicit task ownership and
reusable procedures; Cowboy keeps native execution ownership.

Suger guidance was inspected at `8e381644c4c17ab5c2236f08a2530d3482d3f75a`;
Columbus and Cowboy guidance were read in their current task checkouts on
2026-11-06. These observations do not replace checking current project guidance
at execution time.

The completed Fable Max design review `call-785677337f995ae2ba1a8d5c` returned
`needs-attention`. Its confirmed origin, scope, authority and uncertain-delivery
findings informed the admission and first-release boundaries above. It is not
proof that the implementation or an unattended production run passed.

Implementation review `call-2201e33e16036f2fd73d2354` also completed on
Fable Max with `needs-attention`. Follow-up fixes cover durable causal anchors,
known-unsent reservation recovery, managed-child exclusion, role-only settings
reads, repairable library replacement and complete activity pagination. Native
handoff remains explicitly distinct from acceptance; returned prompts retain a
durable diagnostic. The suggested self-only configuration restriction was not
adopted because existing authorized Operator control and AI project/global
configuration are intentional; the dedicated coordinator remains restricted.
Agent-origin message IDs are generated by `agent_api/control.rs::apply` and
inserted into the request's `cmid` by its control adapter; they are not a
hypothetical prefix. These fixes require deterministic gates, and do not turn
the original review result into a clean re-review.
