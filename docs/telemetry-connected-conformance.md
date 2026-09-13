# Connected immutable telemetry conformance

`just telemetry-connected-conformance <matrix.json> <new-receipt.json>` runs
independent immutable Controller and Machine processes, connected through a
loopback WebSocket fault proxy. It supplements the separate
[reader](telemetry-reader-conformance.md),
[writer-policy](telemetry-writer-conformance.md) and
[background startup](telemetry-background-startup-conformance.md) gates. Run
from clean committed Cowboy source in the pinned Linux shell.

The
[2026-09-13 acceptance receipt](releases/telemetry-connected-conformance-2026-09-13.md)
records the actual Hawk roles, accepted matrix, failed harness attempts and
independently checked production continuity.

The closed schema-one matrix is the same as the reader/writer matrix:
`controller` and `machine`, each with `active`, `rollback` and `cold` absolute
immutable release roots. Every Controller role crosses every Machine role; four
flows over the complete 3×3 product give **36 required results**. Duplicated
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
round-trip/lost-ACK; binding plus Service resolution on the Controller for
disconnect; recovery plus Service resolution on the Controller and recovery only
on the Machine for Prepared recovery. Recovery never implicitly enables binding.

The proxy forwards original message bytes unchanged. It may drop exactly one
selected acknowledgment or close a connection; it cannot synthesize a success
reply, command, timestamp or lease. It records bounded frame-kind counters, not
raw frames. Unexpected Plugin/Provider mutations, worker/session commands and
telemetry export commands fail the gate. No worker executable is supplied. The
normal Core-to-empty-broker `SetDesiredGeneration` startup frame is allowed
exactly once per connection, only for the authenticated Machine's own generation
and only without a worker executable. Readiness waits for this frame as well as
the authenticated connection and actual inventory/audit. A replacement
generation, executable, duplicate initialization or session command is rejected.

| Flow                                 | Required behavior                                                                                                                                                                                                                       |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Binding round trip                   | Preview creates no namespace; logout rejects the old cookie; Controller restart loses the unsubmitted preview; connection replacement cannot revive another preview; separately confirmed select/revoke/restore advances counters 1→2→3 |
| Lost binding acknowledgment          | Machine receives exactly one write; the real 45-second ACK deadline ends; Service performs one fallback observation, not another write, and records completion                                                                          |
| Disconnect after Machine application | Dropped ACK and connection loss leave Service `NeedsAttention`; a new connection cannot adopt the result; fresh separate Service resolution must accept definite Applied evidence                                                       |
| Reopened Prepared recovery           | Real prepared interruption fixture; one recovery command, dropped ACK and the unchanged 15-second deadline; one fallback observation; Service evidence stays unchanged until a separately confirmed resolution records rejection        |

Each flow then restarts **both** child processes against the same durable state.
The proxy records monotonic command-to-fallback-query time; lost-ACK flows
require at least 44/14 seconds respectively (one-second observation margin
around the unchanged real 45/15-second runtime deadlines), not just a query
count. Real cookies remain readable, consumed confirmations cannot be reused,
and historical receipt reads cannot send mutations. New read-only previews query
the reopened Machine head. Recovery additionally requires protocol-18 audit
discovery and exact receipt lookup after the Controller's process-local handle
is gone, including after Service resolution changed the operation's phase.

Evidence is checked between steps: Service document/checksum and structural
ledger validation, exact Machine-file preservation for reads and Service-only
resolution, and changed Machine bytes for admitted effects. Returned intent,
request digest, actor, installation, head and counters must agree. Policy bytes,
device/inode, owner, mode, ctime and link target remain unchanged across the
flow. Reopened children must validate their journals; no test rewrites an
observed result to make it compatible. The original finite writes and
interruption used to seed recovery are disposable test preparation, not fault
injection into a production process or proof of physical power-loss behavior.

After reopen, authenticated actual OTLP protobuf logs, metrics and traces must
grow private local JSONL files without failures or remote attempts. A successful
binding/recovery cannot implicitly create a background exporter. External
Victoria delivery and background-policy activation remain separate.

## Isolation, receipt and limits

Compile before entering the non-root private network namespace, then execute
offline with loopback only. All child environments are cleared and their state,
workspace, socket and telemetry paths are disposable. Three pairs run at most
concurrently; each flow has a 125-second ceiling, bounded HTTP/frame/log capture
and verified child-process-group/proxy cleanup. Timeout or cleanup failure is
not a skipped or successful flow.

The private, atomic create-only schema-one receipt hashes clean source,
manifests/launchers/actual ELF chains, pinned SSH helper, signed fixture package
and release, installation and before/after journals. Closed fields identify
artifact roles, flow, last stage, protocol, wire counters and failure category.
They also identify the two finite-purpose policies, elapsed times, last HTTP
status/closed result category and relay-rejection or connection-stop markers. No
password, cookie, key, frame, raw policy, endpoint, exception or log is copied.
Use a new absolute output path in an existing directory. A failed required flow
writes `accepted: false` and exits nonzero. Do not overwrite failed evidence.

This is **authenticated synthetic cross-process acceptance**, not acceptance of
the actual host's complete configuration or production Operator account. It does
not establish production PostgreSQL startup, external OTLP delivery, arbitrary
network/filesystem faults, physical-device interaction, existing session
continuity or deployment. Independently capture actual roles and continuity, and
follow the [writer cutover contract](telemetry-writer-admission.md). Never open
production writers alone, erase their evidence, or mistake reversible binding
configuration for reversal of already emitted telemetry (`NoRestore`).
