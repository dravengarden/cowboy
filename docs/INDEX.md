---
type: docs_index
description: Cowboy architecture, transport, frontend, deployment, and operations documentation.
---

# Documentation index

## Scope

Start with the product topology, then read the normative Provider contract.
Architecture chapters describe current code; product design documents capture
surface-specific contracts; operations documents are runbooks. Compatibility
integrations are intentionally listed last because they are not part of the
primary phone/desktop product path.

## Reading order

- [`workspace-documents.md`](workspace-documents.md) — Unified Draft/Session navigation, shared directories, content ownership, migration and Undo
- [`editor-plugins.md`](editor-plugins.md) — Installable sandboxed editor plugins (manifest, API, lifecycle, isolation) and Desktop editing parity with Obsidian 1.13.7

- [`architecture/00-overview.md`](architecture/00-overview.md) — Control plane, Machines, workers, storage, and client topology
- [`configuration.md`](configuration.md) — Unified Service/Device configuration files, `cowboy config` CLI, reload classes, rolling-update pre-flight, and how to decide config file vs database vs wiring
- [`execution-environments.md`](execution-environments.md) — Native remote execution, ownership, staged acceptance and OVH egress/context audit
- [`execution-connection-recovery.md`](execution-connection-recovery.md) — Machine heartbeat, native handshake and execution lifetime recovery, including Retry preflight
- [`remote-session-disaster-recovery.md`](remote-session-disaster-recovery.md) — Proposed recovery contract, October 24 incident audit, durable ACK gaps, per-session reconciliation and fault acceptance matrix; not implemented or activated
- [`session-start-latency-2026-10-09.md`](session-start-latency-2026-10-09.md) — Remote Claude start stalls: single-flow Machine transport saturated by bulk writes, credit pacing, one-command startup survey and measurements
- [`agent-api.md`](agent-api.md) — Cowboy MCP: dynamic product control catalog, session/history/usage reads, durable receipts, coverage ledger and protocol 30–32 compatibility
- [`superagent-patterns.md`](superagent-patterns.md) — Proposed global/workspace/project pattern profiles, Markdown contracts, evidence-based refinement, adoption and supervision boundaries
- [`examples/superagent-patterns/global.md`](examples/superagent-patterns/global.md) — Inactive example: owner-global pattern selection
- [`examples/superagent-patterns/observed-columbus.md`](examples/superagent-patterns/observed-columbus.md) — Inactive observed-pattern setup or learning template
- [`examples/superagent-patterns/observed-suger.md`](examples/superagent-patterns/observed-suger.md) — Inactive observed-pattern setup or learning template
- [`examples/superagent-patterns/observed-cowboy.md`](examples/superagent-patterns/observed-cowboy.md) — Inactive observed-pattern setup or learning template
- [`examples/superagent-patterns/learning-candidate-template.md`](examples/superagent-patterns/learning-candidate-template.md) — Inactive observed-pattern setup or learning template
- [`examples/superagent-patterns/patterns/scope-continuation.md`](examples/superagent-patterns/patterns/scope-continuation.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/failure-recovery.md`](examples/superagent-patterns/patterns/failure-recovery.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/review-follow-through.md`](examples/superagent-patterns/patterns/review-follow-through.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/delivery-evidence.md`](examples/superagent-patterns/patterns/delivery-evidence.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/dependency-handoff.md`](examples/superagent-patterns/patterns/dependency-handoff.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/review-ready.md`](examples/superagent-patterns/patterns/review-ready.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/patterns/resource-closeout.md`](examples/superagent-patterns/patterns/resource-closeout.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/observed-global.md`](examples/superagent-patterns/observed-global.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/observed-routing.md`](examples/superagent-patterns/observed-routing.md) — Inactive historical-usage pattern candidate or analysis
- [`superagent-usage-patterns-2026-11.md`](superagent-usage-patterns-2026-11.md) — Inactive historical-usage pattern candidate or analysis
- [`examples/superagent-patterns/suger.md`](examples/superagent-patterns/suger.md) — Inactive example: Suger workspace profile
- [`examples/superagent-patterns/columbus.md`](examples/superagent-patterns/columbus.md) — Inactive example: Columbus workspace profile
- [`examples/superagent-patterns/cowboy.md`](examples/superagent-patterns/cowboy.md) — Inactive example: Cowboy project procedure replacement
- [`examples/superagent-patterns/patterns/follow-through.md`](examples/superagent-patterns/patterns/follow-through.md) — Inactive example: generic authorized-delivery supervision
- [`examples/superagent-patterns/patterns/suger-delivery.md`](examples/superagent-patterns/patterns/suger-delivery.md) — Inactive example: Suger company-project delivery ownership
- [`examples/superagent-patterns/patterns/columbus-delivery.md`](examples/superagent-patterns/patterns/columbus-delivery.md) — Inactive example: Columbus and external-project delivery routing
- [`examples/superagent-patterns/patterns/cowboy-delivery.md`](examples/superagent-patterns/patterns/cowboy-delivery.md) — Inactive example: Cowboy component release evidence
- [`session-context.md`](session-context.md) — Multiple session links, staged fresh-agent selection, account isolation and first-send recovery
- [`session-fork.md`](session-fork.md) — Native ACP session fork from a whole conversation or a turn-final message, forked worktree state, admission and retries
- [`background-tasks.md`](background-tasks.md) — Background tasks: one compact row, inspector and bar for subagents and Calls; durable transcript anchors, terminal-state reconciliation, approval binding, diagnostics (Phase 1 implemented) and the later notification/native-work/capability phases
- [`task-notifications.md`](task-notifications.md) — Task notifications: durable intents, retried per-installation Web Push delivery, presence deferral and per-account acknowledgement (background tasks Phase 2)
- [`call-capabilities.md`](call-capabilities.md) — Call capabilities: per-capability session policy, versioned identity and discovery state over the existing Calls lifecycle (background tasks Phase 4, first slice)
- [`subagents.md`](subagents.md) — ACP subagents: child-session routing, one bounded live row per subagent, attributed permission requests, stopping one subagent (Machine protocol 40), not-reporting rows, per-model usage, ACP bridge forwarding and adapter behavior
- [`tool-results.md`](tool-results.md) — Provider-independent MCP result retention, Cowboy/Matrix tool presentation and Desktop/Mobile inspection
- [`session-extension-plugins.md`](session-extension-plugins.md) — `memory_backend` (Matrix) and `call_profile` (Review) Plugin kinds, Machine-private memory bindings and staged delivery to sessions
- [`mcp-bindings.md`](mcp-bindings.md) — Signed MCP installation, explicit session policy, epoch/CAS revocation and remaining relay integration
- [`managed-agent-calls.md`](managed-agent-calls.md) — Parent-scoped cross-Provider Calls implementation, CLI and desktop/mobile lifecycle UI, with production acceptance boundaries
- [`managed-native-capabilities.md`](managed-native-capabilities.md) — General Calls extension contract, on-demand image generation/editing, isolated native profiles, verified cross-machine artifacts and token costs
- [`releases/native-calls-images-2026-10-18.md`](releases/native-calls-images-2026-10-18.md) — Native image Calls source, signed Plugin closure, exact-artifact validation, worker maintenance and recovery boundaries
- [`requirements.md`](requirements.md) — Normative Provider package, authentication, installation, ownership and OVH Remote deployment invariants
- [`secure-connectivity-design.md`](secure-connectivity-design.md) — Option 1 security/reliability priorities and deferred Option 2 roadmap with unresolved PWA bootstrap questions
- [`plugin-spatiotemporal-design.md`](plugin-spatiotemporal-design.md) — Target architecture: fixed core, typed components and Plugin composition across Service/Machine; first read-only structural slice implemented
- [`plugin-composition-checker.md`](plugin-composition-checker.md) — Implemented core diagnostic, generated Rust/TS contracts, graph rules and remaining authority/recovery work
- [`plugin-installation-incarnations.md`](plugin-installation-incarnations.md) — Durable installation revisions, uninstall CAS, tombstones and reader-first rollout
- [`plugin-recovery-assessment.md`](plugin-recovery-assessment.md) — Read-only recovery assessment across Service evidence, Machine receipts and current installation tombstones
- [`plugin-execution-leases.md`](plugin-execution-leases.md) — Connection-bound, process-monotonic admission for journaled Plugin effects
- [`plugin-service-authorization.md`](plugin-service-authorization.md) — Current confirming-credential and Operator checks at Service effect boundaries
- [`agent-driven-plugin-release.md`](agent-driven-plugin-release.md) — Explicit host delegation, private Operator CLI, exact Plugin upgrades and durable postflight
- [`unattended-release-adoption.md`](unattended-release-adoption.md) — Controller-owned Catalog observation, bounded fallback and shutdown drain; unattended installation intent and host policy remain designs
- [`plugin-no-effect-resolution.md`](plugin-no-effect-resolution.md) — Independently confirmed, atomic resolution of proven pre-effect uninstall interruptions
- [`telemetry-binding-resolution.md`](telemetry-binding-resolution.md) — Independently authorized Service binding resolution, schema-two audit reader and closed production admission
- [`local-observability.md`](local-observability.md) — Host-local OTel storage, retention, bounded remote queries and AI diagnostic commands
- [`telemetry-machine-recovery.md`](telemetry-machine-recovery.md) — Independently authorized reopened Prepared closure, protocol 17, atomic Machine audit and retained Service fence
- [`core-security-client-boundary.md`](core-security-client-boundary.md) — Core-owned local authentication UI and typed native Passkey port; storage/SDK migration remains separate
- [`core-security-storage-bridge.md`](core-security-storage-bridge.md) — Typed core Passkey storage, atomic legacy import and independent ceremony lifetimes; ownership cutover remains separate

