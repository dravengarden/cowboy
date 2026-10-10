# Agent upgrades and Recommended configuration

Use this workflow for Claude, Codex, DeepSeek and other Cowboy Agent upgrades,
including changes only to the Recommended cards. Paths below are relative to
the Cowboy repository root. Run commands there in `nix develop -c …`.

## Resolve the request

Distinguish four independent facts before proposing changes:

| Fact | Authority | Meaning |
| --- | --- | --- |
| Plugin release version | `plugins/<id>/provider.json` → `version`, signed Catalog | Cowboy's independently released package |
| CLI, adapter, gateway versions | `runtime.dependencies`, runtime lock and npm locks | Executable bytes used by that Plugin |
| Model and reasoning selection | Provider's live configuration options and verified upstream model documentation | Hosted model IDs/aliases and allowed options; a CLI version is not a model version |
| Installed generation | Machine inventory and session's exact Plugin identity | What actually runs; a new Catalog entry does not prove installation |

Resolve names to package IDs: Claude → `claude-code`, Codex → `codex`,
DeepSeek → `claude-deepseek` and/or `codex-deepseek`, Gemini → `gemini`,
Grok → `grok`. Discover other Agents from `plugins/*/provider.json`.
For an unqualified DeepSeek audit inspect both variants; for a change, use the
variant(s) and Machine targets established by the request and inventory. Ask
only if choosing between them would materially change the requested outcome.

Classify the work as dependency upgrade, recommendation-only, or both. Establish
whether the user requested an audit/design, publication, or installation on
named Machines. Honor existing authorization rather than asking again. Do not
turn a request to author a skill into a live upgrade. Repository integration,
Catalog publication and Machine installation have separate receipts.

## Inspect upstream and shared dependencies

Start with the existing read-only audit; it reports exact registry candidates,
not an instruction to upgrade everything:

```bash
nix develop -c bun \
  .agents/skills/release-cowboy-plugin/scripts/audit-dependencies.ts all
```

Use a Plugin ID instead of `all` for a focused audit. Inspect official releases
and model documentation at execution time, recording date and source. For
OpenAI-specific research, use the available OpenAI documentation skill when
applicable. Do not freeze today's "latest" versions or guessed model names in
this skill. A registry tag identifies a candidate; committed pins must be exact.

Read the selected source manifests, `components/provider-runtime/lock.json`,
and relevant `components/provider-runtime/packages/*/package{,-lock}.json`.
Search all manifests for each changed dependency ID: Claude/Codex runtime
components can also serve a DeepSeek variant. Shared lock changes must remain
consistent with every consumer. Either retain distinct supported pins or include
all affected Plugins in the release plan; never silently change an unversioned
sibling. Each affected Plugin keeps its own version, artifact and receipt.

DeepSeek model/API changes can affect gateway mappings, context limits, tools,
thinking controls and cache behavior independently of the upstream CLI. Inspect
those owned contracts when implicated; a text substitution in a preset does not
prove compatibility. Preserve isolated Provider homes and closed environments;
never read, copy or link standard Claude/Codex credentials or settings to make a
DeepSeek candidate work. Credentials and model requests are not audit inputs.

## Configure Recommended without product code

The source of truth is `plugins/<id>/provider.json` → `configuration_presets`.
The package builder projects it into `manifest.configuration.presets`, consumed
generically by mobile and desktop through `web/src/runConfigPresets.ts`.

Each entry contains:

| Field | Configuration behavior |
| --- | --- |
| `id` | Unique stable preset identity; rename when its intended identity changes |
| `name` | Card title |
| `detail` | Card description; keep model/version claims accurate for the actual alias |
| `is_default` | Default badge; at most one per Provider |
| `values` | Exact live configuration option IDs mapped to string values |

Array order controls recommendation order. Add, remove or reorder entries in
this array. Do not add Provider/model branches to Composer, desktop controls or
the generic preset projection. The current schema allows at most 32 presets and
32 values per preset; use the SDK validator for the full bounds.

For example, the option names differ: Codex uses `reasoning_effort`, whereas
Claude currently uses `effort`. Obtain values from the intended runtime's live
configuration surface or its authoritative artifact fixtures. Model labels,
aliases and effort support must be checked together; do not infer them from
display text or another Provider's contract.

`is_default` is presentation, not a runtime setting. When the request also changes
the new-session default, update `runtime.behavior.default_preferences` for the
same model/effort keys, preserving unrelated defaults. Inspect any provider-owned
runtime arguments/environment that also select the model. Do not reset existing
sessions or personal selections to match a new recommendation.

The existing presentation resolver can use newer compatible signed Catalog
presets for an older session while keeping its executable generation pinned.
The UI filters presets against the live configuration surface. For a model
switch, it applies the model first and validates reasoning against the returned
model options; the old model's reasoning list alone is not decisive. A visible
card is not proof that the target model accepts every declared setting.

If a new model is absent from the installed runtime's options, changing JSON
alone cannot enable it: upgrade the necessary runtime or report the unavailable
recommendation. Do not bypass the generic support filter. Recommend a core/SDK
change only when the requested behavior cannot be expressed by the existing
contract, and explain that gap first.

This is versioned Plugin configuration, not an unsigned host override or a
Settings editor. Do not introduce another Columbus list, database table or
frontend constant for the same recommendations. A host-local editable policy
or GUI editor would be a separate product capability.

## Verify and deliver

For recommendation-only changes, leave dependency pins unchanged and bump the
selected Plugin version. Build, sign and publish through the parent skill's
normal immutable lifecycle; never edit published package bytes in place or
publish an unbound data-only build. Reuse unchanged immutable runtime inputs
through the owned builder/binder, retaining the complete signed runtime matrix.

Run the parent's applicable package, Provider and repository gates. The focused
preset regression checks can also run with:

```bash
nix develop -c bash -c 'cd web && bun test \
  ./src/runConfigPresets.test.ts ./src/providerSdk.test.ts'
```

For Claude CLI, SDK, ACP or remote runtime changes, follow
[Claude Remote compatibility](claude-remote-upgrades.md). Use the current Mods
integration and named checks in the current conformance runner, rather than the
historical MCP-alias integration or a fixed historical check count. The isolated
DeepSeek variant does not inherit standard Claude remote execution; validate
each affected consumer's own launch/authentication boundary.

Check schema rejection, unique IDs/defaults, order and labels, generic projection,
unsupported model/option filtering, model-before-effort application, and latest
presentation versus exact runtime identity. Existing tests may encode a previous
recommendation policy; update intentional policy expectations when the request
changes them, preserving generic behavior tests. Fixture tests do not prove
availability in the user's actual account. Use an authorized disposable session
to inspect and apply live options when required; do not alter a busy session or
send a paid model prompt just to inspect configuration.

Follow the parent skill's publication and installation paths within the user's
scope. After publication, verify the exact ready Catalog identity and its preset
values, then refresh the normal Catalog view and check mobile/desktop cards.
Do not claim browser verification from a JSON response alone. Recommendation
presentation changes normally need neither a Web/Controller release nor a
Machine restart. Runtime/default changes become effective through the exact
installed generation and normal new-session lifecycle; report active leases
that prevent authorized convergence rather than recycling them.

If reversal is needed, restore the prior recommendation/default values in a new
signed release; do not overwrite or delete Catalog history. Runtime rollback is
an explicit retained-generation operation, not a pretend new "latest" version.

Report each Plugin's old/new release and dependency versions, exact preset diff,
any default change, tested model/options and limitations, source commit, Catalog
receipt and authorized Machine inventory result. Clearly separate "source
updated", "published", "installed", and "verified in the UI".
