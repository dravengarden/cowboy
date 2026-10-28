# Cowboy core requirements

Status: normative plugin and Provider platform contract. Plugin manifest schema
1, component registry schema 3 (with immutable schema-2 history), Plugin package schema 1, Plugin release schemas
1-2, Provider
payload schema 2, Agent runtime-binding schema 2, UI schemas 1-2, host
integration schemas 1-2, Provider SDK 3.0, Machine protocol 7,
Machine-scoped installation, exact session generations, Service-scoped
authentication, and bounded uninstall retention implement this boundary. The
in-tree `LaunchSpec` registry remains only as a drain-compatible fallback for
sessions created before an exact Provider generation was recorded.

## Authority

The [spatiotemporal architecture](plugin-spatiotemporal-design.md), redesigned on
2026-09-09, fixes communication, installation, core security and native outside
the Plugin lifecycle. It unifies component composition across Service/Machine
placements while separating authority, local execution, durable state and
recovery. Local Authentication/native ownership and component release rules
change only through its tested migrations. The target APIs and guarantees are
not implemented; this document remains the contract for existing releases/data.
The [read-only composition checker](plugin-composition-checker.md) now exercises
the first structural slice without changing these live authorities or formats.
The [core security client slice](core-security-client-boundary.md) moves local
login/Passkey presentation out of Plugin slots and closes the client native
Passkey port. It leaves Controller host/storage ownership and native ABI intact;
retained local Authentication releases are still storage migration inputs, not
authority to override core client security surfaces.
The [core security storage bridge](core-security-storage-bridge.md) now owns
Passkey SQL through a checked typed port and one shared startup binding. The
selected legacy host must match core's exact historical storage schema. First
import is atomic and refuses unreceipted nonempty destinations. Physical
storage, host selections and durable reader formats remain unchanged until the
separate ownership cutover.
The opt-in [core ownership path](core-security-ownership.md) now removes local
Authentication package prerequisites while binding existing security storage to
durable core authority. Its first production release is reader-only; the actual
host-policy handoff, accepted recovery configuration and supported-client/native
acceptance remain distinct requirements.

These requirements govern new Provider-platform work. When a target design
choice conflicts with a transitional implementation detail, preserve the live
system until a tested migration exists, then move toward this contract. Do not
silently redefine the contract to match the transition.

The detailed package and type-system design lives in
[Installable Provider packages](plugin-packages.md). Runtime details live in
the numbered [architecture chapters](architecture/00-overview.md).

The [security priorities](secure-connectivity-design.md), updated 2026-10-03,
keep mandatory HTTPS and device authentication (Option 1) as the sole current
delivery baseline, including on private networks. [Its protocol](device-transport-security.md)
is implemented. WireGuard Option 2 is deferred roadmap work: isolated transport
experiments do not authorize a new product mode, weaker TLS/authentication or
changes to core authority before a reviewed bootstrap and migration design.

## OVH Remote deployment invariants

For Codex and Claude runtimes placed on OVH, inference and Provider API
connections MUST use OVH's native public egress. Neither inherited proxy
variables, Provider endpoint overrides nor transparent host routing may send
those connections through a proxy. Check IPv4 and IPv6 and the actual runtime
processes; an empty proxy environment alone is not proof of direct egress.

Tools bound to Hawk, Falcon or MacBook Air MUST automatically use the approved
proxy route for cross-border transport, without a direct fallback. Network
policy belongs to the infrastructure owner; the Agent must not select proxy
exits, create tunnels or repair routing. Keep the
concrete transport protocol and performance tuning outside this contract.
Unavailable execution must fail closed, preserving operation identity and
uncertain mutation outcomes; never run the tool on OVH as a fallback.

Framework-generated model context, tool metadata and infrastructure errors MUST
NOT disclose the execution target's geography, including country, regional
timezone or network topology. Claude's ordinary tools should preserve their
native interface without additional remote-placement narration. Failures must
remain truthful and actionable without exposing private endpoints or raw
transport diagnostics. Keep operator diagnostics outside model context. Project
cwd, OS and shell remain accurate where needed for correct execution; do not
fabricate a local execution environment.
Native calendar context belongs to the runtime's clock and timezone, not the
execution target's regional timezone. OVH runtime timezone is UTC; `TZ` alone
does not establish this when native code reads the system timezone directly.

This boundary covers framework-generated information. User instructions,
repository files, arbitrary command output, hooks and third-party MCP content
retain their semantics and can contain location information. It is not a
guarantee against geographic inference or of Provider account eligibility. Do
not silently rewrite that content or weaken project instructions.

