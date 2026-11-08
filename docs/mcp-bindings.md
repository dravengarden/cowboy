# MCP Plugin bindings

Status: local implementation under construction. Installation, session policy
the Linux Machine process primitive and versioned Machine transport are implemented.
The parent-authorized relay is implemented behind a disabled gateway capability;
Codex and Claude bound native launchers can consume a prepared plan and probe
its exact target CLI. Controller plan delivery and live compatibility admission
remain unconnected.
The policy API reports preparation, worker readiness and installed-grant readiness
separately. No MCP process starts from this API.

## Installed server

Plugin SDK 1.13 introduces `mcp_server`, release envelope 6, and the closed
`stdio_session_v1` contract. Each platform has one signed self-contained runtime
artifact, a command, fixed arguments and a bounded list of tool names. No endpoint,
shell, environment or credential is accepted in the contract. Existing Plugin
verification, private artifact staging, activation and uninstall own this lifecycle.

`MachinePluginStore::prepare_mcp_binding` requires the exact active generation,
verifies its installed bytes, and checks that requested tools are a subset of the
signed contract. A prepared executable is not execution authority.

The contract may also declare up to eight `sources`: names of external systems the
server can read (for example `github`). They are names only. A binding grants a
subset through the same owner policy, Machine preparation rejects an unsigned
source, and the immutable process profile carries the granted names. The server
refuses an ungranted source before reaching it; a granted source uses credentials
already provisioned for the execution Machine user, never credentials in the
contract, policy or profile.

## Signed startup discovery

Native MCP clients may request initialize/tools/list before their parent gateway
is reconciled. `src/agent_api/mcp.rs::available` already handles this ordering for
Cowboy's fixed tools. A generic binding must not start an authorized remote
process to obtain its initial tool schemas, which can deadlock worker startup.

`McpServerContract.discovery` optionally carries the exact signed tool definitions
(name, description and object input schema), capped at 32 KiB. Duplicate/missing
names, external schema references, schema IDs and excessive nesting are rejected.
`discovery_for` projects only the selected signed subset. Missing metadata remains
valid for transport-only packages, but cannot produce native startup discovery.
`PreparedMcpBinding::discovery` verifies through normal installed-generation
preparation and returns metadata without launching the server or granting access.

The native bridge consumes this metadata for startup; actual tool calls still
require the live relay. Machine-side descriptor preparation is implemented;
delivery into provider launchers and runtime negotiation are not yet connected. SDK tests cover exact projection and invalid schemas; the signed
Machine fixture checks discovery from the verified generation. No provider is
claimed compatible based only on these tests.

## Native preparation and compatibility probe

Authenticated Machine transport `prepare_native` accepts a Controller-derived
process profile. `McpSessions::prepare_native` uses a live connection lease and
the Plugin lifecycle fence to resolve the exact signed generation and selected
tool metadata. It starts no MCP server. The response carries the descriptor and
its absolute execution-Machine path; that path is not a runtime-Machine path.

Files use random temporary names and mode 0600. Each successful preparation
returns a distinct opaque `native-` receipt. Up to 256 live receipts share files
for identical profile/metadata selections. Releasing one receipt cannot unlink
a file another receipt owns; a changed epoch can coexist until old owners release.
A weak reuse index never retains a file after its last receipt is released.
`release_native` is connection-local, synchronous, idempotent and bypasses read
capacity. New receipts remain pending for 30 seconds. A single weakly owned
sweeper removes expired pending receipts without another request; the weak index
and shared-file ownership preserve confirmed holders. Confirmation checks expiry
under the same lock and cannot revive an expired or released receipt. Router drop
releases all remaining receipts.

`MachineControl::prepare_native_mcp` validates returned binding, epoch, schema
and path, confirms ownership over the original connection, then returns an owned
preparation. Drop sends release only to the original
connection, including after malformed metadata with a valid receipt. Replacement
connections never receive stale cleanup. This helper does not admit policy.
A lost prepare reply leaves only a bounded pending receipt. Once the Controller
knows a receipt, cancellation or a lost confirmation reply releases it through
its existing owner guard. Prepare/confirm require Machine protocol 43; ordinary
MCP transport still requires 42. Confirmation bypasses read capacity and grants
no policy authority. Live plan delivery must still retain the confirmed owner.

