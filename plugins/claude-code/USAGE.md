# Anthropic plan usage

The signed collector runs `auth status --json`, then the pinned Claude CLI's
`get_usage` control request with `skip_behaviors: true`. This is the native
protocol used by the official Agent SDK's experimental
`usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` method, introduced
in [SDK 0.3.169](https://github.com/anthropics/claude-agent-sdk-typescript/releases/tag/v0.3.169).
It is not a stable public REST API. Validate the response contract when changing
the CLI pin; the current CLI 2.1.280 / SDK 0.3.280 contract is the baseline.

The CLI owns Service-managed authentication and token refresh. The collector
does not read credential files, call an undocumented HTTP endpoint, run a user
prompt, or estimate account limits from local token counts. Project settings,
hooks, tools, MCP servers and session persistence are disabled for the query.
The collector reads at most 256 KiB and spends at most ten seconds across both
native commands, leaving cleanup time before the host's process-group timeout.

Version 3.1.25 disables auto-update, telemetry and error reporting individually.
Do not set or inherit `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` in the collector:
CLI 2.1.278 and later also suppress the plan-usage request under that blanket
switch, returning `rate_limits_available: true` with null limits for a signed-in
Max account. The live subscriber check exposed this in 3.1.24; fake API-key
probes could not detect it because those accounts do not have plan windows.

Version 3.1.32 stops depending on `rate_limits.model_scoped` alone. That legacy
projection sits behind a feature gate that the collector's own `DISABLE_TELEMETRY=1`
also closes, so a signed-in Max account with a per-model weekly window (Fable)
reported five-hour and weekly rows and silently dropped the model row. The
unified `rate_limits.limits` array is emitted with telemetry off and carries the
same percentage and reset time as `kind: "weekly_scoped"` with
`scope.model.display_name`. The collector now reads both shapes and dedupes by
model name. This is the second gate of its kind: keep individual switches, and
re-probe per-model windows with the collector's exact environment — not an
inherited shell — when changing the CLI pin.

Native utilization is a percentage from 0 to 100; reset times are ISO 8601.
The collector projects numeric five-hour, weekly and per-model windows into
Cowboy's generic usage buckets. Enabled extra usage may have a percentage but
no reset time. Null windows remain unknown; malformed numeric data or an
unrecognized response is a transient failure. `rate_limits_available: false`
is valid for API-key accounts or accounts without profile scope and does not
imply a full remaining balance. A true availability flag with a null payload
is a failed refresh.

The existing usage service coalesces refreshes, polls every five minutes,
limits manual refresh to once per thirty seconds, and retries transient errors
after one minute. It retains the last successful snapshot with a stale marker
on transient failures and clears it on authentication failure. Session context
updates must not change the quota observation time or erase its refresh error.
Session rate-limit events remain a fallback when no account snapshot exists.

## Verification

`just plugin-check` exercises the real subprocess protocol with hermetic fake
CLI fixtures: fragmented UTF-8, unrelated replies, lingering processes, timeout,
bounded output, redacted failures and signed-out accounts. Web tests feed native
quota projections into the production card/top-bar parser. Runtime release
probes separately exercise the exact pinned CLI with isolated fake credentials;
they do not prove any particular subscriber's live quota.

The 2026-09-22 dependency release advances the runtime to CLI 2.1.278 and ACP
0.79.0. The native usage interface remains covered by the collector protocol
tests and requires a live subscriber refresh after Machine activation.

The 2026-09-23 dependency release advances the runtime to CLI 2.1.280 and ACP
0.81.0. That CLI adds Claude Opus 5.5 (`claude-opus-5-5`) and makes it the
default Opus model, so the Agent model list and the session model control pick
it up from the pinned runtime; no Cowboy contract or preset changes with it.
The native usage interface is unchanged and still requires a live subscriber
refresh after Machine activation.
