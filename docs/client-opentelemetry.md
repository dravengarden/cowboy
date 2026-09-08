# Client OpenTelemetry

Client integration on top of the local-first telemetry Plugin pipeline.
This is not a publication, activation, or physical-device acceptance receipt.

## Design

- Use pinned OpenTelemetry JavaScript APIs/SDKs and official OTLP serializers.
  No automatic DOM/console/interaction capture, Zone patching, external browser
  endpoints, or ambient exporter configuration. Business wrappers stay local
  to Cowboy. Request bodies, prompt text and credentials are never span data.
- Browser OTLP/HTTP requests go to authenticated same-origin Controller routes.
  The Controller treats all resource/context/attribute fields as untrusted;
  it validates and redacts them before local recording or Plugin export.
- Files remain the default, bounded rotating diagnostic destination under
  `/tmp`. Durable incidents/accounting/conversations retain their existing
  ownership. A Collector service is not required.
- Browser counters and histograms use delta temporality. Controller admission
  deduplicates stable request identities before bounded, low-cardinality
  aggregation into single-writer cumulative streams. Client/session/trace
  identities must not become remote metric labels.
- Explicit traces cover connection/reconnection and command dispatch/response.
  Standard W3C context crosses owned boundaries only. Sampling never suppresses
  the independent durable incident path. A traced operation has bounded
  lifetime and attribute count; it is not a span per keystroke or token.
- Extend the signed telemetry payload to schema 2, Plugin SDK 1.8 and Machine
  protocol 9 for OTLP export. Victoria 1.1 declares protobuf logs/metrics/traces
  routes; private policy enables each lane independently. Schema-1 retained
  packages stay readable. OTLP partial success and permanent errors are not
  retried as whole successful batches.
- Browser queues, SDK aggregations, in-flight exports, retry bodies, flush
  deadlines and Controller aggregation state all need explicit limits.
  Sign-out clears account-associated diagnostic work. Only one layer owns
  network retries. OTLP does not promise exactly-once or page-exit delivery.

## Client baseline verification (d0ff0f56, 2026-09-08)

`nix develop -c just check-compact` passed after the final audit fixes: 716 Rust
library tests (8 environment-dependent tests ignored), 1186 frontend tests,
6 separately exercised isolated PostgreSQL tests, SDK and signed Plugin gates,
native-shell/site contracts, formatting, lint, dependency audits, typechecking,
feature slices and production Web/Rust/Zed-adapter builds.

Coverage includes official JS serializer -> real Controller decoder, redaction
and malformed inputs, full integer attribute precision, delta dedup and
multi-client aggregation, trace propagation and orphan cleanup, actual browser
deadline abort, permanent/partial-success export responses, local recording
without a backend, and signed installation/rollback/uninstall. Immutable Nix
release outputs are built only after committing the validated source; they
have separate source receipts and do not authorize activation. Actual
Safari/PWA/WebView performance and background restoration remain separate
acceptance until exercised.

## Implemented boundaries and limits

The client uses API 1.9.1, core/resources/metrics/trace SDK 2.11.0, and logs SDK /
official OTLP serializers 0.222.0. All pins are exact. Instance-owned
`TracerProvider` avoids the environment-reading compatibility wrapper and does
not register a global context manager. There is no Zone dependency. Traces
sample 10% of roots; counters, histograms, errors and durable incidents are
not trace-sampled.

Connections span attempt -> ready/failure. Commands span send -> matching
acknowledgement, with stable context across retries. First-output spans start
at the live user echo (dispatch), excluding time waiting in the queue, and end
at the first agent message chunk. Foreign echoes, disconnects, sign-out and
five-minute expiry cancel attribution. No snapshot/history replay produces a
measurement. Up to 32 operations are retained. A W3C `traceparent` on submit
creates a Controller child for synchronous WebSocket parsing/authorization/
dispatch. Authorized submits also propagate context through Controller queueing,
Machine routing and the worker's owned ACP execution boundary, as described
below. This does not instrument the agent/model internals.

## Owned runtime trace boundaries

