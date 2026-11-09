# Widget apps: agent-written TSX in a bounded interpreter, MCP Apps later

Status: design, revision 4 (2026-11-09). The owner chose the direction:
Cowboy provides a secure sandbox in which agent-written React runs, and
widgets grow into an advanced, extensible surface. Revision 4 answers the
Astra Max reviews of revisions 2 and 3 (§12). It extends [`widgets.md`](widgets.md);
the `cowboy.widget/1` JSON nodes stay the lightweight, patchable path for
status, tables and decisions.

## 0. Implementation status

- **A1 (Web runtime)**: `web/src/widgets/apps/` — the srcdoc sandbox and
  hash-authorized boot (`appSandboxBoot.ts`, `appSandbox.ts`), the trusted
  Worker hosting QuickJS (`runtime/workerEntry.js`), the VM bundle with React,
  the Cowboy reconciler and Sucrase (`runtime/vmEntry.js`), budgets
  (`runtime/limits.js`, `appTree.ts`), the host renderer (`AppNode.tsx`,
  `AppComponents.tsx`, `AppChart.tsx`), built by `web/appRuntimePlugin.mjs`.
  The `widget-apps` browser suite covers rendering, round-trips and the
  hostile cases of §9 on Chrome.
- **A2 (contract)**: the `app` node in `contracts/widget-v1.ts`, the Web and
  Controller validators, schema-edge-only `#id` lookup and image walks
  (`props` stays opaque), and the authoring contract in `help("widgets")`.
- **A3 (messages)**: `POST /api/sessions/{id}/widgets/{widget_id}/apps/{node_id}/messages`
  (`agent_api/widget_app_messages.rs`) with the decision durability pattern;
  the Web preview, occurrence and confirmation (`apps/appMessages.ts`,
  `apps/AppMessageConfirm.tsx`). The Controller refuses control, bidi and
  invisible format characters instead of escaping them, so the confirmation
  always shows exactly what is sent.
- Implementation review (Astra Max, `call-75530fb67335e655127cf740`) changed
  these details of the design: an app's text is checked and confirmed on the
  device and only the confirmed send reaches the Controller (no preview
  round-trip), held in a local outbox until acknowledged; automation
  credentials cannot reach either widget endpoint; app messages are never
  retired by updates; node lookups follow edges by node type; runtimes live
  in a page registry (`apps/appRuntimes.ts`) so the inline row and the
  inspector share one app, with at most four per page, credit paced by the
  committed work and suspension of hidden pages; the Worker meters CPU over
  a rolling second, gives compiling 1.5 s and the first render 1 s, keeps
  deferred commands, rate-bounds `send`/`log` and tracks whole subtrees; the
  host enforces whole-app text and chart-point budgets and contains drawing
  failures per app; the import allowlist is `react` and `@cowboy/ui` (no
  date-fns or lodash, keeping the VM small); enablement requires desktop
  Chromium 140+ on Linux, macOS or Windows.
- Second implementation review (`call-5344903a82f61fe71acffd38`) changed:
  the registry, not each view, owns which code version runs, so every view
  agrees, a kept version keeps its own props, and sends carry the running
  code's hash; render batches commit under the 1,500-unit frame budget
  (charged by created and changed nodes with their expansions) across all
  apps, with credits returned only on commit, and a session view's apps
  share 12,000 units; a published app's runtime is keyed by (session,
  widget, node) with the exact code compared for updates; off-screen apps
  pause, untouched ones end after 1.5 s, typed-into ones survive until a
  visible app needs the room (idle apps are evicted first, then the one
  hidden longest), and Reload is admitted against the quota; the outbox is
  written before anything is sent, kept on transient refusals (401, 403,
  408, 425, 429, `context_pending`) and retried in the background under the
  same occurrence after load, reconnect and page show, whatever became of
  the widget; the confirmation cannot be dismissed while a send is in
  flight; the Controller answers a recorded occurrence before checking the
  session (only a still-unqueued send needs an admissible one); the Worker
  continues promise jobs after a throttled turn, holds them while the draw
  queue is full, recomputes the timer wake after jobs, skips timers
  cancelled earlier in the same batch, and a suspend sent during startup is
  replayed after the handshake.
