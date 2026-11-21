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
supported usage-reset operations. Additional (named) accounts never inherit it:
a portable named account runs its collector in its own vault replica on a
Machine chosen as described in [agent-api](agent-api.md#account-usage-and-quota),
within the same host runtime restrictions. Session-reported usage remains associated
with the session and does not become account-plan usage.

## Named portable accounts

A named Codex subscription signed in once, or a named DeepSeek API key, keeps
its credential in the account's Service vault, replicated to every Machine at
account scope (Machine protocol 47). Its usage is read-only and reuses the
released signed collector of the Service account's usage host (the first
member of the account's credential scope that declares one: `claude-deepseek`
for DeepSeek), dispatched as `CollectPortableUsage` (Machine protocol 50; an
older Machine is `machine_usage_update_required`). The Machine runs it through
the Service host path at account scope, so the collector sees exactly the
credential projection and home of a session on that account, and starts only
the sidecars the selected exact collector release declares, each from the
member Plugin whose exact installed release fills that slot on that Machine,
within the command's absolute deadline and live connection (including sidecar
readiness).
Every participant must hold one committed shared vault generation; the Machine
refuses otherwise, refuses an observation whose generation changed while the
collector ran, and holds the account's credential guard for the run.

**Refresh-token decision.** A usage query of a named portable account never
redeems a refresh token. A Service compare-and-swap after a refresh cannot
repair a refresh token that a session on another Machine redeemed
concurrently, so coordination must precede the vendor call; the only writer
allowed to redeem is the sessions' existing refresh path. The Controller puts
`"token_refresh": false` in every portable request and the Machine sets it
again. For a Provider whose CLI rotates credentials (`compare_and_swap`) only a
collector whose exact signed release declares (`collector/portable-account.json`,
`token_refresh: "request"` with a `token_refresh_guard`) that it then reports
authentication required runs; otherwise `portable_usage_refresh_unsupported`.
The guarded minimums are **Codex 3.6.20** and **Grok 3.2.19**; earlier releases
(Codex 3.6.16-3.6.19, Grok 3.2.16-3.2.18) carried an unguarded claim and are
refused, so named Codex or Grok usage needs that release (or later) installed
on the chosen Machine. Grok's CLI renews an expiring sign-in on start and after
any 401, so its collector never starts the CLI for such a request: it reads
the current token, reports authentication required when it has (nearly)
expired or xAI rejects it, and otherwise reads billing
(`cli-chat-proxy.grok.com/v1/billing?format=credits`, projected exactly as the
pinned CLI's `_x.ai/billing`), plan and resets directly. No
Machine speaking protocol 50 was released before this, so no other capability
negotiation is needed. An entered DeepSeek key is never refreshed.

Placement: among Machines with an exact released active collector generation
that host runtime policy admits for the collector and every sidecar Plugin it
starts, Cowboy picks the one running most of the owner's live sessions on the
account, then the lowest Machine ID, and never inherits the Service account's
configured usage query Machine. When no Machine qualifies, the reason is the
first unmet check (host policy, a refresh-free collector release, the Machine
protocol, the exact sidecars) of the Machine that met the most of them, so a
Machine already running a refresh-free release reports what it still lacks
instead of an older release elsewhere. A cached placement refusal is no longer
reported once a placement is possible again.

**Named Codex resets.** A named Codex account spends a banked reset immediately
(no schedules), like a named Claude account: typed `confirm`, the grant ID
pinned when the dialog opened, the shared Provider pacing (one collector
start), and only `reset`/`already_used` as outcomes; anything else is unknown
and never retried. It runs on the Machine the account's usage placement selects
(protocol 51, `ConsumePortableReset`), under the account lock with the exact
vault generation and admission revision, with an absolute dispatch expiry, and
only when the exact selected release declares resets and a refresh-free reset
(`collector/portable-account.json` `"reset_token_refresh": "request"`, Codex
3.6.20). Its `consume_reset` runs with `"token_refresh": false` from a private
copy of the replica's sealed credential at the admitted generation (never the
writable session home), and only for an account with a verified subscription
identity (`portable_reset_identity_required` otherwise). The starts are reserved
at admission from the dispatch expiry, so a preceding usage read never blocks a
manual reset and the Provider spacing holds whenever the Machine ran it. A
success must name the confirmed grant; a refusal is unsent only with an explicit
`started: false` and a pre-start code. Grok and DeepSeek named accounts have no
resets.

Refresh-free usage needs a guarded declaration (`"token_refresh_guard"`):
`refresh_endpoint_refused` (Codex 3.6.20) or `cli_not_started` (Grok 3.2.19).
The unguarded claim of Codex 3.6.16-3.6.19 and Grok 3.2.16-3.2.18 no longer
admits named usage, so named Codex usage stays off until 3.6.20 is installed on
the chosen Machine, and Grok until 3.2.19. Rotating subscriptions (Codex, Grok)
read usage from the same private sealed copy.

The Codex app-server refreshes an expired access token on its own
(`proactive_expiry`), whatever the collector asks: an offline check against the
pinned 0.169.1 and a fake token endpoint showed the released 3.6.18 collector
redeeming the refresh token under `token_refresh: false` (four token requests
for a usage read, one for a reset, and a rewritten `auth.json`). From 3.6.20 a
refresh-free request's app-server points its refresh endpoint at a port nothing
listens on (`CODEX_REFRESH_TOKEN_URL_OVERRIDE`), and a rejected token reports
authentication required: the same check then showed no token request and an
unchanged `auth.json` for both.

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
Named portable accounts (Codex, DeepSeek API key) likewise share their
Provider's budget with its Service account, one start per account interval, and
their observations stay isolated by owner, account and vault identity.
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
| OpenAI | Codex app-server `account/read` (a token refresh only after the backend reports it expired), `account/rateLimits/read`, `account/usage/read` | none |
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

A named native Claude account's confirmed reset is immediate only (named
accounts have no schedules) and runs on the Machine holding its native login,
through the same signed collector, CLI identity and token as its quota read
(Machine protocol 49), on the exact release the Controller selected and never
after its dispatch expiry. It takes its start from the same Anthropic Provider
window and spacing, on the account's own lane, and holds the account's
collection lock, so a quota read and a reset of one account never overlap. A
reset that never reached the Machine's collector records no failure backoff;
an authentication failure backs off even after the call was sent. A named reset
runs two collector invocations (the identity read, then the spend with its own
preflight). At admission the Controller reserves two starts one Provider
spacing apart; the Machine runs the identity read, releases its lifecycle lock,
waits for the second slot, rechecks the selected release and spends, all before
the dispatch expiry measured from the original admission. When the Provider
window cannot hold two spaced starts, named resets are unsupported. A named
reset therefore takes at least one spacing interval end to end.

Claude usage reads and resets (Service card and named accounts) contact
Anthropic only from a Machine that host policy explicitly pins for
`claude-code` (`--provider-runtime-machine claude-code=ovh`). Unlike session
placement, a missing restriction fails closed: no Machine is eligible, so a
lost or mistyped host argument can never move these requests to Hawk, Falcon
or a laptop. The check runs at selection and again inside the dispatch
transaction.

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