## Documents

### Architecture

- [`remote-tools-coverage-audit-2026-10-05.md`](remote-tools-coverage-audit-2026-10-05.md) — Claude/Codex remote capability gaps, native extension candidates and fault-injection acceptance matrix
- [`releases/core-dependencies-2026-10-11.md`](releases/core-dependencies-2026-10-11.md) — Compatible Cargo and Web dependency updates, the toolchains and majors retained with their reasons, and remote acceptance
- [`releases/session-extension-plugins-2026-10-16.md`](releases/session-extension-plugins-2026-10-16.md) — matrix and review as session extension Plugins, component release 3.53.0, OVH rollout and the pending Hawk worker pin
- [`releases/agent-runtimes-2026-10-11.md`](releases/agent-runtimes-2026-10-11.md) — Claude 2.1.296, Codex 0.160.1 with ACP 2.2.2, Gemini 0.63.0 and Grok 1.0.50; component release 3.49.0, native difference table and remote acceptance
- [`releases/agent-runtimes-2026-10-28.md`](releases/agent-runtimes-2026-10-28.md) — Claude Code 2.1.312, Codex 0.164.0 with ACP 2.3.0 and Grok 1.0.70 upgrades
- [`releases/agent-runtimes-2026-10-30.md`](releases/agent-runtimes-2026-10-30.md) — Claude Code 2.1.314 with ACP 0.99.1, Codex 0.165.0 with ACP 2.5.0 and dsh rc.5; component release 3.71.0
- [`releases/credential-probe-backoff-2026-10-31.md`](releases/credential-probe-backoff-2026-10-31.md) — failed credential probe backoff, usage after re-login, Ultra badge; Claude Code 3.20.22 and component release 3.73.0
- [`releases/gemini-retirement-2026-10-30.md`](releases/gemini-retirement-2026-10-30.md) — Gemini retired from source, component release 3.72.0 and its fleet rollout
- [`releases/acp-subagents-2026-10-31.md`](releases/acp-subagents-2026-10-31.md) — ACP subagents, form previews and countdowns, URL elicitation completion; fleet rollout
- [`releases/steer-native-cycle-2026-10-29.md`](releases/steer-native-cycle-2026-10-29.md) — Steer joins a cycle Claude starts on its own after a background task
- [`releases/acp-typed-updates-elicitation-2026-10-29.md`](releases/acp-typed-updates-elicitation-2026-10-29.md) — typed compaction, notices and elicitation; ACP SDK 3.6
- [`releases/agent-runtimes-send-2026-10-18.md`](releases/agent-runtimes-send-2026-10-18.md) — runtime upgrade candidates, compatibility constraints and explicit send delivery design
- [`releases/bun-toolchain-2026-10-10.md`](releases/bun-toolchain-2026-10-10.md) — Bun toolchain, component release 3.45.0, six Agent Plugins republished with byte-identical runtimes; Zed 1.20.6 built but not published
- [`releases/remote-parity-2026-10-05.md`](releases/remote-parity-2026-10-05.md) — OVH Claude 3.4.8 output/cancellation fixes, exact acceptance and retained parity gaps

- [`cowboy-ovh-efficiency-2026-10-04.md`](cowboy-ovh-efficiency-2026-10-04.md) — Current cross-host SSH timings, real-session history, recurring usage preparation and scoped optimization order
- [`ovh-native-execution-efficiency-2026-10-04.md`](ovh-native-execution-efficiency-2026-10-04.md) — Live Codex Remote/Claude Mods bindings, reproducible RPC amplification and Controller-to-OVH optimization priorities
- [`claude-target-range-reads.md`](claude-target-range-reads.md) — Target-authoritative range reads, whole-file conflict stamps and safe bounded state cleanup
- [`remote-file-buffers.md`](remote-file-buffers.md) — Hash-checked splice writes from verified Device copies, with configurable exclusions and cleanup on both sides
- [`architecture/00-overview.md`](architecture/00-overview.md) — Current system topology and component map
- [`architecture/01-acp-transport.md`](architecture/01-acp-transport.md) — ACP session lifecycle, streaming, permissions, and cancellation
- [`claude-autonomous-activity.md`](claude-autonomous-activity.md) — Native execution state and loading feedback after a background task resumes Claude
- [`claude-refusals.md`](claude-refusals.md) — Classifier refusal evidence, local failure handling, and unverified upstream trigger
- [`architecture/02-core-hub.md`](architecture/02-core-hub.md) — Authoritative session state, ordering, queues, and fan-out
- [`architecture/03-supervisor.md`](architecture/03-supervisor.md) — Machine routing, detached-worker lifetime, and restart recovery
- [`architecture/04-providers.md`](architecture/04-providers.md) — Provider packages, launch generations, behavior, and compatibility fallback
- [`architecture/05-storage.md`](architecture/05-storage.md) — PostgreSQL/SQLite store, write-behind, pagination, and retention
- [`persistence-admission.md`](persistence-admission.md) — Bounded FIFO admission, oversized/control reservations, shutdown drain and explicit loss limits
- [`releases/persistence-admission-2026-09-16.md`](releases/persistence-admission-2026-09-16.md) — Controller admission repair, healthy new persistence epoch, retained worker/native processes and historical loss limits
- [`architecture/06-server-api.md`](architecture/06-server-api.md) — REST, WebSocket, runtime SPA files, and session reload
- [`architecture/08-memory.md`](architecture/08-memory.md) — Matrix long-term memory and native Provider-state boundaries
- [`architecture/09-frontend.md`](architecture/09-frontend.md) — React state, durable draft delivery, transcript, composer, PWA, and native-shell contracts
- [`usage-refresh-http-errors.md`](usage-refresh-http-errors.md) — Session account-usage identity and actionable HTTP failures
- [`architecture/10-deploy-build.md`](architecture/10-deploy-build.md) — Pinned builds and component-scoped releases
- [`architecture/11-operations.md`](architecture/11-operations.md) — Capacity, store backup and cutover, observability, and incident policy
- [`architecture/12-rolling-updates.md`](architecture/12-rolling-updates.md) — Fencing, replay, draining, readiness, and rollback
- [`architecture/13-code-review.md`](architecture/13-code-review.md) — Session/project context defaults, inline repository navigation, and the stable Git, file, diff and language-intelligence API
- [`architecture/14-admin.md`](architecture/14-admin.md) — Admin console and product registration
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Isolated Zed code-provider integration
- [`architecture/15-multi-machine.md`](architecture/15-multi-machine.md) — Enrollment, outbound connectivity, placement, and Machine lifecycle
- [`architecture/16-product-auth.md`](architecture/16-product-auth.md) — Mandatory product login/device proof, self-host identity, and session-deadline enforcement
- [`architecture/17-authentication-plugins.md`](architecture/17-authentication-plugins.md) — Signed authentication packages, configurable login UI, and server-owned session policy
- [`architecture/18-auth-capacity-sso.md`](architecture/18-auth-capacity-sso.md) — Configurable client/session capacity, fair admission, SSO logout, and isolated automation
- [`architecture/19-code-review-engine.md`](architecture/19-code-review-engine.md) — Proposal: Code Review as a capability engine; implementations such as Zed stay internal and swappable
- [`architecture/20-agent-code-intelligence.md`](architecture/20-agent-code-intelligence.md) — Proposal: Code Review diagnostics and navigation for every agent through cowboy MCP, Mods and hooks
- [`../examples/authentication/README.md`](../examples/authentication/README.md) — Google, Apple, and Cloudflare Email Authentication Provider examples
- [`../examples/editor-plugins/README.md`](../examples/editor-plugins/README.md) — Installable editor plugin example and packing command
- [`architecture/runtime-incident-ledger.md`](architecture/runtime-incident-ledger.md) — Runtime failure evidence and resolved invariants
- [`structural-refactor-plan-2026-11.md`](structural-refactor-plan-2026-11.md) — Measured structural baseline after five months of iteration, ranked decay risks, behaviour-preserving refactor slices with their guards, rejected options and quarterly health metrics
- [`composer-state-model.md`](composer-state-model.md) — Remaining `ComposerWorkspace` state, the focus and overlap constraints any new model must keep, and a staged surface reducer proposal
- [`compatibility-exits.md`](compatibility-exits.md) — Every legacy compatibility path with its exit condition, the check that proves it and whether removal is a Machine maintenance boundary; the Controller enforces Machine protocol 37 and broker protocol 5 while the wire crate minimums stay 1

### Core documents

