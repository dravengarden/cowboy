# Widget apps, phase C: MCP Apps from bound Plugin servers

Status: design, revision 1 (2026-11-11). Not approved; nothing here is
implemented. It replaces the direction recorded in
[`widget-apps-dom.md`](widget-apps-dom.md) §10 and the phase C line of
[`widget-apps.md`](widget-apps.md) §10.

Phase C renders [MCP Apps](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx)
views (`ui://` resources, `text/html;profile=mcp-app`) in the transcript. The
review of phase B revision 4 set the bar this design answers: trust is a
**host-owned assignment naming the executable resource and the capabilities
it may use**, never a property of the package or of what content says, and
privileged response policies stay separate from phase B's network-denying
`/app-shell.html`.

## 1. Scope

In:

- Views of MCP servers reached through Cowboy's **MCP Plugin bindings**
  ([`mcp-bindings.md`](mcp-bindings.md)): signed `mcp_server` contracts,
  owner-written session policy, the Controller's parent-authorized relay.
  Every `tools/list`, `tools/call` and `resources/read` of such a server
  passes through the Controller, so Cowboy sees which tool declared which
  view and what the call returned.
- **Static** views only: the HTML is a file of the signed Plugin release
  whose digest the contract names (§3). Cowboy serves those bytes; it never
  renders HTML a server returns at run time.

Out (each needs its own design):

- MCP servers an agent connects to natively, outside a binding. Cowboy does
  not see their traffic and cannot attribute their UI.
- Dynamic or server-generated views, and views that need `allow-same-origin`
  or a real origin. These are the "third-party" tier of `widget-apps-dom.md`
  §10 and would run on `thundersparrow.top`, never on Cowboy's origin.
- External URLs (`externalIframes`), `sampling/createMessage`,
  `ui/download-file`, app-registered tools, display modes other than inline.

## 2. Why the binding is the right anchor

The phase B review's objection was that "pinned" or "signed" says who
published bytes, not what those bytes may do: a pinned server may be a third
party, and a trusted server may return agent-generated UI. This design keeps
those apart:

| Question | Answered by | Not by |
|---|---|---|
| Which bytes run | The digest in the signed contract, checked by Machine preparation and again when served (§3) | What `resources/read` returns |
| Which capabilities those bytes get | The owner's session binding policy (§4), CAS-written with the existing epoch | The Plugin's signer, its manifest or the view's `_meta.ui` |
| Whether a call is still allowed | The relay's existing live checks (grant, epoch, both connections) at each request and before delivery | A token held by the frame |

A view therefore gets nothing beyond phase B unless the owner granted it, for
this session, by binding name, resource and capability.

## 3. Signed static views

The `mcp_server` contract (Plugin SDK minor bump, release envelope
unchanged) gains an optional `ui_resources` list, at most 8 entries:

```json
{
  "uri": "ui://suger/status",
  "file": "ui/status.html",
  "sha256": "…",
  "tools": ["suger_status"],
  "csp": { "connect": ["https://api.example.com"], "resources": [] }
}
```

- `file` is inside the signed artifact, ≤ 512 KiB, one self-contained HTML
  document (inline scripts and styles; no relative subresources).
- `tools` lists the contract tools whose `_meta.ui.resourceUri` may name it.
- `csp` declares the most the view may ever be granted, in MCP Apps terms
  (`connectDomains`, `resourceDomains`); HTTPS origins only, no wildcards,
  at most 8 each. `frameDomains` and `baseUriDomains` are refused.
- Machine preparation verifies each digest with the rest of the generation;
  the Controller learns `(plugin_id, generation_digest, uri) → sha256` from the
  prepared binding, never from the server.

At run time the relay inspects `tools/list` results: a tool whose
`_meta.ui.resourceUri` is not a declared `ui_resources` entry for that tool
has the key removed before the agent sees it (and the view never renders).
A `resources/read` of a `ui://` URI by the agent is relayed unchanged; it
is data to the agent, never a view source.

## 4. Assignment: the owner's grant

The binding policy of `PUT /api/sessions/{id}/mcp-bindings` gains an optional
`apps` field per binding:

```json
{
  "name": "suger",
  "server": { "plugin_id": "suger", "generation_digest": "sha256:…" },
  "tools": ["suger_status", "suger_read"],
  "sources": ["github"],
  "apps": [{
    "uri": "ui://suger/status",
    "capabilities": {
      "connect": ["https://api.example.com"],
      "call_tools": ["suger_status"],
      "message": true,
      "open_links": true
    }
  }]
}
```

- Omitted `apps` means no view renders for that binding; its tools work as
  today. This is the default for every existing policy.
- Each capability must be a subset of the contract: `connect` of the
  declared `csp.connect`, `call_tools` of the binding's granted `tools`
  carrying MCP Apps visibility `"app"`. A generation change invalidates the
  grant like any other policy field (new epoch, owner re-confirms).
- The grant is stored, epoch-checked, revoked and tombstoned exactly as the
  existing policy; no new setting key and no provider default or fork
  inheritance.

Capabilities and what an ungranted view gets instead:

| Capability | Granted | Not granted |
|---|---|---|
| `connect` | Listed origins in `connect-src` and `Connection-Allowlist` | `connect-src 'none'`, empty allowlist (phase B) |
| `call_tools` | Listed tools called through the relay without a per-call prompt | Every `tools/call` shows the phase A/B confirmation with the tool name and arguments |
| `message` | `ui/message` → the confirmation already used by `useSend` | Refused |
| `open_links` | `ui/open-link` through Cowboy's link opener (HTTPS only) | Refused |

There is no storage, permission (`camera`, `microphone`, …), download or
`allow-same-origin` capability in this phase.

## 5. Serving a view

A separate exact-path Controller route, distinct from `/app-shell.html`:

```
GET /mcp-app/<session>/<binding>/<grant-epoch>/<sha256>.html
```

- Served only when, at request time, the session's current policy has that
  epoch, the binding grants that `uri`, and the prepared generation maps it
  to that digest; the bytes come from the Controller's verified copy of the
  file. Anything else, any query string or any other path is an empty 404.
  Responses are `no-store`, no cookies, not logged with the path.
- The service worker never answers `/mcp-app/`.
- Headers are built per grant: `script-src` and `style-src` are the hashes
  of the document's inline blocks (computed once at verification), plus
  granted `resources` origins; `connect-src` and `Connection-Allowlist` are
  the granted `connect` origins or none; `frame-src 'none'`,
  `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`,
  `frame-ancestors 'self'`; `Permissions-Policy` disables every feature;
  `CORP: cross-origin` as phase B.
- The frame is exactly `sandbox="allow-scripts"`, as phase B: an opaque
  origin, so the view has no Cowboy cookies, storage or API even though the
  route is same-origin. MCP Apps allows hosts to restrict further than
  `allow-same-origin`; views that require storage do not work here.
  `fetch` to a granted origin is cross-origin from `null` and needs that
  origin's CORS; Cowboy does not proxy it.

The grant epoch in the path makes a revoked or replaced grant's URL dead at
once; a running frame is also removed when the policy watch reports a new
epoch (§6).

## 6. Bridge

The view speaks MCP Apps JSON-RPC to `window.parent` (single-frame
architecture; the spec's double frame is optional and adds nothing here
because Cowboy already serves the document itself).

- Messages are accepted only from that frame's `contentWindow` with origin
  `"null"`; per-frame rate limits as phase B (30/s; flood removes the frame).
  Phase B's rule that any window message after the handshake removes the
  frame does not apply to `/mcp-app/` frames, whose protocol is window
  messages; it still applies to `/app-shell.html`.
- Host → view: `ui/initialize` result (host context: theme, locale,
  `platform: "web"`, container size), `ui/notifications/tool-input`,
  `tool-result`, `tool-cancelled`, `host-context-changed`,
  `ui/resource-teardown`, `ping`. Nothing is sent before
  `ui/notifications/initialized`.
- View → host: `ui/initialize`, `ui/notifications/initialized`,
  `size-changed`, `request-teardown`, `ping`, `notifications/message`
  (recorded, bounded), `tools/call` and `ui/message`/`ui/open-link` per §4,
  `tools/list` (the granted `call_tools` only). Everything else returns a
  JSON-RPC method-not-found error; malformed messages remove the frame.
- A `tools/call` goes from the Web to the Controller as a new
  product-authenticated request bound to the session, binding, epoch and
  widget, and the Controller sends it through the existing relay (same
  grant and connection checks, same 32-session bound). The result returns to
  that frame only.

## 7. Transcript

When the relay delivers a `tools/call` result for a tool with a granted view,
the Controller publishes a system widget row with a new `mcp_app` node
`{binding, uri, sha256, epoch, call}`; tool input and result (each ≤ 64 KiB
of JSON, else the view gets a truncation marker) are stored with the widget
and replayed to the view on start. The row shows the binding name, the
granted capabilities in one line, and Run; like phase B a view starts only
when tapped, counts against the page's runtime slots, and stops out of view.
Where the engine is not admitted (phase B's table) the row shows the tool's
text result.

## 8. Acceptance

- The phase B suite unchanged for `/app-shell.html`.
- A `/mcp-app/` suite on the dedicated Mac Chrome: an ungranted view shows
  zero egress on every phase B channel with an allowing control; a granted
  view reaches exactly its granted origin and nothing else (an undeclared
  sink stays at zero); a stale epoch, wrong digest, query string or other
  path is 404; revoking the grant removes a running frame.
- An unmodified view built with the `ext-apps` SDK renders, receives
  tool input and result, calls a granted tool without a prompt and an
  ungranted one with a prompt.
- Relay tests: undeclared `_meta.ui.resourceUri` stripped; a view's
  `tools/call` refused after epoch change, binding change or connection
  replacement, and for a tool without `"app"` visibility.

## 9. Order of work

1. Contract `ui_resources`, Machine verification, policy `apps` with CAS and
   validation; no rendering.
2. `/mcp-app/` route and headers; the suite's network rows.
3. Bridge, relay-backed `tools/call`, transcript row; the SDK fixture.
4. First real view: a Suger status view, granted on one session.

## 10. Decisions for the owner

1. Static, signed views only in phase C, with dynamic or same-origin views
   deferred to the `thundersparrow.top` tier. (Recommended.)
2. Capabilities offered: `connect`, `call_tools`, `message`, `open_links`;
   no storage or device permissions. (Recommended.)
3. Grants are per session, like the rest of the binding policy; a later
   "always for this Plugin generation" default would be a separate,
   reviewed change.
