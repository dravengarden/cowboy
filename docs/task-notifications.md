# Task notifications and per-account acknowledgement

Status: contract for [background tasks](background-tasks.md) Phase 2, not yet
implemented. Installed-app rules come from the Columbus PWA convention.

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
away, exactly once per occurrence as far as they can observe, across a
Controller restart; a failure acknowledged on one device stops being unseen on
all of the account's devices.

Non-goals: an inbox surface, completion digests, email or other transports,
per-task notification rules, and declarative Web Push. The service worker
remains the renderer; moving to the declarative document needs its own physical
acceptance.

## Intents

A **notification intent** is one durable row, committed before any send:

```text
notification_intents(
  dedupe_key PK, owner_user_id, session_id, category, task_key NULL,
  title_hint NULL, created_at_ms, expires_at_ms, state, resolved_at_ms NULL
)
```

`category` is one of `input`, `failure`, `completion`. `state` is `pending`,
`resolved` (the condition went away or was acknowledged) or `expired`.

| Occurrence | Dedupe key | Category | Producer |
|---|---|---|---|
| Permission request | `req:perm:<session>:<request id>` | input | Hub, live ACP event |
| Pending call approval | `req:approval:<approval request id>` | input | approval registry, first time a request waits |
| Subagent waiting on a person | `req:perm:…` of its permission request | input | same path; `requires_action` alone never notifies (R4) |
| Subagent failed | `entity:<child>:<incarnation>:<revision>` | failure | accepted snapshot (I4) with a failure stop reason |
| Call failed | `exec:<call_id>:terminal` | failure | committed ledger transition to `failed` |
| Call completed | `exec:<call_id>:terminal` | completion | committed ledger transition to `completed` |
| Subagent ended normally | `entity:<child>:<incarnation>:<revision>` | completion | accepted terminal snapshot |
| Session error | `session-error:<session>:<fingerprint>:<minute>` | failure | `Outbound::Error`, today's producer |

Rules:

- **Only producers create intents.** Bootstrap, history, gap fills, lookups and
  backfill never do. A replayed subagent snapshot is either fenced (I4) or
  conflicts on its dedupe key, which uses producer identity rather than
  `observedSeq` because the latter changes on replay.
- **Execution failure is not a review result.** A completed review with findings
  is a completion. `cancelled`, `Stopped`, `Ended` and `Not reporting` never
  notify: the first two were asked for or carry no outcome, and contact loss is
  not an outcome.
- **Write order.** For a Call the intent commits in the same transaction as the
  ledger transition. For Hub producers it goes through the store writer with the
  event batch. A crash therefore loses neither or both, and repeated writes are
  `ON CONFLICT DO NOTHING`.
- **Child sessions.** A managed Call's child and a subagent notify on the parent
  session; `owner_user_id` is the parent's. Rows for a system session are not
  written.
- Intents older than their `expires_at_ms` are not delivered: 1 hour for input
  and failure, 15 minutes for completion. An input intent resolves as soon as
  its request is answered or withdrawn.

## Delivery

A single Controller task owns delivery. Sends are at-least-once; the visible
result is deduplicated by the notification `tag`.

```text
notification_deliveries(
  dedupe_key, endpoint_sha256, state, attempts, next_attempt_at_ms, PK(both)
)
```

- A delivery row is created per matching subscription when an intent is first
  processed. A subscription added later does not receive older intents.
- Retry 408, 425, 429, 5xx and transport errors with exponential backoff from
  5 seconds, at most 5 attempts, honoring `Retry-After`; any other 4xx fails the
  delivery; 404 and 410 remove the subscription.
- The task resumes pending rows at start, so a restart neither loses nor resends
  a delivery already recorded `sent`. A crash between the push service's accept
  and the row update resends; the tag replaces the first banner.
- Web Push `Urgency` is `high` for input, `normal` otherwise. `TTL` is the time
  left until the intent expires. `Topic` is a hash of the coalescing key, so the
  push service also replaces an undelivered predecessor.
