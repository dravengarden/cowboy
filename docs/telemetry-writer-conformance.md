# Immutable telemetry writer-policy conformance

`just telemetry-writer-conformance <matrix.json> <new-receipt.json>` exercises
actual Controller and Machine release executables with synthetic private writer
policies and disposable journals. Run from clean committed Cowboy source in the
pinned Linux shell. It never enables a production writer or installs a Plugin.

The [Hawk acceptance record](releases/telemetry-writer-conformance-2026-09-13.md)
binds the first complete run to independently captured actual host roles.

The closed schema-one input is the same complete two-Site matrix as
[reader conformance](telemetry-reader-conformance.md): `controller` and
`machine`, each with `active`, `rollback` and `cold` absolute immutable Nix
release roots. Resolve real paths and independently capture their host roles;
duplicate artifacts prove their bytes, not provenance. A Machine bootstrap
release is allowed only in the cold role, never as an activation candidate.

## Required behavior

Fourteen policies cover absence, all eight combinations of the three purpose
booleans, foreign Service and Machine identities, invalid schema, public file
permissions and a symlink. Actual startup must reject malformed or unsafe
policies for the specific policy error. The Controller must reject a foreign
durable Service; a valid foreign Machine policy must remain unable to authorize
the fixture target. A flag error, unrelated crash or timeout never passes.

Each supplied role runs these scenarios against the same state across cold
starts, giving **294 checks** (three roles × fourteen policies × seven starts):

| Scenario | Cold starts | Required effect |
| --- | --- | --- |
| Service resolution | Three | Preview only; reject that unsubmitted preview after restart, then separately preview/confirm; reopen the exact durable receipt |
| Machine binding | Two | Apply one finite Revoke from absence only with `binding` admission; reopen/query and submit the exact historical duplicate without rewriting |
| Machine recovery | Two | Reject only the exact reopened Prepared attempt with `machine_recovery` admission; reopen/query and return the exact duplicate recovery audit without rewriting |

The Service scenario seeds one local Prepared intent that has never been
dispatched, not an already completed result. It exercises actual same-origin
HTTP view, preview, confirmation and receipt routes with bounded responses and
`no-store`. Only `service_resolution` may persist the abort. A preview creates
no durable effect; it is lost across process restart. The successful separate
confirmation changes only the expected operation and adds its validated audit.
The third start must read that exact receipt, not revive a consumed confirmation.
Every closed purpose must return its exact conflict and preserve journal bytes.

Service children explicitly use the product's **synthetic local Operator** mode
with cleared environments and isolated SQLite. This does not validate real
production Operator credentials, login, account policy or PostgreSQL startup.
The harness never manufactures production authority.

Machine children perform their actual signed SSHSIG handshake with a loopback
fixture peer. Protocol **18** is required, including binding, recovery and
recovery-audit observations before and after each command. Only the scenario's
finite telemetry RPC is sent. No generic Plugin operation admission, installed
Plugin, session command or worker executable is supplied. Other-purpose and
foreign-owner policies must return `reader_only`, not a timeout or another
refusal. A duplicate after restart reads the original result and cannot renew
the deadline, resume Prepared work or replay an effect.

Both Sites validate the resulting journal rather than accepting an RPC/HTTP
success alone. Every refusal, preview, query and duplicate preserves exact
bytes, including absence. Expected mutations are structurally decoded and
checked against the original intent and head. Every Controller start also
accepts actual client protobuf logs, metrics and traces into private local
rotating files with no local failures/drops and zero remote attempts. Historical
resolution or writer admission cannot implicitly create a background exporter.
Policy bytes, link target, device/inode, owner, permissions and ctime remain
unchanged throughout each process run.

## Isolation, receipt and remaining acceptance

Compilation occurs before a non-root private network namespace. The offline
runner has loopback only; child environments, state, sockets, workspace and
telemetry directories are isolated. The same bounded logs and process-group
cleanup as the reader gate apply. Startup uses the hashed pinned SSH helper;
Machine release wrappers retain only their immutable packaged helpers.

The private create-only schema-one receipt includes source revision, immutable
manifest/launcher/ELF hashes, scenario, policy case, cold start, policy fixture
hash, before/after Service and Machine journal hashes, negotiated protocol and
closed failure category. It contains no logs, raw policies, exception text,
endpoints or credentials. Use a new absolute output path in an existing
directory. Invalid setup may produce no receipt; any failed required check
writes `accepted: false` and exits nonzero. Never overwrite or reinterpret it.

This gate accepts a **synthetic executable writer-policy contract**, not complete
production startup configuration or end-to-end Operator authorization. It does
not connect the actual Service to the actual Machine, test network faults,
select a signed Plugin or deliver external OTLP. Signed Victoria runtime tests,
[background startup conformance](telemetry-background-startup-conformance.md),
actual host-role capture, full owned startup configuration, authenticated
cross-end failure/restart, native/session continuity and production cutover
remain distinct requirements.

The [writer-admission contract](telemetry-writer-admission.md) still governs
cutover. Do not open production policies alone, delete managed evidence or
recycle Agent sessions to make this gate pass. Completing this gate is not
completion of P2 or the whole Plugin refactor.
