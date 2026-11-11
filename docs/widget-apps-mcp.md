# Widget apps, phase C: MCP Apps from bound Plugin servers

Status: design, revision 2 (2026-11-11). Not implemented. It replaces the
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
remote scripts, styles, images or fonts; external URLs (`externalIframes`),
`sampling/createMessage`, `ui/download-file`, `resources/read` from a view,
app-registered tools and display modes other than inline.

## 2. Authority model

| Question | Answered by | Not by |
|---|---|---|
| Which bytes run | The digest in the signed contract, verified by Machine preparation and again when served (§3) | Anything a server returns |
| What they may do | The owner's per-session grant (§4), CAS-written with the existing policy epoch | The signer, the manifest or the view's `_meta.ui` |
| Which document holds it | A single-use frame token bound to one resource, digest, epoch and widget (§5) | The frame's `contentWindow` or its `"null"` origin |
| Whether it still holds it | A renewable browser lease (§6) and the relay's live checks at dispatch and delivery (§7) | A URL that happens to return 404 later |

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

The `mcp_server` contract (Plugin SDK minor bump) gains:

- `ui_resources`, at most 8: `{uri, file, sha256, connect}`. `file` is inside
  the signed artifact, ≤ 512 KiB, one document with inline scripts and styles
  only (no `src`/`href` subresources, checked at signing and verification).
  `connect` lists at most 8 HTTPS origins the view may ever be granted.
  Cowboy's own public origins and aliases are refused at signing and again at
  grant time.
- Per signed discovery entry, `ui: {resource?, visibility}` where
  `visibility` is `["model"]`, `["app"]` or both (omitted = both, as MCP
  Apps), and `read_only` mirroring the tool's `readOnlyHint` (omitted =
  false).

Machine preparation verifies each digest with the rest of the generation and
computes inline-script and style hashes once; the Controller learns
`(plugin_id, generation_digest, uri) → {sha256, hashes, connect}` only from
the prepared binding.

Two projections replace "the granted tools":

- **Agent projection**: granted tools with `"model"` visibility. Native
  descriptors list only these; the relay refuses an agent `tools/call` to
  any other tool. Adding a view grant never widens the agent's tools.
- **App projection**: granted tools with `"app"` visibility that the view's
  grant lists in `call_tools` (§4). The relay refuses a view's call to any
  other tool, including cross-binding calls.

## 4. Assignment: the owner's grant

The binding policy of `PUT /api/sessions/{id}/mcp-bindings` gains an optional
`apps` field per binding; omitted means no view renders (the default for every
existing policy):

```json
"apps": [{
  "uri": "ui://suger/status",
  "capabilities": {
    "connect": ["https://api.example.com"],
    "call_tools": ["suger_refresh"],
    "message": true,
    "open_links": true
  }
}]
```

- `connect` ⊆ the resource's signed `connect`; `call_tools` ⊆ the app
  projection. A generation change invalidates the grant like any other policy
  field.
- Storage, epoch, CAS, revocation, tombstones and non-inheritance are exactly
  the existing policy's.

