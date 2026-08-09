# Runtime Incident Ledger

Cowboy separates durable incident identity from high-volume evidence.

## Ownership

- PostgreSQL `runtime_incidents` is the incident ledger: classification,
  fingerprint, affected runtime, evidence window, and recovery outcome.
- VictoriaLogs owns structured log evidence and its retention policy.
- VictoriaMetrics owns client and service time series and its retention policy.
- Session `events` remain transcript facts. Observability payloads must not be
  inserted into the transcript log.

The Logs surface is a read model over existing owners, not a fourth logging
database:

- runtime and client failures come from `runtime_incidents`;
- every managed DeepSeek gateway response with HTTP status 400 or higher comes
  from `provider_usage_events`;
- active-session cache disruptions are derived from adjacent, content-free
  provider usage rows;
- provider automation audit comes from `provider_action_logs`.

List endpoints return only compact summaries and an opaque, stable log id.
Details and structured evidence load only when a user expands one row. Raw
Victoria evidence is not copied into PostgreSQL or eagerly shipped to clients.
The Logs UI defaults to `critical` and `error`; `warning` and `info` remain
available as explicit multi-select chips. Type, severity, lifecycle state, and
runtime are independent multi-select dimensions. Relative `Last N` and exact
start/end ranges share one bounded time-range contract; list pagination keeps
the chosen upper boundary stable.

The ledger does not enforce a duplicate retention period. An incident remains
useful after raw evidence expires because its summary, classification, and
recovery outcome are durable; the UI must represent expired evidence honestly.

## Correlation

Every evidence item should carry the applicable `incident_id`, `trace_id`,
`session_id`, `machine_id`, `client_id`, component, and build. Values absent at
the source stay absent; clients must not infer Machine identity from paths or
hostnames.

Lifecycle crashes and unexpected interruptions create ledger entries at the
controller persistence boundary. A later Running lifecycle closes every active
controller runtime incident for that session as recovered. Failed turn and
command records remain immutable evidence. Client-originated render, network,
and performance incidents enter through the observability batch API.

Every session-scoped `Outbound::Error` also creates a bounded incident entry.
This covers rejected runtime commands, dispatch failures, and other errors that
are user-visible but do not produce a lifecycle transition. A `TurnEnd` stop
reason beginning with `error:` creates a separate failed-turn entry, including
provider errors after which the ACP process remains usable. Internal retries and
process diagnostics remain in the worker's journald stream and therefore in
VictoriaLogs even when they recover before reaching the controller. The Logs UI
must distinguish this raw-evidence coverage from the durable user-visible
incident index rather than claiming that Cowboy can observe provider-internal
retries that a native CLI never emits.

Severity follows observed impact rather than the presence of an error-shaped
payload. A session-ending lifecycle or failed turn is an error. Provider 400,
401, 402, 403, and other non-retryable request rejections are errors (credential
or balance failures are critical). Rate limits, client cancellation, upstream
network failures, and provider 5xx responses are warnings until a session-level
failure proves that the agent was interrupted. Tool-call failures inside an
otherwise live turn are not provider errors and do not contribute to the
DeepSeek blocking-error rate. The DeepSeek card exposes blocking and retryable
counts separately while retaining all failed requests in its expanded detail.

## Cache diagnostics

A cache miss is not itself an incident. DeepSeek caching is best effort and a
normal first request, inactive session, provider eviction, model/configuration
change, compaction, or gateway restart can all produce low hit rates.

Cowboy emits a cache-disruption log only when one attributed session moves
between two successful, measurable requests from at least 90% cache hit to
below 10% within 30 minutes, both observations contain at least 8K input
tokens, and content-free static-prefix fingerprints exist on both sides. Failed
provider calls between those observations do not hide the transition; they are
recorded as an explicit cause. The event records the strongest observed cause:
compaction, gateway build/boot change, model/role/protocol/reasoning change,
compatibility rewrite, static-prefix change, history rewrite, an intervening
provider error, an exact-prefix miss, or an otherwise unexplained active-session
drop. It never stores prompt, response, tool, or reasoning content.

DeepSeek workers attach a stable SHA-256 session token through a local-only
request header. Claude Code uses `ANTHROPIC_CUSTOM_HEADERS`; Codex resolves the
same header from its provider `env_http_headers` configuration. Each gateway
HMACs the token before emitting telemetry and excludes the Cowboy header from
the upstream allowlist. This gives new, resumed, retried, and compacted requests
one explicit attribution key without adding anything to the cacheable prompt
prefix or persisting the Cowboy session identifier.

Longer idle gaps are excluded from anomaly logs. In aggregate low-hit analysis,
a gap of at least six hours is classified as probable eviction before an exact
prefix match is considered; this prevents expected expiration from being
reported as a prefix-stability bug. The 30-minute active threshold is
deliberately conservative relative to provider documentation:

- <https://api-docs.deepseek.com/guides/kv_cache/>
- <https://developers.openai.com/api/docs/guides/prompt-caching>

Cache-prefix stability is a compatibility requirement for every Claude and
Codex session lifecycle path. Resume, load, retry, compaction, hooks, dynamic
system-prompt sections, and future provider adaptations must preserve the
largest possible byte-identical prefix. A deliberate prefix change needs a
documented reason, content-free telemetry attribution, and regression coverage.

DeepSeek cache-protection attempts are also audit events, not session
incidents. Schema-v4 provider usage rows record the attempt number, scheduled
interval, source age, outcome, measured cache tokens, duration, and opaque
source-request fingerprint. Verified hits and agent preemptions are informational;
misses, partial hits, and retryable provider failures are warnings; terminal
credential or balance failures are critical and other terminal failures remain
warnings. A background failure never raises the agent blocking-error count unless
an independent interactive request or lifecycle event proves that the session
was interrupted. Details remain body-free and load lazily through the same Logs
surface.

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
