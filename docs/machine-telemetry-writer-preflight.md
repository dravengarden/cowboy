# Machine telemetry writer-policy preflight

The Machine core provides a configuration-only diagnostic:

```bash
/absolute/immutable-release/bin/cowboy-machine \
  --state-dir /absolute/intended-machine-state \
  --check-telemetry-writer-policy
```

Run the exact candidate as the actual Machine user, not root. The only optional
policy input is `<state-dir>/telemetry-writer-policy.json`; there is no second
policy-path selector or Plugin-supplied validator. No Controller URL is required.
This mode and `--provider-usage-status` are mutually exclusive. The existing
usage-spool diagnostic remains independent of telemetry writer configuration.

## Checked configuration, not authority

The diagnostic uses the same `WriterAdmission::load_optional` as normal journal
startup: closed schema, required fields and purpose booleans, bounded owner
identifiers, exact fence acknowledgment, and owned private single-linked regular
file checks. The read is bounded to 64 KiB and nonblocking. Final symlinks,
dangling symlinks, hard links, directories, FIFOs, unsafe permissions, malformed
or oversized bytes fail; only actual absence means `unconfigured`.

An absent state directory is not created. With a valid file the JSON is:

```json
{
  "schema": "dravengarden.cowboy.machine-telemetry-writer-preflight/v1",
  "writer_policy": {
    "state": "configuration_valid",
    "declared_purposes": {
      "binding": true,
      "machine_recovery": false,
      "service_resolution": false
    }
  },
  "not_checked": [
    "runtime_and_enrolled_owner_identity",
    "other_machine_startup_configuration",
    "binding_journal_and_writer_activation",
    "signed_installation_and_private_destination_policy",
    "operator_authorization_and_connection",
    "local_recording_and_remote_delivery"
  ]
}
```

Without a file, `writer_policy` is exactly `{"state":"unconfigured"}`. Purpose
booleans are declarations, not executable scopes or permissions. In particular,
declaring `service_resolution: true` never delegates Service bookkeeping to a
Machine. Require the exact schema and expected closed fields, not just exit 0
or an older binary's help output. A failure exits nonzero without a successful
report. Reports omit owner values, paths, digests, binding heads, destinations,
credentials and payloads; configuration parse/validation errors do not echo
unknown private keys or values.

Unlike the [Controller preflight](telemetry-policy-preflight.md), this diagnostic
does **not** check effective Service/Machine ownership. Persisted Machine identity
can override CLI input, and enrollment can replace it again. It neither reads
that identity nor enrolls to guess the effective owner from policy declarations.
Actual execution still checks the independently authenticated exact owner pair.

## Read-only boundary and startup order

Inspection does not open ComponentStore, PluginStore, the exclusive journal
owner, binding evidence, Provider identity or credential state, the usage
database, enrollment token/file, workspace configuration or the broker socket.
It does not spawn a worker, start a listener, connect to a Controller, record or
export telemetry. It can run beside the resident Machine even while its journal
is locked. Process-local TLS/tracing initialization still occurs; ordinary file
reads may update filesystem access times. This is not a claim of zero process
initialization or no reads at all.

Private legacy/destination `telemetry.json` is deliberately not inspected or
adopted. Startup must evaluate its durable namespace fence and exact runtime
installation/authority before using destination configuration. Writer inspection
cannot bypass that sequence or infer Victoria readiness.

Ordinary Machine startup now rejects an initially invalid writer policy before
ComponentStore/PluginStore initialization. Direct PluginStore construction opens
the validated journal owner before legacy directory migration or Provider
identity initialization. Journal construction itself loads policy before creating
its directory/lock or reading installation and binding evidence. Previously an
invalid policy could leave these initialization effects despite failed startup.

The startup journal independently reloads and retains its real policy snapshot;
the diagnostic's data-only report never becomes a grant. External edits between
checks are not an atomic configuration transaction, and unrelated later startup
failures may still leave initialized state. Revocation and original execution
budgets remain governed by [writer admission](telemetry-writer-admission.md).

## Acceptance boundary

Regression tests exercise the actual diagnostic and normal startup paths, all
eight purpose combinations, closed malformed policy rejection, unsafe files and
FIFO refusal. A held real journal, invalid unrelated private state, an enrollment
fixture and a listening loopback Controller remain untouched; snapshots compare
contents, ownership, modes, inode, mtime and ctime (not atime). Two tests first
reproduced the journal/Provider initialization side effects before the fix.
The ordinary quality gate also runs the standalone `machine-host` library tests,
not just all-features tests and a binary-only feature check. Two Controller-only
test helpers now have their correct feature guards; no lint was suppressed.

This prerequisite does not complete P2, authorize a production writer-policy
cutover, accept Operator authority or prove connected delivery. Retain the
independent immutable reader/writer/startup/connected gates and actual production
ingestion/query/failure/restart acceptance. A built or published Machine candidate
is not an activated Machine: activation has its separate maintenance boundary.