The runtime extends a sampled client command with this parent chain. Every
duration uses the measuring process's monotonic clock; the wall clock supplies
only its start timestamp. No subtraction of clocks on different Machines is
used to infer latency. Asynchronous children may finish after their parent.

| Span suffix (under `cowboy.`) | Actual measured interval |
| --- | --- |
| `controller.dispatch` | WebSocket command parsing, authorization and synchronous admission |
| `controller.queue` | Authorized submit binding to dispatcher handoff |
| `controller.delivery` | Runtime enqueue through the worker command acknowledgement |
| `machine.dispatch` | Broker prompt receipt through worker-channel handoff, including drain/start wait |
| `worker.queue` | Worker admission through the ACP prompt lock and configuration fence |
| `worker.prompt` | Actual ACP prompt RPC through completion, including bounded upstream retries |
| `worker.first_output` | Actual ACP prompt RPC through the first live `agent_message_chunk` |

Configuration rejection and cancellation before the RPC end only the queue
span; they never invent execution or first-output measurements. A turn with no
agent message emits no first-output span. Cancellation/error/normal completion
use a closed outcome enum; error details, prompts, replies, tools and Provider
configuration are not attributes. Untagged live output follows the serialized
active ACP turn; explicitly foreign turn tags and pre-start/history output
cannot complete its first-output timer.

The Controller binds a new private correlation nonce to the authenticated owner,
exact session, client command ID and Machine after existing mutation fences.
Client-supplied session/user/Machine claims cannot replace this authority.
Runtime records are checked against it, then converted to official OTLP
protobuf with fixed component resources and a server-derived owner hash. Trace
IDs remain untrusted correlation hints, not visibility proofs. The private
nonce and command/session identifiers are not exported as attributes.

Context lives only in optional runtime-v1 command metadata. Completed records
return in the existing sequenced WorkerEvent envelope, **outside** its business
event. ACP requests, Provider environments, durable queue rows, transcripts and
worker snapshots do not gain trace metadata. The v1 wire contract already
permits additive optional fields: missing, malformed or unsupported diagnostics
are ignored without rejecting valid business commands. Plugin SDK versions,
telemetry payload schema and Machine control protocol are unchanged by this
runtime-only extension; there is no new SDK, Collector or outbound transport.

The Controller retains at most 512 correlations for at most 24 hours, with at
most 32 unique remote span IDs each. Under capacity pressure, the oldest
completed correlation with an acknowledged delivery yields to new work; active
entries are not evicted. Machine timers are capped at 512 and 24 hours. Each
worker has 32 queued timers, one active turn, four pending completion records
and at most 128 annotated outbox events (four records per frame). ACK releases
outbox capacity. Telemetry saturation drops diagnostics, not business events;
admitted records still use the existing independently bounded file/export
queues and their health counters.

Command/event replay keeps original context and does not emit duplicate spans.
Epoch/reset/high-water fences precede admission; the Controller separately
rejects wrong-Machine/session/nonce/trace records, duplicate span IDs and
implausible clocks. Restart or expiry discards correlation authority rather
than reconstructing it from conversation history. Delivery and dedup are
best-effort within these bounds, not a durable exactly-once guarantee.

Regression coverage uses real Controller pending commands, broker routing and
dedup, worker command handling, framed IPC and Controller projection, followed
by OTLP admission into the rotating file. It verifies all seven linked runtime
spans, command/event replay, no conversation content in telemetry and no trace
metadata in ACP or business events. The real ACP conformance peer additionally
checks configuration-before-prompt ordering, pre-RPC rejection/cancellation and
completion hooks. These fixtures do not constitute production or device
acceptance and do not invoke a real model.

## Runtime extension verification (2026-09-08)

`nix develop -c just check-compact` passed with 725 Rust library tests (8
environment-dependent tests ignored), 1186 frontend tests, 6 separately run
isolated PostgreSQL tests and the complete formatting/lint/dependency,
SDK/Plugin, native-shell/site, feature-slice and production-build gates.
The nine additional Rust tests cover the runtime boundaries above, including
bounded outbox recovery, forged/mismatched correlations and completion-aware
capacity reclamation. Existing ACP conformance tests now assert actual timing
hook placement as well. Clean-commit Nix release receipts are verified
separately; this is not authorization to publish or activate either component.

