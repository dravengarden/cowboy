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

- [`architecture/00-overview.md`](architecture/00-overview.md) — Control plane, Machines, workers, storage, and client topology
- [`execution-environments.md`](execution-environments.md) — Native remote execution decision: separate runtime placement from target files/processes; core, Provider and infrastructure ownership; staged acceptance
- [`requirements.md`](requirements.md) — Normative Provider package, authentication, installation, and ownership contract
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
- [`telemetry-machine-recovery.md`](telemetry-machine-recovery.md) — Independently authorized reopened Prepared closure, protocol 17, atomic Machine audit and retained Service fence
- [`core-security-client-boundary.md`](core-security-client-boundary.md) — Core-owned local authentication UI and typed native Passkey port; storage/SDK migration remains separate
- [`core-security-storage-bridge.md`](core-security-storage-bridge.md) — Typed core Passkey storage, atomic legacy import and independent ceremony lifetimes; ownership cutover remains separate

## Documents

### Architecture

- [`cowboy-ovh-efficiency-2026-10-04.md`](cowboy-ovh-efficiency-2026-10-04.md) — Current cross-host SSH timings, real-session history, recurring usage preparation and scoped optimization order
- [`ovh-native-execution-efficiency-2026-10-04.md`](ovh-native-execution-efficiency-2026-10-04.md) — Live Codex Remote/Claude Mods bindings, reproducible RPC amplification and Controller-to-OVH optimization priorities
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
- [`architecture/13-code-review.md`](architecture/13-code-review.md) — Stable worktree, Git, file, diff, and language-intelligence API
- [`architecture/14-admin.md`](architecture/14-admin.md) — Admin console and product registration
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Isolated Zed code-provider integration
- [`architecture/15-multi-machine.md`](architecture/15-multi-machine.md) — Enrollment, outbound connectivity, placement, and Machine lifecycle
- [`architecture/16-product-auth.md`](architecture/16-product-auth.md) — Mandatory product login/device proof, self-host identity, and session-deadline enforcement
- [`architecture/17-authentication-plugins.md`](architecture/17-authentication-plugins.md) — Signed authentication packages, configurable login UI, and server-owned session policy
- [`architecture/18-auth-capacity-sso.md`](architecture/18-auth-capacity-sso.md) — Configurable client/session capacity, fair admission, SSO logout, and isolated automation
- [`../examples/authentication/README.md`](../examples/authentication/README.md) — Google, Apple, and Cloudflare Email Authentication Provider examples
- [`architecture/runtime-incident-ledger.md`](architecture/runtime-incident-ledger.md) — Runtime failure evidence and resolved invariants

### Core documents