- [`queue-delivery-design-2026-10-19.md`](queue-delivery-design-2026-10-19.md) — Queue delivery design and implementation evidence: busy-send picker, desktop keyboard, mobile focus, native checkpoints and durable interrupt holds
- [`requirements.md`](requirements.md) — Cowboy core requirements, state ownership, and context-preserving Provider reload
- [`device-transport-security.md`](device-transport-security.md) — Implemented mandatory HTTPS/device binding, proxy boundary and enrollment
- [`secure-connectivity-design.md`](secure-connectivity-design.md) — Current security baseline, acceptance priorities and WireGuard roadmap
- [`wireguard-transport.md`](wireguard-transport.md) — Deferred research: Rust implementations and native interoperability evidence
- [`browser-wireguard-transport.md`](browser-wireguard-transport.md) — Deferred research: browser WASM/WSS runtime evidence and unresolved application integration
- [`plugin-components.md`](plugin-components.md) — Plugin manifests, shared component ownership, and coordinated versioning
- [`product-sync-datasets.md`](product-sync-datasets.md) — Immutable Service/user browser datasets, version-fenced outboxes, explicit legacy recovery and Controller/Web rollout
- [`releases/dataset-bound-maintenance-2026-09-15.md`](releases/dataset-bound-maintenance-2026-09-15.md) — Activated dataset-bound Controller/Web, compatible cold floor, independent Machine maintenance and scoped worker evidence
- [`plugin-lifecycle-history.md`](plugin-lifecycle-history.md) — Bounded typed core install/uninstall history and independent resolution, with no replay authority
- [`plugin-session-deletion-journal.md`](plugin-session-deletion-journal.md) — Terminal Session records, component reader floor, bounded recovery and Hawk writer admission
- [`plugin-activation-authority.md`](plugin-activation-authority.md) — Independent old-tool authority audit and selected trusted-administrator scope; stronger isolation remains unimplemented
- [`releases/plugin-session-deletion-reader-2026-10-03.md`](releases/plugin-session-deletion-reader-2026-10-03.md) — Initial bounded reader, IPC fixtures and Hawk writer-disabled activation
- [`releases/plugin-session-deletion-compatibility-2026-10-03.md`](releases/plugin-session-deletion-compatibility-2026-10-03.md) — Immutable reader declarations and component activation/rollback admission
- [`releases/plugin-session-deletion-floor-2026-10-03.md`](releases/plugin-session-deletion-floor-2026-10-03.md) — Root-owned durable reader floor, live undeclared-artifact refusal and same-generation recovery fixtures
- [`experiments/plugin-session-deletion-process-2026-10-03.md`](experiments/plugin-session-deletion-process-2026-10-03.md) — Disposable broker SIGKILL/reopen and storage-failure acceptance; production writer stays disabled
- [`experiments/plugin-session-deletion-releases-2026-10-03.md`](experiments/plugin-session-deletion-releases-2026-10-03.md) — Exact immutable old/new reader releases, 31-process upgrade/rollback/refusal matrix and native ELF digests
- [`releases/plugin-session-deletion-portable-2026-10-03.md`](releases/plugin-session-deletion-portable-2026-10-03.md) — Portable host/launcher refusal for terminal state and rejected-Welcome restart protection; writer stays disabled
- [`releases/plugin-bootstrap-guard-2026-10-03.md`](releases/plugin-bootstrap-guard-2026-10-03.md) — Pre-copy bootstrap compatibility probe and exact legacy launcher refresh/old-installer authority controls
- [`releases/plugin-bootstrap-snapshot-2026-10-03.md`](releases/plugin-bootstrap-snapshot-2026-10-03.md) — Shared bootstrap probe/install bundle and mutable caller-path replacement acceptance
- [`releases/plugin-machine-repair-2026-10-03.md`](releases/plugin-machine-repair-2026-10-03.md) — Exact failed Machine transaction repair with same-generation reader admission
- [`releases/plugin-owner-state-2026-10-03.md`](releases/plugin-owner-state-2026-10-03.md) — Bounded regular-file owner state and closed journal/receipt/reader fields
- [`releases/plugin-transaction-identity-2026-10-03.md`](releases/plugin-transaction-identity-2026-10-03.md) — Durable transaction ID validation and verified owner activation after observation repair and authorized probe retirement
- [`releases/plugin-journal-phase-2026-10-03.md`](releases/plugin-journal-phase-2026-10-03.md) — Closed journal phase admission and explicit selected-target authority before automatic recovery
- [`releases/plugin-maintenance-authority-2026-10-04.md`](releases/plugin-maintenance-authority-2026-10-04.md) — Durable maintenance authority validation for journals and receipts before recovery acceptance
- [`releases/plugin-store-root-2026-10-04.md`](releases/plugin-store-root-2026-10-04.md) — Canonical durable release roots and verified owner activation after authorized terminal-task retirement
- [`releases/plugin-rollback-predecessor-2026-10-04.md`](releases/plugin-rollback-predecessor-2026-10-04.md) — Same-lane manifest admission before ordinary rollback restoration and verified owner activation
- [`releases/plugin-installed-owner-2026-10-04.md`](releases/plugin-installed-owner-2026-10-04.md) — Supported dispatch pinned to the installed immutable owner, retired candidate transactions and preserved sudo policy
- [`releases/plugin-portable-reader-claim-2026-10-04.md`](releases/plugin-portable-reader-claim-2026-10-04.md) — Signed read-only portable reader declaration and preserved legacy transcript; state admission remains closed
- [`releases/plugin-signed-bootstrap-bundle-2026-10-04.md`](releases/plugin-signed-bootstrap-bundle-2026-10-04.md) — Authenticated three-program portable package before guard execution; floored refresh and recovery remain closed
- [`releases/plugin-bootstrap-startup-authentication-2026-10-04.md`](releases/plugin-bootstrap-startup-authentication-2026-10-04.md) — Retained signed bootstrap evidence and installer-owned verification before publisher code; floor recovery remains closed
- [`releases/plugin-floored-signed-refresh-2026-10-04.md`](releases/plugin-floored-signed-refresh-2026-10-04.md) — Signed enrolled refresh preserves authenticated floor, anchor and cache selection; missing-cache recovery remains closed
- [`releases/plugin-floor-selection-recovery-2026-10-04.md`](releases/plugin-floor-selection-recovery-2026-10-04.md) — Explicit offline repair of missing selection pointers to the intact signed floor anchor; lost-anchor recovery remains closed
- [`releases/plugin-anchor-package-recovery-2026-10-04.md`](releases/plugin-anchor-package-recovery-2026-10-04.md) — Linux restoration of a completely absent anchor from its original floor-bound signed proof and artifact; damaged generations remain untouched
- [`releases/plugin-anchor-quarantine-recovery-2026-10-04.md`](releases/plugin-anchor-quarantine-recovery-2026-10-04.md) — Explicit original-proof repair with retained damage outside cache pruning
- [`releases/plugin-cache-streaming-authentication-2026-10-04.md`](releases/plugin-cache-streaming-authentication-2026-10-04.md) — Stream large cached artifacts at startup while preserving signed proof and floor authentication
- [`releases/plugin-cache-key-input-2026-10-04.md`](releases/plugin-cache-key-input-2026-10-04.md) — Bounded regular publisher-key startup input and immutable key-link/FIFO refusal acceptance
- [`releases/plugin-deletion-namespace-path-2026-10-04.md`](releases/plugin-deletion-namespace-path-2026-10-04.md) — Logical deletion namespace identity, linked-directory refusal and exact native controls
- [`experiments/plugin-deletion-immutable-writer-2026-10-04.md`](experiments/plugin-deletion-immutable-writer-2026-10-04.md) — Independent private writer ELFs, actual reader bridge and nondeployable owner refusal
- [`releases/plugin-deletion-owner-writer-admission-2026-10-04.md`](releases/plugin-deletion-owner-writer-admission-2026-10-04.md) — Installed host-owner writer declaration admission behind the durable reader floor; production writer remains disabled
- [`releases/plugin-deletion-production-writer-2026-10-04.md`](releases/plugin-deletion-production-writer-2026-10-04.md) — Exact production writer binaries, 32 private-root IPC/authority groups and retained-generation Hawk activation
- [`releases/plugin-portable-reader-floor-2026-10-04.md`](releases/plugin-portable-reader-floor-2026-10-04.md) — Portable monotonic reader floor and retained signed anchor; bootstrap/recovery and committed deletion admission stay closed
- [`releases/plugin-host-startup-integrity-2026-10-04.md`](releases/plugin-host-startup-integrity-2026-10-04.md) — Offline portable cached-host signature, package and selection checks before exec; committed-state admission and writer remain closed
- [`releases/plugin-host-cache-integrity-2026-10-04.md`](releases/plugin-host-cache-integrity-2026-10-04.md) — Signed staged Machine host bytes checked before probe and pointer publication; cached startup admission remains closed
- [`releases/plugin-catalog-observer-2026-09-15.md`](releases/plugin-catalog-observer-2026-09-15.md) — Owned Catalog observation and verified Controller activation; actual candidate/predecessor/cold readers agree on 69 signed releases
- [`plugin-service-sites.md`](plugin-service-sites.md) — Core-established Service identity and final Site checks for finite installation, telemetry and recovery transports
- [`plugin-code-read-scopes.md`](plugin-code-read-scopes.md) — Session-scoped responses, connection-bound Zed operations, remaining buffer ownership gaps, bounded diff/file continuations and page ETags
- [`plugin-native-buffer-leases.md`](plugin-native-buffer-leases.md) — Prepared native buffer references and exact Machine runtime retention; HTTP/Web integration and production maintenance remain separate
- [`plugin-native-buffer-sync.md`](plugin-native-buffer-sync.md) — Installed private conditional synchronization; core authority and Review cutover remain separate
- [`plugin-buffer-sync-owners.md`](plugin-buffer-sync-owners.md) — Private adapter exclusive ownership, one-use synchronization and retained unknown fences; no core write grant
- [`releases/zed-native-sync-2026-09-16.md`](releases/zed-native-sync-2026-09-16.md) — Signed Zed 1.7.0 installation and separate Hawk Machine maintenance, exact native gates and bounded worker continuity
- [`releases/zed-sync-owners-2026-09-16.md`](releases/zed-sync-owners-2026-09-16.md) — Signed Zed 1.8.0 ownership exclusion, Hawk upgrade retaining Code/worker processes, and a verified but unactivated Machine candidate
- [`plugin-process-cleanup.md`](plugin-process-cleanup.md) — Core-owned process-group signals without PATH helpers, exact worker mapping and permission-failure fences
- [`releases/plugin-process-cleanup-2026-09-16.md`](releases/plugin-process-cleanup-2026-09-16.md) — Activated Controller cleanup repair and eight-group actual Code installation acceptance; links the subsequent separate Machine activation
- [`plugin-controller-buffer-owners.md`](plugin-controller-buffer-owners.md) — Product-owned Controller references, bounded admitted continuations and path-free original-owner cleanup; Review/native rollout remains separate
- [`plugin-owned-buffer-reads.md`](plugin-owned-buffer-reads.md) — Closed original-owner diagnostic/symbol observations; lower-bound versions are not positional authority, with separate native rollout
- [`plugin-buffer-client-owner.md`](plugin-buffer-client-owner.md) — Typed browser continuation owner, bounded transport and real StrictMode acceptance; ordinary Review cutover remains separate
- [`plugin-buffer-product-context.md`](plugin-buffer-product-context.md) — Core identity/session-end integration, irreversible owner fencing and final local-outbox drain; no Review or native-generation cutover
- [`releases/plugin-buffer-product-context-2026-09-16.md`](releases/plugin-buffer-product-context-2026-09-16.md) — Verified Web-only activation of product lifetime integration; 13 workers retained, with Review/native rollout still separate
- [`releases/plugin-owned-buffer-reads-2026-09-16.md`](releases/plugin-owned-buffer-reads-2026-09-16.md) — Activated Controller observations and verified Zed diagnostic-protocol repair; 13 workers retained, with Machine/Code rollout separate
- [`releases/plugin-controller-buffer-owners-2026-09-15.md`](releases/plugin-controller-buffer-owners-2026-09-15.md) — Verified Controller candidate and 20 new tests; historical publication blocker and later running-source follow-up
- [`releases/agent-publication-2026-09-15.md`](releases/agent-publication-2026-09-15.md) — Six independent signed Agent publications, Linux/Mac gates, four actual Catalog readers and automatic adoption without component restart or Plugin installation
- [`releases/claude-plan-usage-2026-09-16.md`](releases/claude-plan-usage-2026-09-16.md) — Initial native Anthropic usage publication and the subsequent subscriber correction
- [`releases/agent-operator-upgrade-2026-09-16.md`](releases/agent-operator-upgrade-2026-09-16.md) — Live host Operator CLI, completed Claude Code 3.1.25 upgrade and verified Max plan progress
- [`releases/plugin-native-buffer-leases-2026-09-15.md`](releases/plugin-native-buffer-leases-2026-09-15.md) — Verified Zed 1.3.0 and Machine candidates, 29 new tests and real signed-install/path-removal drain; no production activation
- [`releases/plugin-code-read-scopes-2026-09-15.md`](releases/plugin-code-read-scopes-2026-09-15.md) — Verified scoped-code Controller activation with 15 workers retained
- [`releases/plugin-buffered-code-reads-2026-09-15.md`](releases/plugin-buffered-code-reads-2026-09-15.md) — Verified buffered code-response and closed-request Controller activation with 16 workers retained
- [`releases/plugin-file-page-scopes-2026-09-15.md`](releases/plugin-file-page-scopes-2026-09-15.md) — Scoped file continuations, page ETags, UTF-8 boundaries and verified Controller activation; remote adapter maintenance remains separate
- [`releases/plugin-zed-operation-scopes-2026-09-15.md`](releases/plugin-zed-operation-scopes-2026-09-15.md) — Connection-bound Zed operations and verified Controller activation with 12 workers retained; cross-request resource recovery remains separate
- [`releases/plugin-service-sites-2026-09-15.md`](releases/plugin-service-sites-2026-09-15.md) — Accepted Controller Site isolation, 807 immutable checks and production activation
- [`plugin-spatiotemporal-design.md`](plugin-spatiotemporal-design.md) — Master target design for components, Plugins, Service/Machine scopes, authority, generations, state, effects, migration and acceptance gates
- [`history/module-cycle-edges-2026-11.md`](history/module-cycle-edges-2026-11.md) — Phase 5.1 inventory of the former 37-module base-crate cycle: edges classified type, constant or behavioural, the port or move for each behavioural group
- [`history/plugin-refactor-completion.md`](history/plugin-refactor-completion.md) — Current implementation, production-acceptance and independent-recovery exit checklist
- [`plugin-domain-executor-binding.md`](plugin-domain-executor-binding.md) — The rule for binding resolved results to domain executors as the telemetry code states it, an inventory of which domains have it, and why no generic mechanism is built
- [`plugin-state-lease-design.md`](plugin-state-lease-design.md) — Design (unimplemented, deliberately): the contract a state lease must satisfy, and the finding that no dataset has multiple holders so there is no consumer to build it for yet
- [`plugin-state-dataset-design.md`](plugin-state-dataset-design.md) — Design (unimplemented) for general state-dataset compatibility: inventory, advisory versus safety-critical rule, ten-point contract and factoring of the duplicated namespace and owner code
- [`releases/durable-state-diagnostics-2026-10-06.md`](releases/durable-state-diagnostics-2026-10-06.md) — Hawk Machine and Controller releases of the durable-state report; first live read matched the files (7 deletions, 7 lineages)
- [`machine-durable-state-diagnostics.md`](machine-durable-state-diagnostics.md) — Read-only content-free report of a Machine's deletion journal, incarnation and cleanup-continuation state: route, closed schema, limits
- [`machine-worker-pin-advance-runbook.md`](machine-worker-pin-advance-runbook.md) — What advancing the retained worker pin does to Hawk, when it is required, and the verified acceptance, build, conformance and activation steps
- [`plugin-session-incarnation-carriage.md`](plugin-session-incarnation-carriage.md) — Carrying the lineage to the Controller: implemented and tested on a local branch, held off main because the wire change blocks host-only Machine releases until the worker pin advances
- [`plugin-session-incarnation-writer.md`](plugin-session-incarnation-writer.md) — Incarnation writer source: mint before declaring, rotate before a reset's first effect, end after the deletion commit, floor-gated admission before any namespace opens
- [`plugin-session-incarnation-reader.md`](plugin-session-incarnation-reader.md) — Schema-1 incarnation namespace reader with the writer disabled: closed record, exclusive ownership and startup refusal
- [`plugin-session-incarnation-design.md`](plugin-session-incarnation-design.md) — Design (unimplemented) for a Machine-minted durable Session incarnation, its reader-first floor, carriage and acceptance evidence
- [`history/plugin-refactor-status-2026-10-05.md`](history/plugin-refactor-status-2026-10-05.md) — Dated per-exit status against production receipts, corrected historical Web recovery and the dependency order for durable Session incarnation
- [`plugin-session-worktree-containment.md`](plugin-session-worktree-containment.md) — Preparation refuses linked checkouts and escaping selections before mutating reused branches
- [`plugin-session-cleanup-root.md`](plugin-session-cleanup-root.md) — Terminal cleanup retains the original directory handle and retires after observed root replacement
- [`plugin-session-cleanup-markers.md`](plugin-session-cleanup-markers.md) — Directory-relative regular-file marker probes, FIFO refusal and 8 KiB tag bounds
- [`plugin-session-cleanup-continuation.md`](plugin-session-cleanup-continuation.md) — Advisory durable nomination of a deleted Session's original root so a restarted writer-admitted Machine finishes cleanup with a fresh scan
- [`plugin-session-cleanup-targets.md`](plugin-session-cleanup-targets.md) — Bounded retained target handles, replacement/marker refusal and Linux content anchoring
- [`releases/plugin-session-cleanup-finalization-2026-10-05.md`](releases/plugin-session-cleanup-finalization-2026-10-05.md) — Hawk retains markers until content success; 1877/551 tests, partial-error retry fixture, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-retry-2026-10-05.md`](releases/plugin-session-cleanup-retry-2026-10-05.md) — Hawk retains original target observations across I/O retries; 1892/561 tests, 32 native groups and 19 retained worker/keeper processes
- [`releases/cleanup-pass-budget-2026-10-06.md`](releases/cleanup-pass-budget-2026-10-06.md) — Hawk resident release giving each cleanup pass a scan and removal time budget; 45 native groups, 27 retained units, lineages read back
- [`releases/incarnation-writer-2026-10-06.md`](releases/incarnation-writer-2026-10-06.md) — Hawk resident release enabling the incarnation writer: 45 native groups, 8 adopted lineages for 8 live workers, 28 retained units, nothing consumes the value yet
- [`releases/worker-pin-self-throttle-admission-2026-11-05.md`](releases/worker-pin-self-throttle-admission-2026-11-05.md) — Hawk Machine host release advancing the worker pin to the self-throttle admission fix: acceptance matrix, receipts, keepers retained
- [`releases/worker-pin-incarnation-carriage-2026-10-07.md`](releases/worker-pin-incarnation-carriage-2026-10-07.md) — Hawk Machine and Controller release advancing the worker pin to the incarnation carriage: acceptance, receipts, no worker drained, connected Code failure traced to a test-relay gap
- [`releases/hibernation-cache-revoke-2026-10-06.md`](releases/hibernation-cache-revoke-2026-10-06.md) — Hawk resident release: hibernation revokes DeepSeek cache keep-alives; incarnation reader floor anchored; 37 native groups; 28 retained units
- [`hibernation-token-audit-2026-10-06.md`](hibernation-token-audit-2026-10-06.md) — Audit of hibernate and wake token cost: zero model requests on Codex cold resume and packaged ACP wake; end-to-end cost, DeepSeek cache protection and other providers not established
- [`releases/machine-pool-candidate-2026-10-05.md`](releases/machine-pool-candidate-2026-10-05.md) — Worker-pool candidate `worker-5e009066…`: Codex execution 16, logs 11, session 9 and Codex/Claude (Hawk-installed) coexistence accepted; Claude native execution and connected Code outstanding; not activated
- [`releases/plugin-session-cleanup-continuation-2026-10-05.md`](releases/plugin-session-cleanup-continuation-2026-10-05.md) — Hawk resumes deleted-Session cleanup after a resident restart; 1905/573 tests, 32 native groups, startup pending=0 and 19 of 20 retained worker/keeper processes
- [`releases/plugin-session-cleanup-marker-progress-2026-10-05.md`](releases/plugin-session-cleanup-marker-progress-2026-10-05.md) — Hawk resumes owned partial marker finalization; 1894/563 tests, three mount fixtures, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-leaves-2026-10-05.md`](releases/plugin-session-cleanup-leaves-2026-10-05.md) — Hawk pins leaf identities before unlink; 1876/550 tests, actual file/dir mount fixtures, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-descendants-2026-10-05.md`](releases/plugin-session-cleanup-descendants-2026-10-05.md) — Hawk recursively retains directory handles/structure; 1874/548 tests, two actual mount fixtures, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-scan-2026-10-05.md`](releases/plugin-session-cleanup-scan-2026-10-05.md) — Hawk scan opens reject linked ancestors and mounts; 1871/545 tests, actual bind-mount fixture, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-retained-2026-10-05.md`](releases/plugin-session-cleanup-retained-2026-10-05.md) — Hawk retains empty targets to avoid final pathname unlink; 1870/544 tests, 32 native groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-targets-2026-10-05.md`](releases/plugin-session-cleanup-targets-2026-10-05.md) — Hawk target-handle activation, 1868/543 Rust tests, 32 native writer groups and 19 retained worker/keeper processes
- [`releases/plugin-session-cleanup-markers-2026-10-05.md`](releases/plugin-session-cleanup-markers-2026-10-05.md) — Hawk bounded-marker activation, 1865/540 Rust tests, 32 native writer groups and 18 retained worker/keeper processes
- [`releases/plugin-session-cleanup-root-2026-10-05.md`](releases/plugin-session-cleanup-root-2026-10-05.md) — Hawk cleanup-root activation, integrated Rust checks, 32 native writer groups and 18 retained worker/keeper processes
- [`releases/plugin-session-worktree-containment-2026-10-04.md`](releases/plugin-session-worktree-containment-2026-10-04.md) — Hawk writer-host activation, 535 Machine tests, 32 native writer groups and 18 retained worker/keeper processes
- [`desktop-efficiency-redesign.md`](desktop-efficiency-redesign.md) — Desktop information density and interaction contract
- [`ux-principles.md`](ux-principles.md) — Core efficiency contract: bottom controls on Mobile, visible keyboard commands on Desktop, state continuity and rendered acceptance
- [`explore-transcript-design.md`](explore-transcript-design.md) — Explore's read-only transcript projection
- [`mobile-spatial-presentation.md`](mobile-spatial-presentation.md) — Jank-free drawers, pager, transcript, CodeMirror, iPhone PWA status material, iPad standalone chrome, and iOS compositor contract
- [`offline-first-sync.md`](offline-first-sync.md) — Offline-first design: local replica boot, prioritized hydration, one sync status, outbox classes, conflict catalog and required server changes
- [`sessions-folders.md`](sessions-folders.md) — Sessions folders: atomic creation, inherited session/Zed defaults, searchable project tree, scoped batch deletion, CLI and keyboard UX
- [`undo-history.md`](undo-history.md) — 3 s notices, device-local Undo history with Can undo/Undone/Unavailable states, 24-hour TTL and entry points
- [`ios-simulator.md`](ios-simulator.md) — Local iOS Simulator bridge and verification workflow
- [`machine-plugin-membership.md`](machine-plugin-membership.md) — Service-side declaration of which Plugins a Machine runs, and why a declared Machine has no client lifecycle actions
- [`machine-component-convergence.md`](machine-component-convergence.md) — Continuous convergence of signed automatic Machine components: authority, drain, backoff and reported state
- [`machine-operations.md`](machine-operations.md) — Machine operations, including Provider installation and Service-auth replica convergence
- [`plugin-packages.md`](plugin-packages.md) — Package, typed UI, authentication/Transcript presentation, and release contract for independently released, Machine-scoped Provider packages
- [`provider-auth-sync-coordination.md`](provider-auth-sync-coordination.md) — Bounded same-generation reconciliation on one authenticated connection, cancellation and late-receipt fencing

