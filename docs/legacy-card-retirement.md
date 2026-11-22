# Legacy default card retirement

Status: implemented on `task/retire-legacy-cards-20261121` (2026-11-21).
This is the final step of moving subscriptions from legacy default cards to
named account cards: the legacy card code is deleted, so the Claude · Lynx
card disappears.

## Background

A legacy default card was the Service `default` Provider account of one
account-usage family, collected by a Provider-wide Service usage collector
and signed in through Service-scoped Provider authentication (the root vault,
replicated to every enrolled Machine):

| Card | Providers | Production state at retirement |
|---|---|---|
| `anthropic` (Claude · Lynx) | `claude-code` | expired login; cannot be adopted; its Service collector fails hourly |
| `openai` (OpenAI · Pika) | `codex` | to be adopted before this release ships |
| `xai` (Grok · Fox) | `grok` | adopted into a named account |
| `deepseek` (DeepSeek · Mole) | `claude-deepseek`, `codex-deepseek`, `deepseek-harness` | adopted into a named account |

Antigravity is named-only and never had a card. Named accounts (native Claude
logins, sign-in-once portable Codex/Grok credentials, portable DeepSeek API
keys) replace every card.

## What is removed

- Adoption: `accounts.adopt_legacy`, its `adoptions` receipt domain,
  `POST /api/provider-accounts/legacy/{card}/adopt`, the store's
  `adopt_legacy_card`/`complete_legacy_adoption`, `accounts.list().legacy_adoption`
  and the Web adoption flow (`SubscriptionLegacyAdoption`, `legacyAdoption.ts`).
- Legacy card browsers: `accounts.update {type:"set_legacy_browser_enabled"}`,
  `accounts.list().legacy_browsers`, card placement by Service credentials, and
  the `{card}` browser share member. The Web share editor, browser entries and
  the Service sign-in browser no longer know cards.
- The `subscriptions` display preference (`legacy_labels`) and its editor.
- Service usage collection for the card accounts (`anthropic`, `deepseek`,
  `openai`, `xai`): no scheduled, manual or credential-triggered Service
  collection, no Service-lane reset, and no Service usage row for them in the
  usage snapshot. The Web legacy usage cards (`UsageAccountCard.collector`),
  their usage Machine picker (`UsageExecutorPicker`), Service reset actions and
  schedules, and the Service-card fallback in session quota notices go with
  them. Managed-call `quota` reports named accounts only.
- Service (`default`) sign-in for every Provider of a card family:
  `default_login_refusal` and the direct commit route refuse it with
  `named_account_required`, and Settings → Providers shows no Service
  credential row for them (named cards take their place, as for Antigravity;
  `accounts.list()` reports them as `retired_default_providers`). After
  startup admission the root vault itself refuses every write destination of
  such a Provider: a commit, a shared-scope sign-in of another member, a
  shared scope adoption, and a Machine refresh promotion (answered as stale,
  so the Machine is resynchronised to the wipe). `set_default` to `default`
  remains accepted: it clears the named selection (for example to disable the
  last named default), and creation then needs an explicit named account.

## What is kept

- Named accounts of every kind, Antigravity, account controls, portable
  vaults, native logins, revocation, browser shares of named accounts.
- The `default` account identity on the wire, in the store and on Machines.
  Older Machines, historical sessions and the Machine protocol still name it;
  nothing is rewritten.
- Generic Service-scoped Provider authentication (status, wipe distribution,
  logout). It no longer admits a card family, but it still distributes the
  wipes the adoptions and the Lynx sign-out published.
- The completed legacy move records (`provider-auth/legacy-moves/`) and the
  route that promotes a Machine rotation of exactly a moved root generation
  into its account: a Machine that was offline during an adoption may still
  propose one, and dropping it could strand a rotated refresh token.
- Card browser keys (`legacy_cards::browser_key`, UUID v8). Adopted accounts
  use them as their registry identity, and a retired card key with a retained
  Machine placement can still be read, stopped and reset, never opened, so the
  owner can wipe its profile.
- The plugin usage bindings themselves: named account usage, DeepSeek activity
  history and Provider activity decoration read them. Historical DeepSeek
  activity remains readable; the Service collector no longer extends it.
- Service-level things unrelated to subscriptions (configuration, telemetry,
  Plugins, Machine components), and the generic Service usage engine: its
  usage-Machine settings route and Service reset scheduler remain for any
  future non-card binding but serve no card. The Web reset scheduling UI is
  unreachable without a Service lane.
- `browser_share_families.cards` stays as an empty list, so a Web release from
  before the retirement still accepts the account snapshot.

## Existing data

- **Historical `default` sessions** stay listable, readable, searchable and
  usable as context sources. A prompt to one is refused at the dispatch account
  gate and stays queued with the visible reason "This session ran on a retired
  legacy default account; continue it in a new agent on a named account".
  The queue is parked for good: it is never retried, woken, or counted as a
  waiter for capacity reclamation. A direct WebSocket prompt and current-turn
  or next-checkpoint steering are refused with the same reason, and a managed
  call that continues such a child is refused (`named_account_required`) or,
  if it was admitted earlier and its prompt is still queued, settled failed.
  Native fork is refused with the same guidance. Rationale: a native session
  can only resume inside the login home it was created in, and that credential
  no longer exists anywhere (it was wiped or moved into an account vault at a
  different generation). Silently rebinding the session to a named account
  would change its credential lineage under a running native thread; an
  explicit Continue in new agent on a named account is the honest operation,
  and it already exists. The refusal applies only where the retired card's
  root entry exists and is not Ready, so a fixture or Provider that never had
  a card is unaffected; a `default` session of a Provider without any root
  entry still fails as before with "not authenticated at Cowboy Service scope".
