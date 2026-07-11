# Storage

cowboy is an **event-sourced stateful daemon**: growing event logs, mobile
pagination, restart recovery. Persistence is **Postgres** (optional — passed via
`--postgres-url`); with no URL the daemon runs fully in-memory and forgets on
restart. The deployed service uses the host's service-private Postgres.

## Write-behind

The Hub never blocks on the database. It emits a `StoreWrite` intent onto a
bounded mpsc channel (8,192 intents); a background **writer task** drains small
batches, reduces high-frequency events, and executes the corresponding `Store`
operations. Queue overflow or a batch that exhausts four retries marks
persistence degraded through `/healthz` and `/api/metrics`.

```mermaid
flowchart TB
    HUB["Hub.push(Event)"] --> CH["StoreWrite channel<br/>(bounded: 8,192)"]
    CH --> WR["batch + reduce + retry"]
    WR --> PG[("Postgres")]
    PG --> RST["load_all() on boot"]
    RST --> HUB

    style HUB fill:#eef2ff,stroke:#6366f1
    style PG fill:#fef9c3,stroke:#ca8a04
    style RST fill:#dcfce7,stroke:#16a34a
```

Writes normally land within milliseconds. On SIGTERM the HTTP/WS server closes
connections and waits up to ten seconds for the writer to drain. A hard crash can
still lose the latest in-memory batch; synchronous token-path writes would couple
live fan-out latency to Postgres, so cowboy keeps that window observable and bounded.

## Schema

Built incrementally by the `migrations/*.sql` files (sqlx applies them on boot):

| Migration | Adds |
|---|---|
| `0001_init` | `sessions` (id, provider, cwd, title, origin, status, next_seq, timestamps) + `events` (session_id, seq, payload JSONB, ts; PK `(session_id, seq)`) |
| `0002_agent_session_id` | `agent_session_id` — the resume bridge |
| `0003_pending` | `queue` + `drafts` JSONB arrays |
| `0004_session_position` | `position` for user reordering |
| `0005_session_soft_delete` | `deleted_at` — soft-delete for the purge window |
| `0006_auto_resume` | `auto_resume` per-session override |
| `0007_inference` | `inference_config` + `inference_secrets` tables |
| `0008_turn_verdict` | `awaiting_user` + `done` (persisted confirm verdict) |
| `0009_judge_runs` | `judge_runs` JSONB history |
| `0010_system_session` | `system` flag for machine-driven, view-only sessions |
| `0011_scheduled_wakeups` | persisted agent wakeups |
| `0012_compact_event_log` | drops the duplicate event index, removes transient telemetry, folds legacy tool updates into their initial row |

The writer UPSERTs consecutive message/thought chunks into their first sequence
row, folds tool updates into the initial call, and stores only the sequence
watermark for usage/session-info telemetry. Live WS clients still see every
frame. On restore, only the latest 1,000 durable rows per session enter the Hub;
older rows stay in Postgres and are loaded through immutable 200-event pages.
Native Postgres fits this better than a Timescale hypertable: reads and
uniqueness are keyed by `(session_id, seq)`, while the actual cost was redundant
large JSON payloads rather than time-range scans.

## Store API surface

`load_all()` returns every session's metadata + hot event tail + total event
count + queue/drafts/judge runs in one restore. Mutations mirror the `StoreWrite`
variants: `insert_session`, batched event UPSERT, `update_status`, `update_verdict`, `update_title`,
`update_agent_session_id`, `delete_session`, `update_pending`,
`update_session_order`, `update_auto_resume`, `update_judge_runs`, plus settings
(`load_settings` / `put_setting`) and inference (`load/put_inference_config`,
`load/put_inference_secret`). `purge_deleted()` hard-deletes soft-deleted rows.

## NUL-byte stripping

Postgres `jsonb`/`text` reject `U+0000`, but agent stdout occasionally carries
stray NUL bytes. `strip_nul()` / `strip_nul_str()` drop them before write so one
bad byte can never poison a row's durability — a row that would otherwise fail to
insert is kept (minus the NULs) instead of lost.

## Retention

Deletes are **soft** (`deleted_at` stamp), so a mis-tap is recoverable. A
**purge sweeper** (`run_purge_sweeper()`, every 6h) hard-deletes sessions past a
3-day retention window. `/api/metrics` exposes storage/session/RSS values plus
persistence queue, drop, and failed-batch counters.

## Why not file-as-truth (the omega rule)

omega is a near-stateless config panel, so file-only fits it. cowboy's
access pattern — high-rate streaming appends, paginated reads, search, restart
recovery — is exactly a database's job. A pure-file fallback (`events.jsonl` +
`meta.json` per session) is possible, but pagination and search would then be
hand-rolled. Postgres earns its place here.
