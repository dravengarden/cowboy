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
  and it already exists. The refusal applies to every `default` session of a
  card-family Provider whose root entry is not Ready, including one with no
  root entry at all (a Machine's legacy launch path could otherwise resume it
  on its own native login); a Provider that never had a card is unaffected.
  Clear conversation of such a session is refused before it clears anything,
  with the same guidance.
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
  Controller cannot fire it later. Every lane is attempted, and any failure
  fails Controller startup (the activator then restores the previous
  Controller) rather than serving with a schedule left behind.
  Named account resets are untouched. The usage executor settings neither list
  nor accept a retired card's collector.
- **Credentials.** Nothing is wiped implicitly by this release. Every legacy
  root credential must already be wiped through the existing paths: adoption
  (`openai`, `xai`, `deepseek`), or the Service sign-out (`DELETE
  /api/providers/claude-code/auth`, "Sign out" on the Service Provider row) for
  `anthropic`. Both publish a wipe generation that the existing Provider auth
  synchronisation delivers to every enrolled Machine.
  A wipe is retried until the Machine confirms it. The Machine's removal
  lists a directory again when a live process (a still-running `default`
  worker whose home it is) added entries meanwhile, and a Machine whose wipe
  left a projection reports the Provider as `failed` in every inventory and
  credential scan. A Machine applying a new wipe also rolls that account's
  workers (a live worker may still hold the credential), and syncs the
  replica and projection directories before acknowledging it. Every
  successful delivery records a durable per-Machine acknowledgement in its
  credential scope (`deliveries.json` beside the vault), bound to the
  Machine's enrollment (its encryption key), so a re-enrolled Machine id
  starts unacknowledged. The recipients are the enrolled, non-revoked
  Machines (a placeholder row without enrollment keys is none), one query
  shared by delivery, reconciliation and the startup gate. One Controller
  loop (every 10 s, and at once when a Machine connects or reports its
  inventory) sends the current generation of every Service-held credential
  to each connected recipient whose acknowledgement is missing, and
  recomputes the distribution state from the acknowledgements. A failing
  recipient backs off in memory by Machine id (5 s to 1 h); a new connection
  resets it. Automatic repair runs only through this loop: an inventory that
  contradicts an acknowledgement (the projection failed, or an older
  generation) forgets it, so a Machine reporting the same failure keeps its
  backoff.
  On 2026-11-21 Pika's root wipe on Hawk failed with `ENOTEMPTY` because a
  resumed `default` Codex session kept writing into its generation home, and
  nothing retried it until a later sign-out.

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

### Delivery and session gate

Right after migrations, before the card-row cleanup writes anything (a
refused upgrade leaves the database as the previous Controller left it),
the Controller also requires:

- every enrolled, non-revoked Machine to have applied every card Provider's
  latest root wipe: its durable acknowledgement, or a distribution the
  previous Controller recorded as `current` (it reached every Machine
  enrolled then; it kept no per-Machine record). Then no Machine can still
  hold, and later rotate, a moved root generation, so nothing holds a wipe
  back for an offline rotation. A Machine enrolled later never held the root
  credential;
- no session on a retired card's `default` account to be starting, running
  or busy (its worker may still hold a card credential).

The refusals name the Providers, Machines and sessions:

```text
legacy default card wipes are not acknowledged by every enrolled Machine
(codex: falcon): connect those Machines to the previous Controller until the
wipes are delivered, or revoke an unreachable Machine, before installing this
release

sessions on retired legacy default accounts are still live (sess-1): stop
them with the previous Controller before installing this release
```

The admission is recorded with the Service dataset and each card Provider's
root wipe generation (`retirement-admitted.json` beside the vault). A later
start with exactly those (a reboot with a Machine offline) skips the
Machine wipe check; any change (a rollback signed a card in and out again)
runs it again. The live-session check runs on every start (an older
Controller after a rollback could resume a card session without changing any
wipe). Delivery acknowledgements are recorded under exactly the enrollment
key the envelope was sealed to. A reconnecting Machine only wakes the
reconciler for itself; there is no fleet-wide redelivery on reconnect, and
failed-session recovery follows each successful root delivery.

## Release order and operations

### Production cutover (prerequisites already met)

On the production Service the adoptions and the Lynx sign-out are done, so
the cutover only verifies them before releasing:

1. **Verify** with the current Controller:
   - every card-family Provider's Service authentication (`GET /api/plugins`
     `authentications[]`) is `signed_out` with distribution `current`, so
     every enrolled Machine applied its wipe;
   - no session on a card's `default` account is starting, running or busy;
   - on every enrolled Machine, no worker process of a card family's
     `default` account is alive (a `default` worker is launched without
     `--provider-account-id`). Run on each Machine, read-only; the output
     must be empty:

     ```sh
     ps -eo pid,args | grep '[c]owboy-acp-worker ' \
       | grep -E -- '--provider (claude-code|codex|grok|claude-deepseek|codex-deepseek|deepseek-harness)( |$)' \
       | grep -v -- '--provider-account-id'
     ```

     A listed process is such a worker: stop its session with the current
     Controller (or the Machine's own stop), then run the check again. The
     Controller's status rows cannot prove this on their own (a worker can
     outlive a session recorded as crashed).
2. **Web release** (`.#cowboy-web-release`): the new Web works with the old
   Controller and never opens a retired card's sign-in on the device.
3. **Close or reload every open Cowboy tab**, so no older tab still holds a
   cached legacy sign-in challenge.
4. **Controller release** (`.#cowboy-controller-release`): runs both gates,
   then the cleanup.
5. No Machine release is needed for the upgrade; Machines keep the `default`
   identity. A later Machine release adds the wipe hardening (retried
   removal, failed-wipe reporting, a durable worker roll on a new wipe,
   directory sync before acknowledging).

### Deployments without those prerequisites

Only where a card is not adopted or signed out yet, or a Machine's wipe is
not `current`, with the current Controller first:

- Adopt each card into a named account (`accounts.adopt_legacy
  {card:"openai"}`, ...) and sign out Claude · Lynx at Service scope; reset
  (or stop) a Lynx Machine browser if one was enabled. Do this with every
  Machine online: a Machine offline during an adoption may hold a rotation of
  the moved credential that the old Controller's wipe on reconnect can delete
  before it is promoted. A Machine recovered later must not be reconnected
  expecting such a rotation to be promoted; sign its account in again
  instead.
- Connect an offline Machine until its wipes show `current`, or revoke a
  Machine that will not come back; otherwise the new Controller refuses to
  start.
- Then follow the production cutover above.
