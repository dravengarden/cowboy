# Connected immutable telemetry conformance

`just telemetry-connected-conformance <matrix.json> <new-receipt.json>` runs
independent immutable Controller and Machine processes, connected through a
loopback WebSocket fault proxy. It supplements the separate
[reader](telemetry-reader-conformance.md),
[writer-policy](telemetry-writer-conformance.md) and
[background startup](telemetry-background-startup-conformance.md) gates. Run
from clean committed Cowboy source in the pinned Linux shell.

The initial
[36-result acceptance](releases/telemetry-connected-conformance-2026-09-13.md)
records the first four flows and failed harness attempts. The subsequent
[45-result managed-delivery acceptance](releases/telemetry-managed-delivery-conformance-2026-09-13.md)
adds the fifth flow and fresh actual-role/production-continuity evidence.
The separate [Victoria database gate](telemetry-victoria-conformance.md) adds
real isolated database ingestion/query/reopen checks; this protocol receiver
remains required for the full 45-flow fault contract.

The closed schema-one matrix is the same as the reader/writer matrix:
`controller` and `machine`, each with `active`, `rollback` and `cold` absolute
immutable release roots. Every Controller role crosses every Machine role; five
flows over the complete 3×3 product give **45 required results**. Duplicated
paths still represent separately declared roles, not proof of host provenance.
Resolve roles independently; a Machine bootstrap is allowed only in the cold
role, never as an activation candidate. No optional role, flow or protocol skip
can count as acceptance.

## Actual boundaries exercised

The harness seeds an isolated migrated SQLite store with a new fixture account,
random Argon2id password, explicit Operator policy and an enrolled temporary
Machine public key. The real Controller starts with product authentication
**enabled**, CoreSecurity, private writer policy and the fixture Catalog. It
must reject anonymous telemetry access, verify the password through its actual
login route, and issue a genuine cookie resolved by its production middleware.
There is no local-auth fallback, pre-forged session cookie, admin authority or
production credential. Account registration, Passkeys and production identity
providers are not exercised by this fixture.

The fixture creates and verifies a temporary signed Victoria release, installs
it through the real Machine store, and supplies its exact installation in the
Catalog and private dummy destination policy. It does not install over the live
network or publish that fixture. Both processes perform the real enrolled SSHSIG
challenge/response; all connections must negotiate protocol **18**. The
Controller discovers the actual installation from accepted Machine inventory.
Each flow enables only its finite writer purposes: binding on both Sites for
round-trip/lost-ACK/managed delivery; binding plus Service resolution on the
Controller for disconnect; recovery plus Service resolution on the Controller
and recovery only on the Machine for Prepared recovery. Recovery never
implicitly enables binding.

The proxy forwards original message bytes unchanged. It may drop exactly one
selected acknowledgment or close a connection; it cannot synthesize a success
reply, command, timestamp or lease. It records bounded frame-kind counters, not
raw frames. Unexpected Plugin/Provider mutations and worker/session commands
fail the gate. Only the managed-delivery flow permits telemetry export, and only
typed bound attempts for its exact fixture installation, with fresh request
digests and correlated actual receipts. Legacy export remains forbidden. No
worker executable is supplied. The normal Core-to-empty-broker
`SetDesiredGeneration` startup frame is allowed exactly once per connection,
only for the authenticated Machine's own generation and only without a worker
executable. Readiness waits for this frame as well as the authenticated
connection and actual inventory/audit. A replacement generation, executable,
duplicate initialization or session command is rejected.

| Flow                                 | Required behavior                                                                                                                                                                                                                       |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Binding round trip                   | Preview creates no namespace; logout rejects the old cookie; Controller restart loses the unsubmitted preview; connection replacement cannot revive another preview; separately confirmed select/revoke/restore advances counters 1→2→3 |
| Lost binding acknowledgment          | Machine receives exactly one write; the real 45-second ACK deadline ends; Service performs one fallback observation, not another write, and records completion                                                                          |
| Disconnect after Machine application | Dropped ACK and connection loss leave Service `NeedsAttention`; a new connection cannot adopt the result; fresh separate Service resolution must accept definite Applied evidence                                                       |
| Reopened Prepared recovery           | Real prepared interruption fixture; one recovery command, dropped ACK and the unchanged 15-second deadline; one fallback observation; Service evidence stays unchanged until a separately confirmed resolution records rejection        |
| Managed OTLP delivery                | Real client intake, local recording, Controller queue, bound Machine RPC and isolated OTLP HTTP receiver; 16 stages cover explicit activation, failure, ACK loss, revoke/restore and restart without replay                             |

