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
DeepSeek → `claude-deepseek` and/or `codex-deepseek`, Grok → `grok`. Gemini was
retired in component release 3.72.0 and has no Plugin. Discover other Agents from `plugins/*/provider.json`.
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

## Upgrade source-built third-party packages

A dependency listed under `third_party/` (for example `codex-acp`) is built
from its pinned upstream source plus Cowboy's delta series. Upgrade it with
the workflow in `docs/third-party.md`, never by editing patch files:

```bash
nix develop -c bun tools/third-party.ts status codex-acp
nix develop -c bun tools/third-party.ts checkout codex-acp --force
nix develop -c bun tools/third-party.ts upgrade codex-acp <tag>
# decide each delta against dist/third-party/codex-acp-upgrade-<tag>.md
nix develop -c bun tools/third-party.ts export codex-acp
```

Upstream first: drop every delta upstream now implements or covers, adapting
Cowboy to upstream's behavior; keep only deltas Cowboy still needs, each with
its drop condition. Record each delta's decision (dropped, adapted, kept,
added) in the Provider's dependency audit. Then pin the new version in every
declared consumer (`bun tools/third-party.ts check` enforces this) and run the
consumer's source build, which executes upstream's type check and complete
test suite.

## Collector checks follow native pins

Signed usage collectors reproduce native behavior for named portable accounts,
so a native pin change re-measures it offline (fake logins, loopback-only
namespace, no model request):

- Grok: `nix develop -c just grok-billing-capture <absolute grok>` rewrites
  `plugins/grok-billing-cli.fixture.json` from the candidate CLI; commit it with
  `GROK_CLI_VERSION` in `plugins/grok/collector/index.js`. A changed projection
  fails the capture until it is reviewed (`--accept-changes`).
- Codex: `nix develop -c just codex-refresh-guard-probe <absolute codex>` must
  report a refresh against the fake endpoint and none, with `auth.json`
  unchanged, for the collector's refusal URL.
- agy: `nix develop -c just agy-barrier-probe <absolute agy> <receipt>` must
  settle a turn on the Antigravity proxy's barrier request and see it answered
  `-32601` (unauthenticated, no network). A failing pin would settle every turn
  `unknown_refusal` and park its account. A new executable also needs its
  SHA-512 in Columbus `machines/ovh/account-sandbox/agy-sha512.allow` before
  OVH can run it, and one real turn's acceptance after installation.

## Telemetry capability and prompt-cache audit

Every Agent CLI, ACP adapter, gateway, or model-lineup upgrade MUST run this
check before finalizing release notes or proposing default changes. Compare
against the advisory inventory in `models/cache-and-telemetry.json` (which
never grants runtime capability):

1. **Diff upstream metrics and accounting semantics**:
   - Inspect upstream release notes and adapter/gateway code for added,
     renamed, changed, or dropped usage counters (`input_tokens`,
     `cached_input_tokens`, `cache_write_tokens`, `uncached_input_tokens`,
     `reasoning_tokens`, `_meta.quota.model_usage`, OTLP/request observation
     events, and gateway cache headers/ledgers).
   - Preserve Cowboy's disjoint token accounting invariant:
     `uncached_input_tokens` equals `input_tokens - cached_input_tokens` and
     already includes `cache_write_tokens`. Unknown counters stay `null`, never
     coerced to `0`.
2. **Enforce runtime capability truth and graceful App degradation**:
   - Never rely on `models/cache-and-telemetry.json` to enable a runtime
     feature. Any new telemetry emitter (for example `codex-otlp-http-v1`)
     requires **both** its signed manifest declaration and its runtime consumer
     in the same release.
   - Verify `GET /api/provider-accounts/{account_id}/cache-metrics` and
     `GET /api/sessions/{id}/cache-metrics` remain honest when a counter is
     unreported or regresses (`cached_input_share: null`, explicit
     `observed_cache_turns / eligible_turns` coverage, no fabricated cold
     request counts or savings).
3. **Review prompt-cache TTL curves and policy boundaries**:
   - Check official vendor prompt-caching documentation for TTL or pricing
     changes on the upgraded models/plans, recording date and source.
   - Run the read-only periodic audit to check for `telemetry-coverage-drop`
     regressions and inspect the shadow `(provider, model, renewed)` idle-gap
     diagnostic curves:

     ```bash
     nix develop -c bun .agents/skills/model-intel/scripts/periodic-audit.ts \
       --database /var/lib/cowboy/cowboy.sqlite3 --days 14
     ```

   - Treat shadow turn-level idle-gap curves as advisory diagnostics, never as
     an automatic trigger to mutate `context_rollover` defaults. Never weaken
     Antigravity's `docs/requirements.md` consumer-account boundary (automatic
     rollover disabled; background keepalive forbidden) or merge DeepSeek's
     independent `deepseek_cache_protection` flag with idle rollover during an
     ordinary dependency upgrade.
   - Update `models/cache-and-telemetry.json` when verified telemetry capabilities
     or official/empirical TTL observations change.

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

`is_default` marks the Recommended badge and is also the preset the Web New
Session form submits when neither an explicit choice nor an inherited folder
default applies (`web/src/newSessionPreset.ts`). It is not the runtime default:
when the request changes the new-session default, also update
`runtime.behavior.default_preferences` for the same model/effort keys,
preserving unrelated defaults, so sessions created outside the Web form agree. Inspect any provider-owned
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

### Evidence for the order and default

Before changing an Agent Provider's order or default, or after an upgrade that
adds, retires or retargets a model, run the advisory audit of the
[model-intel](../../model-intel/SKILL.md) skill:

```bash
nix develop -c bun .agents/skills/model-intel/scripts/score.ts --audit
```

It reports signed presets without an evaluation target, target values that
differ from the signed preset, catalog age, and differences between the
evidence-derived order/default and the signed ones. Treat it as input, not a
gate: catalog age or a missing model never blocks an unrelated runtime fix.
When the evidence names a better model that the installed runtime does not
offer, confirm account rollout and the live options before proposing a
runtime upgrade. Keep existing preset IDs (the Controller and folder defaults
reference them) and change only order, `is_default` and the matching default
keys unless the evidence requires a new preset.

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
