# Widget apps, phase C: MCP Apps from bound Plugin servers

Status: design, revision 5 (2026-11-11). Not implemented. It replaces the
direction recorded in [`widget-apps-dom.md`](widget-apps-dom.md) §10 and the
phase C line of [`widget-apps.md`](widget-apps.md) §10. Owner decisions are
in §11, the review record in §12.

Phase C renders [MCP Apps](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx)
views (`ui://` resources, `text/html;profile=mcp-app`) in the transcript. The
review of phase B revision 4 set the bar: trust is a **host-owned assignment
naming the executable resource and the capabilities it may use**, never a
property of the package or of what content says, and privileged response
policies stay separate from phase B's network-denying `/app-shell.html`.

## 1. Scope

In:

- Views of MCP servers reached through Cowboy's **MCP Plugin bindings**
  ([`mcp-bindings.md`](mcp-bindings.md)): signed `mcp_server` contracts,
  owner-written session policy, the Controller's parent-authorized relay.
- **Static** views only: one self-contained HTML file of the signed Plugin
  release, named with its digest in the signed contract. Cowboy serves those
  verified bytes; nothing a server returns at run time is ever a view.

Out (each needs its own design): servers an agent connects to natively
outside a binding; dynamic or server-generated views; views that need
`allow-same-origin` or a real origin (the `thundersparrow.top` tier);
remote scripts, styles, images or fonts; **direct network access from a
view** (`connect`, deferred: §6); external URLs (`externalIframes`),
`sampling/createMessage`, `ui/download-file`, `resources/read` from a view,
app-registered tools and display modes other than inline.

## 2. Authority model

| Question | Answered by | Not by |
|---|---|---|
| Which bytes run | The digest in the signed contract, verified by Machine preparation and again when served (§3) | Anything a server returns |
| What they may do | The owner's per-session grant (§4), CAS-written with the existing policy epoch | The signer, the manifest or the view's `_meta.ui` |
| Which document holds it | A single-use frame token bound to one resource, digest, epoch and widget (§5) | The frame's `contentWindow` or its `"null"` origin |
| Whether it still holds it | A server-enforced frame lease and the relay's live checks, both required at dispatch and delivery (§6, §7) | The parent page's timers or a URL that happens to return 404 later |

A view gets nothing beyond phase B unless the owner granted it, for this
session, by binding, resource and capability, and only for as long as the
lease is renewed.

## 3. Signed static views and tool visibility

The production path does not carry the information a runtime design would
need: native bindings answer `tools/list` locally from **signed discovery**,
which has no `_meta`, the backend advertises empty capabilities, and the
Machine transport is `stdio_tool_only_v1` (no `resources/read`). Phase C
therefore takes everything from the signed contract and never from the
server's live answers.

The `mcp_server` contract gains an optional `apps` declaration
(`McpAppsContract`, `components/plugin-sdk/src/mcp_server.rs`):

- `ui_resources`, at most 8: `{uri, html}` with `ui://` URIs of lowercase
  path segments. The HTML is part of the signed contract, so the package
  signature covers the exact bytes that may ever run as the view, and the
  Controller and every Machine already hold it with the package; no host
  bundle, release envelope or install change is needed. Each document is
  ≤ 512 KiB, all of them ≤ 2 MiB (the Machine package limit is 8 MiB), with
  inline scripts and styles only: signing refuses `<link>`, `<base>`,
  frames, objects, `src`/`srcset` attributes, `@import` and remote `url()`.
  That check is a lint; the serving CSP is the boundary. A view's data
  arrives through tool input, tool results and its own granted tool calls,
  never over the network.
- `tools`: per signed tool, `{name, resource?, visibility, read_only}` where
  `visibility` is `["model"]`, `["app"]` or both and `read_only` mirrors the
  tool's `readOnlyHint`. A tool without an entry is visible to both (the
  MCP Apps default), is not read-only and has no view. Contracts without
  `apps` serialize exactly as before; older verifiers refuse the unknown
  field.

The Controller takes a view's bytes only from the verified package of the
exact generation the binding names, and computes inline-script and style
hashes once.

Two projections replace "the granted tools":

