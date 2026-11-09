# Widget apps: agent-written TSX in a bounded interpreter, MCP Apps later

Status: design, revision 3 (2026-11-09). The owner chose the direction:
Cowboy provides a secure sandbox in which agent-written React runs, and
widgets grow into an advanced, extensible surface. Revision 3 answers the
Astra Max review of revision 2 (§12). It extends [`widgets.md`](widgets.md);
the `cowboy.widget/1` JSON nodes stay the lightweight, patchable path for
status, tables and decisions.

## 1. Goals and non-goals

Goals:

- An agent can publish an **app**: a React component in TSX that Cowboy
  compiles and runs, drawn inside a widget row with Cowboy's own components on
  iPhone, iPad and Desktop.
- An app can never read or act as the person: no Cowboy cookies, storage, DOM,
  credentials, native bridges or network. Text the person types into an app,
  and the data the agent gave it, leave the device only inside a message the
  person confirms.
- An app's CPU and memory are bounded by the runtime itself, so a hostile app
  is stopped without exhausting the page, on every engine where apps are
  enabled.
- The host-side bridge follows **MCP Apps** (SEP-1865, final 2026-01-26)
  method semantics, so phase C can render `ui://` resources from MCP servers.

Non-goals for phase A: arbitrary DOM, canvas or WebGL (phase B), opening links
from apps, tool calls (§11), third-party MCP UIs (phase C), persistent per-app
storage.

## 2. What the community does (2026-11)

- **MCP Apps** ([SEP-1865](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp),
  [spec 2026-01-26](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)):
  HTML views in sandboxed iframes behind a different-origin sandbox proxy,
  JSON-RPC over `postMessage`, deny-all default CSP, optional consent for
  `ui/message` and `tools/call`, host-marked boundaries, CPU and memory limits.
