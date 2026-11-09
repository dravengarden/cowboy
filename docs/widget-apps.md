# Widget apps: an MCP Apps host and a Cowboy TSX runtime

Status: design, revision 1 (2026-11-09). The owner chose this direction:
Cowboy provides a secure sandbox in which agent-written React runs, and
widgets will grow into an advanced, extensible surface. This document extends
[`widgets.md`](widgets.md); the `cowboy.widget/1` JSON nodes stay the
lightweight, patchable path for status, tables and decisions.

## 1. Goals and non-goals

Goals:

- An agent can publish an **app**: a React component written in TSX that
  Cowboy compiles and runs in an isolated frame inside a widget row, on
  iPhone, iPad and Desktop, styled by Cowboy's theme.
- The frame cannot read or act as the person: no Cowboy cookies, storage, DOM,
  credentials or network. Everything it shows is data the agent gave it.
- The frame talks to Cowboy only through the **MCP Apps** protocol
  (SEP-1865, final 2026-01-26), so the same host can later render `ui://`
  resources from any MCP server, as Claude, ChatGPT, VS Code and Goose do.
- Every effect an app requests that speaks for the person (a message, a link,
  later a tool call) goes through a product-owned confirmation.

Non-goals for the first phases: network access from apps, third-party MCP
server UIs (phase C), persistent per-app storage, camera/microphone/location.

## 2. What the community does (2026-11)