- **Agent projection**: the binding's `tools` is the agent's grant and may
  name only `"model"`-visible tools; Machine preparation refuses a binding
  that grants an app-only tool to the agent, so native descriptors and the
  Machine process's call check never include one. Adding a view grant never
  widens the agent's tools.
- **App projection**: the `"app"`-visible signed tools that the view's grant
  lists in `call_tools` (§4), whether or not the agent also holds them. The
  view scope's process profile carries exactly these (step 3); a view's call
  to any other tool, including cross-binding calls, is refused.

## 4. Assignment: the owner's grant

The binding policy of `PUT /api/sessions/{id}/mcp-bindings` gains an optional
top-level `apps` map from binding name to view grants; omitted means no view
renders (the default for every existing policy, whose stored shape does not
change):

```json
"apps": {
  "suger": [{
    "uri": "ui://suger/status",
    "capabilities": {
      "call_tools": ["suger_refresh"],
      "message": true,
      "open_links": true
    }
  }]
}
```

- Grants stay in the policy record and never enter a process profile, so
  Machines never parse them (`AppGrant`, `components/wire/src/mcp_binding.rs`).
- Written with the bindings in one CAS update and one new epoch. Every key
  must name a binding of the same update, so `bindings: []` revokes every
  grant; at most 8 grants per binding, unique `ui://` URIs, at most 16
  unique tool names. Unknown capabilities, `connect` included, are refused.
- Whether the exact generation's signed contract declares that view and
  those `"app"`-visible tools is checked when the grant is used (step 2);
  a grant that does not match grants nothing. A generation change replaces
  the binding and therefore needs a new grant.
- A Controller older than this change reads a stored policy with grants as
  invalid; its owner repair (revoke-only) still works.

| Capability | Granted | Not granted |
|---|---|---|
| `call_tools`, read-only tool | Called without a per-call prompt | Refused |
| `call_tools`, other tool | Confirmation per call showing tool and arguments; an uncertain outcome is reported, never retried (supersedes the "no write tools" line of `widget-apps.md` §11 for granted tools only) | Refused |
| `message` | `ui/message` → the confirmation used by `useSend` | Refused |
| `open_links` | `ui/open-link` through Cowboy's opener, HTTPS only | Refused |

## 5. Serving a view

The Web obtains a **frame token** by an authenticated
`POST /api/sessions/{id}/mcp-apps/frames` naming the widget; the Controller
checks the current grant and returns an opaque, random, single-use token
bound to `(session, binding, uri, sha256, epoch, widget, lease)`. The frame
loads `GET /mcp-app/<token>.html`:

- Served once: the first request consumes the token, so a document in the
  frame cannot navigate to another view's URL and inherit its bridge.
  Requests must carry `Sec-Fetch-Dest: iframe` and
  `Sec-Fetch-Site: same-origin`; anything else, an unknown or used token, a
  query string or another path is an empty 404. `no-store`, no cookies set,
  path not logged; the service worker never answers `/mcp-app/`.
- Headers per grant, separate from `/app-shell.html`:
  `Content-Security-Policy: sandbox allow-scripts; default-src 'none';
  script-src <inline hashes>; style-src-elem <inline style hashes>;
  style-src-attr 'unsafe-inline';
  img-src data: blob:; font-src data:; connect-src 'none';
  frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none';
  base-uri 'none'; form-action 'none'; frame-ancestors 'self'`,
  `Connection-Allowlist: ()`,
  `Permissions-Policy` disabling every feature, `CORP: cross-origin`. The
  response `sandbox` keeps the document opaque even if opened top-level or
  framed without the attribute; the frame is also exactly
  `sandbox="allow-scripts"`.
- No grant contains a remote executable or style source: everything that
  runs is the signed bytes. Hashed `<style>` blocks and `style` attributes
  are the supported styling; a view that generates unhashed `<style>`
  elements is not eligible (checked by the fixture, not assumed).

## 6. Lease and revocation

Authority is enforced by the Controller, not by the parent page: a stalled
or throttled parent cannot remove a frame on time, so nothing the frame can
do by itself may depend on the parent's timers.

- A frame token starts a **server-side frame lease** of 30 s, renewed every
  10 s by the Web over its authenticated connection; renewal rechecks the
  grant, epoch, widget and relay scope. Each run has its own lease and run
  identity.