### Plugin platform notes

- [`machine-plugin-operation-receipts.md`](machine-plugin-operation-receipts.md) — Machine-owned Plugin uninstall receipts
- [`plugin-browser-buffer-destinations.md`](plugin-browser-buffer-destinations.md) — Browser-owned navigation destinations
- [`plugin-browser-buffer-navigation.md`](plugin-browser-buffer-navigation.md) — Browser-owned navigation continuation
- [`plugin-browser-buffer-sync.md`](plugin-browser-buffer-sync.md) — Browser-owned Code synchronization
- [`plugin-buffer-cleanup-surface.md`](plugin-buffer-cleanup-surface.md) — Core Code cleanup in Settings
- [`plugin-code-connected-conformance.md`](plugin-code-connected-conformance.md) — Connected core Code buffer acceptance
- [`plugin-code-read-authority.md`](plugin-code-read-authority.md) — Buffered Code read authority
- [`plugin-content-bound-reads.md`](plugin-content-bound-reads.md) — Reads conditional on displayed content
- [`plugin-continuation-finalization.md`](plugin-continuation-finalization.md) — Core Code continuation finalization
- [`plugin-generation-migration.md`](plugin-generation-migration.md) — Broken Plugin generation migration preparation
- [`plugin-install-connected-conformance.md`](plugin-install-connected-conformance.md) — Connected installation acceptance
- [`plugin-install-continuation.md`](plugin-install-continuation.md) — Core-owned install and upgrade attempts
- [`plugin-install-journal.md`](plugin-install-journal.md) — Durable core installation attempts
- [`plugin-local-root-identity.md`](plugin-local-root-identity.md) — Controller-owned identity for locally executed reads
- [`plugin-machine-buffer-navigation.md`](plugin-machine-buffer-navigation.md) — Machine-owned navigation continuation
- [`plugin-machine-buffer-sync.md`](plugin-machine-buffer-sync.md) — Machine-owned buffer synchronization
- [`plugin-machine-install-attempts.md`](plugin-machine-install-attempts.md) — Machine installation attempts
- [`plugin-machine-workspace-identity.md`](plugin-machine-workspace-identity.md) — Machine-owned Workspace root identity
- [`plugin-native-acquisition-budgets.md`](plugin-native-acquisition-budgets.md) — Private native acquisition lifetime budgets
- [`plugin-native-buffer-coordinates.md`](plugin-native-buffer-coordinates.md) — Native coordinates without replacing buffer owners
- [`plugin-native-close-confirmation.md`](plugin-native-close-confirmation.md) — Original-peer native close confirmation
- [`plugin-native-input-bounds.md`](plugin-native-input-bounds.md) — Private native input budgets
- [`plugin-native-navigation-budgets.md`](plugin-native-navigation-budgets.md) — Private whole-query navigation budgets
- [`plugin-native-open-once.md`](plugin-native-open-once.md) — Single-use native buffer acquisition
- [`plugin-native-reload-bounds.md`](plugin-native-reload-bounds.md) — Private native reload input bounds
- [`plugin-native-remote-edits.md`](plugin-native-remote-edits.md) — Private native remote-edit admission
- [`plugin-native-replacement-budgets.md`](plugin-native-replacement-budgets.md) — Private native sync/reload replacement budgets
- [`plugin-native-snapshot-lifetimes.md`](plugin-native-snapshot-lifetimes.md) — Private native text-snapshot lifetimes
- [`plugin-native-text-reads.md`](plugin-native-text-reads.md) — Complete text from an original native buffer
- [`plugin-operation-journal.md`](plugin-operation-journal.md) — Plugin uninstall: Service operation journal
- [`plugin-owned-navigation.md`](plugin-owned-navigation.md) — Owned native navigation candidate
- [`plugin-product-permission-lifetimes.md`](plugin-product-permission-lifetimes.md) — Core product permission lifetimes
- [`plugin-release-leases.md`](plugin-release-leases.md) — Core verified release observations
- [`plugin-review-owned-consumer.md`](plugin-review-owned-consumer.md) — Content-owned Review source consumer
- [`plugin-review-owned-destinations.md`](plugin-review-owned-destinations.md) — Review-owned navigation destination reader
- [`plugin-review-owned-diff.md`](plugin-review-owned-diff.md) — Content-owned working diff reads
- [`plugin-runtime-bindings.md`](plugin-runtime-bindings.md) — Connection-scoped runtime bindings
- [`plugin-service-buffer-navigation.md`](plugin-service-buffer-navigation.md) — Service-owned navigation continuation
- [`plugin-service-buffer-sync.md`](plugin-service-buffer-sync.md) — Service-owned buffer synchronization
- [`plugin-session-lifecycle-fence.md`](plugin-session-lifecycle-fence.md) — Machine Session declaration/deletion fence
- [`plugin-session-read-routes.md`](plugin-session-read-routes.md) — Session read routes
- [`plugin-session-root-observations.md`](plugin-session-root-observations.md) — Machine-owned Session filesystem observations
- [`plugin-session-snapshot-placement.md`](plugin-session-snapshot-placement.md) — Session launch declarations and worker snapshots
- [`plugin-sync-budget-outcomes.md`](plugin-sync-budget-outcomes.md) — Original-operation synchronization budget outcomes
- [`plugin-workspace-configuration-identity.md`](plugin-workspace-configuration-identity.md) — Workspace identity across accepted configuration changes
- [`plugin-workspace-read-scopes.md`](plugin-workspace-read-scopes.md) — Continuous Workspace read scopes