`MachineControl::prepare_native_mcp_plan` prepares up to eight selections in
parallel using the existing single-descriptor confirmation path. It rejects empty
or oversized sets, duplicate names, invalid CLI identity and mixed service,
session, runtime/execution Machine, workspace or policy epoch before sending.
Only a fully confirmed set returns a `NativeMcpLaunchPreparation`; it owns every
receipt and exposes the existing `{schema, command, bindings}` launcher shape in
input order. Failure or cancellation drops known owners; unknown replies remain
bounded by pending-receipt expiry. The caller must supply a keeper-verified CLI,
recheck live policy/placement at delivery, and retain the owner for the worker's
lifetime. Preparation is not readiness and does not authorize a grant.

`cowboy mcp --binding-capabilities` reports this exact CLI's closed
`NativeBridgeCapabilities` contract without reading gateway wiring or stdin.
It is mutually exclusive with `--binding`. Compatibility requires descriptor
schema 1, relay schema 1, support for the MCP grant field and
`stdio_tool_only_v1`; absent, partial, unknown or mismatched reports fail closed.
A consumer must probe the actual execution-side CLI it will launch. Neither a
current resident Machine nor this repository's build proves that a retained
execution environment uses that CLI. The probe does not grant access.

Controller-owned plans now feed launcher delivery; grant eligibility requires the
exact ready worker and current policy epoch (see Session owner and delivery).

## Claude launcher consumption

The shared `components/provider-runtime/mcp-bindings.mjs` consumes this optional
field in the runtime Machine's private session-extension descriptor:

```json
{
  "mcp": {
    "schema": 1,
    "command": "/immutable/generation/bin/cowboy",
    "bindings": [
      {"name": "suger", "descriptor": "/private/execution-machine/binding.json"}
    ]
  }
}
```

Only schema, absolute command and one to eight distinct name/absolute-descriptor
pairs are accepted. Commands, env and extra arguments cannot be supplied per
binding. The producer must resolve the command from the accepted immutable
execution generation; an arbitrary absolute path is not proof of that provenance.
The current Controller does not produce this plan.

Claude's bound launcher reuses `WorkspaceTools.command` to probe that exact
command once on the execution Machine (10-second collection bound, cancellation
on errors), then uses the same command for every binding through the existing
target stdio relay. The shared helper rejects unsuccessful, incomplete, oversized
or incompatible capability reports without echoing their output. It captures
the plan before awaiting and chooses collision-free native names, preserving
user-configured servers. No bindings means no probe. Managed profiles return no
entries and perform no probe even if a plan is present.

The runtime build bundles the shared helper and records its digest in provenance.
This is conditional launcher consumption, not completed negotiation: the worker
still needs to report verified readiness tied to its execution incarnation before
the Controller can enable the grant bit. Codex consumption is described below;
real provider acceptance remains outstanding.

The capability probe now requires a valid keeper-reported `cowboyAgentCli` matching
the plan's exact immutable command. Missing legacy pins, malformed identities and
mismatched commands fail before starting a probe process. Reconnects retain the
original command/hash/version; changed identity is refused, while JSON key order
is irrelevant. Both providers use that absolute command for ordinary Cowboy MCP
as well as binding MCP. Codex rechecks target identity at final native launch
without executing a second capability probe. The Controller observes the exact worker plan after adapter startup; an eligible
ready launch can receive `Grant.mcp`. No provider capability release is activated.

## Codex launcher consumption

Codex uses the same plan validator before starting its ACP adapter. It probes
through `execution-probe.mjs`, using the authenticated execution connection
shared with Claude, with configurable client name/request timeout and a fixed
keeper-reported CLI identity across reconnects. Both providers accept the same executor digest; the shared
client retains the exact executor-version, cwd and Linux checks.

