# Managed background startup conformance

`just telemetry-background-startup-conformance <matrix.json> <new-receipt.json>`
executes immutable Controller releases with explicit synthetic managed policy
and populated temporary Service journals. Run from clean committed source in
the pinned Linux shell. This supplements, not replaces, the two-Site
[reader gate](telemetry-reader-conformance.md) and signed Victoria runtime tests.

The closed schema-one input requires every Controller role:

```json
{
  "schema": 1,
  "controller": {
    "active": "/nix/store/<hash>-cowboy-controller-release",
    "rollback": "/nix/store/<hash>-cowboy-controller-release",
    "cold": "/nix/store/<hash>-cowboy-controller-release"
  }
}
```

Resolve real immutable paths; placeholders cannot run. The same provenance and
actual ELF hashing checks as the reader gate apply. No environment, private
policy, endpoint, credential or optional/skipped role can be supplied. Duplicated
paths prove only those bytes, not the host's current rollback or bootstrap floor.

## Required behavior

Thirteen cases, three roles and two cold reads give **78 checks**:

| Case | Core startup | Background queue |
| --- | --- | --- |
| Unconfigured with completed history | Ready | Absent; history cannot activate export |
| Configured, absent binding | Ready | Stopped |
| Exact completed binding | Ready | Active |
| Subsequent Prepared or Unknown operation | Ready | Stopped |
| Subsequently aborted operation, exact head unchanged | Ready | Active on this new explicit startup |
| Revoked, restored or subsequently advanced head | Ready | Stopped; restoration is a new epoch |
| Bad checksum or re-checksummed invalid recovery audit | Rejected by mandatory journal recovery | None |
| Invalid policy schema or conflicting managed/legacy modes | Rejected for the specific configuration error | None |

The finite Service writer and validated observations build synthetic selected,
revoke/restore and interrupted evidence. The corrupt audit fixture uses the same
real temporary Machine recovery and Service resolution as reader conformance.
No Plugin, Machine destination or production account is needed.

Both reads reopen the **same** temporary journal, policy and telemetry directory.
Every successful startup requires `/healthz`, the exact closed writer/fence log,
and the expected bounded background activation log (or its absence in unconfigured
mode). It then submits the official client protobuf fixtures for logs, metrics
and traces through the actual HTTP intake. Private local JSONL records must grow;
creating an empty file is not acceptance. Local/drop counters must remain zero.

No Machine is connected. An active queue therefore consumes each new batch once
as not-admitted, with exactly one failed batch per signal. Stopped/unconfigured
modes must leave all remote counters zero. Each cold start begins with zero
accepted batches: retained local files cannot be replayed. Journal document and
checksum, Machine fixture bytes and policy bytes must remain unchanged after
every child exits. Timeout, wrong rejection reason, unsuccessful process-group
cleanup and unexpected state fail the check.

## Evidence and limits

The recipe compiles before entering a non-root network namespace with only
loopback, then executes offline with cleared child environments and private
temporary state. It reuses the reader harness's bounded log capture and child
cleanup. No production credential or network destination is inherited.

The private, atomic create-only schema-one receipt records source/artifact/fixture
hashes, role, case, cold read, checked export state and closed failures. It never
copies logs, exceptions or policy content. A failed matrix writes
`accepted: false` and exits nonzero; never overwrite or label it acceptance.

This is **synthetic startup-contract acceptance**, not complete production
configuration acceptance. Older reader-compatible Controllers may reject the
new flag or retain the old stale-policy startup failure. Before a real cutover,
independently bind the tested roles to active profiles, the next transaction's
recovery target and the actual host closure's cold outputs, and accept the full
owned startup configuration including writer admission. Machine startup,
authenticated cross-end mutation/failure/restart, external OTLP delivery,
PostgreSQL startup, existing-session continuity and host activation remain
separate gates. Production policy is not changed by running this command.