- Every view-originated `tools/call`, `ui/message` and `ui/open-link`
  carries the token; the Controller requires the original, unexpired lease
  and run identity at admission, at dispatch and again before delivering a
  result. An expired or revoked lease refuses them server-side whatever the
  page does.
- Teardown (Stop, out of view, removal, failed renewal, epoch change,
  sign-out, lost Controller connection) ends the lease on the Controller
  when reachable and cancels any confirmation still shown for that run; a
  confirmation answered after its lease ended is refused.
- With no network capability (§1), a frame that outlives its lease in a
  stalled parent can still draw and compute, but every effect it can have
  goes through the Controller and is refused. Direct network grants are
  deferred until they can be brokered under the same lease, or the owner
  accepts a best-effort revocation contract for them.

## 7. Bridge

The view speaks MCP Apps JSON-RPC to `window.parent` (single frame; Cowboy
already serves the document, so the spec's proxy frame adds nothing).

- Accepted only from that frame's `contentWindow` with origin `"null"`. The
  first `load` is the view; any later `load` removes the frame. A
  replacement document that posts before its `load` can only be one the view
  itself created (blob or data), which inherits its CSP and holds no more
  than the view already had, because: the token is single-use; the view's
  CSP has `connect-src 'none'`; and the **parent page's `frame-src` stays
  limited to Cowboy's own origin** (an invariant of the product CSP, tested:
  a view navigating its frame to an external origin serving executable HTML
  is blocked).
- Handshake: the view's `ui/initialize` request is answered immediately with
  `protocolVersion` (the pinned spec revision), `hostInfo`
  (`{name: "Cowboy", version}`), `hostCapabilities` listing exactly the
  granted capabilities, and `hostContext` (theme, locale, `platform: "web"`,
  container size). Nothing else is sent before
  `ui/notifications/initialized`; then tool input and result are replayed.
- Host → view: `ui/notifications/tool-input`, `tool-result`,
  `tool-cancelled`, `host-context-changed`, `ui/resource-teardown`, `ping`.
