# Widget apps: agent-written TSX on a Worker runtime, MCP Apps later

Status: design, revision 2 (2026-11-09). The owner chose the direction:
Cowboy provides a secure sandbox in which agent-written React runs, and
widgets grow into an advanced, extensible surface. Revision 2 replaces the
revision 1 phase A (agent code in a same-site DOM frame) after the Astra Max
review in §11. It extends [`widgets.md`](widgets.md); the `cowboy.widget/1`
JSON nodes stay the lightweight, patchable path for status, tables and
decisions.

## 1. Goals and non-goals

Goals:

- An agent can publish an **app**: a React component in TSX that Cowboy
  compiles and runs, drawn inside a widget row with Cowboy's own components on
  iPhone, iPad and Desktop.
- Running an app can never read or act as the person: no Cowboy cookies,
  storage, DOM, credentials, native bridges or network, and no path by which
  text the person types into an app leaves the device except a message the
  person confirms.
- A hung or abusive app can always be stopped without freezing Cowboy, on
  every engine Cowboy ships (Chromium, WebKit on iOS/iPadOS/macOS, Tauri).
- The host side follows **MCP Apps** (SEP-1865, final 2026-01-26) semantics
  for its bridge, so phase C can render `ui://` resources from MCP servers.

Non-goals for phase A: arbitrary DOM, canvas or WebGL in apps (phase B), apps
calling tools (§10), third-party MCP server UIs (phase C), persistent per-app
storage.

## 2. What the community does (2026-11)

- **MCP Apps** ([SEP-1865](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp),
  [spec 2026-01-26](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)):
  HTML views in sandboxed iframes; web hosts MUST interpose a sandbox proxy on
  a different origin; JSON-RPC over `postMessage`; default CSP denies all
  network; `ui/message` and `tools/call` may require consent; hosts should
  mark UI boundaries and limit CPU and memory.
