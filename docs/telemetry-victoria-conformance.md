# Connected Victoria database conformance

`just telemetry-victoria-conformance <matrix.json> <databases.json> <new-receipt.json>`
checks actual VictoriaLogs, VictoriaMetrics and VictoriaTraces ingestion and
querying through immutable Cowboy Controller/Machine processes. Run in the
pinned Linux shell from clean committed source. This is an additional gate,
not a replacement for the mandatory 45-flow
[protocol/fault receiver gate](telemetry-connected-conformance.md).

## Inputs and isolation

The Controller/Machine matrix has the same closed active/rollback/cold schema
as the reader and connected gates. All nine role pairs are required. The
separate database input has exactly these fields:

```json
{
  "schema": 1,
  "logs": "/nix/store/<exact-output>/bin/victoria-logs",
  "metrics": "/nix/store/<exact-output>/bin/victoria-metrics",
  "traces": "/nix/store/<exact-output>/bin/victoria-traces"
}
```

These placeholders are not executable examples. Supply independently selected,
canonical immutable ELF paths, not mutable wrappers or an ambient executable
lookup. The gate hashes their actual bytes and Cowboy's release manifests and
executable chains. It does not download, install, upgrade or select a database
version automatically. Independently bind these paths to the intended target;
accepting a supplied triple alone does not establish production provenance.

There is no endpoint, credential, environment, arbitrary argument or existing
storage-directory input. Compile before entering the non-root private network
namespace; the test requires loopback to be its only interface. Every pair
starts three new database processes with new disposable storage and cleared
environments. Their fixed test profile uses literal loopback addresses,
one-day retention, 128 MiB cache budgets and `GOMAXPROCS=2` per process.
VictoriaMetrics keeps native OTel metric naming rather than enabling
Prometheus renaming. These options are test configuration, not acceptance of
all production flags or a total process-memory limit.

Pairs run sequentially. Process groups, HTTP bodies and captured diagnostics
are bounded; cleanup is required. No production data, account, endpoint policy,
Provider credentials, host unit or Agent session is accessed by the test.
The Victoria Plugin remains a data-only connector, not an installer or a
dependency on these databases for ordinary Cowboy builds or operation.

## Required observations

The existing fixture supplies a temporary signed Victoria installation, exact
Catalog, independently enrolled Machine, actual password login and finite
writer policies. The real authenticated Controller/Machine connection must
negotiate protocol 18. The proxy rejects unexpected mutations and all session
commands. No forged cookie, disabled authentication or production login is used.

Each pair must:

1. Submit standard client SDK fixtures while unconfigured, then after a genuine
   separately confirmed binding. Both phases record locally; all three actual
   databases must still be empty.
2. Explicitly activate an exact-binding background policy. Submit a correlated
   log/span, counter and histogram through authenticated Cowboy OTLP intake.
   Every batch is submitted twice with the same identity to check deduplication.
   Fixture timestamps are moved to the current test window without replacing
   the SDK's encodings, dimensions, values or correlated identities.
3. Require four real bound RPCs/receipts and four actual database HTTP attempts,
   correlated by signal, payload hash and item count. The HTTP relay validates
   the synthetic token and closed routes, strips that token before forwarding,
   and forwards the database's actual status, content type and response bytes.
   It never synthesizes a successful database response, follows redirects or
   retries. Partial/error results cannot count as full delivery.
4. Query the exact stored log text, timestamp and trace/span IDs; query every
   expected cumulative counter/histogram series, dimension, bucket, value and
   sample timestamp; query the trace by ID through the actual Jaeger API and
   check span identity, operation, start time and duration. Empty HTTP 200s or
   successful intake alone do not satisfy these checks.
5. Gracefully stop all three database processes, reopen their same private
   stores and repeat the queries. Then restart the Controller and Machine;
   durable binding/cookie reads remain valid and the old local files/queues
   must not emit another batch.

Local-file growth, deduplication, lane failures, rejected items, queue drops,
unchanged binding journals and original private policy ownership are checked
through the shared connected harness. Database visibility is polled within a
bounded window; absent or mismatched results fail, not skip.

The query contracts are the official
[VictoriaLogs HTTP API](https://docs.victoriametrics.com/victorialogs/querying/),
[VictoriaMetrics OTel integration](https://docs.victoriametrics.com/victoriametrics/integrations/opentelemetry/)
and [VictoriaTraces Jaeger API](https://docs.victoriametrics.com/victoriatraces/querying/).
This narrow acceptance covers the supplied versions and fixed profile, not
arbitrary future API or naming changes.

## Receipt and production boundary

The private atomic create-only receipt distinguishes this database purpose
from protocol-receiver acceptance. It includes clean harness source, all
immutable executable hashes, nine role outcomes, correlated payload-free
transport observations, closed actual database HTTP status/media-type/body-hash
observations, two semantic query results per pair, database reopen
and host-restart/no-replay results. Fixture payloads needed for comparison stay
bounded in memory; raw query responses, logs, credentials, endpoints and
telemetry are not serialized into the receipt. Failed checks exit nonzero and
retain failed evidence under a different path from later success.

A graceful database reopen is not crash/power-loss durability. This gate does
not accept production destination authentication, TLS, PostgreSQL startup,
real Operator authority, external delivery, production failure/restart behavior
or a managed policy cutover. Those remain independent P2 prerequisites under
[writer admission](telemetry-writer-admission.md). First managed intent can
permanently fence legacy export; passing this gate grants no permission to
open production writers, replace host configuration or erase that evidence.
Already emitted telemetry remains `NoRestore`.