## Signal and export bounds

Two monotonic counters (`websocket.reconnects`, `long_tasks`) use `{event}`;
six histograms (`websocket.connect`, `websocket.reconnect`, `long_task`,
`navigation`, `command`, `first_output`, all suffixed `.duration`) use seconds
under `cowboy.client.`. Histograms have fixed explicit buckets from 5 ms to
120 s plus the overflow bucket. Each SDK instrument has a 32-series limit;
the Controller has 512 aggregate streams and rejects unsupported types,
temporality, non-finite/negative values or changed boundaries before mutation.
Queue reservation and dedup precede cumulative-state commitment. Controller
restart resets its single-writer cumulative epoch. Multiple Controllers must
not export the same aggregate resource without distinct writer identity or a
shared aggregation design.

OTLP requests use `application/x-protobuf`, a stable `batch_id` query parameter,
and existing same-origin product authentication. HTTP 200 acknowledges bounded
admission, not fsync or external delivery. SDK queues cap at 64 logs / 64 spans,
16 per export; one transport owns retries with a combined 200-item / 256-KiB
budget, 24-KiB requests, five attempts / five minutes, and an 8-second flush
deadline. Page-exit beacons spend at most 48 KiB per pass. All responses are
bounded to 64 KiB. The Controller accepts at most 200 items / 256 KiB per
request and accounts for protobuf-to-JSON expansion in its 8-MiB pending budget
(1 MiB per retained batch, 32 KiB per local record).

Browser session/Machine/user identity claims and arbitrary resource attributes
are removed at ingestion. Only finite platform/surface attributes survive on
the client resource. Logs/spans carry a Controller-derived owner hash; metrics
carry no owner, client, build, session or trace identity labels. Explicit
client trace IDs are correlation hints, never authentication or visibility
proof. Incident associations keep their separate existing authorization.

`web/src/otelFixture.ts` produces `tests/fixtures/otel-client.json` using the
real SDK/serializer. Rust tests decode this fixture and verify redaction, trace
correlation, delta aggregation, dedup and full-queue retry behavior. Signed
temporary Machine tests exercise all three routes, 503 retry, partial success,
exact policy, upgrade/uninstall/rollback and no Provider credential state.
No real Victoria service or physical iOS device was exercised by these tests.

The export fixture also checks HTTP 200 with an empty body and absent response
Content-Type, as used by the upstream
[VictoriaMetrics response handler](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/master/lib/protoparser/opentelemetry/firehose/http.go)
and [VictoriaTraces HTTP handler](https://github.com/VictoriaMetrics/VictoriaTraces/blob/master/app/vtinsert/opentelemetry/otlphttp.go).
An empty protobuf message is a full-success receipt; this does not relax the
protobuf request type or allow redirects and arbitrary successful HTML bodies.

The complete gate exposed a local-writer teardown race: `flock` can remain on
an inherited/duplicated open-file description after the owner's descriptor is
closed. The acquired lock now explicitly unlocks on all teardown/error paths;
a deterministic duplicate-descriptor test covers this without retrying or
weakening writer exclusion. See the [Linux lock semantics](https://man7.org/linux/man-pages/man2/flock.2.html).

Initial production-build comparison against the local-first baseline 30734017:
all 323 JavaScript assets total 9,813,221 bytes versus 9,693,560; independently
gzipped total 2,996,351 versus 2,963,809 (+32,542 bytes, about 31.8 KiB). This is
an all-chunks transfer-size comparison, not a cold-start CPU or iPhone latency
measurement. Subsequent small fixes and final immutable builds are checked
separately; no physical-device performance acceptance is inferred from size.

Protocol references: [OTLP](https://opentelemetry.io/docs/specs/otlp/),
[JavaScript status](https://opentelemetry.io/docs/languages/js/),
[metric single-writer semantics](https://opentelemetry.io/docs/specs/otel/metrics/data-model/#single-writer),
[Victoria OTLP support](https://docs.victoriametrics.com/opentelemetry/).