- **MCP-UI / Shopify Remote DOM** ([MCP-UI](https://github.com/MCP-UI-Org/mcp-ui),
  [Shopify](https://shopify.engineering/mcp-ui-breaking-the-text-wall)): remote
  code builds a component tree; the host renders it with its own components.
- **Figma plugins**: plugin code runs in a QuickJS realm compiled to
  WebAssembly with only host-provided APIs; UI that needs the DOM goes to a
  separate iframe. [`quickjs-emscripten`](https://github.com/justjake/quickjs-emscripten)
  0.32 (MIT) provides `setMemoryLimit`, `setMaxStackSize` and interrupt
  deadlines; its README notes it is not audited.
- **Claude Artifacts** ([analysis](https://pluto.security/blog/inside-claude-artifacts-how-your-agent-publishes-to-the-web/)):
  per-artifact origins on a separate registrable domain, strict CSP, frozen
  library allowlist.
- **LLM-written TSX in the browser**
  ([component-playground](https://github.com/luisrrv/component-playground),
  [Renderify](https://github.com/webllm/renderify)): Sucrase compile,
  allowlisted import resolver, sandboxed frame, terminable boundary.

## 3. Architecture (phase A)

```
agent ──widgets.publish──▶ Controller: validate bounds, verify images, store, project row
Cowboy Web (main thread)
  AppNode (host): one runtime per app identity; renders the component tree with
  Cowboy MUI after budget checks; consent broker; lifecycle; watchdog
    │ readiness handshake, then one MessagePort (credit-limited, bounded frames)
    ▼
  srcdoc <iframe sandbox="allow-scripts">  opaque origin; trusted boot script only
    │ creates and can terminate
    ▼
  Runtime Worker (trusted Cowboy code): relay + budgets + QuickJS host
    │ evalCode with memory limit, stack limit, interrupt deadline
    ▼
  QuickJS VM (untrusted): Sucrase compile of the app, React + Cowboy reconciler,
  the agent's component; only host functions: emit(ops), send(message), log()
```

Layers, from the agent's code outward, each sufficient on its own for a
different property:

1. **QuickJS VM**: no browser APIs exist (no `fetch`, Worker, WebRTC, timers
   beyond those the host provides, DOM, storage, `postMessage`); memory and
   stack are capped; every evaluation and event dispatch runs under an
   interrupt deadline. This bounds CPU and memory and removes all egress
   except the three host functions.
2. **Runtime Worker**: trusted code. It validates and size-checks every value
   crossing the host functions before building the message it posts, so the
   main thread only ever receives bounded, schema-valid frames, and it never
   forwards anything without credit from the host. Its prelude deletes
   `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, `Worker`,
   `SharedWorker`, `importScripts` and `indexedDB` from its own global, as
   defense in depth should the VM be escaped.
3. **srcdoc sandbox frame**: opaque origin (no cookies, storage, same-origin
   APIs), inherited App CSP plus a stricter meta CSP, no request for a sandbox
   document (no cookie is sent, nothing to navigate to or substitute), no
   access to native script-message handlers that check the main frame (§9).
4. **Host renderer**: draws only allowlisted components with validated props
   and enforced expansion budgets (§6).

Phase B adds a full-DOM tier on a separate registrable domain; phase C renders
third-party MCP Apps through a spec-conformant proxy there (§10).

## 4. Isolation details

### 4.1 Frame and boot

- `<iframe sandbox="allow-scripts" srcdoc=… allow="">`: no `allow-same-origin`,
  forms, popups, top navigation, modals, downloads or permissions.
- The boot script is a fixed inline body copied byte-identically into the
  served `index.html` at build time, so the App CSP's hash list authorizes it
  (`web_security.rs` hashes every inline body; the editor plugin boot uses the
  same mechanism, `web/vite.config.ts`). App source never appears in markup;
  it travels only as message data.
- Effective policy in the frame and the Worker: the App CSP intersected with
  `default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:`.
  The Worker starts from a `blob:` built from the static, content-hashed
  runtime bundle shipped with the Web release; QuickJS instantiation uses the
  App CSP's `'wasm-unsafe-eval'`.

### 4.2 Handshake and channel

1. The host creates the frame with a fresh generation number and waits for
   `cowboy-app-sandbox-ready` whose `event.source` is that frame's
   `contentWindow` (as the editor transport does); 5 s timeout.
2. It then posts one bootstrap carrying a per-load nonce and a transferred
   `MessagePort`. The boot script accepts only the first bootstrap whose
   `event.source` is `window.parent`, echoes the nonce on the port and starts
   the Worker. Later bootstraps are ignored.
3. The port is bound to the app identity (§5.3) and the frame generation.
   Messages on any other channel are ignored.
4. Revocation (close the port, terminate the Worker, cancel any open
   confirmation): navigation or replacement of the frame, Clear, sign-out,
   session close or trash, teardown, and **widget closure**. A closed widget
   never starts a runtime; it shows its last rendered tree inert, or its
   `fallback`.

### 4.3 Egress table

| Channel | Phase A |
|---|---|
| Cookies, storage, Cache API, IndexedDB | Not in QuickJS; opaque origin in the frame; deleted in the Worker |
| `fetch`, XHR, WebSocket, EventSource, `importScripts`, `eval` of strings by the Worker | Not in QuickJS; deleted in the Worker; denied by the inherited CSP |
| Workers, SharedWorkers | Not in QuickJS; constructors deleted in the Worker |
| WebRTC, navigation, popups, forms, downloads | Not in QuickJS or Workers; the frame runs only trusted boot code |
| Images, links, rich Markdown | No component fetches, links or renders rich Markdown (§6) |
| Native handlers | Not reachable from QuickJS or Workers; §9 gate requires main-frame checks |
| Host functions | `emit` (bounded tree ops), `send` (a message the person must confirm), `log` (bounded) |

## 5. Runtime

### 5.1 Execution budgets

- Per app VM: memory 32 MiB, stack 512 KiB.
- Compile (Sucrase inside the VM): deadline 1.5 s; failure shows an error card.
- First render: 500 ms deadline; each later event dispatch or state update:
  100 ms. An interrupted evaluation terminates the app ("This app stopped:
  it ran too long" or "used too much memory") with Reload.
- The host still pings the Worker every second; three misses terminate it
  (covers a defect in the trusted runtime itself).
- At most 4 live app runtimes per session view; at most 1 app per document in
  phase A.

### 5.2 Rendering protocol

- Inside the VM, React 19 runs with a small Cowboy reconciler that emits
  operations: `create(id, type)`, `props(id, props)`, `text(id, value)`,
  `insert(parent, id, index)`, `remove(id)`. The Worker validates each batch:
  types in the allowlist, props schema-valid and JSON-only, ids known, tree
  ≤ 2,000 nodes and depth ≤ 32, batch ≤ 64 KiB, ≤ 30 batches per second; the
  host grants credits per animation frame. Violations terminate the app.
- Events: the host sends sanitized payloads (`value`, `checked`, `index`,
  `key`) for the handler a node declared; the Worker dispatches them into the
  VM under the event deadline.

### 5.3 Identity and updates

- App identity: (session, widget, app node id, SHA-256 of `code`).
- A document revision that changes only `props` (or other nodes) delivers the
  new props to the running app as an update (MCP Apps `tool-input` semantics);
  the runtime keeps its state.
- A revision that changes `code`: if the app holds person-entered input since
  mount, the host keeps the current runtime and shows "The agent updated this
  app — Reload (discard what you typed) / Keep editing"; otherwise it restarts.
- The inline row and the inspector render the same tree from one runtime; the
  inline row shows a placeholder while the inspector has it.
- Off-screen eviction after 60 s, except pinned apps (focused, holding input,
  or awaiting a confirmation).

## 6. Components and their budgets

`@cowboy/ui` is the only import besides `react`, `date-fns` and `lodash-es`
(pure modules, bundled); any other specifier fails to compile with the
allowlist in the message. The host renders each with Cowboy MUI, themed, with
≥ 44 px touch targets on touch surfaces, after checking these budgets:

| Component | Budget enforced by the host |
|---|---|
| `Stack`, `Box`, `Grid`, `Card`, `Divider` | layout props from a closed set |
| `Text`, `Heading` | plain text, ≤ 4,000 chars per node, ≤ 40,000 per app; no Markdown, links or HTML |
| `Chip`, `Badge`, `Alert`, `Progress`, `Tooltip` | label ≤ 200 chars |
| `Button`, `IconButton` | allowlisted icon names; `onPress` only |
| `TextField`, `Select`, `Checkbox`, `Switch`, `Slider`, `Tabs` | value ≤ 8,000 chars; options ≤ 100 |
| `Table` | ≤ 200 rows × 12 columns, cell text ≤ 500 chars; host sort |
| `List` | ≤ 500 items |
| `Chart` (line, bar, area, pie, scatter) | ≤ 2,000 points per app, ≤ 8 series; host-rendered with Recharts |
| `Image` | `source` names an entry of the node's `images`, which the Controller verified and pinned like widget images; no URLs |

Aggregate per app: ≤ 2,000 rendered components and the cost model above;
updates exceeding it are rejected before commit. Hooks: `useHostContext()`
(theme mode, density, platform, locale, container size) and
`useSend()` → `send(text)`.

## 7. Bridge and consent

| MCP Apps request | Phase A |
|---|---|
| `ui/initialize` | Host context: theme, density, platform, locale, time zone, container dimensions, display mode |
| `ui/notifications/size-changed` | Coalesced per animation frame within bounds |
| `ui/message` (`send`) | Text only, ≤ 2,000 Unicode scalars and ≤ 8 KiB; C0/C1 controls except newline rejected; bidi and invisible format characters shown escaped (`⟨U+202E⟩`). The product confirm sheet freezes and shows the exact bytes, labelled "Written by the app". On Send, the Web calls the Controller, which re-checks that the widget is open, the app node and code hash are current and the session admits prompts, then queues exactly those bytes as the person's prompt. Each opened confirmation has a host-owned occurrence id; the `cmid` is derived from (session, widget, node, code hash, occurrence) and reused only for retries of that confirmed payload. One open confirmation per app; at most 1 new confirmation per 5 s |
| `ui/request-display-mode` | `inline` ↔ `fullscreen` (the inspector), only after a person's gesture in the app |
| `ui/open-link` | Not available in phase A: opening a URL would export data outside the message rule. A later phase may add it as a separate, explicitly worded data-export permission |
| `ui/update-model-context`, `tools/call`, `resources/read` | Refused |
| `notifications/message` (`log`) | ≤ 20 per second, ≤ 1 KiB each, ≤ 100 kept |

## 8. Contract and Controller

- New node `{type:"app", id, title, code, props?, images?, height?, fallback}`.
  `code` is one TSX module with a default-exported component; `props` is
  opaque JSON passed to it; `images` maps keys to image sources, verified and
  pinned like other widget images.
- Bounds fit the existing transport (32 KiB serialized tool arguments,
  24 KiB document, 48 KiB snapshot): `code` ≤ 14 KiB, `props` ≤ 6 KiB, both
  measured as serialized JSON; one app node per document.
- `props` is opaque: patch `#id` lookup, image walks, input walks and the text
  projection follow schema edges and never enter `props`; `widgets.update`
  reaches inside `props` only by a JSON Pointer through the app node. Fixtures
  cover colliding ids and image-shaped objects inside `props`.
- App messages use a new Controller endpoint
  `POST /api/sessions/{id}/widgets/{widget_id}/apps/{node_id}/messages`
  (human-only, like decisions) that performs the checks of §7.
- The Controller validates shape and bounds; it never parses or runs TSX.
  Inline fences may not contain app nodes.

## 9. Acceptance and release gates

Apps are enabled per engine and shell only after its suite passes; any other
client shows `fallback` with "Open in the Cowboy app or on Desktop".

The `widget-apps` suite, on every enabled engine (hawk Chrome; the iOS/iPadOS
native shell on a physical device per `conventions/pwa.md`; the macOS native
shell; each Tauri runtime that ships):

- Positive: a TSX app with `@cowboy/ui`, `Chart`, `Table` and `date-fns`
  compiles under the production CSP and renders at 320/375/768/1100 px, light
  and dark; events round-trip; props-only updates keep typed input; the
  inspector shares one runtime.
- Hostile, observed outside the app (sentinel cookie and storage, CDP Network
  log of every request the page makes, Controller request log): no sentinel
  visible; no request leaves the page; no Worker can be spawned; forged
  readiness, bootstrap or port messages are ignored; unknown components and
  props, oversized batches and floods terminate the app; a compile-time loop,
  a render-time loop and an allocation loop each stop within their budgets
  while Cowboy stays responsive and keeps unsent composer text.
- Consent: `send` never queues without the confirm; bidi text shows escaped;
  two confirmed identical messages queue twice, a retried one once; a closed
  widget refuses send at the Controller.

Native gate: every privileged handler (WKWebView script message handlers,
including the passkey bridge, which must add an `isMainFrame` check; Tauri
capabilities) rejects calls from same-origin and opaque subframes, verified by
direct calls without the injected shims on each shipped shell. This also
hardens the existing editor plugin sandbox.

## 10. Phases

- **A — Interpreter apps** (no network or DNS change): §3–§9.
- **B — Full-DOM apps on a separate site**: a dedicated registrable domain
  with one subdomain per app, a cookie-free static endpoint without redirects,
  `frame-ancestors` limited to Cowboy, enabled per engine only where the frame
  is process-isolated and terminable, with the same consent broker.
- **C — Third-party MCP Apps**: a spec-conformant sandbox proxy on the phase B
  domain preserving the standard View-facing `postMessage` transport,
  initialization, capability negotiation, notification ordering and teardown;
  declared CSP domains shown to the person; tools per §11. Compatibility is
  claimed only after an unmodified MCP Apps client fixture passes.

## 11. Decisions (owner delegated, 2026-11-09)

1. Phase B domain: a separate registrable domain for app content (working name
   `stormbirdusercontent.xyz`), bought by the owner when phase B starts.
2. Libraries: phase A as in §6; Three.js and canvas come with phase B; map
   libraries are excluded (network tiles).
3. Tools: none in phase A; later only Cowboy read-only reads with per-widget
   consent naming the data; write tools never.

## 12. Review record

Revision 1 (`call-2b49e88dbf282591a7768b54`, incomplete) led to revision 2:
agent code moved off the DOM into a Worker with host-rendered components; see
revision 2 in Git history for that table.

Revision 2 (Astra Max, `call-d198afcfbf3b06d33a5fe6e9`) — resolutions:

| Finding | Resolution |
|---|---|
| App Markdown reuses the product renderer: authenticated image fetch, unconfirmed links (high) | No Markdown, links or URL images in apps; `Image` takes pre-verified keys (§6) |
| Message limits are not a heap limit (high) | QuickJS VM with a memory limit and interrupt deadlines (§3, §5.1) |
| Apps can spawn untracked Workers (high) | No Worker API in QuickJS; constructors deleted in the Worker (§3, §4.3) |
| Raw bridge traffic can stall the main thread (high) | Trusted runtime Worker validates and bounds frames; host credits (§3, §5.2) |
| Component expansion and rich rendering are unbudgeted (high) | Per-component and aggregate budgets enforced before commit; no rich fences (§6) |
| Compilation not terminable (medium) | Sucrase runs inside the VM under a deadline (§5.1) |
| Boot script and production CSP (medium) | Build-time byte-identical inline body hashed by the App CSP (§4.1) |
| Port sent before frame readiness (medium) | Source-checked readiness handshake with generation and timeout (§4.2) |
| Widget closure does not revoke (medium) | Closure revokes; closed widgets never start; Controller rechecks on send (§4.2, §7) |
| Unrelated updates destroy dirty input (medium) | Identity by code hash; props-only updates keep state; code changes ask (§5.3) |
| `openLink` exports data (medium) | Removed from phase A (§7) |
| Coverage of engines and egress observation (medium) | Per-engine enablement, CDP network log, passkey `isMainFrame` gate (§9) |