Every native runtime/executor upgrade must review its initialization and
model-visible metadata, including newly introduced timezone fields. Recheck
normal execution, connection failure, uncertain mutations and reconnection.
Codex uses its native execution extension; Claude is adapter-backed through the
pinned Mods interface. The residual Cowboy responsibility is authenticated
Machine binding and faithful context/tool projection; replace the adapter when
native Claude supports that same binding and projection contract. See
[execution environments](execution-environments.md#ovh-remote-audit-2026-10-11)
for the bounded evidence and outstanding acceptance scope.

## Reboot convergence

Any Machine may reboot at any time: Hawk (Controller, local runtime and
execution target), OVH (remote runtime) and Falcon (runtime and execution
target), together or independently. Once the host itself boots successfully,
Cowboy MUST return to service without a human step. This is a core product
requirement, not an operational preference.

- Every Cowboy host process starts at boot without a login session (system
  units, or user units with linger) and is restarted by its service manager
  after any exit that is not a deliberate uninstall. A requested host restart
  (exit status 75) is a restart, never a stop. Start-rate limits must not leave
  a unit permanently failed: boot ordering, mounts, DNS, the overlay network
  and the Controller can each become ready later than the process.
- Readiness of another host is waited for and retried with bounded backoff,
  never treated as a terminal startup failure. The Controller can start before
  any Machine, and a Machine before its Controller or its execution target.
- Durable state needed after boot lives on persistent storage. Runtime
  directories, sockets and transient units are reconstructed, never trusted
  across a boot. Loss of a transient worker or execution keeper is proven from
  boot identity or exact unit evidence, then recovered without replaying an
  uncertain effect: native history, queue, workspaces and unknown prompt
  outcomes are preserved, and the next turn proceeds without operator
  maintenance.
- A turn that was running when its host rebooted ends as interrupted. Queued
  prompts and new prompts resume delivery automatically once the runtime and
  execution target are reachable again.

Every change to service units, installers, startup ordering, persistence or
recovery must state how it preserves this property. Verify without rebooting
production hosts: inspect the installed unit and its restart/start-limit
policy, exercise process loss and boot-identity change in isolated fixtures
(`tools/execution_session_conformance.py`: `runtime_reboot` for a runtime host,
`execution_boot_recovery` for an execution target), and use a disposable VM for
boot-ordering changes. An explicit send that meets a transient wait while a
Machine reconnects keeps its intent: it is delivered once the Machine is ready
even if the queue is paused or the session crashed during the reboot. Evidence from an actual unplanned reboot is recorded in
[remote session disaster recovery](remote-session-disaster-recovery.md).

## State ownership

| State | Owner and identity |
|---|---|
| Plugin release | Catalog: `(plugin_id, plugin_version, artifact_digest)` |
| Plugin installation | Machine: `(machine_id, plugin_id)` |
| Provider authentication | Cowboy Service: `(cowboy_service_id, provider_id)` |
| Authentication replica | Derived Machine copy: `(machine_id, provider_id, auth_generation)` |
| Session runtime | `(session_id, machine_id, provider_id, provider_generation_digest)` |
| Project | Target Machine: `(machine_id, project_id)`, with independent display name and real directory |
| AI placement policy | Cowboy Service: permitted Local/Remote placement per Machine and preferred runtime, intersected with Controller-host Provider runtime restrictions |

The [native project contract](native-projects.md) owns registration, discovery
and new-session selection. Machine protocol 24 adds closed project management;
the execution transport and Provider lifecycle remain their existing owners.
Matrix and host-directory naming are not dependencies of remote placement.

An authentication replica is not a Machine login. It is a versioned projection
of the Cowboy Service's authoritative Provider authentication state.

## Product requirements

### Named Provider accounts: isolation acceptance

Multiple personal subscriptions of the same Provider must be independent
credential identities. Supporting them technically does not establish that a
vendor permits any particular subscription usage or guarantee against account
enforcement. Native authentication and published integration contracts remain
the baseline; identity spoofing and automatic quota-driven account rotation
are not account management features.

An account has an opaque stable ID. Its display name, subscription tier,
email, organization and credential generation must not substitute for that ID.
Account metadata, defaults and session bindings belong in the database;
credentials remain in protected authentication storage and must never be
returned by UI, CLI, MCP, telemetry or ordinary error messages. Create Session
must select an account explicitly or resolve a documented default once. The
resulting binding is immutable: refresh, recovery, queue replay, child calls,
hibernation and upgrades must never fall back to another account. Changing a
default affects future sessions only.

Machine and subscription are orthogonal session choices. The same subscription
may run concurrently on several Machines; neither the account record nor its
default may pin execution to one Machine. Readiness is evaluated for the chosen
pair. A missing authorization must offer login on that Machine, never silently
select another Machine or account. Mobile and desktop use the same two choices.

Each account needs independent login state, writable credentials, native
settings/history and revocation. Where the Provider permits managed portable
credentials, encrypted vault entries and signed replicas must also be independent;
their encryption AAD and signatures bind account, Provider and generation.
An account's refresh or logout must not roll another account's workers.
A Service CAS after refresh is insufficient to serialize upstream
refresh-token use: the implementation must coordinate before a refresh can
reach the vendor, including across Machines. Copying a refresh token to several
independent writers is not an isolation design. Prefer independently established
native login grants on each Machine, attached to the same verified subscription
identity. A grant's destination is not an account's execution placement. Separate
native grants need separate vault entries, generations and revocation receipts;
never distribute a Machine-bound grant to another recipient. Do not claim that
separate logins are independent refresh lineages until the exact native Provider
contract has been verified. An API key's documented sharing rules are distinct
from rotating OAuth credentials.

The credential-storage boundary must also satisfy the Provider's current terms.
Rechecked 2026-10-17: Anthropic's
[authentication and credential-use rules](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use)
permit end users to sign into an unmodified hosted Claude Code binary, but prohibit
third-party developers from collecting, storing or intermediating claude.ai
credentials/session tokens. Consequently, named Claude subscriptions must use
Machine-local, native-owned credentials; their OAuth bundles must not enter the
Controller candidate/vault/replica pipeline. The Controller may retain account
metadata, verified identity fingerprints and lifecycle receipts, not OAuth tokens.
The generic portable vault remains applicable only where the Provider's contract
permits managed credential storage. Independent native login per Machine remains
required; encrypted central storage does not itself resolve this policy boundary.
The current candidate-promotion prototype is not an accepted Claude subscription
architecture. Native-owned authorization/status/logout and lifecycle proof must
replace that path before any named Claude runtime admission or real-account test.
An explicit transitional first-party fence enforces this at named login
discovery/admission, candidate acceptance, Machine export/refresh/apply, and
Service vault encryption/decryption/sealing. Its closed reason code is
`native_owned_credentials_required`. Protocol upgrades or identity recipes cannot
remove that fence. Legacy `default` accounts retain their existing migration
boundary, and cleanup/Wipe remain possible. The fence is deleted only when the
separate native-owned lifecycle passes acceptance; it is not that implementation.

Directory separation prevents accidental mixing but is not a security boundary
against an agent with host filesystem access. Strong instance isolation needs
enforced filesystem, process, inherited-descriptor and local-IPC boundaries,
including native credential stores. The Controller, Machine supervisor and
host administrator remain trusted. Existing agent root and passwordless sudo
capabilities are retained by explicit user choice. These privileged agents are
inside that trust boundary: directory isolation cannot prevent them from reading
host-owned credentials. Do not promise adversarial isolation from host root or
remove those capabilities as an implicit prerequisite. Account isolation must
still prevent accidental credential selection, inherited-environment leakage,
cross-account refresh, and lifecycle mutations through supported Cowboy paths.

Rollout is gated on negative tests with synthetic credentials: cross-account
read/write, relabelled encrypted envelopes, concurrent login/refresh/logout,
crash/restart, stale receipts, revoked accounts, old peers and session recovery.
Legacy credentials retain the reserved `default` identity and their original
schema-one encoding. Named-account envelopes require protocol 36. Protocols
through 35 predate account support; the independent login and session rollout
gates default off.
Account metadata uses an owner-scoped DB registry with transactional revisions
and replay receipts. HTTP, CLI and the existing four MCP tools share that writer;
schema discovery is on demand. Metadata operations do not query vendors or
activate workers. The session's immutable account ID is persisted independently
of its Machine, carried in worker launches, and checked during reconciliation.
Legacy rows migrate to `default`; malformed stored identities are quarantined,
never coerced into a different account. These paths do not yet enable named
sessions: native login identity verification, account-aware lifecycle receipts
and public creation UI must pass end-to-end admission before the gate opens.
Native identity binding uses the same transactional registry writer through an
internal-only mutation that HTTP and MCP cannot deserialize. The registry stores
only a domain-separated fingerprint of Provider, native authority, subject and
optional tenant. Machine, label, email and plan are not binding inputs. A bound
identity cannot be replaced; matching re-verification is idempotent, and another
subscription under the same owner cannot bind the same native identity. Disabled
accounts cannot receive bindings. Raw native identifiers are not stored in the
registry or operation receipt. The writer is implemented; fresh native evidence
production and admission remain required before named login promotion opens.
Signed native login methods can declare a `fresh_login_json_v1` identity recipe.
The Machine reads only that request's new login home after the native command
succeeds; it never scans existing profiles. The reader opens every path component
without following symlinks, accepts regular files only, caps input at 256 KiB,
and rejects absent or malformed subject/tenant fields without logging their
contents. The Controller checks the evidence against the signed method's
authority and tenant requirements. Legacy methods omit the recipe from their
serialized contract. The candidate Claude Provider declares the identity and
logout recipes; production native-login acceptance remains a separate rollout
requirement. The owner-scoped promotion transaction and durable login-dispatch
reservation are implemented and tested with synthetic data.
Local runtime protocol five carries subscription identity. Controller handshakes
retain the negotiated version on every connection: older brokers receive neither
named-account declarations nor their queued launches and prompts. Named workers
require protocol five, and the broker refuses older workers before they acquire
ownership of a named session. Authentication changes roll only the affected
account's workers; Provider software maintenance may still roll all its accounts.
Refresh candidates and authentication receipts carry the account identity.
Native login commands, credential candidates and temporary-home finalization
also carry the account identity; named operations require Machine protocol 36.
The Machine selects one account store before launching login and retains it
through credential export and cleanup. A duplicate active login request cannot
replace the original task's input or cancellation channels. Controller candidate
admission checks the executor's account before promotion. Named login requires
a signed native identity recipe and Machine protocol 36; runtime rollout stays
closed until first-party native acceptance and revocation lifecycle are complete.
Candidate validation and credential commit are serialized against cancellation
and duplicate promotion. A failed validation or commit does not consume the
request; after a successful commit, duplicate candidates cannot commit again.
Failure notices must match the full executor identity and cannot overwrite a
successful promotion. Credential export uses descriptor-relative, no-follow
regular-file reads with per-file and remaining-bundle bounds before allocation;
special files cannot block login export.
Credential watches discover only Cowboy-owned account roots and credential
ancestors, without following account-directory symlinks or watching native
history recursively. A named refresh uses an already-authorized Machine grant;
observation and reconnect inventory never create accounts or grants. Its CAS,
rejected-candidate repair and reconnect synchronization target that grant alone.
Legacy Plugin inventory auth fields continue to describe only `default`, and
named refresh events are withheld from Controllers below protocol 36 before
native validation and again at the outbound event boundary.
Runtime authentication failures expire only the session's account and enrolled
Machine grant; a missing recipient never falls back to the default vault.
Automatic authentication recovery also requires the session's exact account ID;
a newer generation for another account cannot revive that session. Account,
Machine-grant and projection directories reject symbolic-link aliases before
creating private state or changing permissions. This does not protect against
concurrent filesystem replacement by the trusted host administrator.
The legacy login APIs reject unknown scope selectors instead of silently
modifying default credentials. Resuming a login requires the same authentication
method and signed release; changing either requires cancellation first.
Login executor discovery retains Machine identity even when multiple Machines
have the same release. A start request can pin `machine_id`; explicit selection
never falls back to another Machine, and resumed logins must match it. The Web
client sends the discovered Machine and checks it in the accepted response;
Machine management uses that Machine's installed release. Legacy unscoped
clients remain supported, but an older catalog without Machine identities
cannot satisfy an explicit selection. Discovery opts in with
`X-Cowboy-Auth-Machine: 1`; legacy catalog readers retain their original closed
schema and release-level deduplication. Responses are not cached and vary on
that header.
Temporary native logins start with a cleared environment and private HOME,
XDG paths, temporary directory and working directory. Only network/certificate
settings and the signed login contract's explicit settings are forwarded.
Native credential export reads the temporary login's files, never ambient
Provider API keys; secret input is a separate explicit executor.
All accounts use the established Service signing identity; a new account cannot
bootstrap another trust root. Until account-aware uninstall receipts are wired,
uninstall refuses before mutation when named-account plaintext projections
exist. A corrupt named vault is quarantined without disabling default accounts.
Machine-bound grants use independent schema-three vaults and include the
recipient key in vault authentication data. Sealing to another Machine is
rejected before credentials are opened. Refreshable named-account credentials
cannot be committed to an unbound portable vault. This is a storage admission
boundary, not proof of native OAuth grant independence: native account matching,
login routing, session binding and grant-aware lifecycle receipts remain rollout
requirements.
Machine-local native profile storage is separate from that portable prototype.
Profiles reserve an account/Provider/generation and login request, pin the login
installation, and bind native identity before becoming usable. Incomplete or
reused reservations refuse another native login. Named Claude runtime home and
credential-directory resolution now use only this local profile path; missing,
linked, incomplete, mismatched or incompatible metadata refuses launch without
opening a sealed portable replica. Runtime preparation does not read credential
files. Default accounts retain their existing compatibility path.
Authenticated account-scoped wipe now also revokes local native profiles. Each
wipe first durably fences the entire account/Provider native profile set. A
partial cleanup or restart retains that fence: profile reads, completion
receipts, new login reservations and uninstall remain refused until the wipe
finishes. Only after every profile is durably revoked and its home removed may
the temporary fence be removed; individual revocation markers remain.
Each profile receives a durable revocation marker before its home is removed;
operation reservations and bound identity survive cleanup and restart. Recreated
homes cannot restore admission. Cleanup does not follow credential-directory
symlinks or read credential contents, and a failed cleanup returns an error
rather than a successful materialization receipt. This local wipe is not proof
of worker exit or upstream native logout.
After a confirmed local wipe, an incomplete historical login need not acquire
an identity to permit a fresh request or uninstall. Its durable revocation and
absent home are required; its old request ID remains permanently reserved.
Recreated homes, including dangling links, block fresh requests and uninstall.
Uninstall permits retained records only when revocation is durable and no home
remains. Pending reservations, malformed records and linked
or recreated homes continue to fence uninstall.
Machine native login dispatch now uses the prepare/complete entry points and
emits metadata-only completion evidence to the Controller journal. The public
entry point remains closed until Controller orchestration, native logout and
worker revocation are connected and accepted. Native generations are reserved
in the same DB transaction as the login operation, independently per owner,
account and Machine. Replays retain their original number; cancellation never
recycles it. Store copy preserves counters, and migration seeds counters from
earlier native completion evidence.
Controller native completion now performs identity binding as a separate DB
transaction after recording the evidence. Binding requires the exact recorded
completion, the latest reserved generation on that Machine, an uncancelled
operation, an enabled account and unchanged active Machine enrollment. Initial
binding also requires the registry revision observed at login preparation.
The resulting revision is saved atomically in that login's receipt; a repeat
requires that exact revision and identity to remain current. Disable followed
by enable cannot renew an old binding, even if the identity still matches or
the caller supplies the new revision. A different native identity
cannot replace the subscription's existing identity. Binding emits the account
preference notification but neither creates a grant nor marks the account
SignedIn. Historical completion remains readable when current binding is refused.
Use of a native binding runs through a synchronous DB-serialized callback that
rechecks its exact owner/operation evidence, current generation, account revision,
identity, enablement and Machine enrollment. Cancellation and revocation use
the same lock order. The callback returns no reusable authorization object;
callers must still verify live connection and action permissions. Controller
completion notifications use this boundary and recheck the original executor.
This boundary alone is not a worker lease or proof of later runtime cleanup.
The internal account-aware runtime command `QuiesceAccountSession` stops a
matching named account's worker, including a busy worker, without deleting the
session declaration or native thread identity. It fences replacement launches
before stopping and keeps that fence after success. A launch still in progress
or an unconfirmed stop returns refusal; only confirmed termination permits a
successful acknowledgement. The caller may retry after reconciliation. Old
runtime protocols reject this command. A separate account-stop fence
refuses Reset (including a replacement account) and Hibernate before either can
clear ordinary launch cancellation or discard the session declaration. The
Machine auth store persists a session/account stop intent before stopping, using
an atomic file replacement and directory sync. After a broker restart, adoption,
launch and worker registration still refuse the stopped session. Unknown storage
errors and damaged markers cannot restore admission. Retrying a stop without a
retained declaration requires the exact persisted session/account identity.
These markers are never cleared by login, credential wipe or ordinary session
maintenance; they record stop intent, not confirmed termination or vendor logout.
Startup validates the bounded recovery set (including filename/record identity)
before runtime admission; incomplete atomic-write staging files are ignored.
For systemd-owned workers, a broker-lifetime background task replays stops
serially, retries unconfirmed stops, and retains the durable intent after success.
It rechecks evidence before every effect and emits no synthetic Controller ack.
Direct mode cannot infer old-process exit from an empty post-restart PID table;
such reconciliation remains explicitly unconfirmed until ownership is established.
This remains a per-session barrier, not an account-wide revocation receipt or a
backward-compatible reader guarantee for older Machine releases.
Ordinary adoption and Reset also reject a different subscription when either
the retained declaration or the attached worker's launch identifies the original
account, before modifying lineage or sending worker commands.
Worker snapshots also preserve that binding: a different account is rejected
against either the declaration or the previous accepted worker launch. A named
account cannot be erased by omitting launch metadata; legacy default-account
snapshots retain compatibility. Refused snapshots do not replace accepted state.
This is a per-session stop primitive.
The Controller transport exposes a disposable observer for a stable,
operation/account/session-scoped stop command. A matching positive broker
acknowledgement confirms process stop; refusal, timeout and observer loss do
not. Dropping or timing out the observer retains the queued idempotent command
for reconnect, and retry attaches a new observer without growing the pending
queue. A stale observer cannot remove its replacement. Command acknowledgements
with a mismatched session ID do not consume pending commands. These transport
results still require durable lifecycle recording before account-wide completion.
The separate `provider_account_stops` journal records historical positive stop
evidence without changing the immutable revocation operation receipt. First
write requires the exact account/session/Machine in that operation's plan, a
currently revoking and disabled account, a non-revoked Machine with the same
enrollment fingerprint, and a live-connection recheck after DB validation.
Identical concurrent/repeated evidence is idempotent; conflicting evidence is
rejected. Historical reads survive restart and enrollment changes but confer no
dispatch authority. The caller must obtain a positive scoped broker result;
timeouts and requests sent are never eligible evidence. This journal does not
record native logout or grant revocation and is not account-wide completion.
The internal single-session driver joins the immutable plan, current runtime,
scoped stop acknowledgement and this journal. Dispatch admission holds the same
owner and enrollment locks as evidence recording and rechecks the active runtime
before enqueue. A positive acknowledgement is persisted only if that runtime
and its captured connection epoch are still current (including reconnects that
reuse the runtime object); offline or unconfirmed targets remain pending and refusals
remain explicit. Historical evidence short-circuits retry without another send.
The driver is not yet a public account-wide revocation or logout endpoint.
An internal account session-stop pass reads the immutable plan once and runs at
most four target steps concurrently. It retains each target's pending, refused
or failed outcome without short-circuiting other Machines, and returns results
in plan order. Restart/retry reads each confirmed stop from the journal instead
of resending. Unplaced and offline targets remain pending. Completing this pass
is not account-wide completion: login cancellation, grant revocation and native
logout still require their own evidence.
Machine login cancellation retains an active task's request reservation until
that task cleans up; accepting a cancel signal cannot permit request-ID reuse
while the old task can still mutate its registration. Cancelled logins refuse
further authorization input. An abandoned task with no exit evidence remains
unconfirmed, not successfully cancelled. Completed portable candidates are
explicitly cleaned before cancellation removes their registration. A cleanup
failure returns refusal, retains the registration for retry and blocks further
authorization input; best-effort Drop cleanup is not a successful cancellation
acknowledgement.
Named login cancellation uses a separately correlated `CancelLoginRequest`
command at the reserved account protocol floor. Native login also persists
observed leader exit and whether login succeeded in its reserved generation,
before emitting completion or cancellation observations. Missing evidence remains
unknown after restart; a failed exit cannot be rewritten as successful. Native
completion and runtime-home recovery require confirmed successful login. These
records do not prove descendant termination or native logout.
Controller registration remains
until the original authenticated Machine accepts that command; refusal or an
unknown transport result retains it for retry. A late login/code-submission
acknowledgement cannot satisfy this request. Machine acceptance remains distinct
from process-exit evidence and is not a durable logout completion receipt.
After receiving acceptance, the Controller rechecks the connection and request
ownership before removing its registration. A replacement connection or changed
request retains the registration and returns an error. Dropping a waiting caller
also retains the registration; a retry uses a new correlation ID, so the previous
attempt's late acknowledgement cannot complete the retry.
Login receipt reads can recover a lost native completion using the separately
correlated `QueryNativeProfile` command at the reserved account protocol floor.
The query reads existing Machine metadata only, creates no account directories,
and runs no Provider process. Missing, incomplete or revoked profiles yield no
completion evidence. Replies bypass diagnostic history and cannot be satisfied
by a generic command acknowledgement. The Controller rechecks the connection,
signed identity recipe and original enrollment before recording historical
completion; the DB writer rechecks cancellation and account admission. Recovery
does not bind a new runtime or declare the subscription signed in. Receipt reads
also finish an interrupted identity-binding stage using the dispatch-time
registry revision, never the current revision as a replacement. That revision
is persisted together with the recipient fingerprint. Binding still rechecks
cancellation, account admission, enrollment and the latest native generation;
concurrent retries converge on the existing binding without advancing it again.
Legacy records without the original revision remain historical observations.
Account-wide enumeration, authorization revocation and native logout still need
their own orchestration and acceptance before public admission opens.
The internal `BeginRevocation` metadata transaction persists `revoking`, disables
the account, removes its default selections and commits its operation receipt
together. That same transaction marks every existing login reservation for this
owner and account as cancellation requested, preserving its historical evidence.
Its receipt also captures an immutable work list of the account's login and grant
operations. The same transaction cancels prepared grants only; committed grants
retain their exact generation and historical receipt for separate revocation.
This cancellation does not certify vault cleanup, replica wipe or native logout.
The internal committed-grant vault revoker authenticates the current vault and
matches the account, recipient and original promotion commitment before writing
a newer empty wipe generation. A different login returns superseded without a
write. An authenticated later empty wipe is reported as already signed out,
without another generation change, including after restart; this observes
current local state, not which operation caused it. The caller still requires
account-revocation admission. Neither result proves Machine cleanup or upstream
logout, and historical committed DB receipts remain unchanged.
An internal bounded local-grant pass applies this writer through a DB admission
callback under the owner lock. It requires the grant in the immutable revocation
plan, exact account/Provider/Machine scope and a currently disabled, revoking
account. Historical recipient fingerprints locate only existing local vaults;
an offline or re-enrolled Machine does not redirect cleanup to a new recipient.
Cancelled promotions use their existing compare-and-swap cleanup. Missing vaults,
superseded grants and failures remain explicit per-target outcomes. This stage
does not mark the whole account revoked or authorize any remote delivery.
Recovery discovery scans the durable operation journal in keyset pages of at
most 32 examined records, advancing across historical records as well as active
ones. It returns only operations whose account is currently disabled and
revoking. Restart recovery uses these locators, not an in-memory task list;
locators do not bypass the execution stage's admission checks. Background
scheduling remains gated until the remaining lifecycle stages are connected.
The immutable work list retains all grant phases and includes login and grant
operation locators and session placements, including deleted sessions and
offline or revoked Machines. The list is captured under the admission lock;
future named-session creation must share that lock. Missing placement remains
unresolved work. Replaying the same operation returns the original list after a
restart, rather than reconstructing it from current online Machines. Locators
are not authority to dispatch and do not prove cleanup or native logout.
Other accounts remain unchanged. This intent does not prove native process exit
or logout. Concurrent retries share that receipt. Public metadata deserialization
cannot invoke this transition, and Enable cannot clear it. Login discovery,
new login claims, identity binding and grant admission require both enabled and
not revoking. Missing `revoking` in historical records defaults to false.
Fresh login dispatch is serialized under the same owner and Machine locks:
the operation stores the dispatch recipient fingerprint in that transaction,
before invoking the enqueue callback. A second invocation with the original
claim is refused. Completion cannot replace that fingerprint with a newly
enrolled Machine key. Historical records without the fingerprint are not
eligible for restart dispatch recovery; current enrollment is not evidence of
the original recipient.
Immediately before enqueue it rechecks the exact uncancelled reservation,
prepared registry revision, account admission and enrolled recipient identity.
The callback must also check the live connection. Historical claims cannot
dispatch. A commit error after enqueue remains an unknown outcome, never a
reason to send again; later revocation still needs runtime cleanup.
This is a durable admission fence, not a completed-revocation receipt; Machine
enumeration, native logout and proof-based completion remain to be connected.
The internal DB promotion journal binds an owner-scoped operation to the exact
account, Machine, recipient key hash, candidate hash, native identity hash and
expected vault generation. Preparing it first locks and checks the current,
non-revoked Machine enrollment and exact recipient key; failed admission must
leave both account identity and operation journal unchanged. It binds native identity in the same DB
transaction. Duplicate operations return history; changing their input is
rejected. Any registry revision change before finalization cancels the attempt,
including disable followed by enable. Journal entries contain no credentials
or raw native identities. A prepared or committed journal receipt alone cannot
authorize distribution. Vault commits bind the exact prepared intent into the
credential file's authenticated encryption data and sync the file and parent
directory before returning an opaque commit proof. Recovery authenticates that
same entry; another candidate, changed credentials or a superseding generation
cannot reuse it. DB finalization requires this proof. The named login callback
prepares this record from its owner-bound executor and verified native identity,
commits under the executor lock, then finalizes and synchronizes only that
Machine grant. Cancellation fences new submissions before DB cleanup; an unknown
cleanup result retains the request for retry. Owner polling can recover an
uncertain post-write result without executing native login again.
Cancellation dispatch failure also retains the fenced request. Explicit cancel,
expiry polling and replacement-login cleanup share this rule; they cannot drop
the request or redirect cancellation onto a replacement connection. Concurrent
retries serialize enqueue/removal. Successful enqueue means cancellation was
requested, not that native process termination has been confirmed.
MCP cancellation resolves only the original parent-scoped login operation and
persists cancel intent before dispatch. The fresh login claimant rechecks that
intent after executor registration, covering cancellation before registration.
Restart preserves the intent but does not imply process-exit confirmation.
Authorization input is one line of at most 4096 bytes at CLI, HTTP and Machine
ingress, with one queued response per login. Native stdout and stderr each have
a 64 KiB line limit and 1 MiB total limit; partial reads survive select cancellation
without losing challenge bytes. Exceeding a limit terminates the login through
the same process cleanup path without logging the rejected contents.
Machine native-login spawn checks cancellation and expiry before launch while
holding the same request registry lock used by cancel and connection teardown.
A queued task whose request was removed, connection revoked or deadline elapsed
must not briefly launch the Provider CLI before noticing cancellation. Already
launched processes remain covered by process-group cancellation and the deadline.
After cancellation, expiry or native I/O failure, the login task must observe
the owned leader's exit within the bounded cleanup wait. Wait errors or timeout
return a cleanup-required error, never a cancelled/signed-out result. This
confirms only the leader; it does not establish descendant or same-boot orphan
termination, which remains part of native-account release acceptance.
Temporary native-login homes have a task-owned cleanup guard. Before spawn or
after confirmed process exit, early return and failed candidate enqueue remove
only that account/request's home. After spawn, task abortion or an unconfirmed
exit retains the home and exclusive login reservation, including across a
same-boot Machine restart. Later cancellation or connection teardown must not
overwrite an unconfirmed-exit error with a signed-out result.
Candidate export/enqueue serializes with connection revocation; successful
enqueue transfers the armed cleanup guard to the connection's request registry,
not proof of promotion. Finalization validates the account and Provider before
releasing that guard; cancellation and connection teardown also delete the home.
Normal cleanup errors are reported, and drop cleanup failures are logged rather
than silently discarded. After native process completion, the Machine durably
marks the temporary home before exporting a candidate. On startup, under its
exclusive journal owner and before accepting logins, it removes marked
completed homes in validated account/Provider scopes. On Linux, a canonical kernel
boot ID is also persisted before native spawn: a different current boot proves
that processes from the recorded boot cannot remain on this Machine, permitting
recovery of interrupted logins after a host reboot. Missing, malformed or
unavailable boot identity never authorizes deletion. Recovery does not follow
directory or marker symlinks and does not touch installed credential replicas.
Directory age is not process-exit evidence: unmarked, legacy or malformed homes
are retained with diagnostics. Recovery of a same-boot crash during native
execution still requires process-lifecycle reconciliation; startup must not
guess that it exited.
Native login home creation serializes inspection and creation across store
clones. Any remaining entry in that account/Provider's candidate directory
blocks another native login on the same Machine until cleanup is confirmed;
this fence survives daemon restart and cannot be bypassed with a new request ID.
Other account and Provider scopes remain independent. Creation rejects linked
Provider/candidate directories rather than writing into their targets.
Named login completion also requires an acknowledged temporary-home cleanup on
the original Machine connection. Enqueue alone is not confirmation, and cleanup
cannot migrate to a replacement connection. Rejection or an uncertain response
leaves completion pending so owner polling can retry the same candidate cleanup.
Concurrent callbacks and owner polls share one completion lock per login request.
Agent-initiated native login reserves an owner/parent-scoped immutable operation
and request ID in DB before enqueue. Only the newly committed claimant may
dispatch. CLI login requires a stable operation ID and shares this reservation
mechanism through the HTTP route; a separate owner-scoped HTTP namespace and
read-only receipt endpoint prevent retries from launching another native login.
Replay and restart never regain dispatch authority. Reservations are not
authentication success or runtime authority, and contain no native secrets.
An uncertain reservation or send must be observed, never retried with a new
operation automatically. SQLite and PostgreSQL enforce the same contract.
Only acknowledged success is cached; failures and interrupted attempts remain
retryable. Completed requests can restore their completion notice after event
history eviction without repeating grant synchronization or Machine cleanup.
Cancellation, expiry and connection replacement are rechecked after waiting.

The shared desktop/mobile Create dialog lists subscription choices independently
of both workspace and runtime Machine. It sends only the chosen immutable ID,
preserves the server-returned binding in its optimistic session projection,
and refuses unavailable or disabled selections instead of falling back.
Account defaults are resolved by the server at creation. Until the runtime
admission gate opens, named choices remain visibly unavailable. A failed registry
load never silently chooses another subscription; the user may explicitly select
the existing `default` account or retry. This creation field is not an account
management or native-login UI.

Remaining release acceptance is explicit: first-party native identity recipes
and independent refresh lineage need exact-native validation; account revocation
needs real worker/grant cleanup acceptance. The shared mobile/desktop Settings panel manages
account metadata, enablement and defaults through the DB writer. It preserves
uncertain writes across panel remounts and tab reloads, refreshes on account
preference notifications, and keeps named defaults unavailable while admission
is closed. Native sign-in now has an explicit Machine picker, official browser
authorization, ephemeral code input, status recovery and cancellation. Only
operation metadata is retained in tab storage; an uncertain reply never creates
a new login automatically. CLI and MCP use the same owner-scoped lifecycle.
The sign-in form treats a verified completion, a cancellation acknowledgement,
and confirmed process cleanup as different states. Only verified completion
offers `Done`; cancelled attempts retain status/restart recovery. Account or
request changes reset form state and authorization input, and stale replies
cannot populate another account's form. Polling the same challenge must preserve
an in-progress code; a new challenge clears it.
Code delivery accepts the HTTP endpoint's empty `202` acknowledgement, then
checks the original operation receipt. Acceptance alone never means signed in.
A metadata-only submission marker is saved before sending: remounts and lost
replies reconcile the existing attempt without submitting the code again.
An explicit HTTP rejection before enqueue clears only that submission marker,
retains cancellation intent, and permits an explicit code retry in the original
login. Transport failures and server errors keep delivery unconfirmed.
Verification hides the authorization input and link. Returning from the browser
resumes status checks, including identity binding, with bounded visible polling;
timeouts and paused checks retain an explicit recovery action. Success exposes
details and an explicit choice to sign in on another Machine, without starting
that login automatically.
Named account authorization opens an external browser rather than replacing the
native shell's shared authentication sheet. A failed OS-browser launch offers
copy-link recovery, without silently navigating that shared sheet. Browser/PWA
links use a fresh `noopener noreferrer` tab. These presentation choices do not
isolate Provider cookies: the UI offers an explicit copy-link path to a separate
browser profile (or a fresh private session) and asks the user to check the
Provider account before authorizing. No cross-origin cookie clearing, automatic
sign-out, account switching, or OAuth URL rewriting is performed. Independent
[Safari profiles](https://support.apple.com/en-us/105100) have separate cookie
stores; an ordinary new tab is not that boundary. This UI verification does not
replace real two-account runtime and revocation acceptance.
Revocation has a separate explicit confirmation and typed HTTP/CLI/MCP entry;
it cannot be invoked through the metadata writer. Its immutable plan and
pending-target inventory can be observed and resumed from another device or
same-owner session. A second operation cannot replace an existing plan. These
interfaces do not establish native login or process cleanup acceptance.
New Direct-worker and native-auth launches persist cgroup ownership before spawn.
Same-boot recovery must verify the original boot/path/device/inode, terminate the
owned subtree and remove its cgroup before confirming exit. Native-auth orphan
recovery also checks the original Machine process incarnation; live owners and
legacy unknown ownership remain fenced. Process termination is independent from
native command success and cannot authorize credential deletion on an unknown
logout result. Real-account release acceptance remains separate from isolated
process crash tests.
Public named-session admission
must remain closed until those requirements pass. Synthetic tests and a passing
repository gate alone do not authorize enabling the unfinished runtime path.
For Claude, the native-owned credential architecture above is a prerequisite to
that acceptance, not a task that can be discharged by testing real credentials
against the current central-vault prototype.
Normal promotion re-reads and matches the owner-scoped DB intent before writing
credentials. The vault independently recomputes the candidate digest from the
exact Provider package, credential bundle and native identity fingerprint;
candidate substitution is rejected before the first write and on terminal
retries. Only the digest belongs in the DB. Promotion shares finalization and
cancellation cleanup with recovery.
Terminal retries return history without replacing newer credentials; uncertain
DB outcomes retain the durable vault proof rather than guessing a rollback.
Recovery of one operation rechecks its owner, Machine and currently enrolled
recipient key, then uses an existing vault only. It never restarts native login
or creates a grant. A cancelled journal is reconciled with conditional vault
cleanup; a newer generation is left intact. Operation-bound credentials cannot
use the legacy distribution, scheduling or refresh paths, even after DB
finalization: historical commit evidence is not current runtime authorization.
Wipe tombstones remain distributable so this admission fence cannot block cleanup.
The internal current-admission callback locks the owner's account registry and
the Machine enrollment row in one DB transaction. It requires a committed
operation, unchanged registry revision, enabled matching native account and the
same non-revoked enrolled recipient. Its borrowed admission cannot be serialized
or independently constructed. Sealing checks the exact vault commitment,
generation and ready state again. This defines an admission point, not a lease:
later revocation still requires runtime cleanup and the caller must use the
current authenticated Machine connection before dispatch.
Named credential synchronization, including reconnect synchronization, resolves
the vault commitment through a primary-key DB route to its owner and operation,
then uses current admission and exact-generation sealing. The route is inserted
atomically with journal finalization and is only a locator, never authority.
Missing routes refuse distribution without scanning accounts or falling back to
default credentials. Historical entries predating this route index require a
new verified login operation; migration does not fabricate authorization.
Signed-out named grants may send only wipe envelopes through the cleanup path,
including a recheck of the envelope action if the vault changes concurrently.
Named native refresh carries identity evidence from the existing signed probe's
isolated home; absence of a signed probe or identity recipe refuses promotion.
The Controller checks the recipe and bound identity within current account and
Machine admission. Refresh uses CAS on that Machine grant alone, retains the
original authenticated login commitment, and advances only the encrypted vault
generation. The journal's committed generation is the lineage's initial floor;
distribution seals the current authenticated generation in that same lineage.
Another login changes the commitment and cannot be authorized by the old route.
Default-account wire messages omit identity evidence, preserving their old shape.
Identity admission must observe a fresh native login in a clean, scoped home;
an existing config directory, email label or cached usage record alone is not
proof of the credential's account. Upstream issue #87447 reports conflicting
identity signals in Windows Desktop-hosted sessions; this is a regression case
to test, not evidence that Cowboy's pinned standalone CLI has the same defect.

Research references (checked 2026-10-17):

- [Claude Code authentication policy](https://code.claude.com/docs/en/legal-and-compliance)
- [Current SDK subscription guidance](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
- [Native Claude configuration directories](https://code.claude.com/docs/en/env-vars)
- [Native multi-account login and Console exceptions](https://code.claude.com/docs/en/authentication)
- [Reported Desktop config-root/account mismatch and author corrections](https://github.com/anthropics/claude-code/issues/87447)
- [CC Switch threat model](https://github.com/farion1231/cc-switch/security)
- [Historical Keychain isolation report](https://github.com/anthropics/claude-code/issues/20553)
- [Historical concurrent refresh report](https://github.com/anthropics/claude-code/issues/25609)
- [Bubblewrap security model](https://github.com/containers/bubblewrap/security)

Historical reports supply regression scenarios, not proof that the same bug
persists in the exact installed native CLI. Native CLI profile support owns
local authentication; Cowboy's residual responsibility is account-bound
scheduling, persistence, distribution and enforced runtime isolation. Replace
local adapters when native runtimes provide the same verified boundary. Codex
uses its native authentication extension surfaces; Claude remains adapter-backed
and must fail closed where an equivalent supported boundary is unavailable.

### CR-0: Integrations are versioned plugins over versioned components

Every first-party integration has a generic `plugin.json` manifest with an
exact ID, SemVer version, kind, entry point, and exact Cowboy component
dependencies. Agent integrations use the `agent_provider` kind; their Provider
contract is a typed payload, not a second release or installation unit.
Product-login integrations use the data-only `authentication_provider` kind
and execute only through Controller-owned protocol drivers. Zed uses the
`code_intelligence` kind and retains its isolated process and license boundary.

Workspace resources use the data-only `workspace_extension` kind (Plugin SDK
1.9+, release schema 3). They share the same exact signed Machine lifecycle.
Their bounded resource contracts consume core-owned workspace context and
declare any runtime Plugin dependencies by version and artifact digest. The
first host connection borrows the Machine user's existing GitHub CLI session
for repository-scoped reads and bounded account PR discovery through an installed
pull-request review capability. Explicit PR selection may cross repositories on
that same remote host; it creates no Provider login, credential replica
or executable runtime. See [workspace extensions](workspace-extensions.md).

Session extensions use the data-only `memory_backend` and `call_profile` kinds
(Plugin SDK 1.12+). A memory backend names a closed `cowboy.memory-client`
protocol, its exact MCP tool names and whether it recalls and observes; its
endpoint, credential and project mapping are a Machine-private binding. A call
profile names managed-call purposes and agent-facing guidance; grants,
snapshots, the read-only sandbox and placement stay core. See
[session extension Plugins](session-extension-plugins.md).

Native cross-Provider capabilities MUST be declared by a signed Provider profile
and discovered on demand through the existing MCP tools. Native runtimes own
their tools, authentication and model context; Cowboy owns scoped scheduling
and artifact transport. A media profile MUST retain read-only code execution,
avoid copying unrelated context, and require verified artifact delivery before
reporting success. See [managed native capabilities](managed-native-capabilities.md).

Telemetry integrations use the data-only `telemetry_backend` kind. Payload
schema 2 requires Plugin SDK 1.8+ and Machine protocol 9+ for OTLP/HTTP protobuf
logs, metrics and traces. Retained schema 1 requires SDK 1.7+/protocol 8+ for
legacy JSON-lines/Prometheus routes. Its signed contract declares
bounded ingestion routes and platform support, never
executable artifacts, service URLs, credentials, or Agent authentication.
It uses the same signed Catalog and Machine install/upgrade/rollback/uninstall
lifecycle. Installation alone grants no egress: Controller policy selects an
exact Machine/Plugin/version/digest and Machine-private policy independently
binds that release to endpoints. Private policy is not published package data.

Diagnostic telemetry defaults to private, host-local, rotating SQLite segments
containing official OTel protobuf records, without a network dependency. Legacy
JSONL remains selectable through a shared evidence interface. Each Machine keeps
its process logs and sampled runtime spans locally by default; explicit remote
queries filter and aggregate at the source. Retention, time/size rotation and
idle expiry are configurable. Local recording remains enabled alongside an
optional export backend. Remote queues, requests and retries are bounded independently from
local evidence and durable incident persistence. Conversation history,
accounting, authentication, and incident lifecycle state remain durable and
are never rotated with diagnostic files. See `docs/telemetry-plugins.md` for
configuration, delivery limits, operational failure counters and rollout. The
AI-facing CLI, source adapters and retention contract are specified in
[`local-observability.md`](local-observability.md).

Authentication payload schema 1 selects OIDC. Schema 2 also selects the closed
`local_password` and `webauthn` drivers, with an empty public configuration;
password algorithms, credentials, relying-party policy, users, and sessions
remain Controller-owned. Local authentication releases require a release-bound
host bundle. SDK build/sign/verify, Catalog ingestion, and Machine validation
check protocol/renderer/capability ownership together: Authentication hosts
cannot carry collector files, RPC, Agent runtime bindings, or unrelated native
capabilities. WebAuthn additionally requires its own migrated storage namespace.
An old OIDC release without a host remains valid but grants no host behavior.

Reusable implementation belongs to an owned component, not a plugin copy or an
external source-tree link. Component releases bind exact versions, source
paths, and source digests. A component source change requires a new component
release and higher versions throughout its actual package dependency closure.
Schema-2 history retains the coordinated all-Plugins-bump rule. Schema 3 starts
with an unchanged baseline and records complete internal package edges and
Plugin source/binding snapshots. Only affected Plugins must increase version;
an unchanged Plugin may keep its historical component-release label only when
every exact transitive input still matches the active matrix. Changed Plugin
source or bindings require a new Plugin version. `just plugin-check` enforces
this [release policy](plugin-components.md), including package pins and digests.

The generic Plugin layer exclusively owns signing, publication, Catalog
selection, Machine generations, activation, rollback, and uninstall. Narrower
capability contracts may validate payloads and provide behavior but cannot own
a second lifecycle. Code-intelligence Plugins remain outside the Agent
execution and authentication model.

### CR-1: Cowboy is Provider-agnostic

Cowboy core owns hosting, lifecycle, persistence, scheduling, component
rendering, security, and typed extension contracts. It must not require a new
agent-specific card, icon branch, loading branch, install flow, settings form,
or runtime protocol branch when a conforming Provider is added.

ACP, an adapter, gateway, agent CLI, model protocol, and their versions are
Provider-private implementation details. Ordinary UI presents the Provider;
only explicitly scoped developer diagnostics may name those internals.

### CR-2: Plugin is the product and installation unit

`claude-code`, `codex`, `gemini`, `grok`, `claude-deepseek`, and
`codex-deepseek` are independent Agent Provider Plugins; `zed` is an independent
code-intelligence Plugin. Each builds, versions, signs, publishes, installs,
upgrades, rolls back, and uninstalls through the same Plugin lifecycle.

A user installs a Plugin version on a selected Machine. Cowboy never asks the
user to install an ACP runtime, adapter, gateway, managed Node, or CLI as a
separate product component. Internal content-addressed blobs may be deduplicated
and leased without changing this UI or lifecycle boundary.

### CR-3: Every release is immutable and independently buildable

A Plugin builds independently against the generic Plugin SDK and its exact
component release. Agent payloads additionally build against the Provider SDK
and contract bundle. First-party sources may be co-located in this repository, but
each has its own source manifest, version, build invocation, artifact, release
envelope, signature, and install transaction. Its data-only package includes
all universal UI and contract layers; its signed release binds exactly one
runtime-artifact set for every declared platform. Its identity binds Plugin ID,
kind, Plugin version, component release, package digest, composite artifact
digest, contract fingerprint, runtime-artifact matrix, and publisher.

Every internal executable and protocol dependency is pinned to an exact version
and digest inside the Plugin. Moving tags, version ranges, `latest`, runtime
package installation, and dependency-owned auto-updaters are forbidden. Any
dependency change creates a new Provider version and new artifact bytes.

If a Provider needs a gateway or another auxiliary runtime, it declares a
closed, typed session-sidecar contract. The worker allocates a unique loopback
endpoint, launches the exact generation-local component, waits for its declared
health path, and resolves signed `sidecar_url` bindings only after readiness.
Component executable bindings are resolved from the same installed generation.
No fixed Machine-global gateway, host profile path, shell template, or ambient
runtime download may complete an installable Plugin release.

### CR-4: Cowboy supplies the typed component library

The Provider SDK exposes typed Cowboy components and shell slots for logos,
icons, loading, cards, setup, settings, information, empty, error, and session
surfaces. A Provider may compose and lay out approved primitives inside those
slots. Cowboy retains the application shell, responsive behavior, keyboard and
touch semantics, focus, accessibility, theme, localization, error isolation,
and destructive confirmation.

Providers do not inject React, JavaScript, HTML, CSS, or DOM code. Authoring
types compile to canonical data-only UI IR that Cowboy validates and renders.
Provider-specific activity uses a closed indicator union and typed label
effects; Provider-specific vector color uses bounded gradient records. Cowboy
owns the animation implementation, reduced-motion behavior, accessibility, and
responsive mechanics. The host renderer must dispatch only on schema kinds,
never on Provider IDs.

Provider-specific Transcript thoughts use a separate closed presentation
contract. A Provider selects one Cowboy-owned `timeline`, `workcell`, `signal`,
or `terminal` variant, a bounded density, an optional active label, and a
`plain` or `soft` current-step surface. Cowboy owns marker geometry, typography,
color application, motion, reading-scale behavior, and accessibility. The
contract cannot carry CSS, DOM, markup, arbitrary dimensions, or executable
rendering logic.

Service authentication presentation follows the same boundary. Cowboy derives
the closed `account` or `api_key` presentation from the Provider's typed auth
method union. Account Providers render sign-in language; Providers whose only
declared methods are `secret_input` render API-key status, actions, and modal
copy. Cowboy owns card geometry and interaction mechanics, and Web must not
branch on a Provider ID to choose either presentation.

Ordinary Web renders `information` and `setup` once in a Cowboy Service
authentication region, while each Machine renders only `card` and its
`empty`/`settings` lifecycle surface. An installed Machine card is rendered from
that exact installed package; a newer Catalog entry is only the upgrade target.
Runtime-advertised configuration options remain protocol data, but any
Provider-specific ordering, full-width layout, or session-lifecycle
availability is declared in that signed package's typed
`configuration.options` contract. Cowboy UI may supply neutral defaults for
portable option concepts; it must not branch on a Provider-specific option ID.
Provider-specific tool rendering follows the same boundary: the exact package
may map an upstream tool name to a closed Cowboy renderer through
`host.tool_presentations`; Cowboy Web must not keep a Provider-ID/tool-name
dispatch table.

### CR-5: Linked behavior remains type-safe

Provider UI behavior uses closed state schemas, typed messages, pure reducers,
derived expressions, and capability-mediated effects. Component props, event
payloads, reducer results, effects, and effect results are checked during build
and checked again when Cowboy installs the artifact. Configuration-option
presentation records are a closed union with unique typed option IDs, bounded
order values, layout, and availability policies; malformed or duplicate rules
fail both Rust package validation and TypeScript Catalog validation. Tool
presentation declarations likewise use unique bounded names and a closed
renderer union.

Host integration schema 1 forbids Transcript presentation fields. Host schema
2 requires Transcript presentation schema 1, validates its exact field set and
closed tokens, and includes it in the UI contract fingerprint. Unknown host or
Transcript schema versions, variants, tokens, fields, control characters, and
oversized active labels fail in both Rust and TypeScript validation.

The UI schema version is part of the runtime type boundary. Cowboy accepts only
an explicitly supported interval, rejects fields introduced after the declared
schema version, and validates every activity frame, phrase, interval, asset
link, gradient stop, and responsive layout constraint in both the Rust package
validator and TypeScript Catalog validator. A new union member requires a new
schema version and host implementation; unknown members fail closed.

Effect ownership is also structural: Service authentication and logout may be
emitted only from `setup`, Machine installation only from `empty`, and Machine
upgrade or uninstall only from `settings`. The host may additionally block a
capability outright — an unpublished release, or a Machine whose Plugins are
declared Service-side — and a blocked action renders nothing at all. Documentation links are the only
lifecycle-neutral effect. Both Rust package validation and TypeScript Catalog
validation enforce the same mapping before rendering.

The current schema has no executable logic escape hatch. Logic that exceeds the
closed DSL requires a future versioned SDK schema and corresponding Cowboy host
implementation; a Provider cannot inject JavaScript, WebAssembly, or ambient
DOM, credential, network, process, filesystem, clock, or randomness access.

### CR-6: Compatibility is derived and fail-closed

SDK SemVer and a Provider-authored compatibility interval are only pre-filters.
The SDK pre-filter accepts the same major only and rejects a Provider built
against a newer SDK than the validating Cowboy host. Cowboy then derives
requirements from the actual UI IR, assets, state machines, effects, host
profiles, runtime commands, exact dependency links, platform matrix, and
authentication contract.

Runtime arguments and environment values are either bounded literals or closed
host bindings to a declared private component command or declared session
sidecar URL. Sidecar IDs, component kinds/slots, loopback transport, readiness,
credential-environment forwarding, and every cross-reference are validated in
both SDK implementations. Unknown, dangling, or platform-incomplete links fail
before installation.

Build, Catalog ingestion, and Machine installation validate schema versions,
structural types, closed component/capability enums, canonical fingerprints,
Controller/Machine contract intervals, complete platform/component binding,
package and composite digests, publisher signature, URL/archive bounds, and
probes. The target Machine independently inspects downloaded bytes. An
incompatible artifact never replaces the active generation.

The generic Plugin SDK owns the versioned `PluginHostSpec`. SDK packaging,
Catalog ingestion, and Machine staging independently parse and validate that
same semantic contract, including renderer/slot relationships, storage SQL,
process grants, and references to exact signed runtime files. Core may project
validated fields but must not maintain a second host schema or weaker builder
validator.

Each Machine advertises two strict capability envelopes in its signed hello.
`PluginContractInventory` covers the generic Plugin SDK, manifest, outer
package/release, payload-kind, host-bundle, and host-contract schema intervals.
`ProviderContractInventory` independently covers the narrower Agent Provider
SDK, package/runtime-binding/UI/host schemas, and Machine Provider contract.
Telemetry schema-one support is derived from the already-signed Plugin SDK
version (>=1.7); the historical challenge-proof-v3 envelope is unchanged.
The Controller repeats both applicable predicates immediately before an install
or upgrade, and Web may offer only the newest ready release accepted by both.
An absent, malformed, or insufficient inventory fails closed with a typed
compatibility code and an update-Machine explanation; Cowboy must not send a
newer package to an older decoder and expose its raw deserialization error.

### CR-7: Installation is dynamic and Machine-scoped

The Cowboy UI joins Catalog releases with one Machine's platform, contracts,
current installation, and health. Install and upgrade use immutable references,
stage side by side, probe before activation, preserve the prior generation on
failure, and retain leased generations for existing sessions and rollback.
Each session launches auxiliary components from its exact retained generation
on its own dynamic loopback endpoint. Old and new Provider or authentication
generations may therefore drain concurrently without sharing a fixed port or
silently adopting replacement runtime bytes.

Publishing a Plugin release does not install it. Installing it on one Machine
does not install it on another Machine.

Which Plugins a Machine runs may instead be declared Service-side, per Machine,
in the document described by
[Machine Plugin membership](machine-plugin-membership.md). That declaration is
the explicit installation decision this requirement asks for — publication
still installs nothing — and convergence applies it. A declared Machine's
clients therefore offer no lifecycle action: the Controller refuses
client-initiated installs, uninstall previews and uninstalls for it, and the
host blocks those capabilities so the Provider's settings surface renders
nothing for them. An undeclared Machine is unchanged in every respect.
Automatic removal never confirms past an affected session.

The [core install continuation](plugin-install-continuation.md) requires an exact
version/digest and binds the current Operator confirmation to the complete
resolved envelope and original Machine connection. An admitted attempt outlives
its HTTP observer and checks authority at effect boundaries. An ambiguous or
generic rejected install ACK retains a live reconciliation fence, never a blind
retry or inverse. The subsequent [Service installation journal](plugin-install-journal.md)
adds an exact operation ID, durable intent/progress and restart fence recovery
to that same executor, behind an active/recovery/cold-reader rollout. Machine
install receipts and independently confirmed restoration remain separate work;
a generic ACK and current inventory cannot establish a historical unknown effect.

Install and uninstall serialize per `(machine_id, provider_id)`. Installation
validates any current Service auth envelope before changing activation and
restores the previous runtime and auth links if the commit fails. Reusing a
retained generation re-hashes its stored runtime artifacts against the signed
release matrix rather than trusting cache metadata or executable existence.

### CR-8: Authentication is Cowboy Service-scoped

Cowboy performs one Provider login for the entire Cowboy Service. Ordinary UI
must not offer Machine-specific Provider login, logout, account selection, or
credential entry. A successful login creates a monotonic `auth_generation` in a
Service-owned encrypted credential vault; the vault key is not stored in the
ordinary Cowboy database.

Authentication starts against an exact signed `(provider_id, version,
artifact_digest, auth_contract_fingerprint)` and may use only a connected
Machine with that exact release active as its temporary executor. The returned
candidate must repeat the same immutable identity before Service commit; an
upgrade race or mismatched method fails closed.

The Service Catalog advertises the deduplicated signed release identities that
are active on connected Machines without exposing Machine identity. The UI may
render newer compatible presentation assets, but it starts a selected method
with the newest advertised exact release that declares that method. If no such
executor exists, the UI must show an actionable install-or-upgrade error rather
than swallowing the failed request or weakening exact-release validation.

Cowboy automatically reconciles that generation to every enrolled, authorized,
non-revoked Machine. Each credential bundle is sealed to the target Machine's
enrollment key, stored in a private Service-managed replica area, and exposed
only to the matching Provider runtime. Every online Machine acknowledges sealed
replica storage; a Machine with the Provider installed additionally acknowledges
typed materialization. The Service reports distribution `current` only after
applicable acknowledgements; partial reachability is `partial`. Offline Machines
remain pending and reconcile automatically on reconnect. A newly enrolled
Machine receives the current generation without another login.

The Service owns auth state `signed_out`, `authenticating`, `ready`, `expired`,
or `error`, plus distribution `none`, `pending`, `current`, `partial`, `failed`,
or `revoking`. A Machine may report only sealed-replica convergence
such as `pending`, `storing`, `current`, `failed`, or `revoking`, plus
materialization `not-installed`, `applying`, `current`, or `failed`. Those are
not login states. An offline Machine may degrade distribution without changing
the Service from authenticated `ready` or blocking a current online Machine.

An active login and its pre-commit error are Service-wide transient status, not
credential generations. They may report generation zero before the first
successful commit, are visible through the Catalog, and are never sealed or
replicated. Scheduling and Machine synchronization consult only a durable,
validated generation.

Temporary native login requests belong to the initiating product user. Only
that user may resume the request, read its challenges, submit a code, or cancel
it. A concurrent login by another user reports a conflict without disclosing
the request. Shared Machine diagnostics exclude login challenges, login state,
credential candidates, and login command results; the owner-checked login
endpoint is the sole challenge-reading surface.
Login publication and command enqueue are serialized with cancellation; rejected
or cancelled requests cannot enqueue a code submission. Challenge responses
must not be cached.

Each Provider declares a typed authentication contract: login flow, portable
credential schema, Machine projection schema, validation, import, refresh,
revocation, and wipe behavior. Refresh either remains Service-owned with
short-lived Machine projections or returns a sealed candidate through a
compare-and-swap generation update that the Service validates and redistributes.
For Machine-owned refresh, the signed replica remains an immutable local
materialization while Provider-owned non-credential state remains isolated by
the session's recorded generation. Declared refreshable credential files for
compatible generations on one Machine share one writable lineage; copied
rotating refresh tokens are forbidden. Machine protocol 6 reports a complete
changed runtime bundle as a secret-bearing CAS candidate; the Controller
commits at most one next Service generation, seals it to every Machine, and
reconciles every runtime view to that winner. Missing or partial runtime
credentials are failures to repair from Service authority, never candidates.
Replicas may not drift into independent accounts.

If upstream credentials are non-exportable or Machine-bound, the Provider must
implement a safe Service broker or token-exchange projection. Otherwise it is
incompatible with Cowboy Service authentication; Cowboy must not fall back to
per-Machine manual login.

Service logout blocks new Provider sessions immediately, invalidates the active
authentication generation, and automatically distributes a signed wipe or
revocation generation. Provider uninstall on one Machine wipes that Machine's
materialized Provider state but does not log out the Cowboy Service.

### CR-9: Sessions bind exact generations

Every session persists Machine ID, Provider ID, Provider version, and Provider
generation digest. Existing sessions may drain on their leased generation while
new sessions use the active generation. Provider names or mutable active links
are never sufficient session identity.

Ordinary session Reload retains that exact release. An explicit idle-only
"Load installed Provider" confirmation, or an explicitly enabled per-session
"automatic Provider updates" policy, may rebind one existing session to its
Machine's trusted active release, bound to the confirmed artifact digest. It
must validate compatible authentication, native-session, and configuration
contracts, retain the original auth-generation runtime home, and fence racing
prompts until the replacement is ready. Cowboy/native IDs, workspace, history,
queue, drafts, and user preferences survive; unsupported preferences follow
the runtime's advertised compatible values. Resume failure is visible and must
never fall back to a blank native session. This shared ACP contract applies to
Codex and followers; a Provider without native resume/load support fails closed.
It does not install software, migrate accounts, or restart other sessions.
The automatic policy is persisted by the Controller and defaults off. Every
30 seconds it considers only running, idle sessions and newer stable installed
versions. It uses the same compatibility checks and prompt fence, never revives
stopped/failed sessions, never downgrades, and keeps selected model preferences.
The policy continues while the Web client is closed.
If a reload is interrupted before the reset and persistence both complete,
reconnection reconciles the release binding with the surviving worker only
when its native ID, workspace, and auth-generation home still match. The UI
then reports the actual release; an upgrade not applied can be retried.

A new session is schedulable only when its Machine has the Provider installed,
the required Service authentication is ready, and the Machine has acknowledged
materializing and probing the current authentication generation.

If an Agent rejects a projected credential generation before allocating a
native ACP session, Cowboy marks only that exact Service generation expired and
must not classify the rejection as a worker-generation rollout failure. After
an explicit successful Service login, the failed registration may compare-and-
swap to the newer auth generation only when the signed auth contract is
unchanged and the selected Machine acknowledged that exact projection. Once a
native ACP session ID exists, its auth-generation identity remains immutable.
A successful refresh or explicit reconnect of the same account may rotate the
credential lineage used by that session; when that changes the credentials
its worker reads, it must drain and resume the exact native session rather
than discard its history. Switching accounts still
requires a new session.

### CR-10: Uninstall is explicit and recoverable for a bounded period

Uninstalling `(machine_id, provider_id)` blocks new matching sessions, identifies
the exact impacted session and active-turn set, soft-deletes those sessions from
ordinary UI, drains or explicitly cancels workers, and releases the installation
lease. It does not affect that Provider's sessions on another Machine.

The confirmation modal names the Machine and Provider, counts idle and active
sessions, distinguishes drain from cancellation, lists retained data classes,
states that source projects and user worktrees remain untouched, and displays
the absolute `purge_after_at` date. Permanent cleanup runs only after that date.
Reinstall does not silently restore soft-deleted sessions. Hard deletion
cascades session events; content-addressed event attachments are reference
scanned and deleted only when no retained event references them and their
race-avoidance grace period has elapsed.

Confirmed uninstall records core-owned durable intent before stopping workers or
sending a remote command. Service session soft-delete and operation completion
commit in one local transaction. HTTP observer cancellation does not cancel an
admitted operation. Restart reconstructs unfinished installation-slot fences
before dispatch, without replaying remote commands or stopping unrelated
workers.

A `NeedsAttention` interruption proven to originate from `Prepared`, before any
worker/Plugin mutation, has one independently confirmed local resolution:
`abort_before_effects`. Current Operator authorization, a bounded process-local
permit and a complete operation-snapshot CAS are required. The new confirming
actor's audit receipt and parent `Aborted` state commit together; the original
intent and failure evidence remain intact. This does not mutate sessions or the
Machine, renew the uninstall grant, restore workers, or authorize clearing a
later-phase fence. See [pre-effect resolution](plugin-no-effect-resolution.md).

A correlated ordinary Machine rejection or a definitively uncommitted local
transaction permits the approved compensation on the original connection:
re-verify/reactivate the exact retained signed generation and request
restoration of the workers that were live. Missing ACKs, changed connections,
expired recovery authority or ambiguous remote outcomes remain fenced, never
blindly retried. The operation reports both primary and compensation failure; a
worker reload enqueue is not verified recovery and remains `NeedsAttention`. It
must never report a successful uninstall with only half of the Machine/session
state committed. See the
[Service operation journal](plugin-operation-journal.md) for its reader-floor
rollout, bounded retention and current recovery limits.

Protocol 10 adds a closed, Machine-owned durable uninstall step and read-only
query, bound to the complete Service intent digest. Identical requests return
saved evidence without re-execution; changed arguments under the same identity
are rejected. A partial effect or missing final durable receipt remains unknown.
The Machine defaults to reader-only until its separate reader-floor maintenance
cutover. This path never compensates through legacy unjournaled reactivation;
installation CAS and verified recovery authorization remain prerequisites. See
[Machine operation receipts](machine-plugin-operation-receipts.md).

Explicit offline host maintenance may finish a recorded failed cleanup only
for the exact already-absent installation, under the native journal owner lock,
with a fresh bounded invocation and an archived original receipt. The Controller
can then complete a zero-session `Uninstalling` interruption from the exact
applied receipt and matching current tombstone, using fresh Operator authority,
an operation-snapshot CAS and an atomic resolution audit. It never derives
success from absence alone or replays the expired forward command. See
[uninstall maintenance](plugin-recovery-assessment.md).

### CR-11: Release automation belongs to this repository

The canonical Provider dependency-audit and release procedure is the repository
skill at
[`../.agents/skills/release-cowboy-plugin/SKILL.md`](../.agents/skills/release-cowboy-plugin/SKILL.md).
It is versioned, reviewed, and released with Cowboy's Provider contracts. Do not
fork or hand-maintain it as a user-home skill.

The skill audits internal dependency updates, updates exact pins, runs Provider
and Cowboy conformance gates, builds and signs one immutable Plugin release,
publishes it, and verifies Catalog availability. It stops before Machine
installation and never performs Service login as a release side effect.

### CR-12: Trust is re-established at every boundary

A successful Provider build receipt is evidence, not installation authority.
Cowboy verifies publisher trust, package and composite digests, signature,
archive safety, interfaces, capabilities, complete platform/runtime binding,
exact dependency versions, and probes again. Credentials, login codes, tokens,
and credential-bearing state never enter UI IR, logs, telemetry, Catalog
metadata, Provider artifacts, or ordinary Cowboy database rows.

## Minimum acceptance suite

The Provider platform is not complete until automated acceptance proves:

- each Provider builds independently from clean source against an exact SDK
  contract without depending on another Provider's source or artifact;
- malformed, mistyped, over-capable, unsigned, incompatible, and wrong-platform
  packages fail before activation;
- a release can install, upgrade, roll back, and uninstall independently while
  existing sessions retain their exact generation;
- install/uninstall races are fenced, corrupt retained runtime bytes fail
  reactivation, and a returned database/Machine failure restores the exact
  pre-uninstall generation and live-session set;
- one Cowboy Service login automatically converges the same authentication
  replica to every connected Machine, materializes it on every installed
  Provider, and later converges it to an offline or newly enrolled Machine;
- concurrent credential refresh cannot fork the authoritative generation;
- Service logout blocks new sessions and causes every Machine replica to be
  wiped or marked pending revocation until it reconnects;
- uninstalling one Machine's Provider removes only its affected sessions and
  does not delete Service authentication, source projects, or worktrees; and
- session hard purge removes cascaded events and later reclaims only truly
  unreferenced event attachments.

## Implemented migration boundary

Plugin package schema 1, Plugin release schemas 1-2, Provider payload schema 2, Agent runtime-binding
schema 2, UI schemas 1-2, host integration schemas 1-2, Controller contract 2,
Provider Machine contract 4, Machine protocol 7, and Cowboy Provider SDK 3.0 in
both Rust and TypeScript are the active contract. The Plugin Catalog embeds all
independently compiled first-party manifests as
typed `unbound` entries and accepts installable releases only after an external
`.cowboy-plugin` package is paired with a complete, signed runtime envelope.
Controller host runtimes are immutable snapshots keyed by the exact Plugin
release identity. Refresh validates and stages every candidate generation,
applies required Plugin migrations, and atomically publishes Catalog entries
and runtime hosts together. Old exact generations remain resolvable while
captured requests and session leases drain; an exact lookup never falls through
to another version. After an ID first receives trusted Catalog authority, a
persistent private marker prevents an absent or interrupted later scan from
silently restoring source-bundled behavior.
Controller host enablement is separate from Catalog publication. The private
`COWBOY_PLUGIN_HOST_CONFIG` policy exact-pins host release identities and merges
the OIDC driver's existing exact authentication selection; conflicts fail
startup. Its opt-in `catalog_only` mode disables every source-bundled host and,
under legacy local-security policy, requires enabled login methods and durable admin/Product WebAuthn storage
to have matching selected, release-bound Authentication hosts before core DB
migration or host staging. Legacy non-Plugin OIDC must migrate before cutover.
Unselected Authentication releases do not become defaults or run migrations
just because they are published, and public auth status contains only enabled
methods/panels. Every released host that declares Controller storage also
requires an exact selection, regardless of Plugin kind and in both `bootstrap`
and `catalog_only` modes. An unselected storage release remains discoverable
by exact identity but is not an ID default and cannot create or migrate a
namespace. Stateless Agent/Code defaults may still follow the unambiguous
latest release; adding storage to a newer release cannot inherit that authority
or fall back to an older ID default. Only pre-Catalog source bootstrap storage
retains its explicit compatibility path. Bootstrap readiness must reject a
missing required WebAuthn storage selection before connecting or migrating the
core database, including when persistent Catalog authority excludes a source
fallback. Missing pinned releases reject refresh without changing the
visible snapshot; selected-host activation errors abort startup. Successful
Catalog-only activation durably records a one-way marker, so a later omitted
policy cannot silently restore bootstrap mode. Default bootstrap remains a
migration path, not authority over a previously signed Plugin.
Explicit `COWBOY_CORE_SECURITY_CONFIG` replaces local Password/Passkey host
requirements with core storage authority; it requires Catalog-only source and
rejects local-auth pins. External OIDC keeps exact signed selections and the
configured login policy. Prepared/ready authority and namespace tokens prevent
legacy fallback, relocation or Plugin migration after handoff. Removing policy
or storage cannot silently recreate credentials. Existing deployments retain
their prior mode until an accepted stopped-Controller cutover.
`cowboy serve --check-plugin-hosts` uses the same configuration and signature
preflight without creating Service/Catalog state, connecting to a database,
staging hosts, running migrations or contacting authentication services. Its
data-only, secret-free report identifies the selected releases and explicitly
distinguishes configuration validation from unperformed activation and storage
checks. Run it with the same protected environment and arguments as the
candidate Controller. Ordinary startup must perform that same check before
creating Service identities or caches; private host/login configuration parse
failures must not echo secret-bearing input values or field names.
Target Machines repeat package, composite digest, publisher, contract,
platform, private dependency, archive, host-bundle, and staged-probe checks before
atomically changing their active generation.

Released usage collection, reset, and activity transforms execute only on a
connected protocol-seven Machine whose active `(plugin_id, plugin_version,
artifact_digest, auth_generation)` exactly matches a retained signed host
generation. The Controller sends a closed operation and bounded JSON payload,
never argv or a path. The Machine revalidates the package, release, host-bundle
bytes and staged host files under its lifecycle lock, resolves component
commands and authentication home from that generation, and runs the signed
collector with no ambient Machine or Controller secrets. Signed
`collector_sidecars` declarations resolve other active exact Provider
generations by adapter slot and sidecar id; dynamic loopback endpoints exist
only for the invocation. Sensitive responses are correlated directly and do
not enter Machine event history or durable inventory. Source-bundled hosts may
use the bounded Controller executor only during the explicit pre-Catalog
bootstrap transition.

Protocol-seven Machines also advertise a proof-v3-bound generic Plugin
contract inventory. Install and upgrade require that inventory and pass the
outer manifest/package/release/payload/host compatibility predicate before
bytes cross the control channel; Agent Plugins then pass the independent
Provider predicate. Web applies the same composition when choosing an upgrade.
Older Machines may still uninstall or reactivate retained exact generations on
protocol 5, but cannot receive a new generic Plugin envelope.

Exact package workers receive a Machine-verified command map for every private
component. Cowboy prepends only those generation-local directories to the
worker path, resolves component-command bindings without a shell, starts
declared gateways as session-owned sidecars on dynamic loopback ports, requires
their typed readiness probes, and keeps their process handles for the complete
worker lifetime. This makes the Provider, rather than a Machine-global adapter,
CLI, gateway, or resource path, the executable installation unit.

Web discovers all installable extensions from `/api/plugins`. The Agent
capability projection renders Provider UI IR and assets while install, upgrade,
rollback, and uninstall remain Plugin lifecycle actions. ACP, adapters,
gateways, and managed CLI components remain
available only to developer diagnostics. The Service authentication surface is
rendered once outside Machine cards; its lifecycle details and each Machine's
Provider lifecycle details are collapsed behind compact summaries by default.
Machine cards use the exact installed manifest and treat the latest ready
Catalog release only as an upgrade target. Session execution stays pinned to
its exact generation; generic shell chrome may adopt the latest compatible
signed presentation for the same Provider when an exact session uses an older
UI or host-presentation schema and would otherwise retain stale brand, activity,
or Transcript UI.
Sessions persist exact Provider and auth generations. Service auth uses one
encrypted, monotonic durable generation plus a Service-wide transient login
status, and automatically seals durable state to enrolled Machines; offline
replicas remain pending and converge on reconnect. Uninstall uses an expiring
exact-impact plan, active
turn confirmation, an absolute three-day purge deadline, and reference-aware
attachment cleanup. Machine protocol 7 adds exact-generation Plugin host
execution, proof-v3 generic compatibility inventory, and new install/upgrade.
Uninstall and retained-generation reactivation remain compatible from protocol
5, while secret-bearing Provider refresh CAS candidates require protocol 6.
Agent auth commands remain capability-specific. Plugin
lifecycle fences serialize install/uninstall, auth generations reject stale or
conflicting replicas, and session launch resolves the writable runtime copy of
the exact recorded auth projection.

The legacy in-tree launch registry is retained only to drain old sessions that
lack an exact package generation. New Machine-backed sessions must resolve an
active signed Agent Plugin generation. Remove that fallback after the last legacy
session generation is no longer restorable.