One probe starts on the target with a fresh process ID and no runtime environment
copied into its request. Output is capped at 4 KiB and collected for at most ten
seconds after start; pending read RPCs use the remaining deadline. Unknown start
results, invalid output and timeout trigger termination of the original ID,
without replay. Declared or keeper-enforced managed execution refuses probing.
The connection is closed afterwards; the existing native MCP namespace relay is
unchanged. Connection establishment/start and cleanup have their own bounded RPCs;
ten seconds is the collection budget, not a whole-launch latency promise.

The native config carries `environment_id`, target cwd, the probed absolute
command, descriptor argv and an empty `COWBOY_CALL_CONTEXT` placeholder. Binding
names join native's awaited-server set. Local fallback is refused. Both runtime
builders package the shared modules and record their provenance digests.

Seven executor-fixture tests cover target routing, unknown-start cleanup, output
limits, repeated cursors, managed refusal and a stalled read. Existing connection
and provider tests cover the extraction. These are protocol fixtures, not a real
Codex conversation or authenticated Controller grant admission.

## Native stdio bridge

`cowboy mcp --binding <private-descriptor>` selects the generic binding bridge;
plain `cowboy mcp` retains its existing tools. `PreparedMcpBinding::native_descriptor`
produces the closed schema-one descriptor from verified generation metadata,
binding selection and epoch. The bridge requires an absolute, private regular
file owned by its effective user, refuses a final symlink, bounds reads to 64 KiB
and validates its tool schemas. It never executes a descriptor-supplied command
or falls back to local Suger mode.

Initialize/tools/list are answered from static discovery without gateway access.
The first admitted tool call lazily opens a relay handle, initializes the backend
and sends its initialized notification before forwarding the call. Later calls
retain that session and are serialized. Gateway wiring is read afresh each time;
errors are not retried. Client frames are capped at 256 KiB, queued input at eight
frames and remembered cancellation IDs at eight. Unsupported methods fail closed.

EOF, matching cancellation and handled termination signals end waiting work and
attempt close for a known handle, with a two-second cleanup bound. EOF also
interrupts blocked stdout. Unknown opens still rely on the remote lease deadline;
SIGKILL cannot run cleanup. The Controller handles authenticated close outside
its read-concurrency budget, while preserving exact grant/handle ownership checks.

Five bridge tests cover offline startup, first-call handshake/forwarding,
unsigned-tool refusal, cancellation/EOF cleanup, blocked stdout, oversized input
suffixes and private-file checks. These use a mock gateway, not native Codex/Claude
or a real Suger managed server. The Controller renews a live remote lease internally;
the bridge never reopens an expired handle or replays an uncertain call.
No provider descriptor injection or compatibility bit is enabled yet.

## Session policy

Product-authenticated `GET/PUT /api/sessions/{id}/mcp-bindings` uses the existing
session visibility and owner mutation checks. PUT accepts:

```json
{
  "expected_epoch": null,
  "bindings": [{
    "name": "suger",
    "server": {
      "plugin_id": "suger",
      "generation_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    "tools": ["suger_status", "suger_read"],
    "sources": ["github"]
  }]
}
```

The digest above is an illustrative value, not a published release. Initial
configuration uses a null epoch. Subsequent updates must send the returned epoch;
`bindings: []` revokes all selections. Every accepted update receives a new random
epoch, including restoring identical settings. A stale epoch returns 409. Storage
failure returns 503 without changing the in-memory policy. Invalid retained policy
returns 409; an owner may repair it only by a revoke-only update (`bindings: []`,
null `expected_epoch`), CAS-bound to the original raw value. JSON null tombstones
remain distinct from missing database rows.

The Controller uses `Store::compare_and_put_setting` before projecting the policy
into the Hub. Its accepted transaction survives an HTTP disconnect. Policy writes
are bounded to 16 accepted tasks and serialized through durable commit and
projection, with no delayed settings
write that could overwrite a later revoke. The registered setting prefix is
`session_mcp_bindings:`. Session removal tombstones this key. The writer rechecks
the live session after acquiring its mutex and refuses a changed placement. A
success response carries that write's own committed epoch, even if another update
has already committed. No provider default, fork or continuation inherits it.