### Telemetry notes

- [`client-opentelemetry.md`](client-opentelemetry.md) — Client OpenTelemetry
- [`machine-telemetry-writer-preflight.md`](machine-telemetry-writer-preflight.md) — Machine telemetry writer-policy preflight
- [`resolved-telemetry-ports.md`](resolved-telemetry-ports.md) — Finite live telemetry resolution
- [`telemetry-background-policy.md`](telemetry-background-policy.md) — Explicit managed background export policy
- [`telemetry-background-startup-conformance.md`](telemetry-background-startup-conformance.md) — Managed background startup conformance
- [`telemetry-binding-journal.md`](telemetry-binding-journal.md) — Durable telemetry binding reader bridge
- [`telemetry-binding-surface.md`](telemetry-binding-surface.md) — Core telemetry binding confirmation
- [`telemetry-binding-transactions.md`](telemetry-binding-transactions.md) — Finite Machine telemetry binding transactions
- [`telemetry-binding-wire.md`](telemetry-binding-wire.md) — Finite cross-site telemetry binding protocol
- [`telemetry-connected-conformance.md`](telemetry-connected-conformance.md) — Connected immutable telemetry conformance
- [`telemetry-execution-leases.md`](telemetry-execution-leases.md) — Telemetry attempt ownership
- [`telemetry-machine-recovery-surface.md`](telemetry-machine-recovery-surface.md) — Core Machine recovery confirmation surface
- [`telemetry-managed-attempts.md`](telemetry-managed-attempts.md) — Managed telemetry: one bounded OTLP attempt
- [`telemetry-plugins.md`](telemetry-plugins.md) — Telemetry backends
- [`telemetry-policy-preflight.md`](telemetry-policy-preflight.md) — Telemetry startup policy preflight
- [`telemetry-reader-conformance.md`](telemetry-reader-conformance.md) — Populated telemetry reader conformance
- [`telemetry-recovery-audit-discovery.md`](telemetry-recovery-audit-discovery.md) — Durable Machine recovery audit discovery
- [`telemetry-resolution-surface.md`](telemetry-resolution-surface.md) — Core telemetry resolution confirmation
- [`telemetry-service-coordination.md`](telemetry-service-coordination.md) — Service telemetry binding coordination and reader floor
- [`telemetry-victoria-conformance.md`](telemetry-victoria-conformance.md) — Connected Victoria database conformance
- [`telemetry-writer-admission.md`](telemetry-writer-admission.md) — Per-target telemetry writer admission
- [`telemetry-writer-conformance.md`](telemetry-writer-conformance.md) — Immutable telemetry writer-policy conformance

