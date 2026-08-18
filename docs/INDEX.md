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

## Documents

### Architecture

- [`architecture/00-overview.md`](architecture/00-overview.md) — Current system topology and component map
- [`architecture/01-acp-transport.md`](architecture/01-acp-transport.md) — ACP session lifecycle, streaming, permissions, and cancellation
- [`architecture/02-core-hub.md`](architecture/02-core-hub.md) — Authoritative session state, ordering, queues, and fan-out
- [`architecture/03-supervisor.md`](architecture/03-supervisor.md) — Machine routing, detached-worker lifetime, and restart recovery
- [`architecture/04-providers.md`](architecture/04-providers.md) — Provider packages, launch generations, behavior, and compatibility fallback
- [`architecture/05-storage.md`](architecture/05-storage.md) — PostgreSQL/SQLite store, write-behind, pagination, and retention
- [`architecture/06-server-api.md`](architecture/06-server-api.md) — REST, WebSocket, runtime SPA files, and session reload
- [`architecture/08-memory.md`](architecture/08-memory.md) — Agent-owned memory and provider-state boundaries
- [`architecture/09-frontend.md`](architecture/09-frontend.md) — React state, transcript, composer, PWA, and native-shell contracts
- [`architecture/10-deploy-build.md`](architecture/10-deploy-build.md) — Pinned builds and component-scoped releases
- [`architecture/11-operations.md`](architecture/11-operations.md) — Capacity, migrations, observability, and incident policy
- [`architecture/12-rolling-updates.md`](architecture/12-rolling-updates.md) — Fencing, replay, draining, readiness, and rollback
- [`architecture/13-code-review.md`](architecture/13-code-review.md) — Stable worktree, Git, file, diff, and language-intelligence API
- [`architecture/14-admin.md`](architecture/14-admin.md) — Admin console and product registration
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Isolated Zed code-provider integration
- [`architecture/15-multi-machine.md`](architecture/15-multi-machine.md) — Enrollment, outbound connectivity, placement, and Machine lifecycle
- [`architecture/16-product-auth.md`](architecture/16-product-auth.md) — Product login and self-host identity
- [`architecture/runtime-incident-ledger.md`](architecture/runtime-incident-ledger.md) — Runtime failure evidence and resolved invariants

### Core documents

- [`requirements.md`](requirements.md) — Cowboy core requirements and state-ownership contract
- [`desktop-efficiency-redesign.md`](desktop-efficiency-redesign.md) — Desktop information density and interaction contract
- [`explore-transcript-design.md`](explore-transcript-design.md) — Explore's read-only transcript projection
- [`mobile-spatial-presentation.md`](mobile-spatial-presentation.md) — Jank-free drawers, pager, transcript, CodeMirror, and iOS compositor contract
- [`ios-simulator.md`](ios-simulator.md) — Local iOS Simulator bridge and verification workflow
- [`machine-operations.md`](machine-operations.md) — Machine operations, including Provider installation and Service-auth replica convergence
- [`provider-packages.md`](provider-packages.md) — Package, typed UI, authentication/Transcript presentation, and release contract for independently released, Machine-scoped Provider packages

### Integrations

- [`integrations/zed.md`](integrations/zed.md) — Optional stdio ACP bridge for Zed External Agents
- [`architecture/14-zed-code-provider.md`](architecture/14-zed-code-provider.md) — Optional isolated Zed-backed code-intelligence adapter