- Deferred: `Image` inside apps shows its `alt` text until node `images`
  (§6) land; apps are enabled only on desktop Chromium outside the native
  shells until **A4** (native `isMainFrame` gates, the `appsSandbox`
  capability, WebKit and iPhone acceptance with the egress sink).

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
  the frame's meta policy `default-src 'none'; script-src 'unsafe-inline'
  blob: 'wasm-unsafe-eval'; worker-src blob:`. Both allow
  `'wasm-unsafe-eval'`, which QuickJS needs; neither allows string `eval` or
  network fetches. The Worker starts from a `blob:` built from the static,
  content-hashed runtime bundle shipped with the Web release; the QuickJS
  WebAssembly bytes arrive as a transferred `ArrayBuffer` in the bootstrap,
  never fetched by the Worker. Initialization under the exact production
  policies is part of the positive suite.

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
- Cumulative CPU: VM execution (evaluations, pending-job drains, scheduled
  React work) is metered per app: ≤ 200 ms in any 1 s window and ≤ 20 s per
  minute. Over the window the runtime stops draining jobs until the next
  window; three consecutive throttled windows or the minute budget terminate
  the app. Pending jobs drained per turn are bounded, and the remaining
  deadline propagates into each drain.
- The trusted Worker (not the VM) answers a heartbeat every second; three
  misses terminate it (covers a defect in the trusted runtime itself).
- Trust assumption: these budgets hold while QuickJS holds. An escape into
  Worker JavaScript defeats the VM heap and bridge budgets; what remains is
  the heartbeat termination, the opaque origin and the CSP. QuickJS is pinned,
  updated with upstream fixes, and fuzzed with hostile app fixtures in CI.
- At most 4 live app runtimes per session view; at most 1 app per document in
  phase A.

Feasibility spike (2026-11-09, Bun on Hawk, `quickjs-emscripten` 0.32,
React 19.2, `react-reconciler` 0.33, Sucrase 3.35; the VM bundle of React,
reconciler and Sucrase is 341 KB minified): loading the runtime into a fresh
VM 40–260 ms; compile and first render of a 50-row app 40 ms cold, 18 ms
warm; a 400-row table 27 ms emitting one 42 KB batch; `while(true)` stopped
by the 300 ms deadline; an allocation loop stopped by the 32 MiB limit
(`out of memory`); a non-allowlisted import fails; `fetch`, `Worker`, XHR,
WebSocket and `postMessage` are undefined in the VM. React's scheduler must
see the VM's timer shim before it loads (a separate first module), updates
use synchronous priority, and the host drains QuickJS pending jobs after each
evaluation.

### 5.2 Rendering protocol

- Inside the VM, React 19 runs with a small Cowboy reconciler that emits
  operations: `create(id, type)`, `props(id, props)`, `text(id, value)`,
  `insert(parent, id, index)`, `remove(id)`. The Worker validates each batch:
  types in the allowlist, props schema-valid and JSON-only, ids known, tree
  ≤ 2,000 nodes and depth ≤ 32, batch ≤ 64 KiB, ≤ 30 batches per second.
- Flow control: at most 2 batches in flight to the host; the Worker keeps at
  most 4 pending batches and 256 KiB, coalescing successive `props`/`text`
  operations for the same node; when the pending store is full the Worker
  stops running the VM until credits return. The host grants credits per
  animation frame and stops granting in a hidden tab, which suspends the VM.
  Violations terminate the app.
- Events: the host sends sanitized payloads (`value`, `checked`, `index`,
  `key`) for the handler a node declared; the Worker dispatches them into the
  VM under the event deadline.

### 5.3 Identity and updates

- App identity: (session, widget, app node id); the registry compares the
  exact `code` to decide whether a version is new, and sends carry the
  SHA-256 of the code that is running.
- A document revision that changes only `props` (or other nodes) delivers the
  new props to the running app as an update (MCP Apps `tool-input` semantics);
  the runtime keeps its state.
- A revision that changes `code`: if the app holds person-entered input since
  mount, the host keeps the current runtime and shows "The agent updated this
  app — Reload (discard what you typed) / Keep editing"; otherwise it restarts.
  A retained runtime can no longer send (its code hash is not current); a send
  from it opens "This app version is out of date — Put the message in the
  composer", which copies the exact text into the person's own composer.
- The inline row and the inspector render the same tree from one runtime; the
  inline row shows a placeholder while the inspector has it.
- Off-screen apps pause. An untouched one ends after a short grace; one
  holding person-entered input (or that asked to send) survives until a
  visible app needs its room, idle apps being evicted first (§0).

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
| `Image` | `source` names an entry of the node's `images` (≤ 8); no URLs. The Controller accepts only still PNG artifacts ≤ 1 MiB and ≤ 2048 × 2048, fully decoded at publish; ≤ 16 Mpx decoded per app. The host loads every image of the node into local blobs before the app runs, so which images are drawn, and when, never causes a request |

Expansion cost, counted by the host before a batch commits: each component
1 unit; each table cell, list item, select option and chart point 1 unit;
each 200 characters of text 1 unit. Budgets: ≤ 6,000 units per app,
≤ 12,000 per session view, ≤ 1,500 units committed per animation frame
(excess waits for the next frame, which also withholds credits). Retained
props and text ≤ 512 KiB per app. Updates exceeding a budget are rejected
before commit. Hooks: `useHostContext()`
(theme mode, density, platform, locale, container size) and
`useSend()` → `send(text)`.

## 7. Bridge and consent

| MCP Apps request | Phase A |
|---|---|
| `ui/initialize` | Host context: theme, density, platform, locale, time zone, container dimensions, display mode |
| `ui/notifications/size-changed` | Coalesced per animation frame within bounds |
| `ui/message` (`send`) | Text only, ≤ 2,000 Unicode scalars and ≤ 8 KiB; C0/C1 controls except newline rejected; bidi and invisible format characters shown escaped (`⟨U+202E⟩`). The product confirm sheet freezes and shows the exact bytes, labelled "Written by the app". On Send, the Web calls the Controller, which re-checks that the widget is open, the app node and code hash are current and the session admits prompts, records the message durably, then queues exactly those bytes as the person's prompt; Controller start re-queues recorded messages of the last 24 h whose echo never landed (the decision pattern). Each opened confirmation has a host-owned occurrence id bound to the payload digest and kept with the pending send in the browser's durable outbox across reloads; the `cmid` is derived from (session, widget, node, code hash, occurrence); a retry with the same occurrence and digest returns the recorded receipt, a different digest under that occurrence is refused. One open confirmation per app; at most 1 new confirmation per 5 s |
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

Apps are enabled by a fail-closed predicate: a tested engine and OS version,
and, inside a native shell, the shell's reported `appsSandbox: 1` capability
(present only in builds whose handlers pass the native gate). Older shells,
untested engines and unknown clients show `fallback` with "Update the Cowboy
app or open on Desktop".

The `widget-apps` suite, on every enabled engine (hawk Chrome; the iOS/iPadOS
native shell on a physical device per `conventions/pwa.md`; the macOS native
shell; each Tauri runtime that ships):

- Positive: a TSX app with `@cowboy/ui`, `Chart`, `Table` and `date-fns`
  compiles under the production CSP and renders at 320/375/768/1100 px, light
  and dark; events round-trip; props-only updates keep typed input; the
  inspector shares one runtime.
- Hostile, observed outside the app (sentinel cookie and storage; an egress
  sink on a Hawk overlay address that the probes target and that logs every
  DNS-free TCP connection; the CDP Network log on Chrome; Web Inspector
  network capture including Workers on WebKit shells; the Controller request
  log). Guest-reported failures never count as proof: no sentinel
  visible; no request leaves the page; no Worker can be spawned; forged
  readiness, bootstrap or port messages are ignored; unknown components and
  props, oversized batches and floods terminate the app; a compile-time loop,
  a render-time loop, an allocation loop and an asynchronous churn loop each
  stop within their budgets while Cowboy stays responsive and keeps unsent
  composer text; withheld credits and a hidden tab suspend the VM; four
  maximum-budget apps on a supported iPhone keep interaction latency under
  200 ms.
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

Revision 3 (Astra Max, `call-b9eed77a58345418f05da624`) — resolutions:

| Finding | Resolution |
|---|---|
| Frame CSP blocks WebAssembly (high) | Meta policy adds `'wasm-unsafe-eval'`; WASM bytes transferred, not fetched (§4.1) |
| Image choice is a network channel (high) | All images preloaded to local blobs before the app runs (§6) |
| Artifact images unbounded when decoded (high) | Still PNG ≤ 1 MiB and ≤ 2048², decoded at publish; 16 Mpx per app (§6) |
| No aggregate expansion cost or commit budget (high) | Unit cost model, per-app/session/frame budgets, iPhone latency gate (§6, §9) |
| Pending batches unbounded without credits (high) | Bounded in-flight and pending store; VM suspended when full or hidden (§5.2) |
| App messages lack durable retry ownership (high) | Record-then-queue, startup recovery, occurrence bound to digest in the durable outbox (§7) |
| Cumulative async CPU unbounded (medium) | Rolling CPU windows, bounded job drains, churn fixture (§5.1, §9) |
| Retained dirty runtime cannot send (medium) | Out-of-date sends go to the person's composer (§5.3) |
| Older native shells would enable apps (medium) | Fail-closed predicate with the shell's `appsSandbox` capability (§9) |
| No external egress observer on WebKit (medium) | Overlay egress sink plus Web Inspector capture (§9) |
| QuickJS integrity is a trust assumption (summary) | Stated with the remaining layers and maintenance (§5.1) |
