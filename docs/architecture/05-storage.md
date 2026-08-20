# Storage

cowboy is an **event-sourced stateful daemon**: growing event logs, mobile
pagination, restart recovery. Persistence is optional and selected by
`--database-url`: PostgreSQL (`postgresql://...`) and SQLite (`sqlite://...`)
implement the same stable `Store` API. With no URL the daemon runs fully
in-memory and forgets on restart. The deployed Hawk service continues to use
its service-private PostgreSQL database. The legacy `--postgres-url` and
`COWBOY_POSTGRES_URL` spellings remain accepted for deployment compatibility.

Regenerable Code review data is intentionally separate from durable product
state. Saved files and lazy directory pages use the bounded Hawk-local
`data_dir/code-cache` SQLite/CAS store described in
[13-code-review.md](13-code-review.md). It can be deleted without losing user
state; Zed owns unsaved buffers.

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
    WR --> STORE{"Store facade"}
    STORE --> PG[("PostgreSQL")]
    STORE --> SQ[("SQLite")]
    PG --> RST["load_all() on boot"]
    SQ --> RST
    RST --> HUB

    style HUB fill:#eef2ff,stroke:#6366f1
    style PG fill:#fef9c3,stroke:#ca8a04
    style SQ fill:#fef9c3,stroke:#ca8a04
    style RST fill:#dcfce7,stroke:#16a34a
```

Writes normally land within milliseconds. On SIGTERM the HTTP/WS server closes
connections and waits up to ten seconds for the writer to drain. A hard crash can
still lose the latest in-memory batch; synchronous token-path writes would couple
live fan-out latency to storage, so cowboy keeps that window observable and bounded.

## Schema

PostgreSQL is built incrementally by the immutable `migrations/*.sql` history
(SQLx applies it on boot):

| Migration | Adds |
|---|---|
| `0001_init` | `sessions` (id, provider, cwd, title, origin, status, next_seq, timestamps) + `events` (session_id, seq, payload JSONB, ts; PK `(session_id, seq)`) |
| `0002_agent_session_id` | `agent_session_id` — the resume bridge |
| `0003_pending` | `queue` + `drafts` JSONB arrays |
| `0004_session_position` | `position` for user reordering |
| `0005_session_soft_delete` | `deleted_at` — soft-delete for the purge window |
| `0006_auto_resume` | legacy `auto_resume` column, retained for immutable migration history |
| `0007_inference` | `inference_config` + `inference_secrets` tables |
| `0008_turn_verdict` | legacy confirm-verdict columns, retained only for immutable migration history |
| `0009_judge_runs` | legacy judge-run history, retained only for immutable migration history |
| `0010_system_session` | `system` flag for machine-driven, view-only sessions |
| `0011_scheduled_wakeups` | persisted agent wakeups |
| `0012_compact_event_log` | drops the duplicate event index, removes transient telemetry, folds legacy tool updates into their initial row |
| `0013_drop_inference` | drops obsolete external-provider config and API-secret tables |
| `0016_mobile_review_state` | Mobile-only tabs, active source, review mode, and reviewed revisions per session |
| `0017_machines` | Stable machine registry plus immutable per-session machine placement |
| `0018_machine_enrollment` | Single-use enrollment tokens and per-machine public-key identity |
| `0025_provider_usage_telemetry_v3` | Provider-neutral model, role, protocol, HMAC lineage, and gateway-build dimensions for DeepSeek cache/cost diagnosis |
| `0027_deepseek_cache_keepalive` | Schema-v4 request purpose, cache-protection outcomes, adaptive timing, source age, and content-free source fingerprints |

The writer UPSERTs consecutive message/thought chunks into their first sequence
row, folds tool updates into the initial call, and stores only the sequence
watermark for usage/session-info telemetry. Live WS clients still see every
frame. On restore, only the latest 1,000 durable rows per session enter the Hub;
older rows stay in Postgres and are loaded through immutable 200-event pages.
Native Postgres fits this better than a Timescale hypertable: reads and
uniqueness are keyed by `(session_id, seq)`, while the actual cost was redundant
large JSON payloads rather than time-range scans.

SQLite starts from the equivalent current schema in
`migrations/sqlite/0001_baseline.sql`. Its timestamps are Unix milliseconds and
JSON is validated TEXT. WAL, foreign keys, a five-second busy timeout, and a
small connection pool make it suitable for one Cowboy controller. A SQLite file
must never be shared by multiple controllers or placed on a network filesystem.

## Store API surface

The rest of Cowboy calls only the backend-neutral `Store`; its private enum
dispatches to `PostgresStorage` or `SqliteStorage`. `StoreWrite`, the reducer,
retry behavior, DTOs, pagination, and error contracts do not expose the backend.

`load_all()` returns every session's metadata + hot event tail + total event
count + queue/drafts in one restore. Mutations mirror the `StoreWrite`
variants: `insert_session`, batched event UPSERT, `update_status`, `update_title`,
`update_agent_session_id`, `delete_session`, `update_pending`,
`update_session_order`, and Mobile review state. `purge_deleted()` hard-deletes
soft-deleted rows. The legacy `auto_resume`, confirm-verdict, and judge-run
columns and settings table are deliberately ignored so old migrations stay
immutable and a rollback can still read its schema.

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
hand-rolled. A relational backend earns its place here; PostgreSQL remains the
production default, while SQLite removes that operational dependency for a
single-controller deployment.

Provider usage rows are the durable, queryable product ledger for both
interactive DeepSeek requests and cache-protection attempts. The
`request_purpose` column keeps those populations disjoint in every aggregate;
background attempts have their own token, outcome, duration, and cost fields.
Both backends store only content-free dimensions and keyed fingerprints. Exact
request snapshots remain bounded gateway memory and are lost deliberately on
gateway restart. VictoriaLogs continues to own high-volume process evidence,
while the Logs UI derives compact cache-protection audit rows from the durable
usage ledger and loads details lazily.
