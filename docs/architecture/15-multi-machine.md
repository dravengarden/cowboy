# Multi-machine runtime

> **Provider packaging transition:** the `acp-runtime`, `provider-adapter`, and
> `provider-cli` slots below describe the current internal rollout machinery.
> The target product groups their exact pins into one independently released,
> Machine-scoped Provider package. Ordinary UI installs, upgrades, and uninstalls
> that Provider unit and does not expose ACP or adapter slots. See
> [Installable Provider packages](../provider-packages.md).

Cowboy evolves from one Hawk-local runtime into a control plane that can route
sessions and code-intelligence work to independently operated macOS and Linux
machines. The durable boundary is a small `cowboy-machine` host agent. Agent
CLIs, ACP adapters/workers, and Zed are payloads supervised by that host; they
are not linked into the Cowboy HTTP daemon and do not share an update
generation.

This design extends the existing `runtime_wire` fencing, command idempotency,
worker snapshots, event replay, and safe drain rules. It does not introduce a
second session protocol or proxy ACP itself over the public network.

## Topology

```mermaid
flowchart LR
    UI["Cowboy clients"] --> CP["Cowboy control plane"]
    CP -->|"UDS · local"| HM["cowboy-machine · Hawk"]
    MM["cowboy-machine · Mac"] -->|"outbound WSS + machine auth"| CP
    FM["cowboy-machine · Falcon"] -->|"outbound WSS + machine auth"| CP
    HM --> HA["ACP generation"]
    HM --> HZ["Zed generation(s)"]
    MM --> MA["ACP generation"]
    MM --> MZ["Zed generation(s)"]
    FM --> FA["ACP generation"]
    FM --> FZ["Zed generation(s)"]
    HA --> HC["Codex / Claude / Gemini"]
    HZ --> HZS["isolated Zed adapter + server"]
```

Remote agents initiate the connection. A machine opens no public listener and
does not need a per-machine public domain. The configured endpoint is the
Cowboy control-plane domain, for example
`wss://cowboy.example/api/machine/connect`. This works through NAT and avoids
turning every development host into an Internet-facing service.

SOCKS is not a runtime transport. It can carry arbitrary TCP, but it provides
none of the machine identity, lease fencing, command deduplication, event
replay, capability negotiation, or component lifecycle semantics Cowboy needs.
The local fast path remains a Unix-domain socket. Remote machines use the same
length-bounded application frames over WebSocket/TLS.

## Stable host, replaceable payloads

`cowboy-machine` owns only mechanisms expected to remain stable:

- machine identity, enrollment, credential rotation, and revocation;
- one reconnecting control-plane connection and heartbeat;
- content-addressed downloads with digest/signature verification;
- atomic generation activation, health probes, drain, and rollback;
- process supervision, per-runtime state roots, logs, and resource reporting;
- the existing detached ACP worker pool and its replay/fencing contract;
- provider and Zed authentication orchestration without reading credentials
  into Cowboy's database or event stream.

It does **not** contain provider-specific UI rules or Zed protocol logic. The
transitional implementation stores those executable concerns in independently
versioned internal payloads:

| Payload domain | Contents | Roll trigger |
|---|---|---|
| `acp-runtime` | ACP SDK, worker, provider launch policy | ACP SDK/worker change |
| `provider-adapter:<id>` | `codex-acp`, `claude-agent-acp`, or a native CLI ACP entry | adapter release |
| `provider-cli:<id>` | Codex, Claude Code, Gemini CLI, or Grok Build | provider release |
| `zed-adapter:<abi>` | Cowboy's GPL-isolated stable product adapter | adapter contract change |
| `zed-server:<zed-version>` | official server matching one Zed revision | matching Zed client/update |

Each domain has its own desired generation, readiness gate, drain set, rollback
target, and update policy. Updating Zed must not drain ACP turns. Updating
Codex must not restart Claude/Gemini workers or Zed language servers. Updating
the Machine host must leave all content-addressed payload executables running;
the replacement host adopts them from snapshots after reconnect.

Under the Provider package contract, these internal domains remain useful for
content deduplication, process supervision, and diagnostics, but a signed
Provider lockset selects all of them. A Provider install transaction is not
`active` until the complete lockset has passed its interface checks and probes;
users never reconcile one private layer as a standalone product operation.

## Zed is a version set, not one global binary

Zed remote development requires the remote server version to exactly match the
connecting Zed client. Cowboy's Code Provider also pins a Zed server and a
matching `cowboy-zed-adapter` revision. Therefore a machine may retain several
Zed server generations concurrently:

```text
payloads/zed-server/<platform>/<zed-version>/<sha256>/bin/zed-remote-server
payloads/zed-adapter/<adapter-abi>/<sha256>/bin/cowboy-zed-adapter
state/zed/<adapter-abi>/<worktree-id>/...
```

The Machine Agent never mutates Zed's user-owned `~/.zed_server` directory.
Zed's native SSH client may continue to own that location. Cowboy-managed Zed
state stays under the Machine Agent root and is selected by an explicit
generation lease. This preserves the existing ownership boundary and keeps the
GPL adapter/process isolated from the MIT Cowboy daemon.

## Provider distribution

The Machine Agent uses a private tool root and never depends on an interactive
shell's `PATH`, Homebrew state, or `npx latest` in the session-start path:

```text
payloads/<component>/<version>/<digest>/...
active/<component> -> ../../payloads/<component>/<version>/<digest>
state/<provider>/...       # provider-owned auth/config, mode 0700
```

- **Codex:** prefer the official standalone macOS/Linux release artifact. It is
  a single native executable and avoids Node/Homebrew coupling.
- **Claude Code:** until Anthropic's native installer leaves its documented
  alpha status, install the official npm package into a Machine-owned prefix.
  Its own auto-updater is disabled; Cowboy stages and activates reviewed
  versions explicitly.
- **Gemini:** install the official stable npm package into a Machine-owned
  prefix. OAuth remains inside the official CLI. API-key/Vertex modes remain
  supported provider choices.
- **ACP adapters:** install as explicit versioned payloads. Session startup
  executes the active immutable path directly; it never installs packages.

The managed Node runtime, when needed, is itself a versioned payload. macOS and
Linux therefore use the same lifecycle. Brew/Nix may bootstrap
`cowboy-machine`, but they are not part of provider runtime correctness.

## Authentication

Credentials stay on the execution machine in provider-owned state. Cowboy only
stores and displays a redacted status such as `signed_out`, `pending`,
`signed_in`, `expired`, or `error`, plus non-secret account labels returned by
the provider.

### Provider usage ledger

Provider-owned account facts and Cowboy-observed activity are separate data
planes. DeepSeek gateways expose only official balance fields through
`/provider-info`. After a completed inference response, a gateway submits a
content-free usage event to the local Machine Unix socket. Machine commits it
to a SQLite WAL before acknowledging the gateway, replays ordered batches over
Machine protocol v2, and deletes them only after the controller commits and
acknowledges the sequence watermark. PostgreSQL uniqueness on Machine,
producer, and sequence makes reconnect replay idempotent.

The UI labels these aggregates `Cowboy measured`; they cover only requests
observed by reporting Cowboy Machines and never imply provider-wide billing
usage. VictoriaLogs remains operational observability and is not a product data
source. Protocol v1 Machines remain compatible but do not contribute usage. The
UI offers bounded 1-hour through 30-day windows; the internal ledger is pruned
after 30 days by the existing Cowboy database sweeper.

Usage event schema v3 adds only content-free request shape and delivery facts:
runtime lane, requested and resolved model family/revision, client and upstream
protocol, translation mode, ordinary/compact operation, request role, thinking
configuration, byte and item counts, tool and system-block counts,
previous-response presence, compatibility repair count, gateway build/boot,
streaming, duration, status, completion, and provider token counters. It never
carries prompt text, tool arguments, reasoning content, response bodies, raw
session/response ids, or credentials. Lineage and stable-prefix correlation use
runtime-namespaced, credential-keyed truncated HMACs generated independently
inside each gateway; raw values cannot be recovered or joined across runtimes
even when two lanes intentionally use the same provider credential. Gateway
delivery remains fail-open, so telemetry cannot change an inference result.

Cache rates use only rows marked `explicit` (the provider returned hit and
miss counters) or `derived` (the provider returned cached tokens plus total
input, making miss tokens an exact subtraction). Rows marked `absent` contribute
to coverage but not the rate. Version-one rows are `legacy` and excluded rather
than silently treated as misses. UI queries choose a bounded 1-hour through
30-day occurrence window and can filter Flash, Pro, Codex, or Claude Code
without refreshing account balance. The card keeps
runtime rates separate, shows hot and low-hit request counts, and breaks model,
role, protocol, translation, revision, operation, and Machine coverage apart so
a token-weighted average cannot hide a bimodal workload.

