# Offline-first synchronization

Status: design, 2026-09-18. Nothing in this document is implemented yet. It
is the app-level contract for how Cowboy Web opens, reads, composes, sends and
reconciles when the controller is slow, unreachable, restarting or when the
device is offline. It is UX-first: implementation cost decides phase order, not
whether a behaviour is in scope.

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

**Status pill.** A compact pill centred under the system clearance inside
the top bar, paint-only, no transform, no shadow, present only when the phase
is not `live` and after a 1.5 s debounce so a foreground blip never flashes.
Tapping it opens an `ObsidianSheet` with the phase, last synced time, queued
and held counts grouped by session, and the actions Retry now, Reload, and
Update when ready. The full-width banner is retained for decisions only:
sign-in required, dataset changed, update ready. `MobileConnectionBanner`
keeps its explicit Update tap.

**Transcript.** A cached tail shows a one-line caption above the newest row:
"Cached · updated 3 min ago". It fades when the session becomes `live`. A
join gap that the fill could not close renders an inline divider at the gap
position: "Some messages not loaded · Load"; it never leaves a blank hole.

**Rows.** Existing chrome stays: `saving`, `sending`, `syncing`, `failed`.
Queued rows add the authored time ("Written 14:02 · sends when online").
`held` rows offer Retry, Edit (returns the text and attachments to the
composer) and Discard. `rejected` rows show the server reason and the
actions that fit it (see the conflict catalog).

**Sessions drawer.** A row with pending obligations shows a small badge with
the count. The drawer header shows "Last synced 3 min ago" while not `live`.
Sessions without a cached tail show a subdued "Not cached" glyph offline so
the user knows before tapping.

**Reconnect.** The pill turns green "Synced" for 2 s, then disappears. If
any row is `held` or `rejected`, the pill stays as "2 messages need
attention" until they are resolved; tapping it jumps to the first one.

### Desktop

**Status line segment.** Always present, next to connection and worker state:
`● Live`, `◌ Reconnecting · 4 s`, `⏳ Waiting for a seat (2nd)`,
`⊘ Offline · 2 queued`, `⚠ 1 needs attention`. Tooltip carries last synced
time and the retry countdown. Click opens the command palette filtered to
`Reconnect now`, `Retry held sends`, `Reload app`, `Update now`.

**Banners.** Only sign-in required, dataset changed, and update ready. The
update banner no longer counts down while the user is composing.

**Update policy (both products).** An update is never applied while any of
these hold: composer text or attachments present, IME composition active, a
row in `saving`/`sending`, a running turn in the active session, or the tab
has been visible for less than 60 s. When all clear, Desktop applies after a
visible 3 s countdown that a keypress cancels; Mobile still waits for the
tap. An update is always applied on the next launch. The service worker
keeps its two-generation cache so the open window survives the swap.

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