- **MCP Apps** ([SEP-1865](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp),
  [spec 2026-01-26](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)):
  the first official MCP extension, co-developed by Anthropic and OpenAI. A
  tool declares a `ui://` resource (`text/html;profile=mcp-app`); the host
  renders it in a sandboxed iframe and talks JSON-RPC over `postMessage`
  (`ui/initialize`, `ui/message`, `ui/open-link`, `tools/call`,
  `ui/notifications/size-changed`, `ui/request-display-mode`,
  `ui/notifications/host-context-changed`, `ui/resource-teardown`). Web hosts
  MUST use a sandbox proxy on a different origin; the default CSP is
  `default-src 'none'` with `connect-src 'none'`; `ui/message` and
  `tools/call` MAY require consent; hosts SHOULD mark UI boundaries and limit
  CPU and memory. MCP-UI now recommends this pattern over its earlier
  Remote DOM path ([MCP-UI](https://github.com/MCP-UI-Org/mcp-ui)).
- **Claude Artifacts** ([analysis](https://pluto.security/blog/inside-claude-artifacts-how-your-agent-publishes-to-the-web/),
  [docs](https://code.claude.com/docs/en/artifacts)): each artifact on its own
  origin under a separate registrable domain (`<uuid>.frame.claudeusercontent.com`)
  with site isolation, a strict CSP, a frozen library allowlist (React,
  Tailwind, Recharts, shadcn/ui, Lucide, D3, …) and a bridge through which the
  host makes privileged calls as the viewer. Remaining risk noted publicly:
  social engineering such as "run this command" ([issue](https://github.com/anthropics/claude-code/issues/77409)).
- **ChatGPT Apps SDK**: sandboxed iframe on `*.web-sandbox.oaiusercontent.com`,
  `window.openai` bridge, declared `connect_domains` / `resource_domains`.
- **Running LLM-written TSX in the browser**
  ([component-playground](https://github.com/luisrrv/component-playground),
  [local-agent-chat #35](https://github.com/aquerman/local-agent-chat/pull/35),
  [Renderify](https://github.com/webllm/renderify)): compile with Sucrase
  (≈200 KB, fast, strips types without checking), resolve every import at one
  allowlisted choke point to vendored modules, run in `sandbox="allow-scripts"`
  without `allow-same-origin`, kill a hung frame with a heartbeat watchdog.

## 3. Architecture

```
agent ──widgets.publish──▶ Controller (validate, store, project)
                                   │ notice row (_cowboy.widget)
Cowboy Web (cowboy.stormbird.xyz) ─┴─▶ WidgetView ─▶ AppNode
   │  MCP Apps host: bridge, consent, sizing, theme, watchdog
   ▼
<iframe sandbox="allow-scripts" src="/widget-sandbox/v1/view.html">   (opaque origin)
   Cowboy TSX runtime: Sucrase → module map → React root → <App props/>
```

- A new node type `app` joins `cowboy.widget/1`:
  `{type:"app", id, title, code, props?, height?, fallback}`. `code` is one TSX
  module whose default export is the component; `props` is JSON passed to it
  and patchable by node id like any other field; `fallback` is the required
  text for notifications, search and older clients.
- The Web renders an `app` node through `AppNode`, which owns one frame and the
  host side of the bridge. Inline fences may not contain `app` nodes (only
  published, Controller-validated widgets run code).
- The runtime speaks MCP Apps inside the frame: the agent's component is the
  View; Cowboy's runtime performs `ui/initialize`, delivers `props` as the
  `ui/notifications/tool-input` arguments and exposes a small `cowboy` client
  (`sendMessage`, `openLink`, `requestDisplayMode`, `useHostContext`) built on
  the standard methods. Phase C reuses the same host for `ui://` resources.

## 4. Isolation (phase A)

### 4.1 The frame

- `<iframe sandbox="allow-scripts" src="/widget-sandbox/v1/view.html" allow="">`
  with no `allow-same-origin`, `allow-forms`, `allow-popups`,
  `allow-top-navigation*`, `allow-modals` or `allow-downloads`. The document
  has an **opaque origin**: no access to Cowboy cookies, `localStorage`,
  IndexedDB, the parent DOM, or same-origin APIs; `window.parent` exposes only
  `postMessage`.
- The sandbox page is served by the Controller with its **own** response CSP
  (a URL-loaded frame does not inherit the App's CSP, unlike `srcdoc`):

  ```
  default-src 'none';
  script-src https://<host>/widget-sandbox/v1/runtime-<hash>.js blob:;
  style-src 'unsafe-inline' https://<host>/widget-sandbox/v1/;
  img-src data: blob:; font-src https://<host>/widget-sandbox/v1/;
  connect-src 'none'; frame-src 'none'; worker-src 'none';
  object-src 'none'; base-uri 'none'; form-action 'none';
  frame-ancestors https://<host>
  ```

  `script-src` names the exact, content-hashed runtime file (not `'self'`, so
  no other same-origin path can be loaded as a script) plus `blob:` for the
  compiled agent module. No `'unsafe-eval'`.
- The runtime and its vendored libraries are static, versioned, immutable
  files under `/widget-sandbox/v1/`; no CDN is ever contacted.
- The host accepts a message only when `event.source` is that frame's
  `contentWindow` and the origin is `"null"`, and posts to the frame with a
  per-frame `MessageChannel` port established once, so another frame cannot
  inject or read bridge traffic.

### 4.2 What the frame can still reach, and why it is acceptable

- **Same-site requests.** The opaque-origin frame is still same-site with
  `cowboy.stormbird.xyz`. Its CSP blocks `connect-src`, images and frames to
  any address; forms and navigation are sandboxed off; script loads are pinned
  to one file. Cowboy's mutating routes also require a matching `Origin`
  (`ProductSessionMutate`), which an opaque origin (`null`) never has.
- **Data the frame holds.** Only the agent's code and `props`. Exfiltration
  channels CSP does not cover (for example WebRTC in some engines) can carry
  only what the agent already had; no person data, Cowboy state or other
  widget's data ever enters a frame. Host context sends theme, size, locale and
  platform only.
- **Side channels.** Same-site frames may share a renderer process. Chromium
  desktop isolates sandboxed frames in their own process; iOS WebKit does
  not. Without cross-origin isolation there is no `SharedArrayBuffer` and
  timers are coarse. Phase B removes this residual risk with a separate
  registrable domain (§9).
- **Resource abuse.** The host pings the frame every second; three missed
  pongs remove it and show "This app stopped responding" with Reload.
  Frames off screen for 30 s are torn down (`ui/resource-teardown`) and
  rebuilt on scroll-in from the stored `props`. At most 4 live frames per
  session view.
- **Phishing and social engineering.** Every app frame sits inside a
  product-drawn border with an "App by the agent" label outside the frame; an
  app can never draw over Cowboy chrome. The confirm sheet for any app-sent
  message says it was written by the app, not composed by Cowboy.

## 5. The runtime

- Compile: Sucrase (`typescript`, `jsx` automatic runtime, `imports` to
  CommonJS) inside the frame. Compile errors render a readable error card;
  types are not checked.
- Imports: one resolver maps an allowlist to vendored modules; anything else
  is a compile-time error naming the allowlist. Initial allowlist:
  `react`, `react-dom/client` (implicit), `@cowboy/ui` (the MUI-style kit:
  Box, Stack, Typography, Button, Chip, Table, Tabs, TextField, Select,
  Slider, Switch, Card, Alert, Tooltip, icons), `recharts`, `d3-*` subsets,
  `date-fns`, `lodash-es`. The kit is pre-themed from host context so apps
  look native without writing styles.
- Rendering: React 19 root, error boundary, `ResizeObserver` →
  `ui/notifications/size-changed` (debounced, clamped to the row budget
  inline and to the viewport in fullscreen).
- Budget: compiled runtime ≤ 900 KB gzip, cached immutably; first app paint
  target 300 ms on Desktop warm cache, 800 ms on iPhone warm cache.

## 6. Bridge (MCP Apps subset)

| View → host | Phase A behaviour |
|---|---|
| `ui/initialize` | Returns `hostContext`: `theme`, `styles.variables` (MUI palette, typography, radius mapped to the standard `--color-*`, `--font-*`, `--border-radius-*`), `containerDimensions`, `displayMode`, `availableDisplayModes`, `locale`, `timeZone`, `platform`, `safeAreaInsets` |
| `ui/notifications/size-changed` | Resizes the frame within its bounds |
| `ui/message` | Opens the product confirm with the exact text, labelled as written by the app; confirmed text is queued as the person's prompt under a `cmid` derived from widget, revision and text (same idempotency as decisions). Bounded to 2,000 chars, one pending confirm per frame |
| `ui/open-link` | `https:` only, through the existing external-link confirmation |
| `ui/request-display-mode` | `inline` ↔ `fullscreen` (the inspector); `pip` declined |
| `ui/update-model-context` | Phase A: stored on the widget (≤ 8 KiB) and returned by `widgets.read`, never injected into prompts automatically |
| `tools/call`, `resources/read` | Refused in phase A (`method not available`) |
| `notifications/message` | Logged (bounded) to the browser diagnostics |

Every message is schema-validated and size-bounded on the host; malformed
messages are dropped and counted.

## 7. Contract and Controller

- `app` node: `code` ≤ 48 KiB, `props` ≤ 16 KiB JSON, `title` and `fallback`
  required; a document containing an app may be up to 64 KiB; at most 2 app
  nodes per document and 20 apps per session view.
- The Controller validates shape and bounds only; it never runs or parses the
  TSX. `props` patches use the existing `widgets.update` patch ops.
- `widgets.read` returns `model_context` (from `ui/update-model-context`).
- Help text teaches the authoring contract with one compact example.

## 8. UX

- Inline: the frame lives inside the widget row's height budget with the
  existing fold and Open; Open (or `ui/request-display-mode: fullscreen`) shows
  the app in the inspector (Desktop modal, phone/iPad cover sheet) where it
  gets the full container.
- Loading: a skeleton of the reserved height, then the app; an error card with
  the compiler or runtime message and "Show code".
- Desktop: the row toggle opens the inspector; inside, focus enters the frame
  and `Esc` is forwarded by the runtime to the host to close (the frame cannot
  capture the product's global shortcuts).
- iPhone: touch targets from the kit are ≥ 44 px; the frame never scrolls
  horizontally; keyboard opens only on explicit text-field focus.

## 9. Phases

- **A — Cowboy TSX apps** (no network or DNS change): sandbox page and runtime
  served by the Controller, `app` node, bridge subset, escape conformance
  suite, watchdog and teardown, light/dark/phone/iPad/Desktop acceptance,
  physical iPhone check through the native shell.
- **B — Separate-site sandbox** (owner decision needed): serve the sandbox from
  a separate registrable domain with per-app subdomains (as
  `claudeusercontent.com`), restoring process isolation on every engine and
  meeting the MCP Apps sandbox-proxy requirement exactly. Requires a domain,
  a wildcard certificate and DNS publication.
- **C — Third-party MCP Apps**: render `ui://` resources declared by MCP
  servers the agents use, with declared CSP domains shown to the person,
  `tools/call` with per-call consent and visibility rules. Requires the
  provider adapters to surface `_meta.ui.resourceUri` and a resource read path.

## 10. Acceptance (phase A)

The `widget-apps` browser suite runs on hawk Chrome in a disposable context
and proves, from inside a hostile app: `document.cookie` is empty; storage
APIs throw; `parent.document` throws; `fetch`, `WebSocket`, `EventSource`,
image, font, CSS `url()` and `<link rel=prefetch>` loads to the Controller
and to an external address are blocked; `window.open`, form submit, top
navigation and downloads fail; an import outside the allowlist fails to
compile; an infinite loop is torn down by the watchdog; a forged message from
another frame is ignored; `ui/message` never sends without the confirm. It
also checks theme propagation, sizing at 320/375/768/1100 px, fullscreen and
teardown/rebuild. The physical iPhone check follows `conventions/pwa.md`.

## 11. Open questions

1. Phase B domain: which registrable domain hosts the sandbox.
2. Library allowlist beyond the initial set (for example Three.js, maps).
3. Whether apps may ever call Cowboy MCP read tools (`tools/call`) and with
   which consent model.
