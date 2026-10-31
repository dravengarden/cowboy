# Account usage execution

Settings → About → Usage lets an operator choose a **Usage query Machine**
for each account. This is a Service-wide, durable selection. It does not move
Agent sessions, change their model routing, or export Provider credentials.

Automatic preserves the existing selection: choose an online Machine with an
active installation matching a signed usage host's exact Plugin version and
generation digest. A named Machine restricts that same eligibility check to
the selected identity. An offline or incompatible pinned Machine produces an
unavailable result; Cowboy never silently falls back to another Machine.
Existing cached usage remains historical data, not evidence of a fresh query.

Controller-host `--provider-runtime-machine PROVIDER=MACHINE` restrictions
intersect this selection by the exact Plugin/Provider identity, independently
of the usage account key. An explicit conflicting choice or an unavailable
allowed Machine produces an unavailable result. Automatic cannot bypass the
restriction, and a restricted account cannot fall back to a Controller-local
bootstrap command. The preference remains editable for recovery after a host
policy change, without granting permission to a currently denied route.

The selection applies to the account's collection, activity decoration and
supported usage-reset operations. Session-reported usage remains associated
with the session and does not become account-plan usage.

## Operator CLI

Use a CLI matching the Controller release through its authorized local Operator
socket. Read configuration and current eligibility:

```sh
cowboy operator usage-executor
```

Set one account, then request a fresh sample:

```sh
cowboy operator usage-executor --provider anthropic --machine ovh
cowboy operator usage --refresh anthropic
```

Use `--machine automatic` to remove an explicit selection. Account keys come
from the response's `providers` map; Machine IDs come from `machines`.
`machine_id` is the configured selection and `selected_machine_id` is the
currently eligible execution target, not proof that the Provider query succeeded.

The Web API uses `GET /api/usage/executors` and operator-authorized
`PUT /api/usage/{account}/executor` with `{"machine_id":"ovh"}` (or `null` for
Automatic). Viewers can inspect settings but cannot change them. The dedicated
`usage_execution_machines` table belongs to usage, independently of authentication
settings. Writes serialize with active refreshes; a slow collector can delay a
save. Changing the route preserves the durable account and Provider budgets.

## Shared collection budget

Service configuration owns four live settings:

| Key | Default | Meaning |
| --- | --- | --- |
| `usage.provider_window` | `2m` | Rolling window shared by all accounts of one Provider |
| `usage.requests_per_window` | `2` | Maximum collector starts in that window |
| `usage.account_interval` | `5m` | Minimum time between starts for one account |
| `usage.failure_backoff_max` | `1h` | Maximum failure backoff; authentication failures wait this long |

Starts are evenly spaced (one minute with these defaults), with at most one
active collector per Provider. The oldest named account is tried first.
Anthropic's Service account and named native accounts share the same Provider
budget; account observations remain isolated by owner, account and native login.
OpenAI, DeepSeek and xAI have independent Provider budgets. Passive ACP usage,
context/cost events and reads of cached observations consume no collection budget.

Manual refresh, HTTP, CLI, MCP and background polling use the same admission.
Reservations are fsynced before dispatch to `usage-rate-limits.json` beside the
Service usage cache. Changing Machine, login generation or restarting the
Controller cannot reset a reservation. A missing ledger waits five minutes;
an invalid/unwritable ledger fails closed and logs an operator diagnostic.
Failure/cancellation spends budget. Account failures back off for 10, 20, 40,
then 60 minutes; Provider failures also pause other accounts (4, 8, 16… minutes,
capped at the configured maximum). Authentication failures affect their account.
Machine, placement and binding failures also back off only their own account;
they do not identify a Provider-wide outage. A failed-closed ledger uses a finite
cache retry hint, so repairing the ledger and restarting needs no cache edit.
There is no immediate retry loop. A caller disconnect does not cancel the
Service-owned admission transaction or collection.

The counted unit is a **collector invocation**, not opaque native CLI HTTP.
Audited per invocation (no collector retries internally; each direct request has
a timeout within the collector's deadline):

| Provider | Native CLI work (opaque HTTP) | Direct HTTP sent by the signed collector |
| --- | --- | --- |
| Anthropic | `claude auth status`, `get_usage` control request | up to 3: `/api/oauth/usage`, `/api/oauth/profile`, prepaid credits |
| OpenAI | Codex app-server `account/read` (no token refresh), `account/rateLimits/read`, `account/usage/read` | none |
| DeepSeek | none | one local gateway `/provider-info` request per lane (2); the gateway owns its upstream balance request |
| xAI | Grok `_x.ai/billing` | up to 2: `/rest/subscriptions`, `GetRemainingResets` |

With the defaults, a Provider therefore receives at most two collector
invocations per two minutes, so at most 6 direct Anthropic and 4 direct xAI
requests in that window, plus the native CLI's own requests. Direct enrichment
failures (including HTTP 429) are discarded without retry; they do not trigger
Provider backoff, and the next attempt still waits for the account interval.
`Retry-After` is not read by the current collectors, so the configured pacing
is the only delay. Native CLI requests, token refresh and their retries are not
bounded by Cowboy beyond the invocation count.

A confirmed or scheduled usage reset runs the same collector with its native
usage read, the reset preflight and the reset request. It takes a start from the
same Provider window, spacing and single-active-collector rule, and delays the
account's next poll, but it does not wait for that account's polling interval.
A paced reset is refused before any Provider call ("no reset was sent"). Resets
are serialized before their start is charged and hold that serialization until
the invocation ends, so the charged time is the actual call time. A scheduled
reset reserves its start before any one-shot claim, so pacing only delays it. A refused or changed reset records no failure backoff; an
authentication failure does.

These controls bound Cowboy's collection starts; they do
not claim a published subscription polling allowance or guarantee against an
account restriction. Provider API rate limits are separate from this polling
policy. See [Anthropic rate limits](https://platform.claude.com/docs/en/api/rate-limits),
[OpenAI retry guidance](https://help.openai.com/en/articles/5955604-troubleshooting-api-rate-limits-and-429-errors),
[DeepSeek rate limits](https://api-docs.deepseek.com/quick_start/rate_limit/) and
[xAI rate limits](https://docs.x.ai/developers/rate-limits).

Web clients share one cache reader per owner. Refresh start and completion
invalidate that owner's native usage across connected clients; Service usage
continues through its shared snapshot broadcast. A slow HTTP snapshot cannot
overwrite a newer push. About, Desktop and Session Settings project the same
account observation. Desktop labels each account separately and keeps secondary
details collapsed inside the account's card.

## Retiring a Provider

`plugins.disabled = ["<plugin-id>"]` in the Service configuration excludes those exact
Plugin IDs from Catalog and Provider discovery, login/session admission and
host activation, including bootstrap fallback. Validate through `cowboy config
check`; this setting takes effect on Controller restart. Configuration schema
and edits are exposed by the existing CLI/MCP configuration surfaces.

First remove an idle Plugin from the Service's Machine membership declaration,
run bounded Operator convergence, and inspect the uninstall receipts. Then
apply the disable setting and activate the Controller. Disabling does not stop
existing workers, delete conversations, erase credentials or rewrite immutable
published package history. Retained sources continue to test historical
compatibility; they are not exposed as available Providers on a disabled Service.

## Verification

The routing test covers explicit selection over a newer alternative and no
fallback when a pin is unavailable. SQLite and isolated PostgreSQL contracts
cover durable restoration, clearing the pin, and rejecting unknown accounts or
Machines. API client tests cover reads, updates, Automatic and permission errors.
