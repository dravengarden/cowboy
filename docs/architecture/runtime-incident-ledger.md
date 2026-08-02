# Runtime Incident Ledger

Cowboy separates durable incident identity from high-volume evidence.

## Ownership

- PostgreSQL `runtime_incidents` is the incident ledger: classification,
  fingerprint, affected runtime, evidence window, and recovery outcome.
- VictoriaLogs owns structured log evidence and its retention policy.
- VictoriaMetrics owns client and service time series and its retention policy.
- Session `events` remain transcript facts. Observability payloads must not be
  inserted into the transcript log.

The ledger does not enforce a duplicate retention period. An incident remains
useful after raw evidence expires because its summary, classification, and
recovery outcome are durable; the UI must represent expired evidence honestly.

## Correlation

Every evidence item should carry the applicable `incident_id`, `trace_id`,
`session_id`, `machine_id`, `client_id`, component, and build. Values absent at
the source stay absent; clients must not infer Machine identity from paths or
hostnames.

Lifecycle crashes and unexpected interruptions create ledger entries at the
controller persistence boundary. A later Running lifecycle closes the newest
open incident for that session as recovered. Client-originated render,
network, and performance incidents enter through the observability batch API.

## Client ingestion

`POST /api/observability/batches` accepts bounded batches. The server validates
names and sizes, strips unsafe attributes, adds trusted receive metadata, and
forwards evidence to Victoria. The browser never receives Victoria write
credentials or endpoints.

Telemetry is best effort and must never delay prompts, scrolling, session
switching, or shutdown. A bounded queue drops ordinary evidence under pressure;
drop counts are exported from Cowboy's native `/metrics` endpoint. Incident
ledger writes remain durable and idempotent.

The client connection timeline must remain reconstructable across reloads. It
records initial connection and reconnect attempts, trigger reason, close code,
socket lifetime, exponential-backoff delay, outage duration, foreground and
network recovery, loaded bundle identity, server and Service Worker versions,
and the update-detected to reload-completed transition. Metrics keep only
bounded dimensions; client, session, and trace identity remain log fields.

## Privacy and cardinality

Do not collect prompts, responses, tool output, clipboard or attachment
contents, authorization material, cookies, environment variables, or absolute
paths. Message and attribute sizes are bounded. Metric names and dimensions
come from allowlists; session, trace, and client identifiers belong in logs and
incident records, not high-cardinality metric labels.
