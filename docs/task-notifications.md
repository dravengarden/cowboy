# Task notifications and per-account acknowledgement

Status: [background tasks](background-tasks.md) Phase 2, implemented; the
physical iPhone acceptance below is outstanding. Revised after the Astra Max
design review (19 findings, none rejected; see [Decisions](#decisions)).
Installed-app rules come from the Columbus PWA convention.

## Starting point

Observed on 2026-10-31:

- `run_web_push_notifications` watches the live Hub broadcast and spawns one
  best-effort send per subscription for `PermissionRequest` and session errors.
  Nothing is persisted: a Controller restart, a lagged broadcast or a failed
  send loses the notification, and a 429/5xx is only logged.
- Subscriptions live in `web-push-subscriptions.json` with per-device
  preferences and no account. Every subscriber receives every session's events.
- Production holds zero subscriptions, so no stored subscription needs
  migration; the shape still stays readable for one that appears before rollout.
- `completed` and `input` exist as preference categories but no producer sends
  them. Their stored default is `true`.
- Phase 1 failure acknowledgement is device-local (`cowboy:background-tasks:ack`).
- The service worker suppresses a push only when that same device shows the
  session.

## Goals and non-goals

Goals: a person learns about actionable input and unresolved failures while
away, with at-least-once delivery per installation that survives a Controller
restart; a failure acknowledged on one device stops being unseen on all of the
account's devices.

Non-goals: an inbox surface, completion digests, other transports, per-task
rules, declarative Web Push, and exactly-once presentation. The service worker
remains the renderer.

## Occurrences and intents

An **intent** is one durable row for one occurrence, committed before any send.

```text
notification_intents(
  dedupe_key PK, owner_user_id, session_id, category, destination, marker,
  due_at_ms, expires_at_ms,
  state, expanded_at_ms NULL, created_at_ms
)
```

`category` is `input`, `failure` or `completion`. `state` is `pending`,
`resolved` or `expired`. `destination` is a task key or empty, see
[Audience and content](#audience-and-content).

| Occurrence | Dedupe key | Category | Durable identity and producer |
|---|---|---|---|
| Permission request | `req:perm:<session>:<event seq>` | input | The envelope seq of `PermissionRequest`. Request ids restart with each agent process; the seq does not |
| Pending call approval | `req:approval:<parent>:<managed request id>` | input | The agent resubmits the same request id; registry `apr-…` ids are random per insertion and are not used |
| Subagent failed | `entity:<child>:<terminalSeq>` | failure | Non-terminal → terminal transition, see below |
| Subagent finished | `entity:<child>:<terminalSeq>` | completion | Same transition |
| Call failed | `exec:<call_id>:terminal` | failure | Ledger transition to `failed` |
| Call completed | `exec:<call_id>:terminal` | completion | Ledger transition to `completed` |
| Session error | `incident:<incident id>` | failure | The `runtime_incidents` id, written with the incident |

Rules:

- **Terminal occurrence.** The Hub stamps `terminalSeq` on the snapshot where a
  child moves from non-terminal to terminal and carries it on later terminal
  snapshots, so metadata updates neither repeat the notification nor change the
  acknowledgement marker. `SnapshotMark` gains the terminal flag and `terminalSeq`,
  and the restart warm restores them. A resumed child that ends again is a new
  occurrence. An equal-revision correction after termination is not a new one.
- **Only producers create intents.** Bootstrap, history, gap fills, lookups and
  backfill never do. A replayed snapshot is fenced by I4 before the transition
  is evaluated.
- **Execution failure is not a review result.** A completed review with findings
  is a completion. `cancelled`, `Stopped`, `Ended` and `Not reporting` never
  notify.
- **Write order.** A Call intent commits in the ledger transaction. An incident's
  is written by the same store write, right after the incident. An event's goes
  through the store writer directly after its event.
- **Durability boundary.** An event intent is exactly as durable as the event.
  The worker frame is acknowledged after projection, before the writer commits,
  so a Controller crash in that window loses the transcript row and its intent
  together. A graceful stop drains the writer. Moving that acknowledgement behind
  commit is a transcript-durability change with its own backpressure contract and
  is not part of this phase.
- **Child sessions.** A managed Call's child and a subagent notify on the parent;
  `owner_user_id` is the parent's. System sessions write no intent.
- **Resolution.** Nothing writes a resolution. Each send asks whether the input
  still waits: the session log for a permission request, the approval registry
  for an approval. The registry is memory-only, so an approval left pending by a
  restart resolves at its next send unless the agent resubmitted.
- **Expiry.** Delivery expiry only: 1 hour for input and failure, 15 minutes for
  completion. It never decides attention. The bar's unseen state stays derived
  from task state and acknowledgements.

## Delivery

A single Controller task owns delivery. Delivery is at-least-once per
installation and never promises exactly-once presentation: after an uncertain
send a dismissed notification can appear again, and tags do not span devices.
The duplicate bound is one retry after a crash or timeout between the push
service's accept and the local record.

```text
notification_deliveries(
  dedupe_key, endpoint_sha256, state, attempts, next_attempt_at_ms, PK(both)
)
```

- **Audience freeze.** The first processing of an intent inserts its delivery
  rows and sets `expanded_at_ms` in one transaction, including for an empty
  audience. A subscription enrolled later never receives older intents.
- Retry 408, 425, 429, 5xx and transport errors with exponential backoff from
  5 seconds, at most 5 attempts, honoring `Retry-After`; any other 4xx fails the
  delivery; 404 and 410 remove the subscription.
- The task resumes pending rows at start.
- `Urgency` is `high` for input and `normal` otherwise. `TTL` is the time left
  until expiry. `Topic` and `tag` are per occurrence for input, so an independent
  request is never replaced. Failure and completion use the aggregate described
  next.
- Delivery rows and finished intents are deleted after 7 days.
- Counters: intents by category; deliveries sent, retried, failed, deferred and
  expired. HTTP success is labelled push-service acceptance, never presentation.
- The rollback setting `notifications.delivery` (default on) stops the task, not
  only producers.

### Aggregation

Failure and completion intents are due 3 seconds after creation. All due intents
of one `(owner, session, category)` are sent as one notification, tag and Topic
`cowboy-<session>-<category>`, with the count of that send. A later burst
replaces the earlier banner; it states its own count and does not claim a total
unseen count. Dismissing and a later burst simply show a new banner. Throttling
never changes occurrence identity.

### Presence

A device showing a session renews a 30 second lease every 15 seconds with
`PUT /api/notifications/presence {session_id}` while visible and focused, and
deletes it on hide. Leases live in Controller memory per account and device.

**Input ignores presence**: it is delivered to every device, always.

A due failure or completion whose session has a live lease of that account is
**deferred** to the lease's expiry and re-evaluated then. While the person stays
it keeps deferring until acknowledged or expired; if they leave without opening
it, a stale lease costs at most 30 seconds and the push still goes out. A
Controller restart empties the table, which delivers.

The service worker **renders every push it receives**. WebKit requires a visible
notification per push and can revoke subscriptions otherwise, so the existing
per-device visibility check in `sw.js` is removed.

### Audience and content

- Subscriptions move to a database table:
  `web_push_subscriptions(endpoint PK, owner_user_id, enrolled_session_id NULL,
  p256dh, auth, preferences, updated_at_ms)`. `web-push-subscriptions.json` is no
  longer read or written. A rollback to an older Controller therefore reads the
  file it left, which holds no new enrollment, and cannot broadcast across
  accounts.
- `PUT` takes over the endpoint for the authenticated account, so the same
  installation under a new account stops serving the old one.
- Explicit logout removes the subscriptions enrolled by that cookie session and
  the Web unsubscribes locally. Pushes already accepted by a push service cannot
  be recalled; they carry only display-safe text.
- Delivery rechecks that the owner can still see the session, that it is not
  muted, and that the intent is `pending` with no matching acknowledgement.
- Intents hold no text. The session title is read at send time and included
  only when `showSessionNames` is on, truncated and never interpreted. Otherwise
  the copy is generic.
- **Destinations.** `destination` is `subagent:<child>` or `call:<id>` for a
  single task failure or completion; empty for requests, incidents and any
  aggregate of more than one. The link is `/?session=<id>` plus `#task=<key>` when
  present. A request opens the session, whose approval surface is already shown.
  A task key that is no longer listed opens the session only. A tap never
  approves, stops or acknowledges.

### Payload compatibility

The push stays `version: 1` with the existing `category` values: `input` or
`permission` for input, `error` for failure, `completed` for completion. New
fields (`tag`, `task`) are optional, and an installed older worker ignores them
and still renders. No capability negotiation is needed, on upgrade or after a
Web rollback.

### Preferences

Stored preferences gain `schema: 2`, and the server stores the mapped result.
Migration from an unversioned value: `input` takes the old `permission` flag,
the only input ever delivered; `error` is kept; `completed` becomes `false`
regardless of its stored value, because it was a default, not an opt-in. From
then on the Settings toggle is the opt-in.

## Acknowledgement

```text
task_acknowledgements(owner_user_id, session_id REFERENCES sessions ON DELETE
  CASCADE, task_key, marker, acknowledged_at_ms, PK(all but the last))
```

- `POST /api/sessions/{id}/tasks/acknowledge {task_key, marker}` is idempotent.
  The marker is `terminalSeq` for a subagent (falling back to Phase 1's marker for
  a row without it) and `finished_at_ms` for a Call.
- The focused bootstrap carries the session's acknowledgements, so another device
  catches up the next time it opens or reconnects to the session.
- Delivery reads this table, so an acknowledgement committed before its intent
  still stops the send; no ordering between them is required.
- Rows live as long as their session, so a failed task does not become unseen
  again on reload.
- A write that fails stays queued in the tab per session and is retried by the
  next bootstrap. The device-local list keeps it hidden here meanwhile, so a
  reload before it lands costs only the other devices' view.
- Phase 1's device-local acknowledgements stay readable on that device and are
  not uploaded: the key is origin-wide and could belong to another account.
- **Opening is not acknowledging for a link.** The inspector acknowledges on open
  for origins `row`, `bar` and `command`. With origin `link`, which a notification
  uses, it does not; the detail shows an explicit **Mark as seen** action.

## Rollout and rollback

- Additive migrations on both databases; an older Controller ignores them.
- Order: Controller, then Web, including `sw.js`. No Machine release.
- New enrollments reach only the new table, so until the Web ships nothing is
  delivered, which matches production's zero subscriptions.
- Rollback: set `notifications.delivery` off, or roll the Controller back; either
  stops delivery with no cross-account path.

## Acceptance

Automated, on both databases where storage is involved: each producer and
excluded case; replay, bootstrap and backfill creating no intent; metadata
updates after termination; a new occurrence after resume; permission seqs after
a worker restart; approval resubmission; audience freeze with an empty audience;
restart resuming deliveries; retry including `Retry-After`; 404/410 removal;
independent input Topics; aggregation; audience recheck; endpoint takeover and
logout; preference migration; acknowledgement before the intent and across sessions reusing a child id; presence deferral, stale lease and
input exemption; link-origin non-acknowledgement.

Physical, per the PWA convention: on a locked iPhone with the installed app, an
input push and a failure push reach the Lock Screen and open the right session
and task from a cold start; two offline requests both arrive; a push received in
the foreground is rendered and a later locked push still arrives, proving the
subscription survived. Push-service acceptance alone proves only that step.

## Decisions

From the design review (`call-d4b18d86e28547320be31fde`):

| # | Finding | Resolution |
|---|---|---|
| 1 | Rollback breaks startup or restores broadcast | Subscriptions move to a table; the old file is left alone; delivery switch |
| 2 | Logout and account switching | Endpoint takeover, logout removal, session-bound enrollment |
| 3 | Write-behind can lose both event and intent | Accepted and stated as the durability boundary; closing it is a separate transcript contract |
| 4 | Old service workers reject new categories | Payload stays v1 with existing categories; no capability gate |
| 5 | Browser suppression risks revocation | Service worker renders every push |
| 6 | Permission ids restart | Dedupe on the event seq |
| 7 | A stale lease hides input | Input ignores presence; others defer, not suppress |
| 8 | Shared Topic drops requests | Per-occurrence Topic and tag for input |
| 9 | Revision is not an occurrence | `terminalSeq` stamped at the transition |
| 10 | Approval ids are nondurable | Managed request id; intents resolved at start |
| 11 | Acknowledgement can precede the intent | Delivery reads the acknowledgement table |
| 12 | Acknowledgement scope | Session in the key; legacy local data not uploaded |
| 13 | "First processed" undefined | Atomic audience freeze with `expanded_at_ms` |
| 14 | Tags cannot give exactly-once | Goal restated as at-least-once with a stated bound |
| 15 | Default change misses stored consent | Versioned preference migration |
| 16 | Producers without a task destination | Typed destinations with a session fallback |
| 17 | Tap implies acknowledgement | Link origin does not acknowledge; explicit action |
| 18 | Minute buckets for session errors | Incident id, written with the incident |
| 19 | Retention resurrects failures | Acknowledgements live with the session; expiry governs delivery only |