Low-hit diagnosis is deliberately observational: it classifies only requests
with at least 8,000 input tokens and a measured hit rate below 10%. Ordered
lineage is scoped to one Machine and producer. An explicit Cowboy session HMAC
or Codex response lineage can distinguish first observations, compaction, model
or provider-revision changes, explicit request-role changes,
protocol/translation/reasoning changes, compatibility rewrites, static-prefix
changes, history rewrites, probable provider eviction, gateway build/restart,
and unexplained exact-prefix misses. Claude's stable first-message HMAC is only
a shared prefix root, not a unique session identity; it is reported as
`prefix_lineage_ambiguous` and excluded from ordered causal claims. These labels
are hypotheses for experiments, not provider billing facts. Lineage attribution
and schema-v3 coverage remain visible so missing or ambiguous lineage cannot
masquerade as an optimization result.

DeepSeek cost is valued only in the backend provider adapter from a pinned
official CNY list-price snapshot, with its source URL, version, and as-of date
in the response. The resolved billing model wins over the requested alias, and
Flash and Pro token aggregates are priced separately. Reasoning
tokens are a diagnostic subset of completion tokens and are never charged a
second time. Unknown models and cache-absent input remain explicitly unpriced,
and the UI displays price coverage, cache savings, and miss premium. Incomplete
valuations use an explicit lower-bound marker rather than appearing as an
invoice.

Codex and Claude gateway traffic records `request_role=unknown` unless the
loopback caller supplies an explicit role header; telemetry must never infer
`executor` from absence. The UI shows attributed-role coverage so role-based
cost conclusions cannot be drawn from unknown traffic.

Schema v4 extends this envelope with an explicit interactive versus
cache-keepalive request purpose, protection outcome and algorithm, attempt and
interval, source age, and an opaque source-request fingerprint. Existing
request/error/cache/spend aggregates filter to interactive rows; protection
attempts, hits, misses, retries, preemptions, tokens, duration, age, interval,
and price have a separate read model. The Machine also exposes a bounded
`deepseek-cache-status` adapter operation that queries only its local gateway;
the controller never receives a retained request body.

Schema-v4 rollout and rollback follow the same controller-first and drain-first
ordering as v3. Roll out the controller and Machine before either v4-producing
gateway. Before reverting a Machine to a release that cannot decode v4, stop or
replace both v4 producers and require `pendingV4Events: 0` plus
`v4Drained: true` from the read-only spool status. A successful gateway restart
is not drain evidence.

Schema v3 and v4 rollout is ordered because old Machine collectors reject a
newer envelope before it can enter the durable spool:

1. Build and deploy the migration-compatible controller bridge by itself. Its
   successful controller receipt must be the active rollback boundary before
   migration 0025 is applied. The bridge tolerates already-applied additive
   migrations newer than its embedded set while preserving checksum
   verification for every migration it knows.
2. Build and deploy the final controller migration/API. Verify migration 0025,
   the provider summary, and the filtered activity endpoint before changing a
   Machine.
3. Build the final Machine release and use the explicit maintenance activator
   on one Machine at a time. Verify reconnect, ACP inventory, and
   `--provider-usage-status` before proceeding to its gateways.
4. Activate that Machine's Codex and Claude DeepSeek gateway components through
   the model-gateway release transaction. Confirm new schema-v4 events receive
   controller ACKs, then repeat steps 3–4 on the peer Machine.
5. Web may follow the final controller independently; it does not authorize or
   imply Machine maintenance.

A gateway-first release creates an unrecoverable telemetry gap even though
inference remains fail-open.

Successful component activations cannot manually select an ancestor revision:
the Cowboy and model-gateway activators both require every later candidate to
descend from the active lane revision. Therefore rollback is a new, clean,
committed release, never a direct move to an old profile. Before maintenance,
prepare two isolated worktrees based on the current deployed revisions:

- in Columbus, create one descendant revert commit that removes the gateway-v3
  change, then build both gateway release outputs;
- in Cowboy, create one descendant revert commit that restores the
  bridge-equivalent application state while retaining the bridge's
  `ignore_missing` migration behavior, then build Web, Machine, and controller
  release outputs.

For each candidate, prove the active revision is an ancestor with
`git merge-base --is-ancestor <active-revision> HEAD`; the activator repeats
this check against fresh remote state. Do not build an ancestor bridge checkout
and expect the release entrypoint to accept it.

Rollback is intentionally asymmetric and uses those descendant releases:

1. Activate the reverted Codex and Claude gateway releases on every Machine so
   no process can append another v3 event.
2. On every Machine, run the active `cowboy-machine
   --provider-usage-status` command documented in `docs/machine-operations.md`.
   Record all producers and wait until `pendingV3Events` is zero and
   `v3Drained` is true. The schema rows expose pending sequence ranges and the
   latest controller ACK; absence of a queryable record is not drain evidence.