- View → host: `ui/notifications/size-changed`, `request-teardown`,
  `ping`, `notifications/message` (recorded, bounded), `tools/list` (the
  view's app projection), `tools/call`, `ui/message`, `ui/open-link` per §4.
  Other requests get method-not-found; malformed messages remove the frame.
- Ingress budget, checked on arrival before any parsing beyond size: every
  message counts against 120 messages and 256 KiB per second per frame
  (enough for one size notification per animation frame plus normal RPC);
  requests additionally count against 30 per second. Exceeding either removes
  the frame. Size notifications are then coalesced to one layout update per
  animation frame.
- A view `tools/call` goes to the Controller with the frame token and runs
  in a **view scope**, never in the agent's. On a view's first call the
  Controller opens one relay handle for `(session, binding)` through the
  existing relay: the same current policy epoch, the same parent grant
  (`managed_calls::authorize`) and the same execution connection the agent's
  scope uses, but its own MCP process. Before the first tool call on a new
  or reopened handle the Controller completes the backend MCP handshake
  itself (`initialize`, then `notifications/initialized`, as the native
  bridge does for the agent) and validates the reply; a failed handshake
  closes the new handle and the call is refused, never replayed. Every
  request there is rechecked like
  any relay request, plus the lease (§6), the app grant and the confirmation
  (if required), at dispatch and again before delivery.
- Separate processes are what keep the agent safe: the Machine process
  serves one request at a time, its 30 s deadline includes queue time, and a
  failed or timed-out request closes the process and retires its relay
  handle (`process.rs`, `relay.rs`). Sharing would let a slow view call
  time out an agent call queued behind it and leave an agent write
  uncertain. With its own process a view can only retire its own handle.
- The view scope serves one request at a time; a second while one is in
  flight gets busy before anything reaches the transport. A dispatched
  request is owned by the Controller until the backend answers or times out:
  frame teardown drops only the delivery. A retired view handle is reopened
  only by a later call under a live lease, current epoch and the same parent
  grant and connection; a changed parent grant or connection refuses view
  calls until the agent's scope is current again. When no frame of that
  binding holds a lease, or after 60 s idle, the scope refuses new calls at
  once but closes its transport only after a dispatched request has answered
  or timed out (relay close cancels in-flight requests, so closing earlier
  could interrupt a confirmed write); authority loss still closes at once.
  It counts
  against the relay's 32 open sessions (at most one per session and binding).
- A server's state is per process. Views therefore get the same tools and
  sources as the agent but not the agent's in-process state; a server that
  needs shared state keeps it outside the process (the eligibility is part
  of the Plugin's acceptance, not assumed).

## 8. Transcript

When the relay delivers an **agent-originated** `tools/call` result for a
tool whose signed `ui.resource` the session grants, the Controller publishes a
system widget row with an `mcp_app` node `{binding, uri, sha256, epoch,
scope, call}`; tool input and result (each ≤ 64 KiB of JSON, else a
truncation marker) are stored with it. View-originated calls never publish
rows; their results go to that frame only. The row shows the binding name,
the granted capabilities in one line, and Run; a view starts only when
tapped, counts against the page's runtime slots, and stops out of view.
Where phase C is not admitted (§9) the row shows the tool's text result.

## 9. Compatibility profile and admission

Cowboy's profile is restricted relative to the spec, which asks web hosts for
`allow-scripts allow-same-origin` on a separate origin: views run opaque, so
no `localStorage`, cookies or IndexedDB; `fetch` to a granted origin needs
CORS that admits `Origin: null`. Each admission records the pinned spec and
`@modelcontextprotocol/ext-apps` revision and the named views known to work
under this profile.

Phase C has its **own admission table**, never inherited from phase B. A row
requires, on that browser:

- the phase B suite unchanged for `/app-shell.html`;
- `/mcp-app/` network rows: a view shows zero egress on every phase B
  channel with an allowing control; navigating its frame to an external
  origin is blocked by the parent's `frame-src`;
- direct top-level navigation, unsandboxed embedding, a reused token,
  `Sec-Fetch-Dest` other than `iframe`, a query string or another path all
  get 404 or an opaque document with no bridge;
- a view navigating its frame to another view's URL, and a blob document
  posting before `load`, gain no bridge;
- a view call, message or confirmation after its lease ended (including
  with the parent page stalled) is refused by the Controller; sign-out and
  policy-epoch change remove the frame;
- view and agent calls in both arrival orders, each fitting its own
  deadline, both complete; a view call that times out retires only the view
  scope while the agent's handle stays open; tearing down a frame with a
  dispatched call leaves both handles open;
- the pinned SDK's initialize sequence, sustained auto-resize without
  removal, a size-notification flood removed, a granted read-only call without a prompt, a write call with one,
  an ungranted call refused;
- the service worker and an offline reload never serve `/mcp-app/`.

## 10. Order of work

1. a. Contract `apps` (views, visibility, read-only) and the agent
      projection enforced by Machine preparation. Done (this revision).
   b. Views inlined in the signed contract (no host bundle); one component
      release for 1a and 1b. Done (this revision).
   c. Policy `apps` with CAS and shape validation; contract matching at use.
      Done (this revision). No rendering.
2. a. Frame tokens, `/mcp-app/` route and headers, the lease (Controller:
      `components/server/src/server/mcp_bindings/apps.rs`,
      `web_security::mcp_app_headers`; views from the Catalog's exact release,
      `PluginCatalog::resolve_mcp_server`). Done (this revision).
   b. End-to-end Controller test with a signed MCP release in a disposable
      Catalog: served once from a same-origin frame, top-level, cross-site
      and query loads refused without consuming the token, a changed epoch,
      an undeclared tool or a revoked binding grants nothing. Done (this
      revision). The browser network and navigation rows need the bridge
      and move to step 3, where a frame is driven by the real Web host.
3. Bridge, scope-bound `tools/call`, transcript row; the SDK fixture rows.
4. A real native-binding test with the retained execution executable (not a
   standalone SDK view), then the first view: a Suger status view granted on
   one session.

## 11. Owner decisions (2026-11-11)

The owner accepted the recommendations:

1. Static, signed views only in phase C; dynamic or same-origin views are
   deferred to the `thundersparrow.top` tier.
2. Capabilities offered: `connect`, `call_tools`, `message`, `open_links`;
   no storage or device permissions. Revision 3 defers `connect` (§6): the
   review showed a page-enforced lease cannot revoke direct network access
   on time, so views get data through tools until network access can be
   brokered under the server lease.
3. Grants are per session, like the rest of the binding policy; a later
   "always for this Plugin generation" default would be a separate,
   reviewed change.

## 12. Review record

Revision 1 (Astra Max design review, `call-7d83b8fd2f8aa9a796d359da`):

| Finding | Resolution |
|---|---|
| Direct navigation runs Plugin code with Cowboy's origin (P1) | Response `sandbox allow-scripts`, `Sec-Fetch-Dest: iframe` required, single-use token; tests (§5, §9). The same response sandbox was added to `/app-shell.html` |
| Resource origins authorize remote scripts (P1) | No remote script, style or font source; `connect` is the only network capability (§3, §5) |
| Bridge bound to a browsing context, not a document (P1) | Single-use frame tokens, Cowboy origins refused in grants, later loads remove the frame; tests (§5, §7, §9) |
| App-only tools exposed to the agent (P1) | Separate agent and app projections from signed visibility, enforced in discovery and the relay (§3) |
| Product authentication lacks relay authority (P1) | Calls bound to the widget's recorded relay scope, rechecked at dispatch and delivery, never reopened (§7) |
| Network grants survive loss of revocation (P1) | 30 s renewable lease; removal on failure, expiry, epoch change, sign-out, disconnection (§6) |
| Discovery assumes traffic the Controller never sees (P1) | UI linkage, visibility and read-only from the signed contract; no `resources/read`; real native-binding gate (§3, §10) |
| Initialization deadlock (P1) | `ui/initialize` answered immediately with defined fields; gating after `initialized` (§7) |
| URL cannot identify the resource (P2) | Token binds the resource identity (§5) |
| Compatibility claim vs `allow-same-origin` (P2) | Explicit restricted profile, pinned revisions (§9) |
| Resize floods (P2) | Coalesced, outside the RPC limit (§7) |
| App calls append widgets (P2) | Rows only for agent-originated calls (§8) |
| Write tools vs `widget-apps.md` §11 (P2) | Write tools confirm per call, uncertain outcomes not retried (§4) |
| Phase C inherits phase B admission (P2) | Separate admission table and rows (§9) |

Revision 2 (Astra Max design review, `call-8434093accddc29e773e8e1f`):

| Finding | Resolution |
|---|---|
| The page-held lease cannot revoke direct network access on time (P1) | `connect` deferred; every effect goes through the Controller under a server-enforced lease (§1, §6) |
| Dispatch does not require a live frame lease (P1) | Lease and run identity required at admission, dispatch and delivery; teardown cancels confirmations (§6, §7) |
| Sharing the agent's relay handle (P1) | One view request per scope, busy before transport admission, dispatched requests owned until answered (§7); superseded in revision 4 by a separate view scope |
| Size notifications bypass flood limits (P1) | One ingress budget for every message, then coalescing (§7) |
| Replacement-document proof relies on the parent's `frame-src` (P2) | Stated as a product invariant and tested (§7, §9) |
| Style hashes disable `'unsafe-inline'` (P2) | `style-src-elem` hashes and `style-src-attr 'unsafe-inline'` (§5) |

Revision 3 (Astra Max design review, `call-86e8a1cc8fc3de3726dcad05`):

| Finding | Resolution |
|---|---|
| Shared-scope scheduling lets a view call time out a queued agent call and retire the shared handle (P1) | Views run in their own relay handle and MCP process under the same parent grant, epoch and connection; a view can retire only its own scope; both arrival orders tested (§7, §9) |

Revision 4 (Astra Max design review, `call-76a420469c9aad458d1c8f9c`): the
separate view scope closes the revision 3 finding, and step 1 was judged
ready to implement.

| Finding | Resolution |
|---|---|
| Closing the last view scope cancels a dispatched write (P1) | New calls refused at once; transport close deferred until the dispatched request answers or times out (§7) |
| View processes are never initialized (P2) | The Controller completes and validates `initialize` before the first call on every new or reopened handle; failure refuses without replay (§7) |