### Product and runtime notes

- [`atomic-idb-outboxes.md`](atomic-idb-outboxes.md) — Atomic browser outbox deltas
- [`branding-and-app-icons.md`](branding-and-app-icons.md) — Cowboy brand and app icons
- [`catalog-reader-bridge.md`](catalog-reader-bridge.md) — Pre-cutover Catalog reader bridge
- [`claude-stream-recovery.md`](claude-stream-recovery.md) — Recovering after a partial Claude response
- [`codex-durable-resume.md`](codex-durable-resume.md) — Codex restore and durable media
- [`core-security-ownership.md`](core-security-ownership.md) — CoreSecurity namespace ownership
- [`core-web-plugin-host.md`](core-web-plugin-host.md) — Core-owned Web Plugin presentation
- [`native-projects.md`](native-projects.md) — Native projects and AI placement
- [`ovh-machine-maintenance.md`](ovh-machine-maintenance.md) — OVH Machine host maintenance
- [`owned-component-scopes.md`](owned-component-scopes.md) — Owned component lifetimes
- [`owned-idb-connections.md`](owned-idb-connections.md) — Owned IndexedDB connections
- [`pending-disclosure-performance.md`](pending-disclosure-performance.md) — Pending panel disclosure during live updates
- [`provider-credential-observation.md`](provider-credential-observation.md) — Provider credential observation
- [`provider-management-ownership.md`](provider-management-ownership.md) — Core-owned Provider sign-in and confirmation
- [`provider-ui-ownership.md`](provider-ui-ownership.md) — Provider UI authoring and core-owned execution
- [`remote-pr-review.md`](remote-pr-review.md) — Session remote pull request review
- [`remote-session-badge-state.md`](remote-session-badge-state.md) — Remote session badges
- [`transcript-streaming-turn-boundary.md`](transcript-streaming-turn-boundary.md) — Streaming carets belong to the current turn
- [`workspace-extensions.md`](workspace-extensions.md) — Workspace extensions

### Experiments and measurements

- [`experiments/claude-execution-integration-2026-10-02.md`](experiments/claude-execution-integration-2026-10-02.md) — Claude execution integration
- [`experiments/claude-mods-execution-2026-10-02.md`](experiments/claude-mods-execution-2026-10-02.md) — Claude native tools through Mods

### Historical records

- [`history/pluginization-handoff.md`](history/pluginization-handoff.md) — Pluginization handoff ledger from the September core extraction (moved from the repository root; finished work)

### Release records