- [`requirements.md`](requirements.md) — Cowboy core requirements, state ownership, and context-preserving Provider reload
- [`device-transport-security.md`](device-transport-security.md) — Implemented mandatory HTTPS/device binding, proxy boundary and enrollment
- [`secure-connectivity-design.md`](secure-connectivity-design.md) — Current security baseline, acceptance priorities and WireGuard roadmap
- [`wireguard-transport.md`](wireguard-transport.md) — Deferred research: Rust implementations and native interoperability evidence
- [`browser-wireguard-transport.md`](browser-wireguard-transport.md) — Deferred research: browser WASM/WSS runtime evidence and unresolved application integration
- [`plugin-components.md`](plugin-components.md) — Plugin manifests, shared component ownership, and coordinated versioning
- [`product-sync-datasets.md`](product-sync-datasets.md) — Immutable Service/user browser datasets, version-fenced outboxes, explicit legacy recovery and Controller/Web rollout
- [`releases/dataset-bound-maintenance-2026-09-15.md`](releases/dataset-bound-maintenance-2026-09-15.md) — Activated dataset-bound Controller/Web, compatible cold floor, independent Machine maintenance and scoped worker evidence
- [`plugin-lifecycle-history.md`](plugin-lifecycle-history.md) — Bounded typed core install/uninstall history and independent resolution, with no replay authority
- [`plugin-session-deletion-journal.md`](plugin-session-deletion-journal.md) — Reader-first terminal Session records, component reader floor and bounded recovery; production writer remains disabled
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
- [`plugin-refactor-completion.md`](plugin-refactor-completion.md) — Current implementation, production-acceptance and independent-recovery exit checklist
- [`desktop-efficiency-redesign.md`](desktop-efficiency-redesign.md) — Desktop information density and interaction contract
- [`explore-transcript-design.md`](explore-transcript-design.md) — Explore's read-only transcript projection
- [`mobile-spatial-presentation.md`](mobile-spatial-presentation.md) — Jank-free drawers, pager, transcript, CodeMirror, iPhone PWA status material, iPad standalone chrome, and iOS compositor contract
- [`offline-first-sync.md`](offline-first-sync.md) — Offline-first design: local replica boot, prioritized hydration, one sync status, outbox classes, conflict catalog and required server changes
- [`sessions-folders.md`](sessions-folders.md) — Sessions sidebar folders: synced tree with project binding, Mobile and Desktop UX, keyboard contract
- [`ios-simulator.md`](ios-simulator.md) — Local iOS Simulator bridge and verification workflow
- [`machine-plugin-membership.md`](machine-plugin-membership.md) — Service-side declaration of which Plugins a Machine runs, and why a declared Machine has no client lifecycle actions
- [`machine-component-convergence.md`](machine-component-convergence.md) — Continuous convergence of signed automatic Machine components: authority, drain, backoff and reported state
- [`machine-operations.md`](machine-operations.md) — Machine operations, including Provider installation and Service-auth replica convergence
- [`plugin-packages.md`](plugin-packages.md) — Package, typed UI, authentication/Transcript presentation, and release contract for independently released, Machine-scoped Provider packages
- [`provider-auth-sync-coordination.md`](provider-auth-sync-coordination.md) — Bounded same-generation reconciliation on one authenticated connection, cancellation and late-receipt fencing

### Integrations

- [`cardea-device-login.md`](cardea-device-login.md) — Native Cardea broker login,
  silent credential renewal, independent device keys, and revocation boundaries
- [`matrix-memory.md`](matrix-memory.md) — Explicit Matrix enrollment, shared recall/capture, and native memory cutover
- [`releases/matrix-memory-2026-10-03.md`](releases/matrix-memory-2026-10-03.md) — Matrix 0.1.1 on OVH, signed Codex/Claude installs, cross-Provider learning, and acceptance limits
- [`releases/usage-execution-2026-10-01.md`](releases/usage-execution-2026-10-01.md) — Account usage placement release, production pins and remaining OVH Anthropic timeout
- [`releases/anthropic-usage-timeout-2026-10-01.md`](releases/anthropic-usage-timeout-2026-10-01.md) — OVH preparation root cause, host-only repair and three real account queries
- [`releases/matrix-workspaces-2026-10-01.md`](releases/matrix-workspaces-2026-10-01.md) — Hierarchical picker, mapped remote tasks, latency samples and accepted releases

- [`usage-execution.md`](usage-execution.md) — Durable account usage Machine selection, CLI and failure behavior

- [`releases/message-retry-retention-2026-10-01.md`](releases/message-retry-retention-2026-10-01.md) — Premature delivery confirmation repair, retry retention and OVH Provider diagnostics
- [`releases/ovh-trusted-ssh-2026-10-01.md`](releases/ovh-trusted-ssh-2026-10-01.md) — Normal-account OVH SSH authority, Luna development tasks and Grok quota blocker
- [`releases/ovh-hawk-ssh-development-2026-10-01.md`](releases/ovh-hawk-ssh-development-2026-10-01.md) — Earlier isolated development-shell feature and storage acceptance
- [`releases/ovh-cli-bootstrap-2026-09-29.md`](releases/ovh-cli-bootstrap-2026-09-29.md) — Remote Machine CLI correction and incomplete permanent OVH enrollment
- [`releases/ovh-luna-acceptance-2026-09-30.md`](releases/ovh-luna-acceptance-2026-09-30.md) — Real OVH Codex/Luna cases, installation receipt and failed network baseline
- [`releases/ovh-hawk-ssh-repair-2026-09-30.md`](releases/ovh-hawk-ssh-repair-2026-09-30.md) — OVH Luna diagnoses, edits, tests and commits an isolated Hawk project through SSH
- [`integrations/zed.md`](integrations/zed.md) — Optional stdio ACP bridge for Zed External Agents
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Optional isolated Zed-backed code-intelligence adapter