- Delivery rows and resolved or expired intents are deleted after 7 days.
- Counters: intents created by category, deliveries sent, retried, failed,
  suppressed and expired. HTTP success stays labelled as push-service acceptance,
  never as device presentation.

### Coalescing

Completion and failure intents wait 3 seconds, then all pending intents of one
`(owner, session, category)` become one notification ("3 tasks failed"). Input
is never delayed. The coalescing tag is
`cowboy-<session>-<category>`; a later burst replaces the banner and states the
current unseen count rather than only the burst.

### Audience and content

- A subscription belongs to the account that enrolled it. A subscription with
  no owner (written by a previous Controller) is not delivered to until the next
  enrollment refresh, because delivering it would leak sessions across accounts.
- The audience is rechecked at delivery: the owner must still see the session,
  it must not be muted, and the intent must still be `pending`.
- `title_hint` is agent-produced text and stays display data. It reaches a
  notification only when `showSessionNames` is on, is truncated and never
  interpreted. With the setting off the copy is generic.
- The deep link is `/?session=<id>#task=<key>`. The service worker validates both
  against their grammars. A tap opens the row's inspector and never approves,
  stops or acknowledges anything.

### Preferences

Category defaults change to the design's: input and failure on, completion off.
The stored `completed` and `input` booleans become effective here, so their
defaults change together with the first producer; `permission` and `error`
remain accepted and map onto `input` and `failure`. The Controller accepts both
shapes, so it must be released before the Web.

### Presence lease

A device showing a session renews a lease of 30 seconds every 15 seconds with
`PUT /api/notifications/presence {session_id}` while the document is visible and
focused, and deletes it on hide. Leases live in Controller memory per account
and device.

A delivery whose session has a live lease of that account is **suppressed** for
all of the account's devices: the person is looking at it. Suppression is
recorded and the intent stays unseen for the bar. Input is exempt, so a request
never depends on a visible row being noticed from across the room: it is
delivered to every device except the one holding the lease. A missing or expired
lease delivers. Lease failure therefore over-notifies rather than under-notifies,
and a Controller restart empties the table.

## Acknowledgement

```text
task_acknowledgements(owner_user_id, task_key, marker, acknowledged_at_ms,
  PK(owner_user_id, task_key, marker))
```

- `POST /api/sessions/{id}/tasks/acknowledge {task_key, marker}` is idempotent.
  The marker stays Phase 1's terminal marker, so a resumed child's later failure
  is unseen again. It must name a task of that session; a client cannot write
  arbitrary rows.
- The focused bootstrap carries the session's acknowledgements, and a
  `task_acknowledged` outbound updates other connected devices.
- Acknowledging resolves the matching failure intent, so a pending retry stops.
- Device-local acknowledgements from Phase 1 are uploaded once for tasks still
  loaded, then ignored; the failure list is short-lived, so older ones lapse and
  may be seen once more, which is safe. A write offline stays queued locally and
  the local state wins until it lands.
- Rows are deleted with their session and after 30 days.
- Opening a notification does not acknowledge. Opening the inspector does, as in
  Phase 1.

## Rollout and rollback

- Two additive migrations per database. An older Controller ignores the tables
  (SQLx `ignore_missing`).
- Order: Controller, then Web, then the service worker through the normal Web
  release. Machines are not involved.
- A Service setting `notifications.intents` (default on) disables producers and
  leaves the current best-effort path off as well; it exists only for rollback
  and is removed with the Calls dock switch.

## Acceptance

Automated, on both databases where storage is involved: every producer and
excluded case in the table above; replay, bootstrap and backfill creating no
intent; crash between ledger commit and delivery resuming; retry and backoff
including `Retry-After`; 404/410 removal; coalescing; audience recheck after a
session is muted or reassigned; ownerless subscriptions skipped; preference
shapes; acknowledgement sync across two clients and intent resolution; presence
suppression and fail-open behavior.

Physical, per the PWA convention and not replaceable by the above: on a locked
iPhone with the installed app, one remote push for input and one for a failure
appear on the Lock Screen and open the exact task. Push-service acceptance alone
is recorded as acceptance only of that step.
