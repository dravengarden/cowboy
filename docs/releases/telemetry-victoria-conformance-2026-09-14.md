# Real Victoria database conformance: 2026-09-14

The new [database gate](../telemetry-victoria-conformance.md) passed all **nine
Controller × Machine role pairs** against isolated copies of Hawk's actual
VictoriaLogs 1.52.0, VictoriaMetrics 1.148.0 and VictoriaTraces 0.9.3
executables. Every pair passed actual ingestion/query, same-store graceful
database reopen and Cowboy restart without replay. This closes the real-database
gap left by the protocol receiver, not production managed cutover or P2 exit.

## Source and acceptance

Accepted clean harness source: `5b6e4276d796315365f5ac64aec806fc8d214eb0`.
Changes are test-only Rust helpers, one recipe and documentation, including the
repository-owned release skill. No runtime code, protocol, durable format, SQL
migration, signed Plugin, SDK, native ABI or production policy changed. Per the
canonical release workflow, no runtime activation is needed merely to refresh
source metadata for this test/documentation publication.

`nix develop -c just check-compact` passed with the invoking Provider package
override removed: **1,116 all-feature Rust library tests** (23 explicitly
ignored), **272 standalone Machine tests** (2 ignored), **1,431 Web tests**, all
**15 isolated PostgreSQL tests**, binary/adapter tests, format, lint, dependency
and feature checks, and release builds. These are separate, overlapping suites.
The existing Vite chunk-size warning remains non-fatal.

Both immutable gates ran offline from that same clean revision, using private
loopback-only namespaces, cleared child environments and disposable storage:

| Gate                             | Required observations                                          | Result   |
| -------------------------------- | -------------------------------------------------------------- | -------- |
| Actual Victoria databases        | 9 pairs, 18 query rounds, 36 correlated RPC/HTTP exports       | Accepted |
| Existing protocol/fault receiver | 45 flows, 144 delivery rounds, 324 correlated RPC/HTTP exports | Accepted |

The database gate took 233.76 seconds; the separate protocol regression took
313.00 seconds. No role or fault flow was skipped. The six new ordinary tests
cover closed database inputs, transparent bounded forwarding, empty-response
headers, exact metric values/labels, duplicate or changed metrics, and exact
correlated log/trace query results.

Each real-database pair used actual fixture password authentication, an
independently enrolled temporary Machine and a temporary signed Victoria
installation. Unconfigured and binding-only phases still recorded locally and
left all three databases empty. Only the separately activated exact-binding
background policy permitted four export attempts. Duplicate submissions did not
duplicate export or aggregation.

Queries required the correlated log and span, exact text/IDs/timestamps, and all
**18 metric series**: counter `1`, histogram count `1`, sum `0.125`, exact
cumulative buckets, finite dimensions, aggregate service and native scope
labels. Every database reopened its same store and returned the same verified
semantic digest. Subsequent Controller/Machine restart did not replay the old
queue or local files. All 36 actual database responses were HTTP 200 with an
empty body and no Content-Type; the relay preserved that form.

## Failed evidence and harness corrections

Three unsuccessful database matrices remain retained, never overwritten or
counted as acceptance:

1. The first recorded six observation failures and three cleanup failures.
   Axum's `Vec<u8>` response conversion had added `application/octet-stream` to
   the real databases' empty successes. A dedicated failing regression
   reproduced this. Explicit response construction now preserves the original
   status, Content-Type presence and bytes; closed database response metadata is
   recorded separately from intended relay mode.
2. The second reached successful transport but all nine metric query checks
   failed. VictoriaMetrics 1.148.0 promotes `scope.name` and `scope.version`,
   not the initially assumed names. The corrected oracle is stricter: it checks
   exact metadata values and an independent SDK-fixture value/dimension
   contract, rejecting missing/extra labels and repeated counts or samples.
3. The third exhausted its 12-second query window before Jaeger trace-ID
   visibility. VictoriaTraces 0.9.3's default index flush interval is 20
   seconds. The gate now allows 35 seconds per query round and records
   non-success query HTTP statuses by signal. It does not force flushes or
   shorten the database's real indexing interval. Missing or wrong query results
   still fail.

The first ordinary test run also exposed missing explicit TLS-provider setup in
the new fixture HTTP clients; this was corrected in the harness. These failures
do not demonstrate a production exporter or database defect. The guide links the
pinned upstream field/index implementations.