3. Revert Web if required, then activate the descendant Machine release under
   the explicit maintenance boundary, one Machine at a time.
4. After all Machines are compatible with v2, activate the descendant
   bridge-equivalent controller release last.

Never run an old Machine while a v3 spool is pending. Migration 0025 remains
applied: the controller may return only to a descendant release that retains
the migration-compatible bridge behavior, never to the original ancestor
binary. Controller recovery after that boundary is a forward fix or a
descendant revert, not a schema downgrade. Automatic predecessor restoration
inside one failed activation remains valid because it occurs before that
candidate is recorded as the successful active revision.

DeepSeek credentials and mutable runtime state remain independent per agent
lane. The provider adapter may collapse duplicate official balances only when
both gateways report the same irreversible account fingerprint; distinct
accounts stay separate. Cowboy-measured usage spans credential rotation because
it describes calls made through Cowboy, not one current account balance.

Cache tuning starts only after each lane has at least seven days and 100
verified requests. Compare hit tokens, verified coverage, compact frequency,
hot/cold request distribution, error rate, completion rate, and gateway latency
before changing prompt or compaction behavior. Reject an optimization if agent
quality regresses, cache coverage falls, errors rise, or the apparent gain comes
only from legacy or missing observations.

Login is capability-driven:

- Codex App Server exposes account status plus a device-code flow. The Machine
  Agent returns the verification URL and user code to Cowboy; Mobile opens the
  URL and shows a copyable code, then listens for completion.
- Claude owns its browser login flow. The Machine captures both output streams,
  extracts only the verification URL/code, and leaves credentials in the
  official CLI's state. Cowboy must not scrape or copy token files.
- Gemini currently has no stable login/status subcommand. Its OAuth or API-key
  setup remains in the official CLI on that Machine. Cowboy infers only a
  redacted readiness state from the selected auth method, credential-file
  presence, and relevant environment presence; it never reads credential
  contents. Machines gives the exact local remediation and refreshes status.

Every login request has an id, expiry, cancellation, and one active owner.
Login frames are never replayed into a different UI client after expiry.

## Enrollment and transport security

HTTPS alone authenticates the Cowboy server, not the machine. Remote mode uses:

1. a short-lived, single-use enrollment token created in Cowboy;
2. a new random Ed25519 key generated through the operating system's OpenSSH
   implementation and stored in a mode-0700 Machine state root with a mode-0600
   private key;
3. strict Web PKI validation for the configured `wss://` endpoint, with no
   insecure or plaintext remote override;
4. a server nonce signed by the enrolled machine key on every connection;
5. controller-issued machine identity, key rotation, immediate revocation, and
   an auditable last-seen/source record.

Enrollment is an HTTPS POST; the secret never enters the reconnecting WebSocket
protocol. Cowboy stores only its SHA-256 digest and consumes it atomically. A
successful connection receives a fresh epoch, so a superseded socket cannot
mark its replacement offline when it finally closes.

The signed version-one proof is a canonical, domain-separated transcript. It
binds the challenge id, nonce, expiry, machine id, platform, architecture,
connection mode, Machine protocol range, runtime protocol range, and host
build. It deliberately excludes the mutable display name and inventory. A
captured signature therefore cannot be replayed as another Machine or reused
to negotiate downgraded protocol capabilities.

Cowboy exposes the conventional OpenSSH `SHA256:...` public-key fingerprint for
human verification. An active public key is never replaced by ordinary
enrollment: rotation is the explicit sequence revoke, create a new one-time
token, enroll the replacement key, and verify the new fingerprint. Revocation
clears the current connection epoch and the controller fences the live socket
within two seconds; unused enrollment tokens for that Machine are discarded in
the same transaction. Re-enrollment with the revoked public key is rejected, so
rotation cannot accidentally reactivate a credential that may be compromised.

Production may additionally require mTLS at the reverse proxy, but application
machine identity remains mandatory so credentials can be rotated and revoked
without coupling the product protocol to one proxy. Local UDS mode relies on
filesystem ownership and peer credentials and does not require enrollment.

The server authorizes canonical workspace roots per machine. A session request
contains a `machine_id` and a machine-local workspace id; arbitrary client-sent
absolute paths are rejected. After the server reserves the session id, the
Machine Agent rechecks the canonical source root, fetches its remote default
branch, and prepares a session-owned Git worktree under its private state root.
The prepared path is separately trusted for worker and adapter requests. A
non-Git root remains shared and is reported as non-isolated; a Git fetch or
worktree failure aborts creation instead of falling back to the source root.

## Protocol layers