The stored placement binds runtime Machine, execution Machine, workspace, cwd,
provider, owner and the exact execution binding. A process-local observation is
limited to 30 seconds and must be revalidated at delivery. Changed epoch or
placement, closing/trashed/system sessions, managed children and unknown execution
bindings are refused. This observation supplements the relay's authenticated
parent grant and enrolled connection; it is not a transport credential.
Admissions subscribe before reading policy. A bounded watch channel wakes
observers after committed updates; coalesced updates still require full current
policy revalidation. Failed CAS does not notify. This notification is not yet
connected to the cross-Machine relay; session placement changes and connection
lifetimes still need their own live authority checks.

## Linux Machine process primitive

`PluginExecutionScope::mcp` validates the service and execution Machine against
its live enrolled connection and creates a non-serializable owner/lease pair.
The profile pins session, runtime/execution Machines, workspace, epoch and binding.
Dropping or revoking the owner, losing the connection or reaching the current
30-second deadline ends the lease. Renewal requires a live owner and the exact
original profile; expiry and revocation cannot be undone. The profile alone
grants no authority.

`MachinePluginStore::open_mcp` takes a bounded process slot and the existing
Plugin lifecycle fence, opens the executable without following a final symlink,
then reverifies the signed generation and inode metadata. It launches through a
retained parent `/proc/<pid>/fd` descriptor; replacing the path cannot retarget
that launch. A read-only descriptor supplies the process profile. The child gets
an empty inherited environment, fixed signed arguments and its generation cwd.
This is not an OS sandbox against a malicious process running as the same user.

One actor owns stdio and the existing `PluginProcessGroup`. Requests are bounded
to one queued request and 1 MiB frames; there are at most eight processes.
Each request has a separate 30-second deadline from queue admission, unaffected
by process renewal. Errors close the queue before replying. The
v1 protocol accepts initialize, initialized, ping and tool list/call only, filters
tool discovery and rejects calls outside the admitted subset. Server callbacks,
resources and unsolicited notifications are unsupported. This is a deliberately
restricted contract, not compatibility with every MCP server.

Cancellation, malformed/oversized output and lease termination stop the owned
process group and reject late results. Machine signed-artifact tests cover
installation, profile/tool projection, denied connections, revoke/disconnect/
request cancellation, idle owner/client loss, expiry and invalid output. Tests
verify that the parent and spawned `sleep` descendant are no longer running;
they do not claim that all descendant zombies have been reaped. Renewal tests
also cover exact profile/owner checks, continued idle I/O, terminal authority loss
and request timeout despite renewal. No agent-facing relay is enabled.

`McpSessions` routes open/request/renew/close within one `PluginExecutionScope`.
Random handles are local to that router and cannot be reused on a replacement
connection, even with the same Machine identity. The table holds at most eight
open or opening processes; cancelled launches remove their reservation, and the
next open removes closed entries. Closing a handle removes its lease owner,
cancelling an in-flight read even when that read retains its process reference.
Repeated close is harmless. Dropping the router drops all process owners.
A real-process test covers foreign-router refusal, close during a child-backed
read, router drop and twelve cancelled launches behind the lifecycle fence.
The router holds a weak reference to its connection owner; detached operations
cannot extend connection authority. Machine transport 42 dispatches its requests
from the existing enrolled Controller channel; older peers reject the command.
Non-Linux Machines return `mcp_platform_unsupported`.

`MachineControl::mcp_request_on_connection` uses existing bounded request
correlation with a distinct reply kind and exact connection token. It rechecks
that token after receiving the response, bounds the serialized reply, and never
retries. Machine dispatch bounds concurrent jobs to eight; close bypasses busy
read capacity. Transport errors use closed codes rather than process details.
A correlation test covers protocol 41 refusal, stale-connection responses and
replacement after response completion but before delivery. This does not grant
session authority; the parent-authorized relay adds the checks described below.
An abandoned open can retain an unknown handle until its 30-second lease expires;
there is no replay or recovery of that handle.

## Parent-authorized relay

The gateway's closed `Mcp` action accepts binding, epoch and open/request/close;
it never accepts a caller-generated process profile. The Controller derives the
profile from current session policy and reuses `managed_calls::authorize` for
the exact installed parent grant and execution connection. Each relay handle
pins that grant, policy admission and both runtime/execution connection tokens.
All are rechecked before operations and immediately before result delivery.

