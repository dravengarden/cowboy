# Portable refresh custody lease — design

Status: revision 3, 2026-11-21. Revision 3 adds the owner's choice **Grok
first, Codex later** and the [Grok implementation
design](#grok-implementation-design-revision-3), which takes precedence over
the general design sections below for Grok; Codex keeps today's behaviour.
Revision 2 recorded the [owner decisions](#owner-decisions-2026-11-21) and the
[custodian lock verification](#custodian-lock-verification-2026-11-21).

## Owner decisions (2026-11-21)

1. **Acceptance** uses a fake vendor/IdP only: one that rejects refresh-token
   reuse and exercises concurrent races, restarts, disconnects and expiry
   enforcement. There is no real-account acceptance step; after release the
   real accounts are observed passively.
2. **Codex** shares one `auth.json` per account on the custodian Machine
   across auth generations, provided the pinned CLI serializes it with its own
   lock. If it does not, stop and report before choosing a fallback.
3. **The Service never redeems tokens with the vendor itself** (alternative 3
   rejected): redemption stays on Machines and Cowboy adds no vendor-facing
   origin.
4. **Older Machines** (below protocol 51, today Falcon and MacBook Air) are
   excluded from a lease-managed account: no new placements there, and an
   account becomes lease-managed only after its sessions on older Machines have
   ended. The exclusion is a visible reason in Web and MCP.
5. **Idle renewal** is allowed: a budgeted usage read may make an idle eligible
   Machine the temporary custodian, renew once, distribute, then release.

## Custodian lock verification (2026-11-21)

Decision 2 depends on the pinned CLIs serializing concurrent refreshes of one
shared credential file. It was checked offline: each pinned CLI ran in a user
and network namespace with loopback only, against a fake IdP that issues
single-use refresh tokens and logs every token request, with a fake
`auth.json`, no real credentials and no internet (`CODEX_REFRESH_TOKEN_URL_OVERRIDE`
for Codex; the auth entry's `oidc_issuer` for Grok).

| Case | Result |
| --- | --- |
| Codex 0.169.1, two `app-server` processes, two homes whose `auth.json` is a symlink to one file, sequential forced refresh | Writes go through the symlink (it stays a symlink); the second process re-reads and presents the rotated token. Locks are created per home (`<home>/auth.lock`) plus `<target>.lock`. |
| Same, concurrent forced refresh (`account/read {refreshToken:true}`), 5 runs | **Both processes presented the same refresh token in 5/5 runs.** |
| Codex, two processes in **one** `CODEX_HOME`, concurrent forced refresh, 3 runs | **Both presented the same token in 3/3 runs.** |
| Codex, one `CODEX_HOME`, expired access token, concurrent natural refresh (`refresh_reason: proactive_expiry`), 3 runs | **Both presented the same token in 2/3 runs.** |
| Grok 1.0.77, two `agent stdio` processes, one `HOME`, expired access token, concurrent startup refresh, 3 runs | Exactly one redemption per run (3/3); the other process adopted the rotated token. |
| Grok, refresh token removed (access-only projection), expired token | No token-endpoint request; the CLI stays signed out. |
| Codex, empty refresh token, expired token | The CLI still calls the token endpoint (with an empty token) and cannot redeem anything. |

Conclusion: **the pinned Codex CLI does not serialize concurrent refreshes,
even for processes sharing one `auth.json` in one home.** Its lock protects
the file commit ("Credentials changed before refresh commit."), not the
vendor request. Decision 2 therefore fails as specified, and implementation
stopped before choosing a fallback. The Grok CLI's lock does serialize
refreshes of one shared file, and an access-only Grok projection cannot
redeem.

Candidate fallbacks for Codex, for the owner to choose:

- **One Codex process holds the refresh token.** On the custodian only one
  process at a time gets the full projection (a single designated worker, or
  a Machine-side refresh helper), every other Codex worker, on every Machine,
  runs access-only and picks up rotations as new generations. Correct, but it
  means at most one concurrently refreshing Codex worker per account, and
  other workers fail authentication if the access token (ten days, per
  `plugins/codex/collector/index.js:124`) expires before the holder renews it.
- **The Machine serializes Codex refreshes itself** (a per-account Machine lock
  taken by a wrapper around the app-server's token request, for example a
  loopback proxy set through `CODEX_REFRESH_TOKEN_URL_OVERRIDE` that admits one
  in-flight refresh and answers others from the result). This keeps every
  Codex worker refresh-capable on the custodian, but adds a Machine-local
  component in the vendor path and must be accepted against the exact CLI.
- **Grok first, Codex later**: implement the lease for Grok (whose lock was
  verified) and keep Codex on today's behaviour until one of the above is
  chosen.

## Grok implementation design (revision 3)

The owner chose to implement the lease for **Grok** named subscription
accounts only. Codex keeps today's behaviour; the two Codex fallbacks above
remain the follow-up. The verification also shows that **concurrent Codex
processes already double-redeem today**: two app-servers of one account on one
Machine (or on two Machines) can present the same refresh token, so a Codex
account can be signed out by its own sessions. That risk is unchanged by this
work.

Two further facts were verified offline, as above, against Grok 1.0.77:

- `GROK_AUTH_PATH` relocates the credential file and its lock: two processes
  with different `HOME`s and one `GROK_AUTH_PATH` redeemed exactly once in 3/3
  runs, and the lock file is `<GROK_AUTH_PATH>.lock`.
- The access-only copy keeps `oidc_issuer` and `oidc_client_id`: without them
  the CLI does not treat the sign-in as a grok.com account (billing answers
  "requires auth with grok.com"). Removing only `refresh_token` already
  prevents every token-endpoint request.

The general design keeps a resident custodian whose CLIs refresh in the
background. Grok's sessions do not need one, so this revision is smaller:
**the refresh token never rests on any Machine.** It exists in the Service
vault and, briefly, inside one bounded renewal operation.

### Declaration

The Grok Plugin ships `collector/refresh-custody.json` in its signed host
bundle (like `collector/portable-account.json`, no SDK change):

```json
{
  "schema": 1,
  "bundle_key": "auth_json",
  "strip_entry_fields": ["refresh_token"],
  "expiry_entry_field": "expires_at",
  "credential_path_env": "GROK_AUTH_PATH"
}
```

A Provider is custody-managed only when the exact signed release the Service
reads it from declares this file. Codex, DeepSeek, Claude and Antigravity do
not, and none of their paths change.

### Access-only distribution

For a managed account the Service seals, to **every** Machine, the vault
bundle with `strip_entry_fields` removed from each entry of the
`bundle_key` JSON document. The generation is the vault's; same-generation
re-sealing is already accepted by Machines (`validate_auth_replica_transition`,
`components/machine/src/machine_plugins.rs:3649-3674`). Sessions, usage
collectors and logins on any Machine therefore never see a refresh token.

Because the CLI may rewrite an access-only `auth.json` (enrichment), every
ordinary `ProviderAuthRefreshCandidate` for a managed account is refused and
the vault generation is re-projected; only a renewal reply (below) can change
the vault.

### Renewal operation

`RenewPortableCredential` (Machine protocol 51) carries the account, the
exact collector Plugin generation, the vault generation, a fresh renewal ID
and the full bundle sealed to that Machine. The Machine:

1. refuses unless it runs that exact generation, the account replica is that
   vault generation, and the declaration is present;
2. restores the full bundle into a fresh private directory (mode 0700,
   outside every credential watch);
3. runs the collector's ordinary refresh-capable path (no `token_refresh`)
   with `credential_path_env` pointing at that file (plus any declared
   `renew_environment`), so the CLI renews an expiring token at startup;
4. kills the run at a local deadline of 60 s after receipt, reads the file,
   removes the directory, and replies with the collector output and the
   resulting bundle (or "unchanged").

The Service keeps one durable renewal record per account
(`{renewal_id, machine_id, vault_generation, started_at_ms, settled}`), written
before dispatch, in the Controller data directory. A pending renewal is never
followed by a fresh one at the same generation (see Implementation below). A reply is accepted only for the recorded renewal ID; its bundle goes
through the unchanged generation compare-and-swap at the recorded generation
and is then distributed access-only.

### Who triggers a renewal

- **Usage read.** When the refresh-free collector reports authentication
  required on the placed Machine (protocol 51), the same budgeted collection
  runs one renewal there and stores the renewal run's collector output. At
  most one renewal per read; a refusal or failure stores
  `portable_usage_authentication_required` and follows the existing backoff.
  This is decision 5: the placed Machine is the temporary custodian.
- **Sessions.** A protocol-51 Machine with a live worker of a managed account
  sends `PortableCredentialRenewalNeeded { account, provider, generation }`
  when the replica's `expiry_entry_field` is less than 4 minutes away, and when
  it launches a worker on an already expired replica. The Service runs a
  renewal on that Machine unless the vault generation already moved or a
  renewal is in flight. No timer runs on the Service, and a Machine with no
  live worker of the account sends nothing.

### Enabling and old Machines

An account becomes managed (persisted, never automatically reverted) when its
Provider declares custody, the account has no live worker on any Machine, and
none of its untrashed sessions sit on a disconnected Machine. The Service then
re-seals access-only to every Machine. Until then it keeps today's behaviour.

Machines below protocol 51 (Falcon, MacBook Air) get no new placement for a
managed account: session admission and usage placement refuse them with
`portable_refresh_custody_machine_update_required`, shown in Web and MCP.

### Implementation (2026-11-21, branch `task/portable-refresh-lease-20261121`)

Implemented as above, with these refinements found by the fake-vendor race
test (`usage::custody::tests::racing_renewals_never_reuse_a_refresh_token`):

- The renewal record has four states: `pending`, `settled` (provably did not
  redeem), `rotated` (redeemed its generation's token) and `unknown`. A
  pending renewal is never followed by a fresh one at its generation; it only
  **resumes** on its own Machine, which returns its retained reply (kept in
  memory for 15 minutes) and never runs twice. A resume the Machine no longer
  retains, an unreadable credential after the run, or a Machine gone for
  longer than 5 minutes makes it `unknown`, which blocks renewal until the
  vault moves on (a new sign-in). A `rotated` renewal blocks its generation
  even if this process still believes the vault holds it; the test failed on
  exactly that interleaving before the state existed.
- The custody state file is `usage-refresh-custody.json` in the Controller
  data directory, written atomically before every change takes effect. An
  unreadable file poisons the store: every declaring account is treated as
  managed (access-only) and no renewal runs.
- A Provider that stops declaring custody leaves its recorded accounts
  unsealable rather than sealing a refresh token again.
- A session's rejection marks the generation Expired; for a managed account
  usage still collects at that generation, so the next budgeted read renews.
- The declaration carries no `renew_environment`: with
  `GROK_AUTH_EARLY_INVALIDATION_SECS=86400` the CLI redeemed six times in one
  run (each new one-hour token was again "early"). With the default 300 s
  window, an offline end-to-end run of the real collector and the pinned CLI
  against a fake single-use IdP (custody file restored from a token expiring in
  30 s, `HOME` holding the access-only copy) redeemed exactly once, rotated only
  the custody file and left the access-only copy untouched. (Its billing call
  then failed only because the fixture's loopback issuer is not grok.com.)
- The Machine-side sender of `PortableCredentialRenewalNeeded` is **not
  implemented yet**: the event, its protocol gate and the Controller handler
  exist, but sessions currently rely on the usage path (including the existing
  background collection) to renew an expired token. Until the sender ships, a
  session may fail authentication between its access token's expiry and the
  next usage collection.

### Why this cannot double-redeem

- Only a renewal process holds a refresh token, and the Service admits one
  renewal per account at a time, durable across Controller restarts.
- The Machine bounds the renewal (60 s) far below the Service's reuse window
  (5 minutes).
- Every other copy on every Machine, including old Machines, lacks the token
  and cannot redeem (verified).
- A late or foreign result cannot enter the vault: only the recorded renewal
  ID is accepted, and ordinary candidates are refused.

### Residual risks

- A renewal command delayed in delivery by more than about four minutes on a
  live connection could overlap the next renewal. Connection-bound delivery
  makes this unlikely but not impossible.
- An access-only worker in the middle of a turn when its access token expires
  fails authentication if the renewal lands too late; whether a running Grok
  CLI adopts a rewritten `auth.json` without a restart is not verified.
- Sessions that started before the account became managed are not affected
  (enabling waits for them), but a Machine bug that skips the strip or the
  deadline would defeat the guarantee; the fake-vendor tests below target it.

## Goal

1. A named account's single-use refresh token is never redeemed by two
   writers at once, across Machines and across processes on one Machine.
2. An idle account (no session renewing it) still shows quota: a budgeted
   usage read may renew the token, safely, at most once per read.

The requirement this implements is [Named Provider accounts: isolation
acceptance](requirements.md#named-provider-accounts-isolation-acceptance),
`docs/requirements.md:200-204`:
"A Service CAS after refresh is insufficient to serialize upstream
refresh-token use: the implementation must coordinate before a refresh can
reach the vendor, including across Machines. Copying a refresh token to several
independent writers is not an isolation design."

## Current state

### Who redeems today (verified)

- **The Provider CLI, on its own.** A session's worker runs the native CLI
  with the account's replica home. The CLI refreshes whenever it decides to:
  - Grok CLI 1.0.77 refreshes at startup/`initialize` when the access token is
    expired or within `GROK_AUTH_EARLY_INVALIDATION_SECS` (default 300 s),
    proactively in the background, and after any 401 (including billing side
    calls); no setting disables it. Its strings name the OIDC refresher, a
    per-file lock (`auth: file lock timed out, waiting for sibling to finish`,
    `auth: refresh adopted sibling token after lock timeout`,
    `auth: returning transient to avoid refresh token reuse`) and the
    "no token with refresh_token available" outcome.
  - Codex 0.169.1 (app-server) refreshes on demand and after 401, holds
    `auth.lock` beside `auth.json` (`Credentials changed before refresh
    commit.`, `Account credentials changed during refresh.`), and reports
    reuse as permanent: "Your access token could not be refreshed because your
    refresh token was already used. Please log out and sign in again."
    (strings of the pinned binary; `refresh token has expired` and `was
    revoked` are separate permanent outcomes).
  - Grok serializes only processes sharing **one** credential file; Codex does
    not serialize concurrent refreshes at all, even then (see the
    [custodian lock verification](#custodian-lock-verification-2026-11-21)). Neither
    Plugin uses the SDK's `CredentialDirectory` binding
    (`components/provider-sdk/src/lib.rs:845-849`, "Without it each generation
    locks its own private home and concurrent refreshes race a single-use
    refresh token"); `plugins/codex/provider.json:397-401` and
    `plugins/grok/provider.json:234-237` project `auth.json` per runtime home.
- **The Machine proposes, after the fact.** `machine_cli/auth_watch.rs:92-125`
  watches credential files and `machine_cli.rs:1845`
  (`publish_provider_auth_observations`) sends
  `MachineEvent::ProviderAuthRefreshCandidate` with `expected_generation`.
- **The Service compare-and-swaps, after the fact.**
  `server/machine_connection/frames.rs:188-215` accepts the candidate;
  `server/provider_accounts/portable/refresh.rs:139-211` runs
  `compare_and_swap_refresh` (`src/provider_service.rs:738-767`) under the
  owner's account lock: the first candidate at the expected generation wins and
  `distribute_and_mark_scope` (`server/provider_auth_flow.rs:358`) seals it to
  **every** enrolled Machine; a loser is re-projected (`Stale`).
- **Every Machine therefore holds the refresh token.** Distribution seals the
  full portable bundle (the whole `auth.json`) to each Machine.
- **Usage never redeems.** Since Codex 3.6.16 and Grok 3.2.16 the collector
  run for `token_refresh: false` (`native_usage/portable.rs:7-15`, enforced by
  `require_refresh_free` at `:121-141`) reports authentication required instead
  of refreshing (`plugins/codex/collector/index.js:140`,
  `plugins/grok/collector/index.js:557-624`).

### The race

Two writers holding refresh token `RT_n` — two Machines, or two auth-generation
homes on one Machine — can both send `RT_n` to the vendor before either
candidate reaches the Service. The CAS then keeps one result, but the vendor has
already seen `RT_n` twice. It can also happen after a successful rotation: a
busy worker that started on generation `n` keeps `RT_n` in memory after the
vault moved to `n+1`, and its next refresh presents a spent token.

Consequences per Provider:

| | Codex (ChatGPT) | Grok (xAI) |
| --- | --- | --- |
| Reuse handling | Verified: the CLI names "refresh token was already used" as a permanent failure that needs a new sign-in. | Verified: the CLI avoids reuse locally ("returning transient to avoid refresh token reuse"). Inferred, unverified: xAI's IdP rejects or revokes on reuse. |
| Access token lifetime | Unknown; inferred to be days. | Unknown; inferred to be hours (300 s early-refresh default). Observed 2026-11-21: Fox's token had expired while the account was idle. |
| Blast radius | The losing writer, and if the vendor revokes the token family, every Machine: the account needs a new sign-in. | Same if xAI revokes the family. |

What is **not** a defence: the Service CAS (after the vendor effect), the
Controller's `has_live_worker` (`src/supervisor.rs:1005`, only workers this
Controller currently sees; detached workers survive Controller restarts, see
`architecture/03-supervisor.md`), and the Machine-local CLI lock (one file only).

## Design: refresh custody

The refresh token is a capability. Instead of asking every holder to behave,
**only one Machine ever holds it**: the custodian named by a Service lease. All
other Machines receive an access-token-only projection, which cannot redeem.

### Projections

- **Full**: the vault bundle as today, including the refresh token. Sealed only
  to the current custodian.
- **Access-only**: the same bundle with the refresh token removed by a signed,
  Plugin-declared transform (new `authentication.refresh_custody` block naming
  the JSON paths to clear, e.g. `tokens.refresh_token` for Codex and
  `*.refresh_token` for Grok). The Service applies it before sealing, so a
  non-custodian Machine — including an old one — never receives the bytes.

An access-only worker can run until its access token expires; then its CLI
fails authentication (Grok: "no token with refresh_token available"; Codex:
`not_refreshable_auth`). Both outcomes must be confirmed by the conformance
tests below before a Provider is enabled.

### Lease

One row per account in the Service database:

```text
account_id, holder_machine_id, epoch (u64, strictly increasing), granted_at_ms,
expires_at_ms, state: held | releasing | free
```

- **Grant** (Service): only when the state is `free` or the previous lease has
  expired by more than a margin (`expires_at_ms + 30 s`). Epoch increments.
  The grant and the full projection are delivered together on the Machine
  connection; the Machine acknowledges.
- **Renew** (Machine → Service, on the existing connection, every TTL/3 while
  held). TTL 10 minutes. The Machine computes its own deadline as
  `request_sent_at (monotonic) + TTL − 30 s`, so it can only expire earlier than
  the Service believes, never later.
- **Release** (Machine): after it has rewritten its credential homes to
  access-only and its full-projection worker count (below) is zero, it reports
  `released{epoch}`; the Service marks `free`.
- **Expiry** (Machine, local, enforced without the Service): at its deadline a
  Machine still holding the lease stops every worker of that account that
  holds the full projection (sessions stay resumable; they restart access-only)
  and rewrites its homes to access-only, then reports release when it
  reconnects. This is what makes a disconnected custodian safe: the Service
  never regrants before the custodian's deadline plus margin has passed.
- **Durability**: the lease row survives Controller restart; a restarted
  Controller treats a `held` row as held until `expires_at_ms + margin`, never
  as free. The Machine persists `{account, epoch, deadline_wall, boot_id}`; a
  restarted Machine agent with detached workers re-reads it and enforces the
  same deadline (a changed `boot_id` means the processes are gone anyway).
- **Fencing**: every `ProviderAuthRefreshCandidate` for a lease-managed account
  carries the epoch. `refresh.rs::refresh` accepts it only from the current
  holder at the current epoch, then runs the unchanged generation CAS. A stale
  holder's late candidate is refused and the vault generation is re-projected.
  Epoch fencing protects the vault; the vendor effect itself is prevented by
  custody plus local expiry, not by fencing.

### Who may redeem

Only processes on the custodian, against the custodian's single shared
credential file:

- **Session workers on the custodian.** They run the CLI with the full
  projection, through a shared per-account credential path so every auth
  generation's workers use one file and therefore one CLI lock (Grok:
  `GROK_AUTH_PATH`, which the CLI reads, seen in its `AuthManager::new` log;
  Codex: needs a supported way to place `auth.json` outside `CODEX_HOME` —
  open question 2). The CLIs' own background and 401 refresh then remains
  allowed and serialized by the CLI lock; it cannot be disabled and need not be.
- **Usage renewal on the custodian** (below).
- **Nobody else.** Workers elsewhere are access-only. When their access token
  nears expiry they cannot refresh; the custodian's renewal reaches them as a
  new vault generation, which drains idle workers onto it as today
  (`provider-auth-sync-coordination.md:18-20`).

Choosing the custodian: custody follows sessions. A Machine starting or waking
a worker on a lease-managed account requests the lease if it is `free`; if
another Machine holds it, the worker starts access-only. Custody moves only via
release (custodian has no full-projection workers) or expiry — never by
preemption while the custodian runs workers.

Residual risk, stated honestly: a custodian whose expiry enforcement fails
(agent hung while its CLIs keep running, wall-clock and monotonic both broken,
a worker escaping process accounting) could still redeem after the Service
regranted. Margins reduce, but do not eliminate, this risk.

### Live-worker accounting

Each lease-capable Machine reports, per account, the number of live workers
holding the full projection and the number holding access-only, stamped with
its connection incarnation and a monotonically increasing report sequence. It
reports on every change and in its Hello. Detached workers are the Machine's
own processes, so they are counted whether or not a Controller is connected.

The Service never infers "no workers" from absence: a disconnected Machine, a
Machine that has not reported since the Controller restarted, or a report from
an older incarnation counts as "unknown", which only lease expiry resolves.

Hibernated sessions have no process and hold nothing. Waking one is a worker
start and follows the custody rule above.

### Usage flow

Within one budgeted collection (`usage/portable.rs:298`, `refresh`), at most
once:

1. Placement prefers the custodian when it qualifies (it already runs most of
   the account's sessions).
2. If the account has a custodian and it is the placed Machine, the command is
   `CollectPortableUsage { lease_epoch }`. The Machine runs the collector in
   refresh-allowed mode (the CLI path) only if it holds that epoch and its
   local deadline is at least the command deadline (30 s) away; otherwise it
   refuses with `portable_usage_lease_lost`.
3. If the lease is `free`, the Service grants it to the placed Machine first
   (step 2 then applies). Granting is safe without worker accounting because a
   free lease means no Machine holds the refresh token.
4. If another Machine holds the lease (it runs sessions, which renew the
   token), the collection runs refresh-free as today and an expired token
   reports `portable_usage_authentication_required`.
5. After a refresh-allowed run the Machine exports the credential if it changed
   and includes the candidate (with epoch) in the command reply. The Service
   runs the fenced CAS and distributes the new generation (full to the
   custodian, access-only elsewhere) before storing the observation. The
   observation is the same collector run; there is no second collection.
6. Failure modes: lease lost, CAS stale or rejected, vendor refusal (reuse,
   revoked, expired refresh token) and timeout all store
   `portable_usage_authentication_required` (or the closed code they map to),
   charge the same budget permit, and respect the existing auth-failure
   backoff. Nothing is retried in the read. A usage-only lease is released
   after the run when the Machine has no full-projection workers.

No polling is added: renewal happens only inside a collection the shared
budget already admitted.

### Protocol and rollout

- **Machine protocol 51**: lease grant/renew/release/expiry messages, the
  per-account worker report, `lease_epoch` on `CollectPortableUsage`, epoch on
  refresh candidates, and access-only projections in replica delivery. Older
  Machines never become custodians.
- **Older Machines** (today Falcon and MacBook Air): once an account is
  lease-managed, the Service seals only access-only projections to them, so
  they can never redeem. Their pre-existing full replicas and running workers
  still hold the refresh token, which is why enabling an account needs the
  quiescence step below.
- **Enabling an account** (per account, owner-triggered or automatic when
  safe): the Service marks the account lease-managed only when every enrolled
  Machine is either lease-capable and reports zero full-projection workers of
  it after rewriting its homes, or is older and reports no live worker of the
  account through the existing session state. Until then the account stays on
  today's behaviour.
- **Order**: Plugin releases declaring `refresh_custody` → Machines on protocol
  51 → Controller. A Controller that speaks 51 with no lease-managed account
  behaves exactly as today.
- **Rollback**: a Controller rollback to a release without leases would again
  seal full projections to every Machine. To keep that safe the newer
  Controller writes the lease table in a form the older Controller ignores, and
  the operator must first run `cowboy operator lease-release --all`, which
  waits for every custodian to release. Machine rollback below 51 forces the
  account back to non-managed via the same release.
- **Web/MCP**: the account card adds two reasons: `portable_usage_lease_busy`
  ("Another Machine is renewing this account; usage will refresh after it does")
  and `portable_usage_lease_lost`. `accounts.usage` reports
  `custody: {machine_id, managed}`; no credentials, epochs or deadlines are
  exposed to agents beyond that.

## Test plan

Deterministic, no real accounts:

- Fake vendor token endpoint that issues single-use refresh tokens and records
  every presentation; any reuse fails the test and revokes the family.
- Lease state machine: grant only when free/expired+margin, epoch monotonic,
  renew/release/expiry transitions, Controller restart with a held row, Machine
  restart with detached workers and a persisted deadline, clock skew in both
  directions, a custodian disconnected past its deadline.
- Two Machines and the real Machine binary with fake CLIs that refresh at
  startup, in the background and on 401, plus a usage read racing a session
  start: the fake vendor sees each refresh token at most once.
- Access-only projection conformance against the pinned real CLIs in a network
  namespace with a fake IdP (as done for Grok billing): the CLI never contacts
  the token endpoint without a refresh token, and reports authentication
  required.
- Shared credential path conformance: two auth generations on the custodian
  share one file and the CLI lock serializes their refreshes.
- Fenced CAS: stale-epoch candidates refused; generation CAS unchanged.
- Old Machine fixture (protocol 50) only ever receives access-only bytes for a
  managed account.

Real Machines, without risking the owner's accounts: a dedicated throwaway
Codex and Grok account (open question 1), enabled on Hawk and OVH only, with
the fake-vendor suite as the gate before it. The owner's accounts are enabled
only after the throwaway accounts survived a week of sessions on two Machines,
idle usage reads and a forced custodian disconnect.

## Alternatives considered

- **Usage renewal through today's path.** Rejected: redemption precedes
  coordination; it races any session refresh on another Machine.
- **Renew only when the Controller sees no live worker.** Rejected: the
  Controller's view misses detached workers during reconnects and restarts and
  cannot stop a later session start.
- **Service-side refresh** (the Service calls the vendor's token endpoint
  itself). Smallest and strongest serialization, but makes Cowboy an OAuth
  client of OpenAI and xAI with their CLIs' client IDs; this needs the
  Providers' terms checked as was done for Anthropic
  (`requirements.md:213-226`) and an exact token-endpoint contract per CLI
  version. Kept as open question 3.
- **Separate native sign-in per Machine** (requirements' preferred design).
  Removes sharing entirely but needs the owner to sign in on every Machine and
  separate vault lineages; not verified to be independent lineages for Codex
  or Grok (`requirements.md:205-210`).
- **Pin each account to one Machine.** Violates "neither the account record
  nor its default may pin execution to one Machine" (`requirements.md:190-192`).

## Open questions for the owner

Questions 1-5 below were answered in [Owner decisions](#owner-decisions-2026-11-21).
The open question now is which Codex fallback in the
[custodian lock verification](#custodian-lock-verification-2026-11-21) to take.
The original questions are kept for the record:

1. May we create a throwaway Codex and Grok subscription account for real
   acceptance, or should acceptance use only the fake vendor plus the owner's
   accounts with explicit approval?
2. Codex: is a supported way to share one `auth.json` across auth generations
   on a Machine acceptable (for example a shared `CODEX_HOME` only for the
   credential, if the pinned CLI allows it), or should a custodian run at most
   one auth generation of the account at a time?
3. Would you rather Cowboy redeem refresh tokens itself on the Service
   (alternative 3), subject to a terms review?
4. When an account is enabled while Falcon or MacBook Air still run its
   sessions, should enabling wait for those sessions to end, or should those
   Machines be excluded from the account until they are upgraded?
5. Should usage renewal be allowed to make an idle Machine the custodian
   (it holds the refresh token afterwards until release), or only when the
   custodian already exists?
