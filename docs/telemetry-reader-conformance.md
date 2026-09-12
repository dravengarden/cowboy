# Populated telemetry reader conformance

`just telemetry-reader-conformance <matrix.json> <new-receipt.json>` checks
actual immutable Controller and Machine executables against disposable,
populated binding journals. Run it from clean committed Cowboy source in the
pinned Linux shell. It neither deploys nor opens production writer admission.

The closed schema-one matrix requires `controller` and `machine`, each with
`active`, `rollback`, and `cold` absolute Nix release roots. For example:

```json
{
  "schema": 1,
  "controller": {
    "active": "/nix/store/<hash>-cowboy-controller-release",
    "rollback": "/nix/store/<hash>-cowboy-controller-release",
    "cold": "/nix/store/<hash>-cowboy-controller-release"
  },
  "machine": {
    "active": "/nix/store/<hash>-cowboy-machine-release",
    "rollback": "/nix/store/<hash>-cowboy-machine-release",
    "cold": "/nix/store/<hash>-cowboy-machine-bootstrap-release"
  }
}
```

These placeholders are not executable inputs. Resolve each real release and its
provenance independently. A bootstrap release is accepted **only** in the cold
Machine reader position; it remains forbidden as a component activation
candidate. Duplicate paths are allowed when roles really share an artifact.
Putting a candidate in all roles tests those bytes, not the host's configured
recovery floor. No missing role, unsupported protocol or skipped reader passes.

## What the gate proves

Eight cases on both Sites, both cold starts, and all three supplied roles give
**96 checks**:

| Case                         | Expected reader behavior                                                |
| ---------------------------- | ----------------------------------------------------------------------- |
| Absent                       | Remain unmanaged; no binding row/file is created                        |
| Completed                    | Read schema-one ledger containing a completed schema-two binding step   |
| Prepared                     | Preserve the interrupted attempt and unresolved fence                   |
| Unknown                      | Preserve uncertainty without replay or recovery                         |
| Recovered                    | Read schema-two Machine recovery and separate Service resolution audits |
| Advanced                     | Preserve historical recovery while reporting the newer current binding  |
| Checksum corrupt             | Fail startup at the journal integrity check                             |
| Re-checksummed invalid audit | Reject structural corruption even with a valid checksum                 |

The fixture generator uses the real finite Revoke writer, interruption before
the second atomic replacement, validated Machine reopen, a fresh recovery lease,
and independently confirmed Service resolution. It never needs an installed
Plugin, private endpoint or credential. Unknown and corrupt fixtures are
explicit test-only transformations. Runtime/install/upgrade/OTLP acceptance
continues to use the signed Victoria fixtures; this gate does not replace it.

Each Controller starts with an isolated migrated SQLite database and synthetic
Service identity. Success requires both `/healthz` and the existing startup
reader's exact closed-admission/managed-namespace fence. Health alone is
insufficient: an old Controller can ignore the table entirely.

Each Machine starts its own temporary broker and connects to a loopback-only
Controller fixture. The harness verifies its fresh SSHSIG handshake, negotiates
protocol 17, and sends **only** the existing `QueryTelemetryBinding` and
`QueryTelemetryRecovery` commands. Both correlated observations must exactly
match the fixture's receipt, current head, uncertainty and audit. Empty startup
inventories and heartbeats are allowed; other replies cannot stand in for the
expected evidence. No worker executable or session command is supplied.

Both reads reopen the **same** disposable state. The Service row/document and
checksum, and Machine binding-file bytes (including absence), must remain
unchanged after every child exits. Timeout, unrelated startup errors, signal
death, bad protocol, changed evidence and unsuccessful child-group cleanup fail
closed. Corrupt cases must exit nonzero with the specific journal error; an
unrelated failure is not acceptance.

## Isolation and evidence

The recipe compiles before entering a non-root private network namespace, then
runs offline with only loopback available. Child environments are cleared; all
state, sockets, workspace and telemetry paths are temporary. Controller
startup's OpenSSH helper comes from the pinned shell, is exposed through a
single-tool fixture directory, and is hashed in the receipt. Machine release
wrappers retain their immutable packaged helpers. No Service/Provider/SSH-agent
credentials, production policy or host inventory are inherited.

The receipt hashes each source manifest, release launcher, actual ELF reader,
fixture document and Machine ledger. It contains closed result categories, not
logs, exceptions, endpoints, credential material or environment dumps. It is
private, atomically create-only, and must use a new absolute path in an existing
directory. An incompatible matrix writes `accepted: false` and exits nonzero.
Failure before validated inputs or fixture preparation is not a reader result
and may produce no receipt. Do not overwrite a failed or successful run.

The receipt explicitly excludes host-role provenance, production data,
PostgreSQL startup, writer/confirmation admission, actual Plugin and OTLP
effects, native clients, existing sessions and deployment. Unit and isolated
PostgreSQL gates remain separate.

## Host recovery acceptance

Before writers open, bind the supplied paths to independently captured active
profiles, the next transaction's effective recovery target, and the actual
active NixOS closure's absent-profile bootstrap outputs. Recheck after any
intervening activation. The completed component receipt's `previousRelease` is
historical: the next transaction captures the then-current profile, unless an
explicit compatible recovery target is selected. Never restart a Machine only to
rewrite that historical field.

A cold-floor refresh belongs to the owning isolated Columbus worktree and full
host build/activation transaction. Existing profiles and retained sessions must
remain unchanged. Do not manually repoint profiles, delete authority, edit
migration checksums, or claim a candidate-only matrix accepted the actual host.
This conformance gate completes one reader-acceptance mechanism, **not P2 or the
entire Plugin refactor**. See [Machine recovery](telemetry-machine-recovery.md)
and [Service resolution](telemetry-binding-resolution.md).