- **MCP-UI / Shopify Remote DOM** ([MCP-UI](https://github.com/MCP-UI-Org/mcp-ui),
  [Shopify](https://shopify.engineering/mcp-ui-breaking-the-text-wall)): the
  remote code builds a component tree in an isolated realm; the host renders
  that tree with its own component library, so the UI matches the host and the
  code never touches the host DOM. `@remote-dom/core` 1.12 and
  `@remote-dom/react` 1.2 (MIT, maintained in 2026).
- **Claude Artifacts** ([analysis](https://pluto.security/blog/inside-claude-artifacts-how-your-agent-publishes-to-the-web/)):
  per-artifact origins on a separate registrable domain, strict CSP, a frozen
  library allowlist, privileged calls made by the host as the viewer.
- **LLM-written TSX in the browser**
  ([component-playground](https://github.com/luisrrv/component-playground),
  [Renderify](https://github.com/webllm/renderify)): Sucrase compile, one
  allowlisted import resolver, `sandbox="allow-scripts"` without
  `allow-same-origin`, and a terminable boundary (Renderify runs everything
  in a Worker) for hangs.

## 3. Architecture (phase A)

```
agent ──widgets.publish──▶ Controller: validate bounds, store, project row
Cowboy Web ──▶ WidgetView ──▶ AppNode (host): owns one runtime per app identity
   │  renders the remote tree with Cowboy MUI components; bridge, consent,
   │  rates, watchdog, lifecycle
   ▼  srcdoc <iframe sandbox="allow-scripts">   (opaque origin, inherits App CSP
   │                                             + stricter meta CSP; trusted
   │                                             boot code only, no app code)
   ▼  Worker (blob:)  ── Sucrase compile ── module map ── React + @remote-dom/react
                        agent component renders <Button>, <Chart>, … as remote
                        elements; mutations stream to the host as data
```

- **Agent code runs only in the Worker.** A Worker has no DOM, no window, no
  `RTCPeerConnection`, no navigation and no access to native script-message
  handlers. It is created inside a `srcdoc` frame (the editor plugin pattern,
  [`editor-plugins.md`](editor-plugins.md)) so its origin is opaque: no
  cookies, storage or same-origin APIs. The frame's own script is Cowboy's
  fixed boot code; it never evaluates app code.
- **No document is fetched for the sandbox.** The frame is `srcdoc`, so no
  request carries the session cookie and there is no sandbox page to navigate
  to or substitute (the service worker navigation fallback is not involved).
- **The host renders.** The Worker's React tree uses `@cowboy/ui` remote
  elements; `@remote-dom` serializes mutations; the host maps each allowlisted
  element to a Cowboy MUI component with validated props. Unknown elements,
  unknown props and non-JSON values are dropped and counted.
- **Phase B** adds a full-DOM tier (arbitrary DOM, canvas, SVG libraries,
  Three.js) in a frame on a separate registrable domain, enabled only on
  engines that give that frame its own process and a killable boundary.
- **Phase C** renders third-party MCP Apps `ui://` resources through a spec
  conformant sandbox proxy on that separate domain (§9).

## 4. Isolation

### 4.1 Frame and Worker

- `<iframe sandbox="allow-scripts" srcdoc=… allow="">`: no `allow-same-origin`,
  forms, popups, top navigation, modals, downloads or permissions.
- The frame inherits the App document CSP and adds the editor plugin's
  stricter meta CSP (`default-src 'none'; script-src 'unsafe-inline' blob:;
  worker-src blob:`); both apply, and the Worker inherits them. The
  intersection allows the Worker's own `blob:` script (App `worker-src blob:`)
  but no further script loading (App `script-src` has neither `blob:` nor
  `'unsafe-eval'`), no `fetch`, WebSocket, EventSource, images, fonts or
  frames.
- The boot frame compiles the app with Sucrase (input ≤ 14 KiB; the
  transform is linear, so trusted boot code stays responsive) and builds one
  `blob:` from the runtime bundle (React, `@remote-dom`, the `@cowboy/ui`
  remote elements, allowlisted libraries; static, content-hashed, shipped with
  the Web release) followed by the compiled module. That blob starts the
  Worker; nothing evaluates strings and nothing is fetched.

### 4.2 Egress

| Channel | Phase A status |
|---|---|
| Cookies, storage, IndexedDB, Cache API | Opaque origin: unavailable |
| `fetch`, XHR, WebSocket, EventSource, `importScripts`, `eval` | Inherited CSP denies them in the Worker |
| WebRTC, DNS prefetch, navigation, `window.open`, forms, downloads | Not available in a Worker; the frame's own code is trusted |
| Native bridges (WKWebView message handlers, Tauri IPC) | Not exposed to Workers; release gate §8 also requires every privileged handler to reject subframes |
| Bridge to host | Only bounded data messages; the only person-visible effects are confirmed (§6) |

Person-typed text enters the Worker only as values of host-rendered inputs; it
can leave only through a `ui/message` the person confirms. Agent-supplied
`props` are treated as confidential with the same rule: an app cannot
disclose them except into the transcript after confirmation.

### 4.3 Termination and resource limits

- The host owns the Worker lifecycle through the boot frame. A heartbeat
  every second; three misses terminate the Worker (`Worker.terminate()` runs on
  the frame's thread, which never executes app code, so a busy Worker cannot
  block it). The host shows "This app stopped responding" with Reload.
- Host-enforced budgets per app, revoked on violation (Reload required):
  remote tree ≤ 2,000 nodes and depth ≤ 32; mutation batches ≤ 60/s and
  ≤ 256 KiB/s; bridge requests ≤ 20/s with ≤ 4 outstanding; diagnostics
  queue ≤ 100 entries; per-session aggregate ≤ 3× one app. Size changes are
  coalesced per animation frame.
- Memory: Worker heap is not directly limitable in browsers. The host
  terminates an app whose serialized tree, message volume or
  `performance.measureUserAgentSpecificMemory` (where available) exceeds its
  budget; at most 4 live app Workers per session view.
- Eviction: an app off screen for 60 s is terminated and rebuilt from `props`
  on return, **unless** it is pinned: focused, holding person-entered input
  since mount (the host knows because it renders the inputs), or awaiting a
  confirmation. Pinned apps stay live; the limit of 4 counts them.

### 4.4 One runtime per app identity

An app identity is (session, widget, app node id, document revision). The
inline row and the inspector render the **same** remote tree from one Worker;
while the inspector shows the app, the inline row shows a placeholder ("Open
in the app view"). A new revision of the document starts a new runtime.

## 5. The runtime

- Compile: Sucrase (`typescript`, `jsx` automatic, `imports`) in the boot
  frame before the Worker starts (§4.1); types are not checked. Compile and
  runtime errors render an error card with the message and "Show code".
- Imports resolve through one allowlist to modules inside the bundle; any
  other specifier is a compile error naming the allowlist. Phase A allowlist:
  `react`, `@cowboy/ui`, `date-fns`, `lodash-es` (pure modules only).
- `@cowboy/ui` remote elements (host renders with Cowboy MUI, themed, touch
  targets ≥ 44 px on touch surfaces): `Stack`, `Box`, `Grid`, `Text`,
  `Heading`, `Divider`, `Card`, `Chip`, `Badge`, `Alert`, `Progress`,
  `Button`, `IconButton` (allowlisted icon names), `TextField`, `Select`,
  `Checkbox`, `Switch`, `Slider`, `Tabs`, `Table` (sortable), `List`,
  `Tooltip`, `Markdown` (product renderer), `Image` (artifact or verified data
  sources only), `Chart` (line, bar, area, pie, scatter; host-rendered with
  Recharts from data). Event props deliver sanitized payloads (`value`,
  `checked`, `index`) to the Worker.
- Hooks from the runtime: `useHostContext()` (theme mode, density, platform,
  locale, container size), `sendMessage(text)`, `openLink(url)`,
  `requestDisplayMode("inline" | "fullscreen")`.
- Budget: runtime bundle ≤ 450 KB gzip, cached immutably; first paint target
  300 ms Desktop / 800 ms iPhone with a warm cache.

## 6. Bridge and consent

The host↔runtime messages follow MCP Apps method names and semantics.

| Request | Phase A behaviour |
|---|---|
| `ui/initialize` | Host context: theme, density, platform, locale, time zone, container dimensions, display mode |
| `ui/notifications/size-changed` | Coalesced; frame and row resize within bounds |
| `ui/message` | Text only, ≤ 2,000 Unicode scalars and ≤ 8 KiB; C0/C1 controls (except newline) rejected; bidi and invisible format characters shown escaped (for example `⟨U+202E⟩`) in the preview. The product confirm sheet freezes and shows those exact bytes, labelled "Written by the app"; Send queues exactly those bytes as the person's prompt. Each opened confirmation gets a host-owned occurrence id; the prompt's `cmid` is derived from (session, widget, node, revision, occurrence) and reused only for retries of that same confirmed payload. One open confirmation per app |
| `ui/open-link` | `https:` only, ≤ 2,048 chars; a new product confirm sheet shows the frozen, validated destination and app attribution; the opener runs only after confirmation with `noopener,noreferrer`; repeated unsolicited requests are rate-limited (1 per 5 s) and revoke at the third denial |
| `ui/request-display-mode` | `inline` ↔ `fullscreen` (the inspector), only in response to a person's gesture inside the app |
| `ui/update-model-context`, `tools/call`, `resources/read` | Refused in phase A |
| `notifications/message` | Bounded diagnostics only |

Bootstrap and channel authentication:

- The host creates the frame, then posts one bootstrap message containing a
  fresh per-load nonce and a transferred `MessagePort`. The boot script accepts
  only the first bootstrap whose `event.source` is `window.parent`, echoes the
  nonce over the port, and only then starts the Worker; later bootstrap
  attempts are ignored.
- The host accepts messages only on that port; the port is bound to the app
  identity (§4.4). Navigation, replacement, a new revision, Clear, sign-out
  and teardown close the port, terminate the Worker and cancel any open
  confirmation.

## 7. Contract and Controller

- New node `{type:"app", id, title, code, props?, height?, fallback}`. `code`
  is one TSX module whose default export is the component; `props` is JSON
  passed as its props.
- Bounds fit the existing transport (32 KiB serialized tool arguments,
  24 KiB widget document, 48 KiB snapshot): `code` ≤ 14 KiB and `props`
  ≤ 6 KiB measured as serialized JSON; at most one app node per document in
  phase A. Raising them later is a staged transport and snapshot change.
- **`props` is opaque.** Node traversal (patch `#id` lookup, image source
  walk, input walk, description projection) follows schema edges only and
  never enters `props`. `widgets.update` may patch inside `props` only through
  a JSON Pointer under the app node (`/root/…/props/…`). Fixtures: ids and
  image-shaped objects inside `props` are neither addressed nor rewritten.
- The Controller validates shape and bounds only; it never parses or runs TSX.
  Inline fences may not contain app nodes.

## 8. Acceptance and release gates

Browser suite `widget-apps` (hawk Chrome, disposable context, authenticated
fixture with sentinel cookie and storage values, Controller-side request log):

- Positive: a TSX app using `@cowboy/ui`, `Chart` and `date-fns` compiles and
  renders under the production CSP at 320/375/768/1100 px in light and dark;
  events round-trip; inline↔inspector keeps one runtime and its input state.
- Isolation from inside a hostile app: no sentinel cookie or storage visible;
  `fetch`/WebSocket/EventSource/`importScripts` blocked and **no request
  reaches the Controller log**; WebRTC and DOM globals absent in the Worker;
  forged bootstrap or port messages from a sibling frame ignored; unknown
  elements and props dropped.
- Resource: an infinite loop at compile time and at render time is terminated
  within 3 s while Cowboy stays responsive; tree, mutation and request
  budgets revoke; eviction preserves pinned apps.
- Consent: `ui/message` never sends without the confirm; bidi text is shown
  escaped; a repeated confirmed message sends twice, a retried one once;
  `ui/open-link` never opens without its confirm.

Release gates outside Chrome:

- Native shells: every privileged handler (WKWebView script message handlers
  including the passkey bridge, Tauri capabilities) rejects calls from
  subframes, verified by direct calls from a test subframe on macOS, iOS and
  Tauri builds; this also hardens the existing editor plugin sandbox.
- WebKit (iOS/iPadOS physical device via the native shell, per
  `conventions/pwa.md`): Worker termination of a busy app keeps Cowboy
  responsive; the hostile-app probes above pass.

## 9. Phases

- **A — Worker apps** (no network or DNS change): runtime bundle, `app` node,
  host renderer for `@cowboy/ui`, bridge subset with the new confirmations,
  budgets and lifecycle, acceptance and release gates of §8.
- **B — Full-DOM apps on a separate site**: a dedicated registrable domain
  (not a `stormbird.xyz` subdomain) with one subdomain per app, served by a
  cookie-free static endpoint without redirects, `frame-ancestors` limited to
  Cowboy, and enabled per engine only where the frame is process-isolated and
  killable; elsewhere the app shows its `fallback` with "Open on Desktop".
- **C — Third-party MCP Apps**: a spec-conformant sandbox proxy on the phase B
  domain that preserves the standard View-facing `postMessage` transport,
  initialization and capability negotiation, notification ordering and
  teardown; declared CSP domains shown to the person; tool calls per §10.
  Compatibility is claimed only after an unmodified MCP Apps client fixture
  passes.

## 10. Decisions (owner delegated, 2026-11-09)

1. Phase B domain: a separate registrable domain dedicated to app content
   (working name `stormbirdusercontent.xyz`); the owner buys it when phase B
   starts; Cowboy prepares the resource record, Caddy and certificate.
2. Libraries: phase A as in §5; Three.js and canvas libraries come with phase
   B; map libraries are excluded (they need network tiles).
3. Tools: none in phase A. Later, only Cowboy read-only reads, with a
   per-widget consent that names the data read; write tools are never
   available to apps.

## 11. Review record

Revision 1 (Astra Max, `call-2b49e88dbf282591a7768b54`, incomplete: spec
not captured) — resolutions:

| Finding | Resolution |
|---|---|
| Same-site URL sandbox carries the session cookie and is not a different-origin proxy (high) | Phase A loads no sandbox document (srcdoc, opaque); HTML views wait for phase B/C on a separate site with a cookie-free proxy (§3, §9) |
| A parent watchdog cannot stop a shared-thread loop; no heap/DOM limits (high) | App code runs only in a terminable Worker; tree, mutation, request and memory budgets (§4.3) |
| Person-typed and confidential data can leave through channels CSP misses (high) | Worker has no WebRTC/DOM/navigation; inputs are host-rendered; data leaves only by confirmed message (§4.2) |
| `ui/open-link` reused an opener without confirmation (high) | New frozen-destination confirm, `noopener,noreferrer`, rate limit (§6) |
| Bootstrap authentication and revocation (medium) | Nonce bootstrap, port binding, revocation list (§6) |
| Phase C transport adapter unspecified (medium) | Phase C requirements and unmodified-client gate (§9) |
| Aggregate bridge work unbounded (medium) | Per-app and per-session rate and byte budgets with revocation (§4.3) |
| Identical message text dedupes legitimate repeats (medium) | Host-owned occurrence id per confirmation (§6) |
| App-authored confirm text can hide bidi/invisible characters (medium) | Text-only, bounds, controls rejected, format characters escaped, frozen bytes (§6) |
| Sizes exceed transport and snapshot limits (medium) | Bounds fitted to existing limits (§7) |
| `props` participate in node traversal (medium) | `props` opaque; schema-edge traversal; fixtures (§7) |
| Model-context writes lack authority (medium) | `ui/update-model-context` refused in phase A (§6) |
| Eviction and inspector lose unsent input (medium) | One runtime per identity; pinning (§4.3, §4.4) |
| Probes must be observed outside the frame (medium) | Sentinel values and Controller request log (§8) |
| Native IPC denial must be a release gate (medium) | Native-shell gate including the passkey bridge (§8) |