At most 32 open/opening relay sessions are retained. Policy commits wake the
watcher; connection and parent changes are checked every 100 ms, with a separate
30-second admission deadline. A separate task renews every ten seconds after
checking the parent grant, policy and both connections. Renewal waits are bounded
by the old admission deadline; the watcher remains active throughout. Only a
successful ACK followed by revalidation of both old and candidate admissions
commits a new deadline. Delayed ACKs cannot restore expiry or a replaced epoch.
Renew and close bypass Machine read capacity. Invalid authority closes the original remote
handle and wakes waiting requests. Dropping a Controller request future also
closes its handle. Close is sent at most once and never rerouted onto a replacement
connection. Native client cancellation still needs to propagate through the
provider bridge; these tests do not prove that path.

A separate `Grant.mcp` bit gates the action locally and at the Controller. False
is omitted from serialization, preserving old grant readers. Installing a true
bit requires Machine protocol 42. The Controller issues true only for a current prepared owner and exact ready
worker authority. Signed-capable launchers probe the keeper-pinned default CLI
before native startup. Machine version alone is insufficient. The grant registry
pins the policy epoch, rotates grants on eligibility/epoch changes, and repeats
readiness/epoch validation on every action. `runtime_ready` requires an acknowledged MCP grant on the current execution
connection and successful live authorization. Native E2E remains unverified.

Validation: Controller policy/relay tests cover policy revocation, exact grant
selection, either connection being replaced, cancelled request cleanup and close
routing, renewal admission/late-ACK boundaries and the gateway response envelope.
Transport tests verify cancellation removes only its own waiter, timeout discards
late replies, and disconnect releases a waiter before its deadline. The relay
cleanup test drops a polled transport future and checks one close; completed
requests retain their session. Three wire-contract tests cover
bounded input, profile injection refusal and grant protocol compatibility. They exercise relay primitives rather than a complete authenticated
remote provider session. The earlier Fable review did not cover this relay.

## Agent observation

`cowboy.configuration.read("mcp")` exposes the current session's binding policy
and `launch_prepared`, `worker_ready`, `runtime_ready` observations through the
same projection as HTTP GET. Optional `session_id` follows the existing same-owner
visibility checks; `path`, `limit` and `describe` use the existing JSON selection
contract. For example, `cowboy.configuration.read("mcp", {path:"/runtime_ready"})`
returns only that field. This read neither prepares a launch nor grants access;
policy updates retain their authenticated owner/CAS entry point.

## Remaining integration

- Negotiate provider/runtime gateway capability and generate Codex/Claude native
  descriptors. Keep managed `read_only_v1` without MCP; reject native imports
  without an admitted binding.
- Verify bridge cancellation across the complete authenticated parent-to-relay-to-
  Machine path, including native provider clients and delivery races.
- Complete descriptor delivery and verify automatic renewal over the authenticated
  transport with long-lived native MCP clients before enabling them.
- Route Suger managed CLI and MCP through the same binding, generation and policy.
- Run cross-provider lifecycle and whole-task token/latency acceptance before
  claiming availability or enabling a live binding.

## Scoped review

Fable Max `call-840683326c6722854c33d89b` completed with no High/Critical findings;
its full 6,934-character result was read. The confirmed low-severity issues were
fixed: stale pre-lock session metadata, raw-null CAS, revoke-only recovery of an
unreadable policy, mutation-specific responses and bounded accepted writes.
This was not a clean review of a complete relay. Subsequent local work adds both connection checks, revoke notifications and
launch-boundary executable verification; it has not received that review.
A verified path returned by preparation alone cannot satisfy that boundary.

### Execution CLI identity and remaining provenance boundary

As of 2026-11-07, optional `agent_cli` in `execution::Configuration` and
`execution_protocol::LaunchContract` reuses the executor command/SHA-256/version
shape. New ordinary keepers prepend its directory to the saved target PATH;
managed keepers omit it. Startup checks the saved PATH and file digest before
launching the executor, then reports `cowboyAgentCli` in target initialization.
Legacy contracts omit the field; reusing a keeper does not retrofit the current
Machine's configured CLI. Recovery retains the recorded identity.