There are three deliberately separate compatibility contracts:

1. **Machine control protocol:** identity, inventory, component reconciliation,
   login actions, health, and multiplexed runtime frames. Additive v1 fields
   default safely; min/max negotiation rejects non-overlap.
2. **ACP runtime protocol:** the existing `runtime_wire` contract between the
   controller and detached workers. Machine transport carries these frames
   without reinterpreting ACP updates.
3. **Zed adapter contract:** Cowboy product requests such as open worktree,
   hover, symbols, and navigation. Raw Zed protobuf never crosses the Cowboy
   control-plane boundary.

ACP SDK code is bundled in the `acp-runtime` payload, not in the stable Machine
host. Zed protobuf code is bundled only in the GPL-isolated Zed adapter. This is
what lets ordinary provider/Zed upgrades avoid a Machine Agent release.

## Scheduling and session ownership

Every session persists its immutable `machine_id`. The default for a new
session is the UI's current/local machine when healthy, otherwise the most
recently used healthy compatible machine. A machine is selectable only when it
reports:

- online and not draining;
- the requested provider capability installed and authenticated;
- the selected workspace available and trusted;
- compatible machine/runtime protocol versions;
- sufficient free capacity for a new worker.

Once created, a session never silently migrates to another machine: the agent's
native session store, session worktree, credentials, and running tools are local. A
future explicit migration operation must first prove provider resume support,
workspace identity, and credential compatibility. Offline sessions remain
bound and show a recoverable Machine Offline state.

## Update coordination

Cowboy publishes signed desired manifests; each Machine Agent independently
downloads, verifies, stages, runs the manifest-bound executable probe, and
reports readiness. Activation is
explicitly either `manual` (the Machines UI requests reconciliation) or
`automatic` (the signed manifest entry is sent after authentication), and never
means "latest at process start". Busy ACP generations still drain through the
runtime broker rather than inventing a third component-policy state.

Automatic entries without a probe are rejected. Probe failure leaves the
previous active generation and rollback link untouched. For ACP payloads, the existing safe-drain invariants apply. Busy workers finish
on their current immutable generation; new sessions use the activated
generation; failed candidates roll back. Zed worktrees are leased and drain
independently. Old payloads are retained while referenced by a live process or
rollback slot, then evicted by size/age policy.

Cowboy itself may update without changing the Machine desired manifest. Machine
Agents may update at a slower cadence as long as protocol ranges overlap. The
control plane must retain at least one protocol version accepted by every
enrolled online machine before removing an old version.

## Product surfaces

- Session rows and headers show a compact machine tag only when more than one
  machine exists or the bound machine is not local.
- New Session groups `Machine`, `Workspace`, and `Provider`; changing Machine
  refreshes the other two from that machine's capability snapshot. Current
  machine is preselected.
- Machines lists connectivity, platform/architecture, capacity, allowed
  workspaces, Provider install/auth/version/health state, Zed generations,
  pending updates, and last error. ACP and adapter generations move to an
  explicitly scoped developer-diagnostics surface during Provider-package
  migration.
- Login and update actions are explicit sheets/dialogs with progress and
  cancellation. Secret material is never rendered after submission.
- Offline, incompatible, draining, and revoked are visually distinct and carry
  a concrete remediation action.

## Implemented boundary

The controller and Machine now implement immutable session placement, outbound
authenticated WSS enrollment, epoch fencing, runtime replay, session-owned Git
worktrees from trusted remote workspaces, provider inventory/login actions, signed independently activated
components, Code Adapter routing, isolated Zed adapter/server supervision,
capacity/drain-aware scheduling, Machines UI, and user-scoped macOS/Linux
installation. The stable Machine wire remains additive and distinct from ACP
runtime and Zed adapter contracts.

Operational rollout is deliberately separate from implementation. A target is
selectable only after its one-time enrollment, signed desired manifest,
provider login, workspace declaration, and capacity report are healthy. An
unreachable target remains offline; Cowboy never falls back to a different
machine or silently runs its session on Hawk.

## Upstream contracts

- [Codex installation](https://github.com/openai/codex/blob/main/README.md)
  and [App Server account/login API](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)
- [Claude Code setup and update channels](https://docs.anthropic.com/en/docs/claude-code/getting-started)
- [Gemini CLI releases and authentication](https://github.com/google-gemini/gemini-cli)
- [Grok Build source and ACP entrypoint](https://github.com/xai-org/grok-build)
- [Zed remote development and exact server matching](https://zed.dev/docs/remote-development)
- [Zed release artifacts](https://github.com/zed-industries/zed/releases)
