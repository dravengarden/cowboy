# Telemetry backends

Source implementation and operational contract. This document is not a
publication or production-activation receipt.

## Ownership and scope

Cowboy owns instrumentation, validated/redacted events, bounded queues, health
counters, and the durable Runtime Incident Ledger. Diagnostic telemetry is not
conversation persistence, an authentication store, or the usage/accounting
ledger. Those durable records do not move to `/tmp`.

The client now sends standard protobuf to `/api/telemetry/v1/{logs,metrics,traces}`.
`/api/observability/batches` remains for durable incidents and legacy clients.
Incident-only requests are ledger/local evidence; their diagnostic OTel log is
emitted separately, not exported twice through incompatible encodings.
Existing process `tracing` output remains on stderr/journald and
the Controller's Prometheus `/metrics` scrape remains available. Neither raw
agent stdout, prompts, nor conversation content is automatically exported.

The default backend is a private, bounded, rotating JSON-lines file directory
under `/tmp`. Rotation must cap both segment size and retained segment count,
reject symlinks and competing writers, and preserve complete UTF-8 records.
Temporary storage is intentionally not durable across reboot, tmpfs exhaustion
or host cleanup. Local write failures must be observable without blocking the
application or recursively generating more telemetry.

An optional `telemetry_backend` Plugin connects an existing external telemetry
service. The first example is Victoria (VictoriaLogs, VictoriaMetrics and optional VictoriaTraces),
not an installer for the databases themselves. It uses the existing immutable
Plugin package/signature/Catalog/Machine-install/rollback/uninstall lifecycle.
It must not pretend to be an Agent Provider or borrow Provider authentication.

Core mediates a closed data-only HTTP export contract. The signed package
declares supported encodings and relative ingestion routes; operator-private
configuration selects the exact installed Machine/Plugin/release and service
endpoints. Packages do not contain endpoint credentials, executable browser
code, shell commands, runtime downloads or deployment policy. A missing,
uninstalled or incompatible remote Plugin cannot enable an ambient fallback.

Local recording remains enabled when remote export is selected. Remote work
uses an independent bounded queue and bounded requests/retries; an unavailable
destination cannot delay local evidence or incident persistence. Remote
delivery is best effort, not exactly once. Stable batch/event identities make
retries identifiable. Failures and queue drops need separate counters.

## Local configuration and guarantees

No Victoria URL is attempted by default. The old
`COWBOY_VICTORIA_LOGS_URL`/`COWBOY_VICTORIA_METRICS_URL` settings no longer enable
export. Migrate explicitly to the signed backend before retiring old service
configuration; this task does not modify NixOS or running services.

| Setting | Default | Bounds |
| --- | --- | --- |
| `COWBOY_TELEMETRY_DIR` / `--telemetry-dir` | `/tmp/cowboy-telemetry-<instance hash>` | Absolute private child directory |
| `COWBOY_TELEMETRY_SEGMENT_BYTES` | 8388608 (8 MiB) | 65536–67108864 |
| `COWBOY_TELEMETRY_RETAINED_FILES` | 8, including current | 2–32 |
| `COWBOY_TELEMETRY_PLUGIN_CONFIG` | absent, local only | Exact Controller selection file |

Instance directory identity includes effective UID and data directory. The
writer owns mode-0700 directory, mode-0600 files, and an exclusive writer lock.
It rotates `telemetry.jsonl` through `.1` to `.7` at segment capacity or the
first write after a UTC day boundary: at most 64 MiB by default. It never
rotates by a client's supplied clock. Symlinks, hard links, foreign ownership
and public permissions are rejected. Explicitly archive existing oversized or
excess segments before lowering limits; startup does not silently delete
evidence to accommodate a smaller configuration. Unrelated filenames are
never deleted. JSONL crash-tail repair discards only an incomplete final line.

Unsafe configuration/open failures are startup errors. Later write/rotation
failures increment a local-failure counter, not a recursive telemetry event.
Files are diagnostic evidence, not a WAL: admission is not fsync, loss is
possible on crash, reboot, disk-full or temporary-directory cleanup. There is
no automatic historical file replay to remote services.

Admission is bounded to 64 batches / 8 MiB, 200 items / 256 KiB per request.
In-process dedup retains 2048 scoped batch identities and rejects reused IDs
with different content; restart/eviction ends that dedup window. IDs are
namespaced per authenticated user and client (OTLP: user, signal and batch ID).
Legacy incident session associations are checked against authenticated
visibility, and Machine context is derived by the Controller. OTLP client
session/Machine/user resource identities are discarded; a server-scoped owner
hash and explicit W3C trace IDs provide diagnostic correlation without granting
authority. Messages and scalar attributes receive bounded credential/URL
redaction. Instrumentation must still never submit raw prompts or credentials;
regex redaction is defense in depth, not a content-classification guarantee.

Legacy metric names use portable Prometheus syntax. OTLP uses a closed standard
counter/histogram instrument registry and delta-to-cumulative aggregation; see
[client OpenTelemetry](client-opentelemetry.md). Only finite connection,
transport, operation and reconnect-reason dimensions are exported, alongside normalized
platform/surface. Build/client/session/trace IDs and arbitrary attribute keys
are not time-series labels. Victoria log streams similarly use normalized
component/platform fields. These tighten the previous metric-label contract;
unknown instruments and reserved labels cannot create new time-series identities.