- **Creation** of a `default` session (new session, managed child, context
  next-agent, Continue, the store-less local path) for a retired Provider is
  refused with `named_account_required`, by Provider family and whether or not
  a root entry exists; managed readiness reports the same code. Folder and
  registry defaults that name a named account are unaffected.
- **Database rows.** No migration: the Controller runs
  `Store::retire_legacy_card_rows` at every start, after the credential
  admission gate and its migrations, before anything is served (never a CLI
  such as Machine enrollment, which may share a database with a Controller
  from before the retirement). It is idempotent and takes the account
  writer's owner lock before reading each registry. It
  removes every `card:<card>` browser share member and clears
  `provider_legacy_browsers`, and in the same transaction advances each
  affected share's revision and the owner's account registry revision (column
  and serialized value), so a share reset or account mutation captured against
  the earlier membership fails its compare. Running at every start also covers
  a re-upgrade after a rollback (the previous Controller may write card rows
  again, and SQLx never reruns a recorded migration) and a copied store (which
  is migrated empty and imported before a Controller opens it). Family
  bindings recorded for a card are kept. A binding of a retired card under its
  own family is a known, read-only identity: it keeps every member of that
  family out until a confirmed profile reset (so Claude · Lark can never
  inherit Claude · Lynx's cookies), its network requirements still apply, it
  is listed in the share's `bound` families (`retired_card`, account id = the
  card's browser key), and the share's other members keep their browser. The
  `provider_legacy_browsers` table stays (empty) so a rolled-back Controller
  can still read it; the stored `subscriptions` preference is no longer
  written, but `accounts.list()` still returns it as `display_preferences`, a
  read-only projection a Web client from before the retirement compares.
- **Scheduled resets.** A pending Service reset on a retired card lane (the
  closed set `anthropic`, `codex`, `deepseek`, `openai`, `xai`, plus any reset
  id of a retired binding) is cancelled at startup together with its audit
  row (`cancelled`, "the legacy default card is retired") in one
  transaction, also when its Plugin binding is absent, so a rolled-back
  Controller cannot fire it later.
  Named account resets are untouched. The usage executor settings neither list
  nor accept a retired card's collector.
- **Credentials.** Nothing is wiped implicitly by this release. Every legacy
  root credential must already be wiped through the existing paths: adoption
  (`openai`, `xai`, `deepseek`), or the Service sign-out (`DELETE
  /api/providers/claude-code/auth`, "Sign out" on the Service Provider row) for
  `anthropic`. Both publish a wipe generation that the existing Provider auth
  synchronisation delivers to every enrolled Machine.
  A wipe is retried until the Machine confirms it: the Machine's removal
  lists a directory again when a live process (a still-running `default`
  worker whose home it is) added entries meanwhile; a Machine whose wipe still
  left a projection reports it as `failed` (so every inventory report and
  reconnect sends the wipe again); and the Controller delivers a generation a
  connected Machine failed again with backoff (5 s to 1 h, then hourly)
  until it succeeds or is superseded, also for a wipe redelivered at
  reconnect. The Machine's periodic credential scan reports an incomplete
  wipe as failed too. A reconnecting Machine that still holds a moved root
  generation receives that Provider's wipe only after its first credential
  observation on the new connection (its scan publishes the inventory, then
  any rotation candidate, which is routed into the account) plus 15 s, or
  after 5 minutes when it reports nothing (older Machines, or no rotation). On 2026-11-21 Pika's root wipe on Hawk failed with
  `ENOTEMPTY` because a resumed `default` Codex session kept writing into its
  generation home, and nothing retried it until a later sign-out.

## Startup gate

The Controller refuses to start while any root vault entry of a card-family
Provider is not `signed_out`:

```text
legacy default cards still hold Service credentials (codex): adopt them into
named accounts or sign them out with the previous Controller before installing
this release
```

The component activator then restores the previous Controller profile, so
shipping this release before Pika is adopted cannot break Pika: the old
Controller keeps serving it and its adoption path. After a successful start no
card credential can reappear (the root vault refuses every write to a card
Provider), so the gate cannot fail a later reboot.

An adoption interrupted after its root wipe but before its move journal was
marked completed passes the credential check. The gate finishes such a
journal (the root entries are wiped and the account holds the copy), so a
Machine's later rotation of the moved generation is still promoted into the
account; it refuses a journal whose root entry still holds a credential, and
leaves unrouted one whose account no longer holds a copy (a revoked account).
A journal without the copy's account generation (the previous writer stopped
before recording it, possibly before copying) is never routed: an account
status cannot prove it is the move's copy rather than an independent
sign-in. Startup logs it at error level for an explicit repair. Production
had none at release time (every journal completed with account generation
1).

## Release order and operations

1. With the current Controller: adopt OpenAI · Pika (`accounts.adopt_legacy
   {card:"openai"}`), sign out Claude · Lynx at Service scope, and confirm
   the Service authentications (`GET /api/plugins` `authentications[]`) show
   every card-family Provider `signed_out`. Reset
   (or stop) a Lynx Machine browser if one was enabled.
2. Controller release (`.#cowboy-controller-release`): runs the migration and
   the gate.
3. Web release (`.#cowboy-web-release`). A Web shipped first only hides the
   cards early; it breaks nothing.
4. No Machine release is needed; Machines keep the `default` identity.
