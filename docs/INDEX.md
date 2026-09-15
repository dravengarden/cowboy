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
- [`requirements.md`](requirements.md) — Normative Provider package, authentication, installation, and ownership contract
- [`plugin-spatiotemporal-design.md`](plugin-spatiotemporal-design.md) — Target architecture: fixed core, typed components and Plugin composition across Service/Machine; first read-only structural slice implemented
- [`plugin-composition-checker.md`](plugin-composition-checker.md) — Implemented core diagnostic, generated Rust/TS contracts, graph rules and remaining authority/recovery work
- [`plugin-installation-incarnations.md`](plugin-installation-incarnations.md) — Durable installation revisions, uninstall CAS, tombstones and reader-first rollout
- [`plugin-recovery-assessment.md`](plugin-recovery-assessment.md) — Read-only recovery assessment across Service evidence, Machine receipts and current installation tombstones
- [`plugin-execution-leases.md`](plugin-execution-leases.md) — Connection-bound, process-monotonic admission for journaled Plugin effects
- [`plugin-service-authorization.md`](plugin-service-authorization.md) — Current confirming-credential and Operator checks at Service effect boundaries
- [`unattended-release-adoption.md`](unattended-release-adoption.md) — Controller-owned Catalog observation, bounded fallback and shutdown drain; unattended installation intent and host policy remain designs
- [`plugin-no-effect-resolution.md`](plugin-no-effect-resolution.md) — Independently confirmed, atomic resolution of proven pre-effect uninstall interruptions
- [`telemetry-binding-resolution.md`](telemetry-binding-resolution.md) — Independently authorized Service binding resolution, schema-two audit reader and closed production admission
- [`telemetry-machine-recovery.md`](telemetry-machine-recovery.md) — Independently authorized reopened Prepared closure, protocol 17, atomic Machine audit and retained Service fence
- [`core-security-client-boundary.md`](core-security-client-boundary.md) — Core-owned local authentication UI and typed native Passkey port; storage/SDK migration remains separate
- [`core-security-storage-bridge.md`](core-security-storage-bridge.md) — Typed core Passkey storage, atomic legacy import and independent ceremony lifetimes; ownership cutover remains separate

## Documents

### Architecture

- [`architecture/00-overview.md`](architecture/00-overview.md) — Current system topology and component map
- [`architecture/01-acp-transport.md`](architecture/01-acp-transport.md) — ACP session lifecycle, streaming, permissions, and cancellation
- [`claude-autonomous-activity.md`](claude-autonomous-activity.md) — Native execution state and loading feedback after a background task resumes Claude
- [`architecture/02-core-hub.md`](architecture/02-core-hub.md) — Authoritative session state, ordering, queues, and fan-out
- [`architecture/03-supervisor.md`](architecture/03-supervisor.md) — Machine routing, detached-worker lifetime, and restart recovery
- [`architecture/04-providers.md`](architecture/04-providers.md) — Provider packages, launch generations, behavior, and compatibility fallback
- [`architecture/05-storage.md`](architecture/05-storage.md) — PostgreSQL/SQLite store, write-behind, pagination, and retention
- [`architecture/06-server-api.md`](architecture/06-server-api.md) — REST, WebSocket, runtime SPA files, and session reload
- [`architecture/08-memory.md`](architecture/08-memory.md) — Agent-owned memory and provider-state boundaries
- [`architecture/09-frontend.md`](architecture/09-frontend.md) — React state, durable draft delivery, transcript, composer, PWA, and native-shell contracts
- [`usage-refresh-http-errors.md`](usage-refresh-http-errors.md) — Session account-usage identity and actionable HTTP failures
- [`architecture/10-deploy-build.md`](architecture/10-deploy-build.md) — Pinned builds and component-scoped releases
- [`architecture/11-operations.md`](architecture/11-operations.md) — Capacity, store backup and cutover, observability, and incident policy
- [`architecture/12-rolling-updates.md`](architecture/12-rolling-updates.md) — Fencing, replay, draining, readiness, and rollback
- [`architecture/13-code-review.md`](architecture/13-code-review.md) — Stable worktree, Git, file, diff, and language-intelligence API
- [`architecture/14-admin.md`](architecture/14-admin.md) — Admin console and product registration
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Isolated Zed code-provider integration
- [`architecture/15-multi-machine.md`](architecture/15-multi-machine.md) — Enrollment, outbound connectivity, placement, and Machine lifecycle
- [`architecture/16-product-auth.md`](architecture/16-product-auth.md) — Feature-gated product login, self-host identity, and session-deadline enforcement
- [`architecture/17-authentication-plugins.md`](architecture/17-authentication-plugins.md) — Signed authentication packages, configurable login UI, and server-owned session policy
- [`architecture/18-auth-capacity-sso.md`](architecture/18-auth-capacity-sso.md) — Configurable client/session capacity, fair admission, SSO logout, and isolated automation
- [`../examples/authentication/README.md`](../examples/authentication/README.md) — Google, Apple, and Cloudflare Email Authentication Provider examples
- [`architecture/runtime-incident-ledger.md`](architecture/runtime-incident-ledger.md) — Runtime failure evidence and resolved invariants