Full-release configuration now records the immutable Nix wrapper path and its
build-time SHA-256. Preparation retains that wrapper's entire store closure as
`agent-cli-root`, alongside the existing executor root. Recovery derives this
root from the saved contract, never from the current configured CLI. Missing or
failed retention rejects preparation. Resident-only configuration omits the pin.

This is identity plumbing, not MCP readiness. Provider negotiation tied to the
actual incarnation remains required before producing plans or enabling
`Grant.mcp`. A startup hash of a mutable file alone is insufficient; a binding-only
CLI probe cannot prove an older default Cowboy reader understands a new grant
field.

### Mandatory worker launch metadata

`StartSession.mcp_plan` uses runtime protocol 6; retained protocol-5 peers cannot
receive a planned launch. The signed Provider must opt into
`provider.mcp-bindings.v1` and bound execution. Machine admission rejects system,
managed and unbound sessions, invalid plans, and descriptor-write failures.
Memory-only launches keep their existing optional-extension behavior.

The worker checks the signed capability and exact descriptor contents before
starting the adapter, then keeps a private immutable descriptor copy for its
lifetime. Snapshots and reconnect/adoption/recovery checks preserve the plan and
Provider identity. `RemoteRuntime::mcp_launch_authority` requires an exact plan on
a connected, non-resetting Running/Busy worker and reuses existing worker authority.
Controller-owned receipts and the exact worker observation now gate grant
eligibility. No current Provider manifest advertises the new capability, so this
code does not enable existing deployed providers.

`controller_hooks::launch_mcp` resolves a current Controller-owned plan for
nonempty MCP policies and refuses startup when preparation is unavailable. Both fresh-session
`ensure_worker` and persisted-session `session_declaration` consult it. Missing,
tombstoned and valid empty policies retain ordinary startup; corrupt rows refuse
startup. A runtime without Controller hooks rejects any non-null policy.

`remote_runtime::native_admission::send` repeats this check before sending queued
launch/recovery declarations and after asynchronous native-account admission.
Stop/cancel remain available. This closes silent fallback and queue-delay paths;
plan ownership is described below and does not enable MCP grants.

### Keeper-observed session preparation

`MachineControl::prepare_session_native_mcp` obtains the default CLI from the
existing keeper `Describe` response over an exact enrolled connection. It checks
the returned binding/revision/incarnation scope, validates `cowboyAgentCli`, and
only then prepares descriptors. Profile service, runtime, execution Machine and
workspace must match before the read; managed bindings are refused. This path
does not accept a caller-supplied CLI identity or replay onto a replacement
connection.

`NativeMcpSessionPreparation` holds the confirmed receipt owner, observed CLI,
full execution binding and original connection. `is_current` compares execution
identity and connection only; live policy and worker readiness remain separate.
This is the preparation input for Controller lifecycle wiring, not an enabled
session cache or native provider E2E.
The keeper removes any executor-supplied `cowboyAgentCli` before attaching its
saved pin; legacy unpinned contracts cannot inherit an executor's identity claim.


### Session owner and delivery

The Controller holds at most 64 session preparations in `SessionExtensions.launches`.
PUT policy attempts preparation after durable commit; `launch_prepared` reports
availability separately from `worker_ready` and `runtime_ready`. `worker_ready`
requires a current owner and an exact connected Running/Busy worker plan, with
owner revalidation after observing the worker. It is an observation, not a grant;
`runtime_ready` additionally requires a currently authorized, acknowledged MCP grant.
Installation ACKs recheck worker authority, policy epoch and connection; stale
ACKs cannot mark a grant installed. Authenticated session mutators may
retry `POST /api/sessions/{id}/mcp-bindings/prepare` with `{ "epoch": "..." }`.
Retry does not change the policy epoch or start a worker. These are product-user
control surfaces; agent-side binding mutation remains unavailable while grant
negotiation is unfinished, so an agent cannot authorize its own MCP extension.

