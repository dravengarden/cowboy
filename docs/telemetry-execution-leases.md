# Telemetry attempt ownership

Status: nineteenth spatiotemporal slice, 2026-09-11. This hardens the actual
Service → Machine → Victoria path. It does not yet implement P2's durable
binding selection/revocation coordinator or Machine recovery receipts.

## Admission, not cancellation or undo

The authenticated Machine connection creates an opaque `PluginHostInvocation`
before spawning its task. A telemetry invocation owns its exact request and a
non-renewable 15-second process-monotonic admission budget. Neither the request
nor a stored response can construct the lease. Usage-host execution retains
its existing behavior; it does not acquire telemetry's guarantees.

The connection scope remains the unique owner. Disconnect/drop invalidates its
leases; a new connection with identical Service/Machine names cannot revive
them. A queued command checks the original lease after acquiring the lifecycle
lock. Lock waiting is bounded by the remaining budget, including on retries.
An observed expiry or failed admission permanently retires this invocation.

Every actual HTTP attempt, including each legacy lane and each OTLP retry,
requires a fresh checkpoint under the Machine lifecycle lock:

1. The original connection and original admission budget are still valid.
2. The installation is unfenced, active, exactly the requested telemetry
   release, and has no Provider authentication generation.
3. The installation revision, contract fingerprint and local activation-link
   observation still match the initial snapshot.
4. The retained package still passes the existing signature/digest checks;
   its signed version and contract fingerprint match the request/inventory,
   rather than trusting a mutable inventory cache as the contract authority.
5. The original private policy file is still private, valid and unchanged.

The policy snapshot retains an open file to pin its inode. Each checkpoint
reopens the configured path with the existing no-symlink/ownership/link-count/
size restrictions and compares file identity, change timestamp and byte
digest. An atomic replacement, even with identical JSON, retires the old
invocation. A changed endpoint/token is never adopted by a retry. A fresh call
may separately pass the current exact policy. No secret, path, policy digest
or raw checkpoint error is added to delivery receipts or telemetry.

For tracked installations, the existing durable installation revision fences
same-release reinstall. Untracked installations have only a conservative Unix
link identity/change-time observation; this is **not** a new durable incarnation
or a guarantee against privileged out-of-band filesystem/clock manipulation.
Policy observations are not monotonic durable policy epochs either. These
limitations remain requirements for P2's binding journal, not hidden guarantees
of the current protocol.

## Effects and boundaries

The final checkpoint admits one bounded HTTP attempt, then releases the
lifecycle lock before awaiting the network. Install/uninstall and unrelated
Agent work never wait on Victoria HTTP while holding that lock. There remains
at most one active Machine export, with the existing bounded lane retry policy.

An already-admitted attempt may finish after disconnection, policy revocation,
uninstall or budget expiry. Its valid HTTP receipt is retained; it is not
rewritten as an undo. If another attempt would be needed, it must pass a new
checkpoint. OTLP partial success, invalid successful responses and permanent
HTTP rejection retain their existing no-retry semantics.
Revocation after preparation but before the first attempt returns the existing
preflight rejection with `started: false`; once any attempt is admitted, a lost
or failed receipt cannot make that assertion.

The 15 seconds starts on Machine receipt, not Service enqueue. It does not
replace the Service's 15-second observation timeout, bound a synchronous
filesystem syscall, guarantee suspend-inclusive time, or grant offline/restart
execution. An attempt admitted just before expiry can still use its bounded
HTTP timeout. Losing a Service ACK does not prove no emission; the RPC layer
still does not resend or reroute. Restart discards these ephemeral leases and
does not replay diagnostic history.

Local rotating files and the durable incident ledger retain independent queues.
Export admission failure never moves either authority into Victoria or `/tmp`.
External logs/metrics/traces remain `EmitExternal / NoRestore`.

## Compatibility and verification

No Plugin package, SDK pin, Machine wire vocabulary, private policy schema or
durable migration changes. Old Controllers can use the hardened Machine; an
old Machine retains its weaker attempt semantics. Protocol 8/9 alone does not
prove this implementation is deployed. Production enforcement requires the
exact accepted Machine component release, through its separate maintenance
boundary; publishing the Git change or deploying Web/Controller is insufficient.
No production Plugin install, new destination, token read or authentication is
required to verify the implementation.

Hermetic signed-install fixtures drive the production invocation/export path
and real loopback HTTP with explicit request/response barriers. They cover
queued disconnect/expiry, replacement connections, old-policy atomic/in-place
changes, invalid/private-file checks, no retry retargeting, uninstall/reinstall,
tampered bytes, concurrent admission, every OTLP signal, both legacy lanes,
fresh-call recovery and in-flight success after revocation. Existing fixtures
cover Catalog resolution, RPC lost/foreign replies, partial success, HTTP
timeouts, local-first delivery and independent incident persistence.

Still missing: durable exact binding plans on both Sites, monotonic policy
epochs, lost-binding-ACK queries, separately authorized restart recovery and
CAS restoration of managed configuration. This slice does not claim P2 complete.