| Capability | Granted | Not granted |
|---|---|---|
| `connect` | Listed origins in `connect-src` and `Connection-Allowlist` | `connect-src 'none'`, `Connection-Allowlist: ()` |
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
  script-src <inline hashes>; style-src <inline hashes> 'unsafe-inline';
  img-src data: blob:; font-src data:; connect-src <granted connect | 'none'>;
  frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none';
  base-uri 'none'; form-action 'none'; frame-ancestors 'self'`,
  `Connection-Allowlist` listing the granted origins or `()`,
  `Permissions-Policy` disabling every feature, `CORP: cross-origin`. The
  response `sandbox` keeps the document opaque even if opened top-level or
  framed without the attribute; the frame is also exactly
  `sandbox="allow-scripts"`.
- No grant contains a remote executable or style source: everything that
  runs is the signed bytes.

## 6. Lease and revocation

A running view holds a lease of 30 s, renewed every 10 s by the Web over its
authenticated connection; renewal rechecks the grant, epoch, widget and the
relay scope (§7). The parent removes the frame when renewal fails or the
lease expires, on a policy-watch epoch change, on sign-out, and when the
page loses its Controller connection. A browser that keeps Internet access
but loses Cowboy therefore keeps a granted network origin for at most one
lease.

## 7. Bridge

The view speaks MCP Apps JSON-RPC to `window.parent` (single frame; Cowboy
already serves the document, so the spec's proxy frame adds nothing).

- Accepted only from that frame's `contentWindow` with origin `"null"`. The
  first `load` is the view; any later `load` removes the frame. Because the
  token is single-use and grants never include Cowboy's origins, a
  replacement document that posts before its `load` can only be one the view
  itself created (blob or data), which inherits its CSP and holds no more
  than the view already had.
- Handshake: the view's `ui/initialize` request is answered immediately with
  `protocolVersion` (the pinned spec revision), `hostInfo`
  (`{name: "Cowboy", version}`), `hostCapabilities` listing exactly the
  granted capabilities, and `hostContext` (theme, locale, `platform: "web"`,
  container size). Nothing else is sent before
  `ui/notifications/initialized`; then tool input and result are replayed.
- Host → view: `ui/notifications/tool-input`, `tool-result`,
  `tool-cancelled`, `host-context-changed`, `ui/resource-teardown`, `ping`.
- View → host: `ui/notifications/size-changed` (coalesced to one per animation
  frame and never counted against the RPC limit), `request-teardown`,
  `ping`, `notifications/message` (recorded, bounded), `tools/list` (the
  view's app projection), `tools/call`, `ui/message`, `ui/open-link` per §4.
  Other requests get method-not-found; RPC beyond 30/s or malformed messages
  remove the frame.
- A view `tools/call` goes to the Controller with the frame token. The
  widget records the **relay scope** that produced it (the native client's
  relay handle with its parent grant and both connection incarnations); the
  Controller dispatches through that scope only, rechecking the app grant,
  the confirmation (if required) and the scope at dispatch and again before
  delivery. A closed or replaced scope refuses the call; it is never reopened
  against current connections or another MCP process.

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
- `/mcp-app/` network rows: an ungranted view shows zero egress on every
  phase B channel with an allowing control; a granted view reaches exactly
  its granted origin and an undeclared sink stays at zero;
- direct top-level navigation, unsandboxed embedding, a reused token,
  `Sec-Fetch-Dest` other than `iframe`, a query string or another path all
  get 404 or an opaque document with no bridge;
- a view navigating its frame to another view's URL, and a blob document
  posting before `load`, gain no bridge;
- revocation while the browser is disconnected from the Controller removes
  the frame within one lease; sign-out and policy-epoch change remove it;
- the pinned SDK's initialize sequence, sustained auto-resize without
  removal, a granted read-only call without a prompt, a write call with one,
  an ungranted call refused;
- the service worker and an offline reload never serve `/mcp-app/`.

## 10. Order of work

1. Contract `ui_resources`, discovery `ui` and `read_only`, Machine
   verification; agent and app projections enforced in native descriptors and
   the relay; policy `apps` with CAS and validation. No rendering.
2. Frame tokens, `/mcp-app/` route and headers, the lease; the suite's
   network and navigation rows.
3. Bridge, scope-bound `tools/call`, transcript row; the SDK fixture rows.
4. A real native-binding test with the retained execution executable (not a
   standalone SDK view), then the first view: a Suger status view granted on
   one session.

## 11. Owner decisions (2026-11-11)

The owner accepted the recommendations:

1. Static, signed views only in phase C; dynamic or same-origin views are
   deferred to the `thundersparrow.top` tier.
2. Capabilities offered: `connect`, `call_tools`, `message`, `open_links`;
   no storage or device permissions.
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