- [`releases/about-recovery-2026-09-15.md`](releases/about-recovery-2026-09-15.md) — About: compact local recovery and separate telemetry diagnostics
- [`releases/agent-refresh-2026-10-01.md`](releases/agent-refresh-2026-10-01.md) — Agent runtime and Recommended refresh — 2026-10-01
- [`releases/agent-settings-usage-2026-09-08.md`](releases/agent-settings-usage-2026-09-08.md) — Agent settings, delivery feedback and usage — 2026-09-08
- [`releases/agent-upgrades-2026-09-19.md`](releases/agent-upgrades-2026-09-19.md) — Agent publication, Hawk upgrades and Controller activation — 2026-09-19
- [`releases/atomic-idb-outboxes-2026-09-14.md`](releases/atomic-idb-outboxes-2026-09-14.md) — Atomic browser outbox Web release — 2026-09-14
- [`releases/browser-buffer-sync-2026-09-17.md`](releases/browser-buffer-sync-2026-09-17.md) — Browser buffer synchronization delivery — 2026-09-17
- [`releases/browser-destination-candidate-2026-09-18.md`](releases/browser-destination-candidate-2026-09-18.md) — Browser destination handoff candidate — 2026-09-18
- [`releases/browser-navigation-candidate-2026-09-18.md`](releases/browser-navigation-candidate-2026-09-18.md) — Browser navigation continuation candidate — 2026-09-18
- [`releases/catalog-null-digest-2026-09-30.md`](releases/catalog-null-digest-2026-09-30.md) — Nullable Catalog digest convergence repair
- [`releases/claude-mods-execution-2026-10-02.md`](releases/claude-mods-execution-2026-10-02.md) — Claude native tool Mods, 2026-10-02
- [`releases/claude-native-execution-2026-10-02.md`](releases/claude-native-execution-2026-10-02.md) — Claude native execution rollout, 2026-10-02
- [`releases/claude-presets-2026-10-02.md`](releases/claude-presets-2026-10-02.md) — Claude recommendations and default, 2026-10-02
- [`releases/claude-stream-recovery-2026-09-15.md`](releases/claude-stream-recovery-2026-09-15.md) — Claude partial-stream recovery — 2026-09-15
- [`releases/claude-tool-concurrency-2026-10-02.md`](releases/claude-tool-concurrency-2026-10-02.md) — Claude remote tool scheduling, 2026-10-02
- [`releases/codex-resume-2026-09-12.md`](releases/codex-resume-2026-09-12.md) — Codex resume transport — 2026-09-12
- [`releases/cowboy-upgrades-2026-09-30.md`](releases/cowboy-upgrades-2026-09-30.md) — Cowboy dependency upgrade — 2026-09-30
- [`releases/curlseal-lilac-flow-2026-09-19.md`](releases/curlseal-lilac-flow-2026-09-19.md) — Curlseal / Lilac Flow branding release
- [`releases/device-transport-security-2026-10-03.md`](releases/device-transport-security-2026-10-03.md) — Mandatory device authentication and HTTPS — 2026-10-03
- [`releases/execution-binding-readers-2026-10-01.md`](releases/execution-binding-readers-2026-10-01.md) — Execution binding readers, 2026-10-01
- [`releases/machine-buffer-sync-candidate-2026-09-17.md`](releases/machine-buffer-sync-candidate-2026-09-17.md) — Machine-owned synchronization candidate — 2026-09-17
- [`releases/machine-installer-refresh-2026-10-03.md`](releases/machine-installer-refresh-2026-10-03.md) — Enrolled Machine installer refresh — 2026-10-03
- [`releases/machine-navigation-candidate-2026-09-17.md`](releases/machine-navigation-candidate-2026-09-17.md) — Machine navigation continuation candidate — 2026-09-17
- [`releases/machine-writer-preflight-2026-09-14.md`](releases/machine-writer-preflight-2026-09-14.md) — Machine writer-policy preflight candidate: 2026-09-14
- [`releases/machine-writer-preflight-activation-2026-09-14.md`](releases/machine-writer-preflight-activation-2026-09-14.md) — Machine writer-policy preflight activation: 2026-09-14
- [`releases/native-acquisition-budgets-candidate-2026-09-18.md`](releases/native-acquisition-budgets-candidate-2026-09-18.md) — Native acquisition lifetime candidate — 2026-09-18
- [`releases/native-close-confirmation-candidate-2026-09-18.md`](releases/native-close-confirmation-candidate-2026-09-18.md) — Original-peer native close candidate — 2026-09-18
- [`releases/native-execution-rollout-2026-10-02.md`](releases/native-execution-rollout-2026-10-02.md) — Native execution rollout, 2026-10-02
- [`releases/native-input-bounds-candidate-2026-09-18.md`](releases/native-input-bounds-candidate-2026-09-18.md) — Native input-bounds candidate — 2026-09-18
- [`releases/native-navigation-budgets-candidate-2026-09-18.md`](releases/native-navigation-budgets-candidate-2026-09-18.md) — Native whole-query navigation candidate — 2026-09-18
- [`releases/native-open-once-candidate-2026-09-18.md`](releases/native-open-once-candidate-2026-09-18.md) — Single-use native Open candidate — 2026-09-18
- [`releases/native-reload-bounds-candidate-2026-09-18.md`](releases/native-reload-bounds-candidate-2026-09-18.md) — Native reload input/cleanup candidate — 2026-09-18
- [`releases/native-remote-edit-budgets-2026-09-19.md`](releases/native-remote-edit-budgets-2026-09-19.md) — Native remote-edit admission release — 2026-09-19
- [`releases/native-replacement-budgets-candidate-2026-09-18.md`](releases/native-replacement-budgets-candidate-2026-09-18.md) — Native sync/reload replacement candidate — 2026-09-18
- [`releases/native-snapshot-lifetimes-2026-09-19.md`](releases/native-snapshot-lifetimes-2026-09-19.md) — Native text-snapshot lifetime release — 2026-09-19
- [`releases/native-text-reads-candidate-2026-09-18.md`](releases/native-text-reads-candidate-2026-09-18.md) — Original native text reader candidate — 2026-09-18
- [`releases/owned-navigation-candidate-2026-09-17.md`](releases/owned-navigation-candidate-2026-09-17.md) — Owned native navigation candidate — 2026-09-17
- [`releases/owned-navigation-handoff-2026-09-17.md`](releases/owned-navigation-handoff-2026-09-17.md) — Owned navigation handoff candidate — 2026-09-17
- [`releases/plugin-buffer-cleanup-surface-2026-09-16.md`](releases/plugin-buffer-cleanup-surface-2026-09-16.md) — Core Code cleanup Web delivery — 2026-09-16
- [`releases/plugin-buffer-outcome-authority-2026-09-19.md`](releases/plugin-buffer-outcome-authority-2026-09-19.md) — Owned buffer outcome authority — 2026-09-19
- [`releases/plugin-code-connected-conformance-2026-09-16.md`](releases/plugin-code-connected-conformance-2026-09-16.md) — Connected core Code acceptance — 2026-09-16
- [`releases/plugin-code-read-authority-2026-09-19.md`](releases/plugin-code-read-authority-2026-09-19.md) — Buffered Code read authority — accepted Controller release
- [`releases/plugin-cold-reader-floor-2026-09-19.md`](releases/plugin-cold-reader-floor-2026-09-19.md) — Plugin cold-reader recovery floor — 2026-09-19
- [`releases/plugin-colocated-connected-2026-09-25.md`](releases/plugin-colocated-connected-2026-09-25.md) — Connected coverage of the colocated topology — 2026-09-25
- [`releases/plugin-colocated-permission-2026-09-24.md`](releases/plugin-colocated-permission-2026-09-24.md) — Colocated execution needs permission, not a claim — 2026-09-24
- [`releases/plugin-colocated-withdrawal-2026-09-28.md`](releases/plugin-colocated-withdrawal-2026-09-28.md) — Connected colocation permission withdrawal — 2026-09-28
- [`releases/plugin-completion-2026-09-08.md`](releases/plugin-completion-2026-09-08.md) — Plugin completion and retirement — 2026-09-08
- [`releases/plugin-content-bound-reads-2026-09-16.md`](releases/plugin-content-bound-reads-2026-09-16.md) — Content-bound Code observations — 2026-09-16
- [`releases/plugin-continuation-finalization-2026-09-19.md`](releases/plugin-continuation-finalization-2026-09-19.md) — Core Code continuation finalization — 2026-09-19
- [`releases/plugin-install-continuation-2026-09-14.md`](releases/plugin-install-continuation-2026-09-14.md) — Core Plugin installation continuation release: 2026-09-14
- [`releases/plugin-install-journal-2026-09-14.md`](releases/plugin-install-journal-2026-09-14.md) — Durable core installation release: 2026-09-14
- [`releases/plugin-install-receipt-readers-2026-09-14.md`](releases/plugin-install-receipt-readers-2026-09-14.md) — Typed installation receipt readers: 2026-09-14
- [`releases/plugin-install-writers-2026-09-14.md`](releases/plugin-install-writers-2026-09-14.md) — Durable installation writer activation — 2026-09-14
- [`releases/plugin-language-read-authority-2026-09-19.md`](releases/plugin-language-read-authority-2026-09-19.md) — Legacy language read authority — 2026-09-19
- [`releases/plugin-local-root-identity-2026-09-21.md`](releases/plugin-local-root-identity-2026-09-21.md) — Controller-owned identity for locally executed reads — 2026-09-21
- [`releases/plugin-machine-workspace-identity-2026-09-20.md`](releases/plugin-machine-workspace-identity-2026-09-20.md) — Machine-owned Workspace root identity — 2026-09-20
- [`releases/plugin-native-coordinates-2026-09-16.md`](releases/plugin-native-coordinates-2026-09-16.md) — Zed native-coordinate candidate — 2026-09-16
- [`releases/plugin-product-permission-lifetimes-2026-09-19.md`](releases/plugin-product-permission-lifetimes-2026-09-19.md) — Core product permission lifetimes — 2026-09-19
- [`releases/plugin-publication-2026-09-07.md`](releases/plugin-publication-2026-09-07.md) — Plugin publication — 2026-09-07
- [`releases/plugin-release-leases-2026-09-15.md`](releases/plugin-release-leases-2026-09-15.md) — Verified release observations — Controller release, 2026-09-15
- [`releases/plugin-root-identity-adoption-2026-10-03.md`](releases/plugin-root-identity-adoption-2026-10-03.md) — Hawk root-identity adoption observed October 3
- [`releases/plugin-session-lifecycle-fence-2026-10-03.md`](releases/plugin-session-lifecycle-fence-2026-10-03.md) — Machine Session declaration/deletion fence — 2026-10-03
- [`releases/plugin-session-read-routes-2026-09-19.md`](releases/plugin-session-read-routes-2026-09-19.md) — Session read routes — accepted Controller release
- [`releases/plugin-session-root-observations-2026-10-03.md`](releases/plugin-session-root-observations-2026-10-03.md) — Machine-owned Session read roots — October 3
- [`releases/plugin-session-snapshot-placement-2026-10-03.md`](releases/plugin-session-snapshot-placement-2026-10-03.md) — Session worker snapshot placement fence — 2026-10-03
- [`releases/plugin-workspace-alias-identity-2026-10-03.md`](releases/plugin-workspace-alias-identity-2026-10-03.md) — Workspace aliases share the Machine root identity — October 3
- [`releases/plugin-workspace-configuration-identity-2026-09-28.md`](releases/plugin-workspace-configuration-identity-2026-09-28.md) — Workspace configuration identity candidate — 2026-09-28
- [`releases/plugin-workspace-read-scopes-2026-09-19.md`](releases/plugin-workspace-read-scopes-2026-09-19.md) — Continuous Workspace reads — Controller release, 2026-09-19
- [`releases/product-datasets-and-lifecycle-2026-09-15.md`](releases/product-datasets-and-lifecycle-2026-09-15.md) — Product dataset bridge and durable lifecycle diagnostics
- [`releases/provider-auth-sync-2026-09-14.md`](releases/provider-auth-sync-2026-09-14.md) — Controller Provider-auth reconciliation release: 2026-09-14
- [`releases/remote-pr-review-2026-10-01.md`](releases/remote-pr-review-2026-10-01.md) — Remote PR review implementation — 2026-10-01
- [`releases/resolved-telemetry-ports-2026-09-14.md`](releases/resolved-telemetry-ports-2026-09-14.md) — Resolved telemetry ports — Controller release, 2026-09-14
- [`releases/review-destination-reader-2026-09-18.md`](releases/review-destination-reader-2026-09-18.md) — Review-owned destination reader — 2026-09-18
- [`releases/review-owned-consumer-2026-09-17.md`](releases/review-owned-consumer-2026-09-17.md) — Content-owned Review source delivery — 2026-09-17
- [`releases/review-owned-diff-2026-09-17.md`](releases/review-owned-diff-2026-09-17.md) — Content-owned working diff delivery — 2026-09-17
- [`releases/service-buffer-sync-2026-09-17.md`](releases/service-buffer-sync-2026-09-17.md) — Service-owned synchronization — 2026-09-17
- [`releases/service-navigation-candidate-2026-09-17.md`](releases/service-navigation-candidate-2026-09-17.md) — Service navigation continuation candidate — 2026-09-17
- [`releases/session-auto-update-2026-10-01.md`](releases/session-auto-update-2026-10-01.md) — Session automatic updates and cached transcript recovery — 2026-10-01
- [`releases/sync-budget-outcomes-2026-09-19.md`](releases/sync-budget-outcomes-2026-09-19.md) — Original-operation Budget outcomes — 2026-09-19
- [`releases/telemetry-2026-09-08.md`](releases/telemetry-2026-09-08.md) — Local-first telemetry and OTel rollout — 2026-09-08
- [`releases/telemetry-background-policy-2026-09-13.md`](releases/telemetry-background-policy-2026-09-13.md) — Managed background policy implementation: 2026-09-13
- [`releases/telemetry-background-startup-2026-09-13.md`](releases/telemetry-background-startup-2026-09-13.md) — Managed telemetry startup availability: 2026-09-13
- [`releases/telemetry-binding-resolution-2026-09-12.md`](releases/telemetry-binding-resolution-2026-09-12.md) — Service binding resolution reader-only release — 2026-09-12
- [`releases/telemetry-binding-surface-2026-09-13.md`](releases/telemetry-binding-surface-2026-09-13.md) — Ordinary telemetry binding surface acceptance — 2026-09-13
- [`releases/telemetry-binding-wire-2026-09-12.md`](releases/telemetry-binding-wire-2026-09-12.md) — Telemetry binding reader/transport rollout — 2026-09-12
- [`releases/telemetry-cold-start-floor-2026-09-13.md`](releases/telemetry-cold-start-floor-2026-09-13.md) — Managed telemetry cold-start floor: 2026-09-13
- [`releases/telemetry-connected-conformance-2026-09-13.md`](releases/telemetry-connected-conformance-2026-09-13.md) — Authenticated immutable telemetry pair acceptance: 2026-09-13
- [`releases/telemetry-machine-recovery-2026-09-12.md`](releases/telemetry-machine-recovery-2026-09-12.md) — Machine binding recovery reader-only release — 2026-09-12
- [`releases/telemetry-machine-recovery-surface-2026-09-13.md`](releases/telemetry-machine-recovery-surface-2026-09-13.md) — Machine recovery surface acceptance — 2026-09-13
- [`releases/telemetry-managed-attempts-2026-09-12.md`](releases/telemetry-managed-attempts-2026-09-12.md) — Managed single-attempt telemetry rollout — 2026-09-12
- [`releases/telemetry-managed-delivery-conformance-2026-09-13.md`](releases/telemetry-managed-delivery-conformance-2026-09-13.md) — Immutable managed OTLP delivery acceptance: 2026-09-13
- [`releases/telemetry-policy-preflight-2026-09-14.md`](releases/telemetry-policy-preflight-2026-09-14.md) — Controller telemetry policy preflight release: 2026-09-14
- [`releases/telemetry-reader-floors-2026-09-13.md`](releases/telemetry-reader-floors-2026-09-13.md) — Populated telemetry reader floors — 2026-09-13
- [`releases/telemetry-recovery-audit-discovery-2026-09-13.md`](releases/telemetry-recovery-audit-discovery-2026-09-13.md) — Durable recovery audit discovery: Hawk acceptance
- [`releases/telemetry-resolution-surface-2026-09-13.md`](releases/telemetry-resolution-surface-2026-09-13.md) — Core telemetry resolution surface — 2026-09-13
- [`releases/telemetry-victoria-conformance-2026-09-14.md`](releases/telemetry-victoria-conformance-2026-09-14.md) — Real Victoria database conformance: 2026-09-14
- [`releases/telemetry-writer-admission-2026-09-13.md`](releases/telemetry-writer-admission-2026-09-13.md) — Per-target writer admission implementation: 2026-09-13
- [`releases/telemetry-writer-conformance-2026-09-13.md`](releases/telemetry-writer-conformance-2026-09-13.md) — Immutable telemetry writer-policy acceptance: 2026-09-13
- [`releases/workspace-extensions-2026-09-29.md`](releases/workspace-extensions-2026-09-29.md) — Workspace extensions — 2026-09-29
- [`releases/zed-budget-rollout-2026-09-19.md`](releases/zed-budget-rollout-2026-09-19.md) — Zed Budget rollout — 2026-09-19
- [`releases/zed-remote-edit-install-2026-09-19.md`](releases/zed-remote-edit-install-2026-09-19.md) — Hawk Zed remote-edit admission installation — 2026-09-19
- [`releases/zed-snapshot-install-2026-09-19.md`](releases/zed-snapshot-install-2026-09-19.md) — Hawk Zed snapshot-lifetime installation — 2026-09-19
- [`releases/zed-open-target-admission-2026-11-06.md`](releases/zed-open-target-admission-2026-11-06.md) — Zed 1.24.0 open admission confined to the target, publication and Hawk installation
- [`releases/zed-pulled-versions-2026-11-06.md`](releases/zed-pulled-versions-2026-11-06.md) — Zed 1.25.0 pulled diagnostics attributed to the version pulled, publication and Hawk installation
- [`releases/zed-initial-version-2026-11-06.md`](releases/zed-initial-version-2026-11-06.md) — Zed 1.26.0 nonzero initial LSP versions, exact native ownership and production document-current evidence
- [`releases/review-call-failures-2026-11-06.md`](releases/review-call-failures-2026-11-06.md) — Retained review failure audit and exact Fable continuation configuration equivalence
- [`releases/worker-pin-code-routing-2026-11-06.md`](releases/worker-pin-code-routing-2026-11-06.md) — Worker pin advance to bc6efdbea so idle code worktrees follow new Zed generations; Hawk Machine maintenance
- [`releases/zed-open-recovery-2026-11-06.md`](releases/zed-open-recovery-2026-11-06.md) — Zed 1.23.0 same-process native open recovery publication and Hawk installation
- [`releases/zed-analysis-frontier-2026-11-05.md`](releases/zed-analysis-frontier-2026-11-05.md) — Zed 1.22.0 analysis-frontier evidence publication and Hawk installation
- [`releases/zed-trust-revocation-2026-11-05.md`](releases/zed-trust-revocation-2026-11-05.md) — Zed 1.21.0 native trust revocation publication and Hawk installation

