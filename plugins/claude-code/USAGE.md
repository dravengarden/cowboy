# Anthropic plan usage

The signed collector runs `auth status --json`, then the pinned Claude CLI's
`get_usage` control request with `skip_behaviors: true`. This is the native
protocol used by the official Agent SDK's experimental
`usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` method, introduced
in [SDK 0.3.169](https://github.com/anthropics/claude-agent-sdk-typescript/releases/tag/v0.3.169).
It is not a stable public REST API. Validate the response contract when changing
the CLI pin; the current CLI 2.1.272 / SDK 0.3.270 contract is the baseline.

The CLI owns Service-managed authentication and token refresh. The collector
does not read credential files, call an undocumented HTTP endpoint, run a user
prompt, or estimate account limits from local token counts. Project settings,
hooks, tools, MCP servers and session persistence are disabled for the query.
The collector reads at most 256 KiB and spends at most ten seconds across both
native commands, leaving cleanup time before the host's process-group timeout.

Version 3.1.25 disables auto-update, telemetry and error reporting individually.
Do not set or inherit `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` in the collector:
CLI 2.1.272 also suppresses the plan-usage request under that blanket switch,
returning `rate_limits_available: true` with null limits for a signed-in Max
account. The live subscriber check exposed this in 3.1.24; fake API-key probes
could not detect it because those accounts do not have plan windows.

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

The 2026-09-16 upstream audit found CLI 2.1.273 and ACP 0.78.0 available. This
collector release retains CLI 2.1.272 and ACP 0.77.0, whose native usage interface
was verified. Those independent dependency updates require their own runtime
and compatibility review.
