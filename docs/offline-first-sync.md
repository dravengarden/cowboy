# Offline-first synchronization

Status: design 2026-09-18; the boot path became network-independent on
2026-09-19 (see [Boot on a weak connection](#boot-on-a-weak-connection-service-worker-cowboy-v1738))
and now opens on the user's real last screen (see
[Boot presentation](#boot-presentation-service-worker-cowboy-v1740)) and
revalidates a session instead of re-downloading it (see
[Reopening a session](#reopening-a-session-controller-and-web-service-worker-cowboy-v1743));
Phase 1 implemented on Web the same day; the
Phase 2 submission ledger, addressed results and idempotent sync, plus the
Phase 3 prefetch and sessions-list affordances, landed 2026-09-19 (see
[Implementation status](#implementation-status)). This is
the app-level contract for how Cowboy Web opens, reads, composes, sends and
reconciles when the controller is slow, unreachable, restarting or when the
device is offline.
It is UX-first: implementation cost decides phase order, not whether a
behaviour is in scope.

Related contracts that stay authoritative: the Hub as sole arbiter and `seq`
owner ([core hub](architecture/02-core-hub.md)), the optimistic state-sync
engine and its durable outbox ([frontend](architecture/09-frontend.md),
[atomic outboxes](atomic-idb-outboxes.md), [product datasets](product-sync-datasets.md)),
the network action feedback grammar ([frontend](architecture/09-frontend.md#network-action-feedback)),
the Mobile compositor rules ([mobile spatial presentation](mobile-spatial-presentation.md))
and the Desktop status line ([desktop redesign](desktop-efficiency-redesign.md)).

## Why

Every open of the installed PWA today pays a full loading chain before the
composer exists, even when the device has already seen all of this data:

| Gate | Waits on | Offline result |
|---|---|---|
| `ProductAuthGate` | `GET /api/auth/status` (`no-store`, bypasses the SW) | retry page forever; the app never mounts |
| `MachineSetupGate` | `machinesLoaded`, set only by the WebSocket `machines` message | spinner forever |
| `App` active session | `sessions` WebSocket message; `sessionsLoaded` latches; 7 s stall auto-reload (`web/src/App.tsx`, `cowboy:stall-reloaded`) | `LoadingState`, then a reload that loads the same thing |
| Transcript | `GET /api/sessions/{id}/bootstrap` (`no-store`) | skeleton; composer is a sibling and already usable |

Once mounted the app is already tolerant: typing never reads `connected`,
an offline Send goes through the same durable IndexedDB outbox as an online
Send and is resent on reconnect with the same `cmid`. The gap is not the send
path. The gap is that nothing server-derived is persisted: the sessions list,
the machine registry, the last 200 events of any session, per-session config
options, and the delivery status of a failed row all live in memory only. The
device therefore cannot open without the server, cannot show what it already
knew, and cannot tell the user honestly what state it is in.

## Goals

1. Opening the app paints the last known state at once. A full-screen loading
   surface appears only when the device holds no data for this dataset at all.
2. The composer is mounted and typeable as soon as an active session can be
   resolved locally. Sending never blocks on connectivity.
3. Loading is fine-grained and prioritized: the active session first, then
   sessions the user is likely to switch to, then everything else on demand.
   Each region carries its own freshness instead of one global spinner.
4. Reconnection converges without duplicates, silent drops or unexplained
   error toasts. Every conflict class has a defined outcome and a defined
   presentation.
5. Connectivity and sync are one visible, calm status. Banners are reserved
   for states that need a decision from the user.

## Non-goals

- Client-side conflict resolution. The Hub remains the arbiter; the client
  mirrors, replays and presents outcomes.
- Collaborative text merging in the composer. Drafts are device-local.
- Creating a Machine-placed session offline. Session creation needs a
  worktree fetch on a Machine and stays online-only in this design (see
  [Open decisions](#open-decisions)).
- Cross-tab shared presentation state. The atomic outbox already keeps two
  tabs of one device from losing each other's mutations; row chrome remains
  per tab.
- Replacing the service worker strategy for immutable history pages. It stays
  as is and is extended, not rewritten.

## Current facts the design builds on

This section records the baseline the design started from;
[Implementation status](#implementation-status) lists what has changed since.

Client (`web/src`):

- The WebSocket handshake is lazy: `client_capacity`, `sessions`, `settings`,
  `sync_patch` with `resync: true` for `title`, `order`, `folders` and every
  `mobile-review:<sid>`, then `machines`, then `bootstrap_complete`. No
  transcript, queue or config options ride the socket bootstrap.
- The focused session is hydrated over HTTP by `hydrateSession()`:
  `snapshot` (tail of at most `SNAPSHOT_TAIL = 200` events and
  `SNAPSHOT_MAX_BYTES = 128 KiB`), `config_options`, and a `queue:<sid>`
  resync. It accepts no forward cursor; overlap is deduplicated by `seq`, and
  a non-overlapping tail is backfilled through `/api/history` for at most
  `SNAPSHOT_GAP_FILL_PAGES = 32` pages.
- Persisted today, dataset-fenced in IndexedDB `shared-utils-sync`/`clients`:
  `service:title`, `service:order`, `service:folders`,
  `session:<sid>:queue` (queue, drafts, in-flight optimistic rows) and
  `session:<sid>:mobile-review`. Their `base` is documented as an offline
  paint cache; their `pending` is the outbox.
- Persisted in `localStorage`: the active session id
  (`cowboy:active-session`), composer drafts per session
  (`cowboy:composer-draft:<sid>`, images inline, quota fallback strips
  images silently), pending edits of queue rows, preferences.
- Not persisted: `sessions`, `machines`, `timelines`, `hydrated`,
  `pagination`, `configOptions`, and `qStatus` (so a row the user left as
  `failed` reverts to `pending` after a reload and is resent automatically,
  the opposite of the in-tab rule).
- Connection state is spread across `State.connected`, the app-shell
  banner store (`down` / `reconnected` / `update`), `activeCapacity` and
  non-reactive module flags. Intermediate states are not representable.
- The transcript keeps an MRU of `TRANSCRIPT_SESSION_CACHE_LIMIT = 6`
  sessions; live events for sessions outside it are dropped.

Server (`src`):

- `Submit` is deduplicated by `cmid` only while the row is still in the queue
  or drafts. Once dispatched, a replayed `Submit` runs the turn again. `cmid`
  is never persisted on events (`src/store.rs`, "live-only reconcile tag").
- `Sync` mutation ids are deduplicated by an in-memory `SyncArbiter.seen`
  set that resets on controller restart and is never pruned. A duplicate is
  acknowledged with `Ok(())` and no patch.
- `sync_patch.version` and `machines.revision` reset on restart, which is why
  every reconnect is a forced full resync.
- Errors are broadcast to every client and carry no `cmid`, mutation id or
  code. Most plain commands silently no-op on an unknown session id.
- `reset_session` (Clear conversation) is not idempotent.
- A duplicate `permission` response still appends a new `permission_resolved`
  envelope.
- Browser clients hold fenced active-client leases; `client_capacity` can
  park a reconnecting client as `waiting` or evict it as `lost`.

## Principles

1. **Local data is a replica, never authority.** Everything painted from the
   device is labelled by freshness and replaced by the arbiter's value on
   contact. The existing `base` versus `pending` split extends to the new
   replica: replica values are paint caches, outbox entries are obligations.
2. **Typing and reading are never blocked by network state.** Only actions
   whose effect cannot be made idempotent or bound to a target are refused
   offline, and they say so in place.
3. **Honest, quiet status.** The app shows one status, only when it is not
   `live`, only after a short debounce, and escalates to a banner only when
   the user must decide something.
4. **Server-attributed outcomes.** Every replayed obligation ends in a
   confirmation, an addressed rejection, or a visible held state. Silent
   drops are a server bug to fix, not a client state to paper over.
5. **Compositor discipline on Mobile.** Every new chrome element on the
   swipe path is paint-only. No transform, no shadow, no `setState` on
   finger-down.

## Architecture

### 1. Local replica

A new client module owns server-derived value caches under the existing
product dataset owner. The database, store, schema version and outbox
contract are unchanged; the closed scope list in
`web/src/productSyncDatabase.ts` grows by these value-cache keys, all
prefixed `cowboy:dataset:<dataset-id>:`:

| Key | Value | Written | Read |
|---|---|---|---|
| `service:sessions` | `{ receivedAt, sessions: SessionMeta[] }` raw list before overlays | every `sessions` message, debounced 250 ms, flushed on `pagehide` | boot, before the socket answers |
| `service:machines` | `{ receivedAt, revision, machines }` | every `machines` message | boot; latches `machinesLoaded` |
| `service:boot` | `{ lastLiveAt, lastServerVersion, principalUserId }` | on `bootstrap_complete`, on socket close | boot decision and "last synced" copy |
| `service:workspaces` | last `GET /api/workspaces` answer | when the picker fetches it | New Session picker while offline (read-only listing) |
| `session:<sid>:tail` | `{ receivedAt, epoch, firstSeq, lastSeq, reachedStart, events, configOptions }` bounded to the server tail limits | on `snapshot`; during live events at most every 2 s per session and immediately on `turn_end`, `permission_request`, `lifecycle`; flushed on `pagehide` | when the session becomes active or is scheduled for prefetch |
| `session:<sid>:delivery` | `{ [cmid]: { status: "held", attempts, lastAttemptAt, lastError? } }` | when a row times out, is retried, or is retired | boot, before resend |

Rules:

- The replica uses the `persistence<S>` value-cache facade, never `outbox`.
  Writes are last-writer-wins by `lastSeq` (tails) or `receivedAt`
  (lists). A tab holding an older tail must not overwrite a newer one.
- Tails are kept for the active session plus the transcript MRU. Eviction
  follows `transcriptSessionCache`; a deleted session's keys are removed when
  the `sessions` broadcast no longer lists it.
- The replica is cleared on explicit sign-out and on a dataset change, by the
  same shutdown barrier that seals the outboxes. It is never migrated across
  datasets.
- A tail stores at most what the server tail would return. Older history
  continues to come from the immutable `/api/history` pages the service
  worker already caches.

### 2. Boot without gates

Boot order becomes: paint from replica, then converge. The four gates are
rewritten as freshness sources, not blockers.

| Gate today | New behaviour |
|---|---|
| `ProductAuthGate` | The auth layer keeps its own small cache of the last successful status (`me`, `user_id`, deadlines, `capturedAt`) in `localStorage`. A network failure with a cached principal whose earliest deadline has not passed mounts the apps in `offline (cached identity)` and keeps probing with the existing backoff. `401`, `403`, `200` with a different `me`, or a passed deadline keep today's behaviour. `src/auth/*` still never imports the store. |
| `MachineSetupGate` | `machinesLoaded` latches from `service:machines`. With no machines cached and no sessions cached the gate keeps its splash; if the device is offline in that state it shows the honest empty screen described below. |
| Active session | `resolveActiveSession` runs against replica sessions plus `cowboy:active-session`. The stall auto-reload is removed whenever a replica exists; it stays only for the nothing-cached case. |
| Transcript | Renders the tail from `session:<sid>:tail` immediately with a cached caption. The skeleton appears only when no tail is cached. The bootstrap fetch then merges by `seq` and fills any join gap. |
| Composer | Mounts as soon as the active session resolves. No change to enablement rules. |

The one unavoidable blocking screen is "nothing cached for this dataset":
first sign-in on this device, or after sign-out. Its offline copy is
"Cowboy is offline and this device has nothing cached yet" with Retry; it
never spins silently.

Boot timelines after this change:

- Warm, online: module load, replica paint (sessions, active tail, queue,
  drafts, machines), socket opens in parallel, `bootstrap_complete` flips the
  status to `live`, the active bootstrap merges the tail. No spinner at any
  point; the only visible motion is the cached caption fading.
- Warm, offline: identical paint. Status pill says `Offline`. Typing, sending
  to the queue, switching between cached sessions, reading cached history
  pages all work. Non-cached sessions show a read-only empty state with the
  composer still usable.
- Cold, online: unchanged from today except that the second open is warm.
- Cold, offline: the honest empty screen.

### 3. Hydration scheduler

One scheduler owns every non-bootstrap fetch so priorities are explicit and
a session switch can preempt background work.

| Priority | Work | Trigger | Concurrency |
|---|---|---|---|
| P0 | socket handshake and its bootstrap | `connect()` | 1 |
| P1 | active session bootstrap with forward cursor, join-gap fill | session open, `bootstrap_complete`, foreground | 1, abortable on switch |
| P2 | tails for sessions with `status: busy` and the transcript MRU | after P1 settles; on `sessions` changes | 2 |
| P3 | Desktop hover or Mobile long-press prefetch of a sidebar row | pointer intent, 150 ms | 1 |
| P4 | older history pages | scroll (existing) | 1 per session (existing) |

Rules: the same session is never fetched twice concurrently; P2 and P3 are
cancelled when P1 starts; background fetches pause while the device is
offline and while the tab is hidden; nothing in the scheduler gates a send.
The existing send-latency fixture remains the guard that replica hydration
adds no time to the send path.

Freshness is tracked per session as
`{ source: "replica" | "live", lastSeq, syncedAt, gap?: { afterSeq, beforeSeq } }`
and drives the transcript caption and the gap divider.

### 4. One sync status

A new `syncStatus` store derives one value from the socket, capacity,
`navigator.onLine`, heartbeat age, the auth pause, the dataset fence and the
outboxes:

```ts
type SyncPhase =
  | "live"            // bootstrap complete, heartbeat fresh
  | "connecting"      // socket opening or backoff wait; retryAt
  | "waiting"         // client_capacity waiting/channel_limit; position
  | "degraded"        // socket open, heartbeat older than 30 s or repeated fetch failures
  | "offline"         // navigator offline, or attempts >= 2 without success
  | "auth_required"   // 4001 or 401; local data preserved
  | "fenced";         // dataset changed; reload required

interface SyncStatus {
  phase: SyncPhase;
  since: number;                 // when this phase began
  retryAt?: number;
  position?: number;             // capacity queue
  lastLiveAt?: number;           // from service:boot on cold start
  outbox: { pending: number; held: number; sessions: string[] };
  updateReady: boolean;          // orthogonal
}
```

`State.connected`, the banner store and `activeCapacity` remain as inputs;
all presentation reads `useSyncStatus()`. `reconnectNow()` is the single
retry action.

### 5. Outbox classes and delivery metadata

Every mutation the UI can issue is assigned one of four classes. The class
decides what the button does offline and what replay does on reconnect.

| Class | Semantics | Members |
|---|---|---|
| A. Queue and replay | idempotent by id or by absolute value; safe after any reconnect | `submit`, `add_draft`, `schedule_draft`, `edit_draft`, `remove_draft`, `activate_draft`, `move_draft`, queue edit/remove/reorder, `title`, `order`, `folders`, `mobile-review`, `set_config_option`, `set_paused`, `reorder_sessions`, `delete_session` (soft delete, undoable) |
| B. Queue, bound to a target | replayed only if the target still exists in the same state; otherwise retired with an addressed outcome | `cancel` (bound to the running turn), `permission` (bound to `request_id`), `edit_queued` and `remove_queued` (bound to the row), `retry_turn` |
| C. Live only | effect is not idempotent or needs a Machine round trip now | `reset_session` (Clear), Compact, `POST /api/sessions/{id}/reload`, Provider auth and uninstall, `new_session`, usage refresh |
| D. Local only | never leaves the device | composer drafts, pending edits, viewport, folder collapse, preferences |

Class C controls stay enabled. Pressing one offline resolves through the
existing `NetworkActionFeedback` failure path in place with "Needs a
connection"; no modal, no disabled-without-explanation.

Delivery metadata per queued row is persisted in `session:<sid>:delivery`
so a row's user-facing state survives reload:

| State | Meaning | Persisted | Replayed on reconnect |
|---|---|---|---|
| `saving` | durability barrier running | no | n/a |
| `queued` | committed locally, socket not live | via outbox | yes |
| `sending` | frame left the tab, awaiting confirmation | via outbox | yes |
| `held` | timed out, user chose not to retry yet | yes | no, until Retry |
| `rejected` | addressed server outcome; needs a decision | yes | no |

Automatic retry: `sending` rows that time out while the phase is `live`
retry with backoff 2 s, 5 s, 15 s, then become `held`. Rows that time out
because the phase is not `live` simply return to `queued`; a timeout is not
evidence of failure when the socket is down.

## User experience

### Mobile

**Status pill.** The one connection indicator on the Agent surface: a
tinted pill centred under the system clearance in the same language as the
transcript's activity pills, paint-only, no transform, no shadow, present
only when the phase is not `live` and after a 1.5 s debounce so a foreground
blip never flashes. The transcript tail paints nothing about transport (the
former "Reconnecting…" tail row is gone) and the Composer never reads it.
Tapping the pill opens an `ObsidianSheet` with the phase, last synced time,
queued count, the sessions that hold unsent rows with an Open action, and
the actions Retry now, Reload, and Update when ready. The full-width banner
is retained for decisions only: sign-in required, dataset changed, update
ready. `MobileConnectionBanner` narrates the update it is about to apply;
it carries no action.

**Transcript.** A cached tail shows a one-line caption above the newest row:
"Cached · updated 3 min ago". It fades when the session becomes `live`. A
join gap that the fill could not close renders an inline divider at the gap
position: "Some messages not loaded · Load"; it never leaves a blank hole.

**Rows.** Existing chrome stays: `saving`, `sending`, `syncing`, `failed`.
Queued rows add the authored time ("Written 14:02 · sends when online").
`held` rows offer Retry, Edit (returns the text and attachments to the
composer) and Discard. `rejected` rows show the server reason and the
actions that fit it (see the conflict catalog).

**Sessions drawer.** Connection state is app-level, not per session, so the
floating pill hides while the drawer is open and the drawer carries its own
inline line at the top of the list: "Reconnecting… · list from 3 min ago",
tappable into the same sheet. Row status dots keep meaning agent status. A
row with pending or held rows shows a small badge with the count. Sessions
without a cached tail show a subdued "Not cached" glyph while the Hub is not
live so the user knows before tapping.

**Reconnect.** The pill turns green "Synced" for 2 s, then disappears.

**Attention.** Held or rejected rows outside the opened session raise the
pill as "2 messages need attention"; rows of the opened session are not
counted because their own chrome already offers Retry, Return and Discard.
Tapping opens the sheet, which lists the sessions and opens one on tap.
"Hide reminder" acknowledges exactly the rows held now, so a resolved row
never re-raises the pill for the rest, while a new failure does. The
acknowledgement persists across reloads.

### Desktop

**Status line segment.** Always present, next to connection and worker state:
`● Live`, `◌ Reconnecting · 4 s`, `⏳ Waiting for a seat (2nd)`,
`⊘ Offline · 2 queued`, `⚠ 1 needs attention`. Tooltip carries last synced
time and the retry countdown. Click opens the command palette filtered to
`Reconnect now`, `Retry held sends`, `Reload app`, `Update now`.

**Banners.** Only sign-in required, dataset changed, and update ready. The
update banner no longer counts down while the user is composing.

**Update policy (both products).** A deployed build is applied by the client
itself; no surface offers an update control to press. An update is never
applied while any of these hold: composer text or attachments present, IME
composition active, a row in `saving`/`sending`, a running turn in the active
session, or a focused editor. When all clear, both products apply after a
visible 3 s countdown; a busy moment rewinds it to its start rather than
freezing it, and the check re-arms every second so the reload lands on the
first real pause. Mobile requires 60 s of uninterrupted foreground on top of
that, counted again from every resume, because an installed PWA restores a
frozen page and an immediate reload reads as a crash. A download that does not
finish keeps this build running and starts another countdown a minute later.
An update is always applied on the next launch. The service worker keeps its
two-generation cache so the open window survives the swap.

### Copy

| Phase | Pill / segment | Detail sheet or tooltip |
|---|---|---|
| `connecting` | Reconnecting… | Retrying in 4 s. Your messages will send automatically. |
| `waiting` | Waiting for a seat (2nd) | Another client holds this account's active seat. |
| `degraded` | Connection unstable | Last heard from Cowboy 45 s ago. |
| `offline` | Offline · 2 queued | Last synced 3 min ago. Everything you write is saved on this device. |
| `auth_required` | Sign in to sync | 3 queued messages will send after you sign in. |
| `fenced` | Reload required | This device was signed in as a different account. |
| reconnect flash | Synced | |
| attention | 1 message needs attention | |

## Conflict catalog

The arbiter serializes; the client replays pending obligations on the
arbiter's value. Most concurrency therefore converges without a decision.
This catalog lists every case where replay is wrong, ambiguous or invisible,
and fixes each with a server outcome and a presentation.

| # | Case | Outcome | Presentation |
|---|---|---|---|
| 1 | `submit` queued offline; the same `cmid` was already dispatched (device reloaded, or controller restarted) | server dedupes against a durable per-session submission ledger and confirms the id | row confirms silently |
| 2 | `submit` queued offline; session deleted, provider uninstalled, or view-only meanwhile | addressed `not_found` / `rejected` | `rejected` row: "This session is gone" with Move to another session (existing `move_draft`) or Discard |
| 3 | `submit` queued offline to a session that is now `busy` | Hub places it in the queue | row shows "Queued · sends after the current turn" |
| 4 | Several devices queued prompts to one session while some were offline | arrival order at the Hub; no reordering by authored time | rows show the authored time so the user can see why the order differs |
| 5 | `edit_queued` or `remove_queued` for a row that was already dispatched | addressed `not_found` | edit: text parks as a new draft via the existing orphan claim; remove: quiet notice "Already sent" |
| 6 | `cancel` pressed offline; on reconnect the targeted turn already ended or a new turn runs | server ignores a cancel whose `turn_seq` no longer matches | notice "The turn already finished" or "A newer turn is running" |
| 7 | `permission` answered offline; already answered elsewhere | addressed `already_resolved`, no duplicate envelope | card shows "Answered on another device" |
| 8 | Rename or reorder on two devices | last writer at the Hub wins; both are absolute sets | none; the list converges. A base-aware rename with a Keep / Use mine prompt is a Phase 4 option |
| 9 | Folder create or move replayed across a controller restart | folder mutations are idempotent by id server-side; a same-id same-name create is a no-op confirm | none |
| 10 | `delete_session` offline; the session received new messages meanwhile | soft delete applies; retention keeps it 3 days | row deletes with an Undo notice while offline; on reconnect nothing further |
| 11 | Session cleared (`reset_session`) elsewhere while a tail is cached here | snapshot carries `transcript_epoch`; a mismatch discards the cached tail instead of merging | transcript replaces cleanly; caption "Conversation was cleared" once |
| 12 | Long offline: more than 200 events plus more than 32 fill pages | forward cursor returns `gap` bounds | inline gap divider with Load |
| 13 | Auth expired while offline | reconnect returns 401 or 4001; local data and outboxes preserved | `auth_required` banner; after signing in as the same user the same dataset resumes and drains |
| 14 | Different account signs in on this device | dataset fence, existing behaviour | `fenced`; retained records stay recoverable through Settings → About → Storage |
| 15 | Two tabs, one device | atomic outbox deltas (existing); replica writes guarded by `lastSeq` | none |
| 16 | Client parked by capacity on reconnect | `waiting` with position; obligations stay queued | pill "Waiting for a seat (2nd)" |
| 17 | Replay partially fails with an unaddressed broadcast error | addressed results replace broadcast errors for client-originated commands | only the originating row shows the reason; other clients are not toasted |
| 18 | Held row after reload | `held` persisted in `session:<sid>:delivery` | stays held; never auto-resent |
| 19 | Composer draft with images hits `localStorage` quota | drafts move to the dataset-scoped IndexedDB with text mirrored to `localStorage` for synchronous seed | no silent image loss; a pending paste placeholder is still lost on crash and stays documented |
| 20 | Update becomes ready mid-composition | update policy above | no reload; banner or pill only |

## Server changes

Required for correctness of replay (Phase 2); the client design assumes them
and degrades to today's behaviour without them.

1. **Durable submission ledger.** Persist `cmid` on the user-message
   envelope, or a `submissions(session_id, cmid, seq)` table, bounded per
   session. `Submit`, `force_submit`, `requeue_prompt`, `add_draft` and
   `schedule_draft` dedupe against the queue, drafts and this ledger. The
   focused-session bootstrap returns `confirmed_cmids` for its tail so a
   reloaded client can retire outbox entries.
2. **Addressed command results.** A new `Outbound::CommandResult { ref, outcome, detail }`
   where `ref` is `cmid`, mutation `id` or `request_id` and `outcome` is
   `ok | not_found | stale | already_resolved | rejected`. Unknown-session
   no-ops become `not_found`. Broadcast `error` stays for genuinely global
   failures only.
3. **Idempotent sync by id, with confirmation.** Folder mutators accept a
   same-id replay as a no-op; a duplicate `Sync` re-emits a `sync_patch`
   carrying the id in `confirmed`. `SyncArbiter.seen` is pruned per session
   removal and bounded.
4. **Forward cursor.** `GET /api/sessions/{id}/bootstrap?after_seq=N`
   returns events after `N` when they are within the retained window,
   otherwise the tail plus `gap: { after_seq, before_seq }`. The response
   and `SessionMeta` carry `transcript_epoch`, incremented by
   `reset_session`.
5. **Target binding.** `cancel { session_id, turn_seq }` and
   `permission { request_id }` return `stale` / `already_resolved` without
   emitting a new envelope.
6. **Sessions revision.** `Outbound::Sessions { revision, sessions }` and
   `SessionMeta.updated_at_ms`, for freshness copy and cheap diffing. The
   full list stays; it is small.

`reset_session` remains live-only and is never placed in an outbox.

## Client changes

| Area | Files | Change |
|---|---|---|
| Replica | new `web/src/replica.ts`, `web/src/productSyncDatabase.ts` | value caches, closed scopes, eviction, sign-out clearing |
| Boot | `web/src/auth/ProductAuthGate.tsx`, `web/src/auth/authStatus.ts`, `web/src/setup/MachineSetupGate.tsx`, `web/src/App.tsx` | cached-identity mount, replica-latched gates, stall reload only without replica, skeleton only without tail |
| Store | `web/src/store.ts` | replica hydration in `connect()`, reducer writes, `after_seq` hydration, delivery metadata, addressed results, held-aware resend |
| Status | new `web/src/syncStatus.ts`; `components/app-shell/connection-banner.tsx` | derived phase, retry action, banner limited to decisions, idle-gated update |
| Scheduler | new `web/src/hydrationScheduler.ts` | priorities, preemption, offline pause |
| Mobile | `web/src/mobile/shell/*`, new `MobileSyncPill.tsx`, `MobileSyncSheet.tsx`, `MobileConnectionBanner.tsx` | pill, sheet, caption, gap divider, drawer badges |
| Desktop | `web/src/desktop/DesktopStatusLine.tsx`, `web/src/desktop/commands/*` | segment, palette commands, tooltip |
| Rows | `web/src/Composer.tsx`, `web/src/Transcript.tsx`, `web/src/localFirstDelivery.ts`, `web/src/durableDelivery.ts` | `held` / `rejected` states, Edit action, authored time |
| Drafts | `web/src/draftStore.ts` | IndexedDB attachments with `localStorage` text mirror |
| Protocol | `web/src/protocol.ts` | `CommandResult`, `after_seq`, `transcript_epoch`, `revision` |

## Phasing

Ordered by user-visible value; each phase ships independently.

1. **Open instantly, type immediately** (client only). Replica for
   sessions, machines, active tail and config; gate rewrite; unified status
   with the Mobile pill and Desktop segment; idle-gated updates; persisted
   `held`; cached caption; drawer badges. Replay risk is unchanged from
   today.
2. **Trustworthy replay** (server and client). Submission ledger, addressed
   results, idempotent sync by id, forward cursor with epoch, target
   binding; client retires or flags outbox rows from addressed outcomes;
   conflict rows 2, 5, 6, 7, 11, 12.
3. **Fine-grained prefetch and richer offline reading.** Scheduler P2 and
   P3, drafts in IndexedDB, gap divider, workspaces cache, "Not cached"
   glyphs.
4. **Optional.** Base-aware rename prompt, offline session drafts that are
   created on reconnect, cross-tab status via `BroadcastChannel`.

## Acceptance

Deterministic Deno tests cover status derivation, scheduler ordering and
preemption, replica write guards and eviction, delivery metadata across a
simulated reload, and the conflict table outcomes as reducer cases. The
pinned-Firefox conformance runner adds one suite: load online, populate the
replica, go offline, reload, assert first paint from the replica with no
splash, type, send to the queue, reconnect, assert exactly one server echo
per `cmid`. The send-latency fixture must show no regression.

Manual matrix on the physical iPhone PWA and a Desktop window:

| # | Scenario | Expected |
|---|---|---|
| 1 | Warm open, online | no spinner; live within the existing reconnect budget; caption fades |
| 2 | Warm open, airplane mode | full UI from the replica; pill `Offline`; type and queue; switch between cached sessions |
| 3 | Cold open, airplane mode | honest empty screen with Retry |
| 4 | Queue three messages offline, reconnect | all three echo once, in order, pill flashes `Synced` |
| 5 | Same as 4 but the controller restarts in between | still exactly once (Phase 2) |
| 6 | Session deleted from Desktop while the phone holds a queued prompt | `rejected` row with Move / Discard (Phase 2) |
| 7 | Update ready while typing on Desktop | no reload; applies after idle or on next launch |
| 8 | Sessions drawer swipe with the pill visible | 1:1 tracking, no dropped frames |
| 9 | Auth expiry while offline, then reconnect | `Sign in to sync`; after login the outbox drains |
| 10 | Two tabs, one device, both queue offline | both rows survive and both drain once |

## Implementation status

### Reopening a session (Controller and Web, service worker `cowboy-v1743`)

Opening a session paints from the cached tail with no network, and then
fetches `GET /api/sessions/<id>/bootstrap` to reconcile. That fetch sent no
cursor and the response carried no validator, so every open re-downloaded a
tail already on the device — up to `SNAPSHOT_MAX_BYTES` (128 KiB) of it,
competing with the socket on exactly the weak connection this design is for.

The response is now validated by a digest of its own body. A reader that still
holds the timeline a given response produced replays that `ETag` as
`If-None-Match` and gets `304` with no body.

A digest rather than a `since_seq` cursor, deliberately. Transcript rows are
coalesced in place under an existing seq (`EventReducer::reduce`): a streamed
message grows inside its first row, and a tool call is rewritten from
`pending` through `completed` under the seq it was created at. "Everything
after seq N" would therefore report nothing new while a tool call the reader
is watching has finished. `src/server/bootstrap_validator_tests.rs` pins
exactly that: the last seq does not move, the validator does. A forward cursor
remains possible later, but it needs a per-row update watermark first, which
is a durable schema change on the hot write path.

The client only revalidates a timeline that is actually mounted from that
exact response, because a `304` carries nothing to apply. The validator rides
in the cached tail (`ReplicaTail.etag`), so it survives a reload, and it is
dropped wherever the timeline it describes is dropped — eviction, a discarded
replica, an epoch bump. A `304` also promotes the transcript to `live`: the
server has just confirmed the reading is current, so the caption says so
instead of "cached".

The reconciling fetch now follows the local tail read instead of racing it,
because that record is where the validator lives. Nothing visible waits on
it — the transcript paints from the same read — and a 2 s ceiling keeps a
database that never answers from withholding the network.

Verified end to end against a fake Hub in Chrome: a cold device sends no
validator and gets `200`; reopening sends it and gets `304` with the
transcript intact; and after the server's tail changes, the same validator
correctly yields `200` and the new content appears.

### Boot presentation (service worker `cowboy-v1740`)

Making boot network-independent removed the wait. It did not remove the
*impression* of one: the app still opened on a centred spinner and then cut to
a finished screen. Cowboy is client-rendered and per-user, so it cannot be
pre-rendered at build time — but it can pre-render itself. Three layers, each
a strict improvement on the one below, and each optional:

| Layer | What it paints | When |
|---|---|---|
| 1. Boot shell | a static skeleton of Cowboy's own layout (rail, bottom-anchored feed, composer, nav) in `index.html`, coloured from `cowboy:boot-theme` | with the document |
| 2. Last screen | a sanitized static copy of the resting screen the user left, mounted in a closed shadow root above the app | as soon as Cache Storage answers |
| 3. Live app | React, replacing content in place | when the active session's content is on screen |

Layer 1 is the floor: whatever else fails, the first frame reads as Cowboy
rather than as a blank canvas or a spinner. `BootSkeleton.tsx` renders the
same markup, so `main.tsx`'s Suspense fallback, the auth gate's pre-probe view
and the setup gate's pre-list view are all one shape — nothing flashes between
the document's first frame and the app's first paint. `bootSnapshot.test.ts`
holds the two copies together.

Which chrome that skeleton wears — Desktop rail or Mobile bottom nav — is the
app's own last answer, replayed from `cowboy:boot-surface`. It cannot be
derived in CSS: `classifySurface` reads `navigator.maxTouchPoints` and the
native host and has no width term at all, so an iPad in landscape is touch and
a narrow desktop window is not. Before that key has ever been written the
shell approximates with `(pointer: fine) and (hover: hover)`, undone by
`(any-pointer: coarse)`, in that order.

Layer 2 is the SSG-like part. `bootSnapshot.ts` captures when the app is left
(`visibilitychange` to hidden, `pagehide`) and at idle moments in between, but
only from a *resting* screen: no sheet, dialog, drawer, keyboard, gesture,
placeholder or Review page, and not mid-turn for the periodic capture. It
clones `#root`, drops everything outside the viewport (the overlay cannot
scroll, and this is what keeps a long transcript small), pins image and media
boxes, records scroll offsets, strips scripts and event handlers, and
serialises only the CSS rules that can still match. A capture of a real mobile
session is about 122 KB: 25 KB markup, 84 KB CSS, 13 KB `@font-face`.

The overlay is a picture, never the app: closed shadow root (no shared ids,
selectors or focus), `inert`, `aria-hidden`, `pointer-events: none`. It is
shown only when the user, the viewport, the colour scheme, the session about
to open and a 7-day age check all agree, and it is sanitized again on mount.
The viewport check is an 8% tolerance per axis, not an exact match: the copy
is live DOM and reflows at the current size, so only the pinned boxes and the
restored scroll offsets carry the capture's geometry. Requiring an exact match
cost Desktop the feature entirely (any window resize) and lost it in a browser
tab whenever the URL bar came or went; 8% still rejects a rotation or a halved
window, and the feed is bottom-anchored under a clipping frame, so a slightly
short picture loses the top, which is the right end to lose. `signalBootReady()` cross-fades it out two frames after the live
app has painted the active session; a 4 s safety timeout guarantees a stuck
app can never hide behind a picture of itself. `clearBootSnapshot()` runs on
sign-out, a login answer and a changed account.

Measured with a warm cache and EVERY response delayed by 12 s, mobile
viewport (in-page observer, Chrome, fake Hub):

| Moment | Time |
|---|---|
| document starts executing | 4 ms |
| boot skeleton on screen | 11 ms |
| the user's last screen on screen | 18 ms |
| handed over to the live app | 903 ms |

Not implemented: capturing the Review page (its content is the workspace, read
over the network), restoring a snapshot for a different session than the one
being opened, and any snapshot at all where Cache Storage is absent (native
WKWebView) — those fall back to layer 1.

### Boot on a weak connection (service worker `cowboy-v1738`)

Phase 1 opened from the replica only when requests FAILED. A weak connection
is slow, not failed, so four steps still waited on the network and the
installed PWA showed a white page, then a long splash, on every open:

| Step | Before | Now |
|---|---|---|
| App shell | the service worker answered navigations network-first with no timeout, so the document itself waited | cache-first; the deployed shell is fetched behind it and promoted only once its boot assets (`cowboy-boot-assets`, emitted by `vite.config.ts` for both surfaces) are cached, so the launch after a deploy is just as instant. A new worker generation opens from the previous generation's shell until its own is ready. Explicit `cowboy-update` / `cowboy-recover` navigations stay network-first |
| Identity | `ProductAuthGate` awaited `/api/auth/status` and used the cached principal only after the probe failed | mounts at once from a still-valid cached principal (`bootAuthDecision`); the probe runs in the background, is bounded to 10 s, is never superseded by a newer probe, and still tears the app down on a login answer or another account |
| Dataset | every IndexedDB read awaited `GET /api/sync/dataset` (8 s timeout), so the replica could not paint | the adopted dataset is remembered per user (`localStorageDatasetCache`); `connection()` still verifies it before socket admission, fences the owner and forgets the record when the Service was replaced |
| Surface chunk | `Suspense fallback={null}` painted white while the lazy surface evaluated | the same splash as the static document |

Applying an update downloads the deployed shell and its boot assets through
the worker first (`cowboy.refresh-shell`) and reloads only when they are
cached; on a weak connection the running build stays and the banner says the
download did not finish, instead of clearing every cache and reloading into a
white page.

Measured with a warm cache and EVERY response delayed by 12 s (in-page
observer, Chrome, fake Hub; `web/src/serviceWorkerShell.test.ts` holds the
worker's contract):

| Build | Document | Session list | Transcript |
|---|---|---|---|
| before | 12.0 s (white until then) | never within 45 s | never |
| now | 7 ms | 0.4 s | 1.1 s |

The numbers now equal an undelayed open. The launch that installs this
worker is still served by the old one; every later launch is cache-first.

Phase 1 shipped as a Web-only release (service worker `cowboy-v1722`):

- `web/src/replica.ts` owns the paint caches through new closed
  `ProductCacheScope` keys (`service:sessions`, `service:machines`,
  `session:<sid>:tail`, `session:<sid>:delivery`) on the existing dataset
  owner; sign-out discards them inside the same shutdown barrier as the
  outboxes, while an expired sign-in or a changed dataset only seals them.
- `web/src/replicaTail.ts` bounds a stored tail to the daemon's snapshot
  limits and detects a restarted transcript epoch by seq overlap, since the
  server does not yet carry `transcript_epoch`.
- `store.ts` paints replica sessions and machines in `connect()`, restores a
  cached tail in `openSession()`, persists tails on checkpoints, tracks
  `sessionsSource` / `transcriptSources`, persists held deliveries, derives
  the sync status (`useSyncStatus`, `retrySyncNow`, `canApplyUpdateNow`).
- Gates: the auth gate mounts from `cowboy:auth-status-cache` on a `retry`
  probe and keeps polling; the setup gate latches from the replica and names
  an empty offline start; the stall auto-reload fires only with no replica.
- Presentation: `MobileSyncPill` with its detail sheet, the Desktop status
  line segment, `TranscriptCachedCaption`, the update banner restricted to
  the update decision and gated by `canApplyUpdateNow`.

Phase 2 shipped as a Controller and Web release (service worker
`cowboy-v1725`):

- Submission ledger: the Hub keeps a bounded window of client `cmid`s it
  handed to a worker, and the user echo persists its `cmid` beside the event
  (`src/persistence.rs`, read back by both stores). `submit` and
  `force_submit` dedupe against the queue, that window and the restored log,
  and answer a replay with a queue patch confirming the id; `add_draft`
  confirms a replayed draft the same way. `requeue_prompt` releases the id
  when a crashed turn hands the prompt back. Hub-synthesized ids
  (`cowboy-*`, `__*`) stay reusable and never take part in this dedupe.
- Addressed results: `Outbound::CommandResult { session_id, cmid, outcome,
  message }` with `not_found` for a submit, draft, scheduled draft, edit or
  removal whose session is gone, `rejected` for a session the principal may
  not mutate or a view-only system session, and `stale` for an
  `edit_queued`, `remove_queued`, `edit_draft` or `remove_draft` whose row
  already left the queue or drafts (those commands carry the outbox mutation
  id as `cmid`). The Web store holds a refused row with the reason as its
  caption; for `not_found` it parks the content in the opened session's
  drafts before retiring the orphan, and for `stale` it keeps an edit's text
  as a new draft, retires the mutation and says so once (conflict 5).
- Idempotent sync by id: a duplicate `Sync` delivery re-emits a `sync_patch`
  confirming the id; a folder `create` that already produced the identical
  folder for the same actor is confirmed after a restart; the per-session
  `queue:` and `mobile-review:` dedupe sets are dropped with the session.
- Cleared transcripts: a snapshot whose tail carries a `context_cleared`
  boundary drops every cached row before it without paging history, and a
  join gap the fill cannot close truncates the unjoined prefix instead of
  leaving a hole; ordinary scrollback pages it back on demand.

Phase 3 shipped in part with the same release (service worker
`cowboy-v1730`):

- Hydration scheduler P2: `web/src/hydrationScheduler.ts` ranks busy sessions
  first, then the transcript MRU newest first, skipping the opened and the
  already hydrated sessions and capped below the MRU limit. The store runs
  at most two background bootstraps after the opened session's bootstrap
  settles, on `bootstrap_complete`, on `sessions` changes and on foreground,
  and cancels them when a session is opened or the socket closes. A
  prefetched tail lands in the replica like any snapshot.
- Sessions list: a paint-only badge counts a session's unsent or held rows
  (`useSessionObligations`), a "not cached" glyph marks sessions without a
  cached tail while the Hub is not live (`useCachedTailSessions`), and the
  Mobile drawer shows its own tappable connection line once the presentation
  debounce passes while the floating pill hides.
- Drafts in IndexedDB (service worker `cowboy-v1732`, conflict 19): a
  composer draft's attachment bytes are written to the dataset-scoped
  `session:<sid>:draft` cache while localStorage keeps the text with its
  inline tokens and a flag; the bytes are read back once the product
  database knows its dataset and a mounted composer adopts them, or drops
  the tokens of images that are gone (`web/src/draftRestore.ts`). Without a
  database the old text-only quota fallback still applies; a pending paste
  placeholder is still lost on a crash.
- P3 hover prefetch: a mouse resting on a Desktop sessions row for 150 ms
  fetches that tail ahead of the P2 queue (`prefetchSessionTail`).
- Conflict 7 (Controller and Web release, service worker `cowboy-v1734`):
  the Hub appends no second `permission_resolved` row for a request already
  resolved in the session log, so an answer given on two devices at once
  resolves once; the first broadcast already clears the other device's card,
  so no addressed `already_resolved` result is needed. Creating a session
  while Cowboy is unreachable now says "Needs a connection" in place instead
  of the browser's fetch error, and Stop while unreachable says so too
  instead of dropping the tap (`cancelTurn`, class C; service worker
  `cowboy-v1735`).
- One indicator per surface (service worker `cowboy-v1731`): the transcript
  tail's "Reconnecting…" row is removed on both products; the Mobile pill is
  restyled to the transcript pills' tinted language; "needs attention"
  excludes the opened session, lists sessions with an Open action, and can
  be hidden until a new row is held (`useHeldDeliveries`, `attentionCount`).

Not yet implemented: the forward bootstrap cursor with `transcript_epoch`
(a plain `after_seq` cursor is unsafe while canonical rows such as tool calls
and streaming messages are updated in place under their first seq; it needs
a per-row update watermark first), a turn-bound `cancel` outcome (`stale`),
the sessions `revision`, the Mobile long-press prefetch, and the inline gap
divider (superseded by dropping an unjoinable prefix). The New Session
picker already lists workspaces from the replicated Machine registry, so a
separate `service:workspaces` cache is not needed.

## Open decisions

- **Cached identity offline.** Mounting the app from a cached principal while
  the server cannot be reached is a product decision. The proposed rule is:
  allowed until the earliest known session deadline, cleared on explicit
  sign-out, and superseded by the lock screen the moment the server says so.
  Cached history pages already live in the service worker cache today, so
  this widens reading, not exposure class.
- **Offline session creation.** Deferred to Phase 4. Until then New Session
  stays a Class C action with an in-place "Needs a connection" outcome.
- **Rename conflicts.** Last writer wins is proposed as sufficient; a
  base-aware prompt is listed as optional because the cost lands in the
  arbiter and the frequency is low.
