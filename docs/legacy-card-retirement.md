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
  `named_account_required`, `set_default` to `default` is refused, and
  Settings → Providers shows no Service credential row for them (named cards
  take their place, as for Antigravity; `accounts.list()` reports them as
  `retired_default_providers`).

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
  next-agent, Continue) for a retired Provider is refused with
  `named_account_required`. Folder and registry defaults that name a named
  account are unaffected.
- **Database rows.** Migration `0086_retire_legacy_cards.sql` (SQLite `0060`)
  removes every `card:<card>` browser share member, bumping its share's
  revision, and clears `provider_legacy_browsers`. Family bindings recorded
  for a card are kept: they become unreadable bindings that block every member
  of that family until a confirmed profile reset, so Claude · Lark can never
  inherit Claude · Lynx's cookies. The `provider_legacy_browsers` table stays
  (empty) so a rolled-back Controller can still read it; the stored
  `subscriptions` preference row is ignored.
- **Credentials.** Nothing is wiped implicitly by this release. Every legacy
  root credential must already be wiped through the existing paths: adoption
  (`openai`, `xai`, `deepseek`), or the Service sign-out (`DELETE
  /api/providers/claude-code/auth`, "Sign out" on the Service Provider row) for
  `anthropic`. Both publish a wipe generation that the existing Provider auth
  synchronisation delivers to every enrolled Machine.

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
card credential can reappear (Service sign-in and commit are refused), so the
gate cannot fail a later reboot.

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