### Core documents

- [`requirements.md`](requirements.md) — Cowboy core requirements, state ownership, and context-preserving Provider reload
- [`plugin-components.md`](plugin-components.md) — Plugin manifests, shared component ownership, and coordinated versioning
- [`product-sync-datasets.md`](product-sync-datasets.md) — Immutable Service/user browser datasets, version-fenced outboxes, explicit legacy recovery and Controller/Web rollout
- [`releases/dataset-bound-maintenance-2026-09-15.md`](releases/dataset-bound-maintenance-2026-09-15.md) — Activated dataset-bound Controller/Web, compatible cold floor, independent Machine maintenance and scoped worker evidence
- [`plugin-lifecycle-history.md`](plugin-lifecycle-history.md) — Bounded typed core install/uninstall history and independent resolution, with no replay authority
- [`releases/plugin-catalog-observer-2026-09-15.md`](releases/plugin-catalog-observer-2026-09-15.md) — Owned Catalog observation and verified Controller activation; actual candidate/predecessor/cold readers agree on 69 signed releases
- [`plugin-service-sites.md`](plugin-service-sites.md) — Core-established Service identity and final Site checks for finite installation, telemetry and recovery transports
- [`plugin-code-read-scopes.md`](plugin-code-read-scopes.md) — Session-scoped responses, connection-bound Zed operations, remaining buffer ownership gaps, bounded diff/file continuations and page ETags
- [`plugin-native-buffer-leases.md`](plugin-native-buffer-leases.md) — Prepared native buffer references and exact Machine runtime retention; HTTP/Web integration and production maintenance remain separate
- [`plugin-controller-buffer-owners.md`](plugin-controller-buffer-owners.md) — Product-owned Controller references, bounded admitted continuations and path-free original-owner cleanup; Review/native rollout remains separate
- [`releases/plugin-controller-buffer-owners-2026-09-15.md`](releases/plugin-controller-buffer-owners-2026-09-15.md) — Verified Controller candidate and 20 new tests; historical publication blocker and later running-source follow-up
- [`releases/agent-publication-2026-09-15.md`](releases/agent-publication-2026-09-15.md) — Six independent signed Agent publications, Linux/Mac gates, four actual Catalog readers and automatic adoption without component restart or Plugin installation
- [`releases/claude-plan-usage-2026-09-16.md`](releases/claude-plan-usage-2026-09-16.md) — Native Anthropic plan usage in Claude Code 3.1.24, verified publication and Controller activation; Hawk Plugin upgrade remains pending
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
- [`ios-simulator.md`](ios-simulator.md) — Local iOS Simulator bridge and verification workflow
- [`machine-operations.md`](machine-operations.md) — Machine operations, including Provider installation and Service-auth replica convergence
- [`plugin-packages.md`](plugin-packages.md) — Package, typed UI, authentication/Transcript presentation, and release contract for independently released, Machine-scoped Provider packages
- [`provider-auth-sync-coordination.md`](provider-auth-sync-coordination.md) — Bounded same-generation reconciliation on one authenticated connection, cancellation and late-receipt fencing

### Integrations

- [`integrations/zed.md`](integrations/zed.md) — Optional stdio ACP bridge for Zed External Agents
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Optional isolated Zed-backed code-intelligence adapter