Preparation reuses the 30-second policy admissions and aborts on revocation,
connection loss, placement/provider/account changes or shutdown. A session owner
keeps confirmed receipts after preparation returns. Both launch entry points and
queued delivery resolve it through the Hub's Controller callback; MCP launches
also carry the required session-extension metadata on the fresh-session path.

Delivery checks policy epoch, full placement, exact connections and pinned
Provider identity. Once an exact ready worker is observed, replacement of that
worker invalidates the owner. The weakly owned cleanup loop removes invalid
entries within one second; delivery refuses them immediately. Owners are neither
serialized into Hub settings nor persisted across Controller restarts. After a
restart, a weakly owned recovery loop rebuilds missing owners from current policy.
It attempts one session per five-second tick, rotates past failures, and shares
the mutation capacity limit. Source preparation is serialized; concurrent requests
receive a retryable refusal, and valid owners are reused. Every attempt repeats
keeper identity and live policy checks. Recovery prepares metadata only: it does
not start/reset a worker or grant authority. Native-provider recovery/E2E remains unfinished. Grant eligibility alone does
not prove a grant was installed; `runtime_ready` projects the latter separately.

## Archive resource verification

The Linux `signed_archive_server_reads_its_generation_resources` fixture installs
a signed `tar_gz` artifact with a nested executable and adjacent JSON resource.
It exercises download, signed generation installation, live-lease process launch,
relative resource access and refusal after lease-owner drop. The existing private
artifact installer supports this layout without a new MCP packaging protocol.
This shell fixture does not package Suger or its Bun/isolation dependencies;
external Nix store paths in a development generation do not prove portability.

## External CodeAct archive acceptance

The opt-in `external_signed_codeact_archive` test accepts
`COWBOY_MCP_CONFORMANCE_CASE`, a local JSON file with `package`, `release` and
`archive` paths plus `tool`, `arguments` and `expected` (the CodeAct value).
It checks the archive digest against the release, replaces only its download URL
with loopback, signs with a disposable fixture identity and installs into a
temporary MachinePluginStore. It verifies signed tool projection, a real CodeAct
call and refusal after dropping the lease owner. Optional `cancel_arguments`
starts a long-running call, observes the artifact process tree, drops the lease
owner and requires both request rejection and observed PID termination within
two seconds.

Two `signed_codeact_transport_*` tests reuse the same installed artifact through
`MachineControl` and the real `McpLink` dispatcher. Commands/events round-trip
through JSON over in-process channels. They check normal calls and sampled
process termination after transport close or connection-scope loss; both require
`cancel_arguments`. This does not emulate a network disconnect or Controller
policy/parent-grant admission.

Run all three explicitly with `cargo test -p cowboy-machine --lib --all-features
signed_codeact -- --ignored --test-threads=1 --skip signed_codeact_native_bridge`.
Missing inputs fail; ordinary
suites leave these external-artifact tests ignored. This is not registered-Machine
installation, authenticated Controller relay, native Provider or production
signing acceptance.


## Native bridge archive acceptance

The opt-in `signed_codeact_native_bridge_*` tests also require an absolute
`COWBOY_MCP_CONFORMANCE_CLI` pointing to a freshly built `cowboy` executable.
They launch `cowboy mcp --binding` with a confirmed Machine descriptor and an
isolated Unix socket gateway. The gateway checks fixture capability, binding and
epoch before forwarding to real Machine transport; it does not implement the
Controller's policy or parent-grant admission.

Tests cover offline discovery, a real archive call, one lazy open/close, and
cancellation or EOF with bridge exit and sampled server/sandbox process cleanup.
Cancellation keeps the stdin writer open: native pipe input uses Tokio's async
pipe receiver so an uncancellable blocking stdin read cannot stall CLI shutdown.
Non-pipe stdin retains its existing reader. These are native bridge tests, not
native Codex/Claude Provider sessions or authenticated Controller relay acceptance.
Run with `cargo test -p cowboy-machine --lib --all-features
signed_codeact_native_bridge -- --ignored --test-threads=1` after
`cargo build -p cowboy --bin cowboy`.