Local, incident and remote workers are independent of remote delivery; incident
and remote queues each hold at most 16 batches. Incident writes are queued
before local/remote work, run independently of remote receipts, and never
rotate the durable ledger; a queue/storage failure remains observable in its
own counter and local diagnostic evidence.
Remote import is best effort and can duplicate an ambiguously acknowledged
request. Each lane has a 1-second connect / 3-second request timeout, at most
two attempts, no redirects or ambient proxies. OTLP retries transport failures
and HTTP 429/502/503/504 only; legacy routes also retain 408/other-5xx behavior.
OTLP 200 partial success is parsed from bounded protobuf (64 KiB) and never
retried as a whole batch. One Machine export runs at a time.
Controller calls have a 15-second outer deadline. Shutdown drains workers
for at most 10 seconds before aborting unfinished work.

Frontend OTLP pending data is capped at 200 items / 256 KiB, including in-flight
and retry bodies. SDK queues are separately bounded at 64 logs and 64 spans;
the incident queue remains independently bounded at 200 items / 256 KiB.
Context is captured at instrumentation time, request bodies are at
most 24 KiB (protobuf for OTLP, UTF-8 for incidents), fetch aborts after 8 seconds, and retries retain exact bytes
with backoff (five attempts / five minutes maximum). Permanent HTTP errors
are discarded. Beacon delivery has no server receipt, and an in-flight fetch
is not duplicated on pagehide. Sign-out clears queued/in-flight identity.

## Remote activation and diagnosis

Use the [Victoria package guide](../examples/telemetry/victoria/README.md) for
the generic build/sign/publish workflow, management page and exact policies.
Endpoint URLs/tokens exist **only on the selected Machine**, not in Controller
commands or public packages. Controller selection is loaded at startup;
Machine policy is checked for every request. Updating/revoking the policy or
uninstalling fails closed for subsequent exports. A bounded in-flight request
may finish. Changing a Plugin version/digest does not silently inherit old
activation authority.

`/api/metrics` reports `observability_pending`, `observability_pending_bytes`,
`observability_accepted_batches`, `observability_duplicate_batches`,
`observability_dropped_batches`, `observability_failed_file_batches`,
`observability_failed_incident_batches`, `observability_dropped_export_batches`,
`observability_failed_log_batches`, `observability_failed_metric_batches`,
`observability_failed_trace_batches` and `observability_rejected_export_items`.
The loopback-authorized Prometheus endpoint exposes the corresponding
`cowboy_observability_*` gauges and `_total` counters. Monitor file and ledger
failures separately from remote lane failures and backpressure. Missing
installed generation/policy/connection appears as failed remote delivery;
there is no fallback to an arbitrary Catalog version or localhost destination.

The usage service now shares the Controller's single `MachineControl`; the
previous duplicate initialization gave usage an empty, disconnected inventory.

## Implementation and gates

1. `src/observability.rs` and a private file-writer module: default file backend,
   bounded validation/redaction, timestamps, valid metric names/labels,
   queue/delivery accounting, incident isolation and shutdown draining.
2. `components/plugin-sdk`, Plugin contract/build tooling, generic Catalog and
   Machine capability boundaries: add the closed telemetry payload and signed
   install lifecycle. This is a new SDK capability, not a source-only plugin
   registry entry. Append the component release and bump existing first-party
   versions if their component inputs change; never rewrite old releases.
3. `examples/telemetry/victoria`: independently buildable data-only Plugin,
   explicit configuration, and real temporary signed-install/export tests
   against loopback fixtures. Initially advertise only verified platforms.
4. `web/src/observability.ts`: hard bounds even for incident-only floods,
   context retained at capture time, bounded UTF-8 request bodies, stable retry
   identity, no infinite retry of permanent HTTP rejection, request deadlines,
   and privacy-safe messages/attributes. No composer changes.
5. Run targeted failure-path tests, SDK/Plugin gates, frontend checks and the
   full pinned Cowboy gate. Publication and live activation remain separate;
   do not send production telemetry to a new destination during implementation.

## Local-first baseline verification (30734017, 2026-09-08)

`nix develop -c just check-compact` passed: component/Plugin/SDK validation,
native-shell and site contracts, formatting/lint/dependency checks, feature
slices, Rust and 1176 frontend tests, isolated PostgreSQL tests, and production
Web/Rust builds. Final file-integrity and frontend-bound changes additionally
passed the four local-writer tests, six type-checked telemetry tests, Rust
Clippy, and frontend typecheck/lint.

The signed temporary Machine fixture covers install without export authority,
exact private policy, real JSONL/Prometheus HTTP requests, transient retry,
permanent rejection, upgrade, uninstall, retained rollback, and package
tampering. Separate fixtures cover slow/redirecting destinations and a stuck
remote worker while local files and the incident ledger continue.

These are implementation/build gates, not production acceptance. No running
Controller/Machine was restarted, no live Catalog was published or refreshed,
and no production endpoint or credential was used. Victoria server installation,
production endpoint acceptance and non-Linux platforms are not claimed.
The subsequent OTLP implementation and its validation are recorded in
[client OpenTelemetry](client-opentelemetry.md).

## Initial audit

- Victoria URLs are mandatory CLI strings with loopback defaults; there is no
  local file backend.
- A shared writer awaits network delivery before recording incidents, and its
  HTTP client has no request timeout.
- Frontend trimming can leave an unbounded incident-only queue; failed batches
  are rebuilt with new identities/current context, and every HTTP rejection is
  retried.
- Metric validation permits characters invalid in label names, collisions with
  built-in labels, and unbounded-cardinality identity dimensions.
- Batch context, incident fingerprints and several strings lack independent
  validation; message redaction is client-only and does not reliably consume
  spaced authorization values.

Protocol references: [VictoriaLogs JSON-line ingestion](https://docs.victoriametrics.com/victorialogs/data-ingestion/#json-stream-api)
and [VictoriaMetrics Prometheus ingestion](https://docs.victoriametrics.com/victoriametrics/#how-to-import-data-in-prometheus-exposition-format).
