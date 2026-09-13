# Telemetry startup policy preflight

`cowboy serve --check-plugin-hosts` now also validates explicitly supplied
Controller telemetry writer and managed-background policies. Previously this
command returned before their startup validators, so a successful host check
could hide malformed, foreign or unsafe telemetry configuration.

Use the intended Service arguments/environment and the exact immutable
Controller being evaluated. In particular, preserve the data directory,
authentication/host configuration, database setting and explicit telemetry
settings. Run as the actual Service owner, not root: private policy ownership is
part of validation. Do not print the environment, private policy or credentials
as evidence.

## What is checked

The check uses the same `ControllerPolicy::load` and private-policy constructors
as ordinary startup. It validates closed schemas, required fields, exact Service
ownership, the configured database prerequisite, explicit signal/startup/fence
choices, file ownership/permissions/link count/size, and the managed/legacy mode
conflict. Programmatically constructed arguments cannot bypass mode checks.

An explicit writer or managed-background policy requires an **existing** valid
Service identity in the intended data directory. Inspection never creates an
identity from a candidate policy or initializes a new Service. Its bounded
identity read refuses non-regular files and final symlinks; a FIFO cannot stall
the check. With neither managed policy supplied, a fresh empty Service directory
still supports ordinary host inspection without creating state.

Success adds a closed `telemetry` object to the existing host-preflight report:

```json
{
  "schema": "dravengarden.cowboy.telemetry-policy-preflight/v1",
  "writer_policy": "configuration_valid",
  "managed_background_policy": "configuration_valid",
  "legacy_selection": "unconfigured",
  "not_checked": [
    "database_connectivity_and_binding_journal",
    "current_binding_and_background_activation",
    "machine_admission_installation_and_destination_policy",
    "operator_authorization_and_connection",
    "legacy_fence_and_selection",
    "local_recording_and_remote_delivery"
  ]
}
```

Each managed policy field is either `unconfigured` or `configuration_valid`.
This is not an `active`/`ready` state: no writer scope, background permit, queue
or export is created. The report contains no policy path, owner, installation,
binding head, epoch, digest, endpoint or credential. Rejection exits nonzero
without printing a successful report or changing Service state.

For automated acceptance, require the nested `telemetry.schema` and the expected
closed policy fields, not just the outer `status: "configuration_valid"`. An
older Controller's missing telemetry report is **not evidence that it checked
these policies**. This diagnostic distinction does not by itself establish or
invalidate that artifact's independently accepted startup/reader compatibility.
The Catalog-only `--check-plugin-catalog` remains a separate, narrower
operation.

## Deliberately not checked

Inspection never connects to or migrates the database, acquires a Service owner,
opens a binding journal, contacts a Machine, validates its private destination,
authenticates an Operator, records telemetry or starts a listener. A database
setting being present is not proof that its store is durable or reachable.

An explicitly configured legacy selection is reported as `not_checked`, not read
or validated. Normal legacy startup first checks the durable namespace fence and
may intentionally ignore an obsolete selection file. A configuration check
cannot determine that fence without opening the store and must not adopt old
selection bytes to invent managed authority. With no legacy setting the field is
`unconfigured`.

This does not check unrelated Machine component manifests, Web roots, listener
availability or the complete production startup. Policy validation cannot prove
that a binding is current/resolved, that optional export will activate, or that
Victoria accepts/queryably stores a signal. Retain the independent populated
[reader](telemetry-reader-conformance.md),
[startup](telemetry-background-startup-conformance.md),
[writer](telemetry-writer-conformance.md) and
[connected delivery](telemetry-connected-conformance.md) evidence and the owned
production [cutover requirements](telemetry-writer-admission.md).

The first managed intent still permanently fences legacy export. A successful
preflight does not authorize opening production writers, performing a binding
confirmation, replacing host policy or replaying telemetry (`NoRestore`).

## Regression coverage

Tests exercise the actual `serve` early-return path, not only the helper. Valid
writer/background combinations leave fixture files and metadata unchanged even
with an invalid SQLite file. A listening PostgreSQL fixture observes no
connection. Negative cases require the same managed-policy rejection as the
startup constructor for malformed/unknown fields, wrong ownership/target,
private-file failures, oversized input, missing store and conflicting modes.
Missing/invalid/oversized/symlink/directory/FIFO identities fail without
creating state. Legacy configuration remains explicitly unchecked, and bounded
reports and errors cannot reveal fixture secrets.
