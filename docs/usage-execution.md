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
The guarded minimums are **Codex 3.6.22** and **Grok 3.2.21**; earlier releases
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

**Idle renewal (Grok 3.2.36, Machine protocol 52).** Refresh-free usage
leaves an account whose sessions all ended with an access token that expires
a few hours later (about four for Grok), after which every read reports
authentication required although the sign-in is intact. Such a read schedules
one idle renewal (`renewal.rs` in the Controller's portable usage) bound to
that read's account binding and vault generation (a new sign-in or a
session's refresh since then cancels it). It takes its own paced start from
the Provider's usage budget, never sooner than the account's collection
interval after the failed read, and, at dispatch and again under the account
lock that worker sends also take, requires:

- a **quiet account**: every session of the account, trashed ones included,
  stopped (exited, crashed or interrupted, not closing, no live worker) on a
  connected Machine. A starting, running or closing worker, or any session on
  a disconnected Machine (a detached worker may still run there, or its
  rotated credential may not have reached the vault), defers renewal;
- the placed Machine speaking protocol 52 with an exact collector release
  whose declaration names `"idle_renewal": "cli_startup"`;
- the account's **affinity**: a renewal that renewed or may have started is
  recorded durably (`usage-renewals.json`, directory-synced; an unreadable
  journal turns renewals off) with its Machine and vault generation, and until
  the vault advances past that generation only that Machine may renew again,
  so no other Machine presents the spent token. An attempt proven never to
  have started restores the earlier record;
- its spacing: 30 minutes after a renewal, 6 hours after one that changed
  nothing or failed after starting (leaving "sign in again" for the owner).
  A proven pre-start refusal releases the attempt.

From the durable record until the vault advances (at most 60 seconds after
the dispatch deadline or a renewal, and kept when the outcome is uncertain),
the Controller's launch gate holds new workers of the account as a resource
wait (and, while the journal is unreadable, every named launch for one
dispatch window and settle period after it was first read), so no worker starts from another generation's home with the token being
rotated and a queued prompt resumes on its own. `RenewPortableCredential` is the only route: the generic
`InvokePluginHost` never carries `renew_credential` (its minimum protocol is
unreachable and the Machine refuses it). The Machine admits the renewal like
a read (account credential guard, replica generation, refresh-free
declaration plus `idle_renewal`) and refuses before start unless every
session home of the account holds exactly the sealed credential (an
uncommitted refresh or another sign-in is the refresh observation's to
reconcile). It then runs the collector's `renew` in the account's own
session home, never a private copy, holding the lifecycle lock like a reset;
the collector starts the pinned Grok CLI and makes one billing read through
it, so the CLI refreshes exactly as a session's startup would. The renewed
credential leaves only through the Machine's ordinary refresh observation and
the Service compare-and-swap (which validates its identity); the reply is
only `renewed` (a home now holds a credential the vault does not, which no
home did before the run) or `unchanged`, decided from the homes even if the
billing read failed after the CLI refreshed; a throttled billing read after a
rotation is reported beside `renewed` so the Provider is still paced. The
Machine's check of the homes is strict: an account credential root or session
home that cannot be enumerated refuses before start and is indeterminate
after it. A renewal is followed by usage
reads, each once the account's budget allows, until one reads past the
pending distribution. Codex declares no idle renewal: its CLI does not
serialize concurrent refreshes even in one home.

Known residual (owner decision, 2026-11-24): the Controller cannot see a
worker launch already past the gate whose status has not changed yet, a
reconnecting Machine whose worker snapshots have not arrived, a session
refresh whose observation has not reached the vault, or a Machine that has
not applied a renewed generation yet. Each is the same exposure as the owner
starting a session, whose CLI performs the same startup refresh with none of
the checks above; closing them needs every Machine to confirm and reconcile
its credential state (the paused custody design).

Offline acceptance (2026-11-24, pinned Grok CLI 1.0.92, user and network
namespace with loopback only, a fake IdP issuing single-use refresh tokens
that rejects reuse): a renewal of a token expiring in 30 s or expired ten
hours earlier redeemed exactly once and rewrote the home's `auth.json`; a
token valid for two hours redeemed nothing; two concurrent renewals in one
home, and a renewal concurrent with a session's CLI start in that home,
redeemed exactly once in each of three runs.

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
or on a schedule ([named reset schedules](#named-reset-schedules)), like a
named Claude account: typed `confirm`, the grant ID
pinned when the dialog opened, the shared Provider pacing (one collector
start), and only `reset`/`already_used` as outcomes; anything else is unknown
and never retried. It runs on the Machine the account's usage placement selects
(protocol 51, `ConsumePortableReset`), under the account lock with the exact
vault generation and admission revision, with an absolute dispatch expiry, and
only when the exact selected release declares resets and a refresh-free reset
(`collector/portable-account.json` `"reset_token_refresh": "request"`, Codex
3.6.22). Its `consume_reset` runs with `"token_refresh": false` from a private
copy of the replica's sealed credential at the admitted generation (never the
writable session home), and only for an account with a verified subscription
identity (`portable_reset_identity_required` otherwise). The starts are reserved
at admission from the dispatch expiry, so a preceding usage read never blocks a
manual reset and the Provider spacing holds whenever the Machine ran it. A
success must name the confirmed grant; a refusal is unsent only with an explicit
`started: false` and a pre-start code. Grok and DeepSeek named accounts have no
resets. While a card's own usage reports an available reset that cannot be
spent, the usage read carries `reset_unavailable_reason` and its fixed
`reset_unavailable_message` (for example `portable_reset_identity_required` for
an adopted account that was never signed in through Cowboy), and the card shows
the message instead of only hiding the action.

### Named reset schedules

The owner may schedule one reset of a named account that can spend a reset now
(`PUT /api/provider-accounts/{account_id}/usage/reset/schedule` with
`fire_at_ms` one minute to one year ahead and typed `confirm`; `DELETE`
cancels; `usage.account_reset_schedule` / `usage.account_reset_cancel` in the
Agent API). One row per owner and account
(`provider_account_reset_schedules`) replaces the earlier schedule; the usage
read reports it as `reset_schedule`, and a card shows a pending schedule with
Cancel even while its reset cannot be spent. Scheduling needs the same
eligibility as a reset now, including a usage admission ledger that has not
failed closed. The timer looks every 15 seconds:

- The schedule is bound to the exact sign-in it was created under, in two
  parts. A change of the account's verified identity or admission revision, a
  disabled or revoking account, or a disabled owner withdraws it unspent,
  logged `cancelled`. The reset route's own binding (the native login, or the
  portable sign-in generation, read again after the vault was sampled) is
  compared under the account lock after admission and rechecked by the
  dispatch: another binding (a new sign-in to the same identity, or another
  Machine's login while the scheduled one is offline) is never spent from,
  and the schedule waits, uncounted. A fire more than an hour late (a
  Controller that was down or rolled back, or a binding that never returned)
  withdraws it. The owner and the cutoff are checked again just before the
  claim; an owner disabled in the milliseconds between that check and the
  enqueue is not caught, as for a confirmed reset, and a same-identity
  Codex sign-in committing inside the dispatch's own lock window is the same
  verified subscription. A token rotation changes nothing. The cutoff is
  checked at the claim; a native reset's paced spend may start up to one
  Provider spacing after it. The timer follows the reset route's ranking, so
  with logins on several reset-capable Machines it waits for the scheduled
  one only while that one ranks first (Claude resets are pinned to one
  Machine by host policy).
- It spends the nearest available grant of the admitted binding's successful
  observation, at most ten minutes old. An observation older than two minutes
  is refreshed first through the ordinary budgeted read (never waiting behind
  a spend that holds the account); without a fresh one nothing is reserved
  and the attempt waits a minute, uncounted.
- Its route's binding is compared before anything is reserved, so a schedule
  waiting for its binding never charges or holds the Provider window. It is
  then admitted exactly like a confirmed reset (route, account lock, Provider
  starts). Pacing only delays it to the named time (never past the cutoff); a
  wait over two minutes (an account backoff, a long Provider window) releases
  the Provider hold so other accounts keep polling, and the timer returns a
  minute before the slot to hold it then. Any other refusal sent nothing and
  is retried a minute later, three attempts in all, then logged `failed`. An
  admission dropped before the claim (a shutdown) releases its starts unused,
  and the pre-admission refresh waits at most 20 seconds.
- Only after admission is the row deleted under its idempotency key (the
  one-shot claim), before the audit row and the dispatch, so it fires at most
  once even across a restart. Without a written `started` audit row nothing is
  sent (this also holds for a confirmed reset). After the claim it is never
  retried: an unknown outcome is logged `unknown`, a refusal `failed`. Each
  claimed spend runs in its own task, so a slow one never delays others.

The action log uses the account's lane (`account-<id>`, trigger `scheduled`).
A cancellation logs the key of exactly the row it removed and never stops a
reset the timer already claimed.

Refresh-free usage needs a guarded declaration (`"token_refresh_guard"`):
`refresh_endpoint_refused` (Codex 3.6.22) or `cli_not_started` (Grok 3.2.21).
The unguarded claim of Codex 3.6.16-3.6.19 and Grok 3.2.16-3.2.18 no longer
admits named usage, so named Codex usage stays off until 3.6.22 is installed on
the chosen Machine, and Grok until 3.2.21. Rotating subscriptions (Codex, Grok)
read usage from the same private sealed copy.

The Codex app-server refreshes an expired access token on its own
(`proactive_expiry`), whatever the collector asks: an offline check against the
pinned 0.169.1 and a fake token endpoint showed the released 3.6.18 collector
redeeming the refresh token under `token_refresh: false` (four token requests
for a usage read, one for a reset, and a rewritten `auth.json`). From 3.6.22 a
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
A paced reset is refused before any Provider call ("no reset was sent"). The
refusal names the exact time the window has room for all of the reset's starts
(`retry_after_ms` and `Retry-After` on a named reset's `429 usage_reset_paced`)
and holds the Provider for that account until shortly after it: polls of every
account and other accounts' resets wait, so background quota reads cannot
starve a confirmed reset. The hold is in memory, ends when the reset starts and
otherwise lapses 30 seconds after the named time. The Web dialog shows the wait,
resends the same confirmed reset then, and lets Cancel stop a wait (never an
in-flight request). Resets
are serialized before their start is charged and hold that serialization until
the invocation ends, so the charged time is the actual call time. A scheduled
reset reserves its start before any one-shot claim, so pacing only delays it. A refused or changed reset records no failure backoff; an
authentication failure does.

A named native Claude account's confirmed reset (or a
[scheduled](#named-reset-schedules) one) runs on the Machine holding its native login,
through the same signed collector, CLI identity and token as its quota read
(Machine protocol 51, which enforces the reset's spacing; older Machines are
never selected or dispatched to), on the exact release the Controller selected and never
after its dispatch expiry. It takes its start from the same Anthropic Provider
window and spacing, on the account's own lane, and holds the account's
collection lock, so a quota read and a reset of one account never overlap. A
reset that never reached the Machine's collector records no failure backoff;
an authentication failure backs off even after the call was sent. A named reset
runs two collector invocations (the identity read, then the spend with its own
preflight). At admission the Controller reserves two starts one Provider
spacing apart, deriving the expiry, both reservations and the dispatched
spacing from one configuration snapshot; the Machine runs the identity read,
releases its lifecycle lock, waits for the second slot and a full spacing
measured monotonically from the identity read's actual spawn, rechecks the
selected release and spends, all before the dispatch expiry measured from the
original admission. A spend that provably never spawned is an unsent
`native_reset_preflight_failed`; a reply that carries both a success and an
error, or a refusal with a result payload, is an unknown outcome. When the Provider
window cannot hold two spaced starts, named resets are unsupported. A named
reset therefore takes at least one spacing interval end to end.

Known residuals, accepted for this release: a remote quota poll reserves its
Provider start before the account's binding transaction, so a poll refused
inside that transaction still consumes its paced slot (pacing only, never a
Provider call); a named reset's audit insert that times out after the
row was written leaves a `started` row with no terminal row (the permit settles
unsent); and the Service Grok collector's equal-time reset tie-break is not
aligned with the code-unit ordering of the named collectors (no Service Grok
card is operated).

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