### Integrations

- [`cardea-operations.md`](cardea-operations.md) — Human-reviewed exact Plugin
  installation through the shared Cardea CLI/SDK and Cowboy-owned authority
- [`cardea-device-login.md`](cardea-device-login.md) — Native Cardea broker login,
  silent credential renewal, independent device keys, and revocation boundaries
- [`product-cli.md`](product-cli.md) — One-year device login and the `session`,
  `api` and `send` commands that reach every Web UI capability from the CLI
- [`matrix-memory.md`](matrix-memory.md) — Explicit Matrix enrollment, shared recall/capture, and native memory cutover
- [`releases/matrix-codeact-2026-10-05.md`](releases/matrix-codeact-2026-10-05.md) — TS/JS CodeAct, native and Remote acceptance, signed Claude 3.4.7 on OVH, and feedback limits
- [`releases/matrix-memory-2026-10-03.md`](releases/matrix-memory-2026-10-03.md) — Matrix 0.1.1 on OVH, signed Codex/Claude installs, cross-Provider learning, and acceptance limits
- [`releases/usage-execution-2026-10-01.md`](releases/usage-execution-2026-10-01.md) — Account usage placement release, production pins and remaining OVH Anthropic timeout
- [`releases/anthropic-usage-timeout-2026-10-01.md`](releases/anthropic-usage-timeout-2026-10-01.md) — OVH preparation root cause, host-only repair and three real account queries
- [`releases/matrix-workspaces-2026-10-01.md`](releases/matrix-workspaces-2026-10-01.md) — Hierarchical picker, mapped remote tasks, latency samples and accepted releases

- [`usage-execution.md`](usage-execution.md) — Shared quota budgets, per-account cache and loading, durable Machine selection, and Provider retirement

- [`releases/message-retry-retention-2026-10-01.md`](releases/message-retry-retention-2026-10-01.md) — Premature delivery confirmation repair, retry retention and OVH Provider diagnostics
- [`releases/ovh-trusted-ssh-2026-10-01.md`](releases/ovh-trusted-ssh-2026-10-01.md) — Normal-account OVH SSH authority, Luna development tasks and Grok quota blocker
- [`releases/ovh-hawk-ssh-development-2026-10-01.md`](releases/ovh-hawk-ssh-development-2026-10-01.md) — Earlier isolated development-shell feature and storage acceptance
- [`releases/ovh-cli-bootstrap-2026-09-29.md`](releases/ovh-cli-bootstrap-2026-09-29.md) — Remote Machine CLI correction and incomplete permanent OVH enrollment
- [`releases/ovh-luna-acceptance-2026-09-30.md`](releases/ovh-luna-acceptance-2026-09-30.md) — Real OVH Codex/Luna cases, installation receipt and failed network baseline
- [`releases/ovh-hawk-ssh-repair-2026-09-30.md`](releases/ovh-hawk-ssh-repair-2026-09-30.md) — OVH Luna diagnoses, edits, tests and commits an isolated Hawk project through SSH
- [`releases/matrix-relay-2026-11-01.md`](releases/matrix-relay-2026-11-01.md) — Matrix on every Machine and Agent over the Machine link, with outage acceptance.
- [`integrations/zed.md`](integrations/zed.md) — Optional stdio ACP bridge for Zed External Agents
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Optional isolated Zed-backed code-intelligence adapter