## Actual target and unchanged production boundary

Read-only captures at `2026-09-14T11:21:19+08:00` and
`2026-09-14T12:02:10+08:00` agree on host closure/source, component profiles and
receipts, Controller/Machine PID/start, all three live Victoria PID/start/ELF
hashes, and the independently resolved role matrix. No component transaction was
in progress. The current production Machine writer policy and binding journal
remain absent. `/healthz` returned `ok`; Machine remained connected and online
with desired generation `worker-240c2080a8bf9eb8968f`. This is not an inspection
of every existing worker or native session.

The role matrix uses the same outputs recorded after the
[authorized Machine maintenance](machine-writer-preflight-activation-2026-09-14.md):

| Site / role                       | Immutable release                                                              | Embedded revision |
| --------------------------------- | ------------------------------------------------------------------------------ | ----------------- |
| Controller active / next rollback | `/nix/store/3wbj6ky4kqp5b88fiv7p3h2pxb28lifn-cowboy-controller-release`        | `77756dfa`        |
| Controller cold                   | `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`        | `c1a7752f`        |
| Machine active / next rollback    | `/nix/store/w0dszrabai56bar9zsc59m6bbqwvb5bn-cowboy-machine-release`           | `327b7f9e`        |
| Machine cold                      | `/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release` | `c1a7752f`        |

Next-transaction rollback is the current active release, not the prior
transaction's historical `previousRelease`. Cold outputs were resolved from the
actual active system closure, not the stable Columbus checkout. Actual database
executables were resolved from the three live systemd main processes:

| Database | Exact immutable program                                                                    |
| -------- | ------------------------------------------------------------------------------------------ |
| Logs     | `/nix/store/g35w4vsn12adscgkjpa2k774gj5kp0wr-VictoriaLogs-1.52.0/bin/victoria-logs`        |
| Metrics  | `/nix/store/bvwxphp231gc8bpdarwkk5a9xf7hsxax-VictoriaMetrics-1.148.0/bin/victoria-metrics` |
| Traces   | `/nix/store/az43sqrrjsyg6qpv1vyz218r7q94kdwa-VictoriaTraces-0.9.3/bin/victoria-traces`     |

The independent audit rehashed every manifest, launcher/ELF chain and database
program, bound them to the captured roles and required all receipt outcomes.
After testing, only the original three production Victoria processes remained;
no temporary database process remained running.

## Retained receipts and remaining work

Private evidence: `/tmp/cowboy-victoria-database.eufm5G/`. Receipts are atomic,
create-only, mode 0600; raw query responses and fixture payloads are not copied
into them. Quality logs, failed test logs, before/after captures and the
read-only audit are retained separately.

| Receipt                    | SHA-256                                                            |
| -------------------------- | ------------------------------------------------------------------ |
| Accepted `victoria-4.json` | `792ab038873c3cc82fbfbbd4420daf22aa89f8708473fffbc65579c4de60772a` |
| Accepted `connected.json`  | `3f47592081153b16c915103695147401e2de62d6335b74a04dafd8f9d5870fc3` |
| Failed `victoria-1.json`   | `6ff081ac03433ff6c8c68fc48cab870f5096912bbf781734f82a4ff57a2793cb` |
| Failed `victoria-2.json`   | `57b5e1ce2e1180631991a351244ac6fa168e986d79fc078f2eb085d7a3b5c9e7` |
| Failed `victoria-3.json`   | `5098d656c6d210852a6685d461414463e32bfd16292e0e744e9614894f9fd739` |

Matrix SHA-256:
`7dfefbffb4e431e2302eef1c6976562df6cfa29d82199aa420ac28f1828d5d19`. Database
input SHA-256:
`bc3cc179dd473484645dfb77df6dff6c1191d3449593972b179557802e9f9384`.

Still separate: complete intended production configuration, real Operator
confirmation, destination authentication/TLS, production managed policy cutover
and production failure/restart acceptance. This gate did not query or write
production Victoria data. Graceful database reopen is not crash or power-loss
durability. No production component, Plugin installation, endpoint policy,
Provider state or legacy-export fence was changed. First managed intent can
permanently fence legacy export; emitted telemetry remains `NoRestore`. P2 and
the later generic composition/lifecycle work remain incomplete.