The first four flows then restart **both** child processes against the same
durable state. The proxy records monotonic command-to-fallback-query time;
lost-ACK flows require at least 44/14 seconds respectively (one-second
observation margin around the unchanged real 45/15-second runtime deadlines),
not just a query count. Real cookies remain readable, consumed confirmations
cannot be reused, and historical receipt reads cannot send mutations. New
read-only previews query the reopened Machine head. Recovery additionally
requires protocol-18 audit discovery and exact receipt lookup after the
Controller's process-local handle is gone, including after Service resolution
changed the operation's phase.

Evidence is checked between steps: Service document/checksum and structural
ledger validation, exact Machine-file preservation for reads and Service-only
resolution, and changed Machine bytes for admitted effects. Returned intent,
request digest, actor, installation, head and counters must agree. Policy bytes,
device/inode, owner, mode, ctime and link target remain unchanged across the
flow. Reopened children must validate their journals; no test rewrites an
observed result to make it compatible. The original finite writes and
interruption used to seed recovery are disposable test preparation, not fault
injection into a production process or proof of physical power-loss behavior.

After reopen in these four flows, authenticated actual OTLP protobuf logs,
metrics and traces must grow private local JSONL files without failures or
remote attempts. A successful binding/recovery cannot implicitly create a
background exporter.

## Managed-delivery flow

The fifth flow adds a loopback-only protocol receiver, **not a Victoria
database**. The installed signed fixture's real Machine exporter sends official
OTLP protobuf over HTTP to the three Victoria-compatible routes, with a
synthetic bearer token. The receiver decodes each signal, checks cumulative
metric temporality and stores only counts, signal, response class and payload
hash. The proxy independently checks the bound attempt's owner, installation,
digest and actual export receipt; its payload hash must equal the receiver's
hash. Neither component saves raw telemetry, request IDs or credentials in the
receipt.

All nine role pairs must complete these stages in order:

1. Unconfigured and then binding-only: all signals stay local, with no exporter.
2. Explicit exact-head policy activation: all three signals are delivered.
3. Protobuf partial success, HTTP 503, 429 and 307: exact rejection/failure
   accounting, local recording preserved, no retries or redirect following.
4. Drop one actual success receipt and wait for the real 15-second ACK deadline;
   then disconnect after another delivery. Both are uncertain to the Controller,
   not invitations to resend. A new connection delivers only new batches.
5. Revoke and restore binding: the original policy cannot export under either
   changed head. Reopen both children with the stale policy: optional export
   stays stopped, local intake remains healthy.
6. Explicitly activate a new exact-head policy, then reopen both children with
   it: only new batches export, never the local file or an old queue.
7. Activate a metrics-only policy: logs/traces still record locally; only
   metrics reach the Machine and receiver.

These are 16 individually recorded rounds. Every submitted batch is repeated
once with the same identity to require deduplication before aggregation/export.
The two ACK fault rounds use one log batch each; other rounds use the four SDK
fixtures, including two metric batches. Every round requires bounded local-file
growth, exact intake/dedup/remote-failure counters, one-to-one RPC/HTTP
observations and unchanged durable binding journals. Newly created background
policy files retain their bytes and private inode/ownership metadata throughout
the flow. Reactivation uses new explicit policy files, not a rewrite of observed
evidence. Emitted telemetry is `NoRestore`; only binding configuration is
restored.

## Isolation, receipt and limits

Compile before entering the non-root private network namespace, then execute
offline with loopback only. All child environments are cleared and their state,
workspace, socket and telemetry paths are disposable. Three pairs run at most
concurrently; each flow has a 125-second ceiling, bounded HTTP/frame/log capture
and verified child-process-group/proxy/receiver cleanup. Timeout or cleanup
failure is not a skipped or successful flow.

The private, atomic create-only schema-one receipt hashes clean source,
manifests/launchers/actual ELF chains, pinned SSH helper, signed fixture package
and release, installation and before/after journals. Closed fields identify
artifact roles, flow, last stage, protocol, wire counters and failure category.
They also identify the two finite-purpose policies, elapsed times, last HTTP
status/closed result category and relay-rejection or connection-stop markers.
Managed-delivery results additionally contain the 16 stage outcomes, bounded
counter deltas, policy hashes and correlated payload-free RPC/HTTP records. No
password, cookie, key, frame, raw policy, endpoint, exception or log is copied.
Use a new absolute output path in an existing directory. A failed required flow
writes `accepted: false` and exits nonzero. Do not overwrite failed evidence.

This is **authenticated synthetic cross-process acceptance**, not acceptance of
the actual host's complete configuration or production Operator account. It does
not establish production PostgreSQL startup, real Victoria ingestion/query,
external OTLP delivery, arbitrary network/filesystem faults, physical-device
interaction, existing session continuity or deployment. Independently capture
actual roles and continuity, and follow the
[writer cutover contract](telemetry-writer-admission.md). Never open production
writers alone, erase their evidence, or mistake reversible binding configuration
for reversal of already emitted telemetry (`NoRestore`).
