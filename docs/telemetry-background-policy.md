# Explicit managed background export policy

The Controller can feed its existing bounded remote queue into the protocol-16
single-attempt path under an independent **host-owned standing policy**. This
implements policy admission; it does not enable production binding writers or
change the configured Victoria destination. The initial production configuration
continues to use its explicitly configured legacy exporter.

Controller implementation release and populated-reader acceptance:
[2026-09-13 receipt](releases/telemetry-background-policy-2026-09-13.md).

## Authority and restart semantics

Core owns this policy, not the installed Plugin or a UI composition. The host
operator explicitly supplies `--telemetry-managed-export-policy <absolute-file>`
or `COWBOY_TELEMETRY_MANAGED_EXPORT_POLICY`. It is mutually exclusive with
`--telemetry-plugin-config` / `COWBOY_TELEMETRY_PLUGIN_CONFIG`; there is no
managed-to-legacy fallback. Without either configuration, recording stays local.

The file is a closed schema-one object with these required fields:

| Field | Meaning |
| --- | --- |
| `schema` | Integer `1` |
| `service_id`, `machine_id` | Exact owners; not a default/available Machine selector |
| `binding` | Complete `BindingSnapshot`: canonical decimal-string `revision` and `policy_epoch`, plus non-null exact `selection` |
| `binding.selection` | `plugin_id`, `plugin_version`, `generation_digest`, `installation_revision`, `contract_fingerprint` |
| `signals` | Exactly three required booleans: `logs`, `metrics`, `traces`; at least one true |
| `startup` | Exactly `"activate_exact_binding"` |

There are no endpoint, token, actor, retry, wildcard, or write-admission fields.
Use the actual independently verified binding identity; a copied receipt is only
evidence for that choice, not permission to write this configuration. Endpoints
and credentials remain exclusively in the selected Machine's private policy.
The binding's policy epoch is not a digest of that private endpoint file.

The required `startup` choice explicitly authorizes NEW background batches after
each Controller restart, subject to fresh checks. This is persistent host policy,
**not** replay of a saved Operator confirmation. Removing the startup flag/file
removes that authority; a binding/recovery record alone never selects an exporter.
Policy setup is an independent machine-configuration maintenance action, not a
side effect of installation, binding confirmation, resolution, or deployment.

Before starting background writers/Plugin hosts, the Controller requires a durable
Service store, a validated resolved ledger, and the exact policy-selected binding
and owners. Absence, corruption, an unresolved operation, a revoked selection or
a later revision fail startup. A Machine need not be connected during Controller
startup; every subsequent batch independently requires its current authenticated
connection, supported protocol, signed Catalog entry and exact installation.

## Lifetime and failure rules

The file must be an owned private regular file, single-linked, at most 64 KiB;
symlinks, public permissions and unsafe replacements are rejected. The activation
holds the original open inode and rechecks metadata and bytes before every
attempt and after the Service ledger read. A changed, removed or invalid file
permanently stops that in-process activation. Even a byte-identical atomic
replacement is a different policy owner. Repairing the file cannot revive an
already rejected permit or activation; an explicit Controller startup rereads and
independently validates the standing policy.

Each batch gets a distinct non-serializable `BackgroundPermit`, bound to its full
OTLP request digest and original at-most-15-second monotonic budget. Its typed
`ExportScope<BackgroundPermit>` cannot use the Operator-only execution method;
the existing `ExportScope<TelemetryExportAuthority>` cannot use background
execution. Neither type can construct binding, recovery or resolution authority.

The Service ledger, lifecycle fence, accepted Catalog, exact installation and
original connection are rechecked before enqueue. Observing missing/corrupt,
changed or unresolved Service binding evidence stops this activation, even if the
same head is later repaired/resolved. A connection failure consumes only that
batch: a future NEW batch may obtain the then-current connection under unchanged
standing policy. There is no read-and-resend of the old batch. Restore advances
the binding epoch, so it does not inherit the old background policy.

Machine handling reuses the existing independently checked local binding,
installation, private endpoint-policy snapshot and receive-time lease. One
signal/batch admits at most one HTTP attempt. Timeout, 429/5xx, redirect or partial
success never cause a whole-batch retry. Once admitted, network emission can
finish after revocation/disconnect and is **NoRestore**, not an atomic distributed
revocation barrier or exactly-once delivery.

No new scheduler, unbounded queue, payload journal, replay key or telemetry-file
reader is added. The existing 16-batch remote queue and local/incident isolation
remain. Managed mode accepts only standard OTLP; legacy JSONL/Prometheus batches
are not converted or forwarded. Explicitly disabled signals/lanes are intentional
skips, using the existing non-failure accounting; partial results increment the
bounded rejected-item counter. Missing authority or uncertain delivery increments
the existing lane failure counters without logging private configuration.

## Verification and rollout boundary

Tests cover closed policy decoding and exact u64 values, file/identity changes,
sticky rejection, full batch correlation, the original deadline, mutually
exclusive CLI modes, and separately typed Operator/background scopes. Signed
temporary Victoria fixtures exercise all three protobuf signals, partial
acceptance, failed HTTP with independent local recording, no legacy fallback,
current/revoked/restored/unresolved bindings, original connections, Catalog trust,
installation and lifecycle fences. Tests do not use production credentials.

No Machine protocol, SQL/file journal schema, signed Plugin/SDK, native ABI,
Provider state or worker-generation input changes. The Controller release still
requires populated immutable [reader conformance](telemetry-reader-conformance.md)
against the actual active, next rollback and cold floors. That gate is not a
production writer/export policy acceptance receipt.

The reader gate deliberately runs without the new policy setting. Older
Controller floors do not implement this startup option: accepting journal bytes
does not establish that they can parse a new CLI flag or retain background export
under rollback. Before a configuration cutover, separately accept the complete
candidate/recovery/cold startup configuration and its explicit export behavior.

The subsequent [per-target writer policy](telemetry-writer-admission.md) supplies
separate typed binding/recovery admission; it does not enable this export mode.
Remaining P2 work: an owned configuration cutover and cross-end production write/failure/
restart acceptance. First binding intent fences legacy export even on an aborted
operation, so the cutover must account for that interval; do not silently migrate
the existing policy, open write gates alone, or delete evidence to regain legacy
egress. This slice is not the completion of the generic Plugin DAG/refactor.
