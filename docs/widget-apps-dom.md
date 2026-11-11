# Widget apps, phase B: full-DOM apps in an opaque sandbox

Status: design, revision 5 (2026-11-11). Extends [`widget-apps.md`](widget-apps.md)
(phase A, interpreter apps), whose consent broker and outbox this phase reuses.
Owner decisions: 2026-11-09, apps run on a separate site, then 2026-11-11,
after the same-origin experiment of §7.3, no separate domain for phase B: the
shell is a Cowboy route, isolated by the opaque sandbox, its own response
headers and Chromium's sandboxed-frame process isolation. `thundersparrow.top`
stays registered for phase C (§10). Revisions 2-5 answer the design reviews
(§11).

## 1. Goal, invariants and what a browser frame cannot give

Goal: an agent can publish an app that uses the real DOM (HTML, CSS, canvas,
WebGL, SVG, DOM events), drawn with its own pixels in a widget row.

- **I1, no credentials**: the app never reaches Cowboy's cookies, storage,
  DOM, tokens, native bridges or Controller API.
- **I2, no egress**: nothing the app holds, including text the person types,
  leaves the device except inside a `send` the person confirms. Closed by the
  engine and proven by observation outside the page (§7); script patches the
  app could undo do not count.
- **I3, bounded harm (best effort)**: a browser iframe offers no enforceable
  per-app CPU, memory, GPU or IPC quota, removing a frame does not
  synchronously kill a renderer, and page JavaScript cannot attest which
  process a frame runs in. On admitted configurations Chromium puts the
  sandboxed frame in its own renderer (§7 proves it for the suite's
  configuration, including a negative test with that isolation disabled);
  on the person's own browser it is expected, not attested. Containment
  therefore also rests on the product: a DOM app never starts by itself, it
  shows its title and a Run button and starts only when the person taps it
  (after any reload nothing runs until tapped again); at most one runs per
  page; floods and missed heartbeats remove it, best effort. A hostile app can
  still slow or freeze the page until it is removed or the page is reloaded.
  This is weaker than phase A and is stated to the person (§6).
- **I4, fail closed**: only engine, OS and policy-version combinations listed
  in the phase B admission table (§7) run DOM apps; everything else shows the
  `fallback`. Phase A admission never implies phase B admission.

Non-goals: third-party MCP Apps (phase C), network or Cowboy tools, storage
across reloads, links leaving the app.

## 2. Why phase A cannot simply grow

Phase A's code has no network or DOM API at all. A DOM page has the whole
platform: CSP does not cover WebRTC in any shipped browser (the CSP `webrtc`
directive is unimplemented), and a fresh realm (a same-origin `about:blank` or
srcdoc frame) undoes any constructor deletion. Chromium 152 ships Connection
Allowlists: a `Connection-Allowlist` response header restricting connections a
document, its local child documents and local workers initiate, WebRTC
included. Phase B is built so that an empty allowlist plus a strict CSP leaves
the app no network path at all, including to its own host.

## 3. Network-free boot

Cowboy serves exactly one document for DOM apps: the shell. Nothing the app
does can make a second request that any server answers usefully.

- `GET /app-shell` (exact path, no query) on Cowboy's own origin returns the
  shell, a file of the Web release; any other path below it, any query
  string and any other method get an empty 404. The Controller answers this
  route with the headers below instead of Cowboy's document headers, and
  never logs its query.
- The shell's only script is inline and authorized by its hash. It loads
  nothing over the network: the parent sends the runtime bytes (React, ReactDOM,
  Three, Sucrase, the bridge) over the verified port, and the shell creates its
  own `blob:` URLs for them and for the compiled app module.
- Response headers of `/app-shell`, replacing (not adding to) Cowboy's:
  - `Content-Security-Policy: default-src 'none'; script-src 'sha256-<shell>'
    blob: 'wasm-unsafe-eval'; style-src 'unsafe-inline' blob:; img-src blob:
    data:; font-src blob: data:; media-src blob: data:; connect-src 'none';
    frame-src 'none'; child-src 'none'; worker-src 'none'; manifest-src
    'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self';
    webrtc 'block'`
  - `Connection-Allowlist: ()` (empty: no destination, WebRTC included; no
    `response-origin` exception)
  - `Permissions-Policy` listing every feature the targeted engines support
    with an empty allowlist; the release lists them exactly. Disabling media
    permissions does not disable WebRTC data channels; Connection-Allowlist
    does.
  - `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`,
    `Cache-Control: no-store`, no `X-Frame-Options` (framing is governed by
    `frame-ancestors 'self'`), no `Set-Cookie`.
- Self-navigation (the frame navigating itself, possibly with data in the
  URL) is a request Connection-Allowlist must govern; §7 proves it per engine.
  Any request after the shell fails admission, whatever the route answers.
- Cowboy's App CSP gains `frame-src 'self'` (its `default-src 'self'` already
  implies it; stated explicitly so a later narrowing cannot drop it); nothing
  else in the Cowboy policy widens.
- **The frame is never same-origin.** Because the shell shares Cowboy's
  origin, `allow-same-origin` on this frame would hand the app Cowboy's
  cookies, storage and API. The frame's sandbox is exactly `allow-scripts`,
  enforced by a unit test of the frame host and by a check in the
  conformance suite, and phase B adds no other sandbox token.

## 4. Frame, handshake and generations

- The parent mounts `<iframe sandbox="allow-scripts" allow="<every feature
  ()>" referrerpolicy="no-referrer" src="/app-shell">` with a fresh nonce and
  a fresh generation per mount. Opaque origin: no cookies, storage or service
  worker. The shell document's request carries Cowboy's cookies like any
  same-origin navigation; the route ignores them and the document cannot read
  them.
- Handshake, accepted once, with a 5 s timeout:
  1. The shell posts `ready` to `parent` with target origin
     its own URL's origin (Cowboy's, read from `location` before anything
     else runs); the parent accepts it only from
     `event.source === frame.contentWindow`, `event.origin === "null"`, for the
     current generation, before any other message from that source.
  2. The parent transfers one `MessagePort` with the nonce and generation.
  3. The shell checks `event.source === parent`, the exact parent origin and
     exactly one port, then echoes nonce and generation on the port; only then
     does the parent send runtime, code, props and images.
- The parent binds the port to its own record (session, widget, node, code
  version, runtime kind, generation); the guest never names any of these.
- Any `load` event after the first, a reload, replacement, closure,
  sign-out or removal revokes the generation: the port closes, pending
  unconfirmed requests are discarded and late messages are ignored. Confirmed
  sends already in the outbox keep phase A's retry semantics.
- Every guest message (height, pong, error text, send) is untrusted input,
  bounded and validated; guest error text is never sent to telemetry.

## 5. Messages, floods and lifetime

- The guest can call `parent.postMessage` directly, bypassing the port. The
  parent counts every window message whose source is a DOM frame; any message
  after the handshake other than through the port, or more than 30 port
  messages per second, removes the frame. Delivery cost already incurred is
  accepted (I3).
- Heartbeat over the port every second; three misses remove the frame.
- `send(text)` follows phase A §7 exactly (one confirmation open, one per 5 s,
  exact-text confirmation, outbox, record-before-queue).
- One DOM app per page, admitted against the same page quota as phase A
  runtimes (starting and retained runtimes count). The inline row and the
  inspector never both mount a frame: the inspector shows the DOM app and the
  row a placeholder, and moving between them restarts the app (DOM state is not
  preserved; the person is told before moving if the frame ever had focus).
- No suspend: a hidden page or off-screen row removes the frame after the
  release grace. "Dirty" is conservative: once the frame has had focus, the
  app counts as holding input, and removal shows "The app's state was
  discarded" afterwards; prompts never delay a removal.

## 6. Contract

- A new node type `dom_app` `{id, title, fallback, code, props?, images?,
  height?}`: older clients parse unknown node types as unsupported and draw
  their fallback, never running DOM code in QuickJS. Adding the node type to
  the send check keeps existing `app` hashes and outbox retries valid.
- Bounds as phase A's `app` (code ≤ 14 KiB, props ≤ 6 KiB, images ≤ 8 still
  PNG, document ≤ 24 KiB); one app (`app` or `dom_app`) per widget.
- The app identity includes the node type, so a `dom_app` never shares a
  runtime with an `app`; the Controller's send check (`code_sha256`) also
  binds the node type (`app_updated` if it changed).
- Imports: `react`, `react-dom/client`, `three`, `@cowboy/dom` (`useSend`,
  `useHostContext`, size). The row header says "Full app: runs its own code;
  Cowboy can stop it" (I3 disclosure).

## 7. Admission and acceptance

Phase B admission is a table of (engine, version range, OS, shell, header
policy version) rows proven by the `widget-apps-dom` suite; it starts empty.
The suite runs against the real app host configuration and records evidence
per policy version:

- **Positive controls first.** With the production headers, the shell boots,
  every probe executes and reports. A second fixture with an allowing policy
  and synthetic data shows each probe can reach the sink, so silence under the
  production policy means "blocked", not "never ran".
- **Observation outside the page.** An egress sink on a Hawk overlay address
  and on a public host logs requests and payloads (not just connections),
  answers TLS, HTTP/1-3, STUN and TURN (UDP, TCP, TLS) enough to drive later
  stages, and is the authoritative DNS server of a probe subdomain so DNS
  queries are observed. The app host's own access is observed too.
- **Channels**: script/style/font/image/media loads (including `@import`,
  `FontFace`, SVG resources, media posters and tracks, Blob-wrapped
  secondaries) to the sink, the app host and Cowboy; fetch-like APIs;
  `preconnect`, `dns-prefetch`, `preload`, `modulepreload`, speculation rules
  and prerender; self-navigation with a query, meta refresh, links, top
  navigation, popups, downloads and external schemes; WebRTC data channels
  with STUN/TURN and preset remote descriptions without `getUserMedia`;
  WebTransport; nested realms (initial `about:blank`, srcdoc, blob and data
  documents, `document.write`); classic, module and nested workers,
  SharedWorker (refused by `worker-src 'none'`; recorded) and worklets
  (AudioWorklet and PaintWorklet modules follow `script-src`, so blob worklet
  modules are allowed: exercise them, their module dependencies and teardown,
  and admit only with containment shown); browser-mediated
  services (e.g. Web Speech remote voices) where present; and Cowboy's own
  telemetry for guest-originated text.
- **Containment**: the frame runs out of process (Chromium task manager or
  process introspection), a message flood and a missed heartbeat remove it,
  and the Cowboy page stays responsive while the app burns CPU.

Initial target: desktop Chrome ≥ 152 (the Mac's disposable Chrome profile
for the prototype). WebKit (Safari, PWA, iOS shell) stays out until a
mechanism closes WebRTC and frames are process isolated there; Lockdown
Mode's peer-connection switch and WKWebView security restriction modes are
candidates to investigate for the native shell only.

### 7.1 Prototype evidence (2026-11-09)

Step 1 of §9, Chrome 155.0.8059.86 on the Mac, disposable browser context,
fictitious origins served by DevTools interception, a logging sink on the
Mac's loopback (UDP and TCP 3478, TCP 8899). The frame is
`sandbox="allow-scripts"` from the app host; the probe runs as the shell's
hashed inline script.

| Probe | Control (no headers) | §3 headers (`Connection-Allowlist: ()` + CSP) |
|---|---|---|
| `RTCPeerConnection` data channel, STUN UDP and TURN TCP at the sink | Sink received STUN datagrams and a TURN TCP connection | Sink received nothing; host candidate only |
| `<link rel=preconnect>` to the sink | Two TCP connections | Nothing |
| Self-navigation to the app host with data in the query | Host received `/?leak=self-navigation` | No request |
| Nested `about:blank` realm (`RTCPeerConnection`, `fetch`) | `SecurityError`: the nested frame of an opaque sandbox is cross-origin to it | Same |
| `fetch` and image to the loopback sink | Refused by Chrome's Local Network Access before any policy | Refused |

So on this Chrome the combined headers blocked the tested STUN UDP, TURN TCP,
preconnect and app-host self-navigation attempts. The nested-realm row shows
only that the outer opaque frame cannot borrow constructors from that child;
it does not show a probe running inside nested documents. Not shown: the full
boot (runtime transfer, blob module graph, React and Three rendering, blob
images and fonts), nested-document paths with working controls, worklets,
fetch-like and navigation channels against a public sink (the loopback
control was refused by Local Network Access), DNS, process separation and
flood recovery, and every engine other than desktop Chrome.

### 7.2 Steps 1a and 1b (2026-11-09)

Chrome 155.0.8059.86 (Mac), disposable context; the parent page carried
Cowboy's App CSP plus `frame-src` for the app host; the child carried the §3
headers and `sandbox="allow-scripts"`. An HTTP sink (`https://sink.invalid`,
answered by DevTools interception, which sits after CSP and
Connection-Allowlist and before the network, and is not subject to Local
Network Access) and the loopback STUN/TURN sink observed egress.

- **1a, full boot**: the handshake completed; the runtime (React, ReactDOM,
  Three, Sucrase; 1.1 MB) arrived over the port; the TSX compiled into blob
  modules linked through blob shims; React rendered; Three drew a WebGL frame
  (sampled pixel `34,68,255`, the scene's background); a blob font and a blob
  image loaded; a click updated state; `send` reached the parent. The child's
  only requests were `blob:` loads: no network request, no sink hit.
- **1b, nested documents and worklets**, with an allowing control (no
  sandbox, no headers) in which every probe ran and reached a sink:

| Probe (from the app's code) | Control | §3 policy |
|---|---|---|
| `fetch` to the sink | Sink hit | No hit |
| Initial `about:blank` child running `fetch` | Executed; sink hit | `SecurityError` (cannot run in it) |
| `document.write` child document | Sink hit | `TypeError` (cannot write) |
| srcdoc, blob and data child documents | Sink hits | No hits |
| AudioWorklet and PaintWorklet module importing from the sink | Both imports hit | No hits (the AudioWorklet import was initiated but never reached interception) |
| Worker | Sink hit | No hit |
| WebRTC STUN from every nested realm | 20 sink observations | None |

Steps 1a and 1b pass on this Chrome. Step 2 repeats the egress suite
against the real host with network sinks, DNS observation and the lifecycle
and containment checks; admission stays empty until it passes.

### 7.3 Same-origin shell (2026-11-11)

The 7.2 suite with the shell served from the parent's own origin
(`/app-shell`) instead of a separate site, same headers and sandbox:

- the sandboxed frame still attached as its own `iframe` target, i.e. an
  out-of-process frame (Chromium isolates sandboxed frames); the control
  without `sandbox` did not;
- the full boot passed (React, Three, blob font and image, click, `send`)
  with no network request;
- every nested-document, worklet, worker and WebRTC probe reached no sink,
  while the allowing control reached every sink.

### 7.4 First admission (2026-11-11)

`tools/dom-apps-conformance.ts` against this tree: the real `/app-shell.html`
of a disposable Controller (a query string answered 404), the production
frame host, and a dedicated headless Google Chrome 157.0.8103.39 on the Mac
(own profile, DevTools port and network log).

| Mode | Rendering | Out-of-process frame | Intercepted requests | Network-log mentions of the run | STUN/TURN |
|---|---|---|---|---|---|
| `policy` | React, Three pixel `34,68,255`, image, click | yes | 0 | 0 | 0 |
| `control` (shell without its security headers) | same | yes | 9 channels (fetch, image, prefetch, blob/srcdoc/data documents, both worklets, worker) | 71 | 18 |
| `no-isolation` (`--disable-features=IsolateSandboxedIframes`) | same | **no**: detected | 0 | 0 | 0 |

The app's self-navigation removed its frame. After the implementation review
(`call-a92d7ad39b50a2475de490cc`) the verdict requires every probe to have
reported, every control channel to be observed and both logs to exist (a
missing log fails the run); a rerun passed all three modes (`control`: 9
channels, 62 network-log mentions, 18 STUN/TURN). The same review changed the
product: a `dom_app` send identity is `dom_app:<hex>` (no `app` code can
produce it), imports link through Sucrase's imports transform and a mapping
`require` (no source text is rewritten), props that arrive during startup are
replayed, a stopped frame drops its unconfirmed requests, Run reserves the
full-app ownership and one of the page's shared runtime slots, and an app out
of view or on a hidden page stops after 1.5 s. Admission row: Google Chrome on
macOS, major ≥ 157 (`domAppsEnabled.ts`). Later majors are admitted on the
owner's direction to keep practical use; the suite is rerun on them and a
failure removes the row. Every other engine shows the fallback.

### 7.5 Product row (2026-11-11)

The `policy` run now also drives the product's `dom_app` node inside
`WidgetView` (`web/src/domAppsProductConformance.tsx`) on the same dedicated
Chrome 157: no frame before Run; Run starts an `allow-scripts` frame whose
WebGL app's send reaches the person's confirmation with its exact text and,
once confirmed, carries a `dom_app:<hex>` identity; a second Run stops the
first (one full app per page); Stop removes the frame; an app pushed out of
view is stopped after its grace. All four passed with the run's egress,
network-log and STUN/TURN counts still 0. The verdict requires them.

The shell response also carries `sandbox allow-scripts` in its CSP (from the
phase C review, `call-7d83b8fd2f8aa9a796d359da`), so it is opaque even when
opened top-level or framed without the attribute; `policy` and
`no-isolation` passed again with it.

## 8. Infrastructure

None in production beyond Cowboy: the shell is a Web release file and a
Controller route. First admission still requires the full §7 evidence, run in
a disposable Chrome launched for the suite on the test machine (its own
profile directory, flags and DevTools port; never the person's or the
bridge's browser):

- Chrome's network log (`--log-net-log`) as the authoritative record of every
  host resolution and socket the browser opened during the probes, and
  DevTools interception of every URL (any host, Cowboy's included) for
  request contents;
- STUN/TURN listeners (UDP, TCP, TLS) for WebRTC, which the network log does
  not cover;
- process evidence: the frame attaches as its own out-of-process target; a
  second run with `--disable-features=IsolateSandboxedIframes` must make the
  suite fail (the negative control for the isolation check).

The DevTools-interception and loopback results of §7.1-§7.3 are prototype
evidence only.

## 9. Order of work

1. Prototype on the Mac's Chrome with the §3 headers (fixture origins):
   a. the full boot: exact parent CSP, hashed shell, port verification,
      transferred runtime, TSX compiled into blob modules, React and Three
      rendering, an event and a consent request, child-created image and font
      blobs, and no child network request after the shell;
   b. nested documents (initial `about:blank`, srcdoc, blob and data
      documents, `document.write`) and worklets, each with an allowing control
      that runs a probe inside it.
   Stop here if either fails.
2. Contract, Controller route and validation, Web frame host and registry
   admission, with the `widget-apps-dom` suite run against the real route on
   a disposable local stack.
3. The first admission row (desktop Chrome on the Mac, the version the suite
   passed on), then real-use checks on Cowboy.

## 10. Phase C trust tiers (owner direction, 2026-11-11; not yet approved)

Superseded by the phase C design draft, [`widget-apps-mcp.md`](widget-apps-mcp.md).

Phase C renders MCP Apps (`ui://` resources) from MCP servers. Most of them
will be Cowboy's own servers; the owner asked that those not be held to the
third-party bar. Trust follows who wrote the code, never what content says:

- **Agent-written** (`app`, `dom_app`): untrusted always, since a prompt
  injection can author them; phases A and B as designed.
- **First-party MCP Apps**: served by MCP servers whose package Cowboy
  verifies (a signed Cowboy plugin or a server pinned in Cowboy's own
  configuration). Same opaque frame and same-origin shell, never
  `allow-same-origin`, but the policy may grant the app's declared network
  origins (`connect-src` and `Connection-Allowlist` list them), its own
  server's tools without a confirmation per call, host-backed storage over the
  port, and links opened through Cowboy.
- **Third-party MCP Apps**: everything of the agent-written tier plus the MCP
  Apps contract; declared domains shown to the person; tools confirmed per
  call; and, if an app needs `allow-same-origin` or a real origin, it runs on
  `thundersparrow.top`, never on Cowboy's origin.

Review of revision 4 (`call-26918e60062dfe76ed7d5b86`): package verification
is not authority for privileged execution, since a pinned server may be a third
party and a trusted server may return agent-generated or external UI. Before
phase C, trust is a host-owned assignment naming the executable resource and
the capabilities it may use, and privileged response policies stay separate
from phase B's network-denying `/app-shell` policy. This section is a
direction, not an approved design.

## 11. Review record

Revision 1 (Astra Max design review, `call-79e62aad9a656fd53c04ecdc`):

| Finding | Resolution |
|---|---|
| `Connection-Allowlist` syntax invalid; scripts, styles and self-navigation still reach the app host (P1) | Empty allowlist `()`; inline hashed shell, runtime sent over the port, no host-loaded assets; exact-path host with empty 404s and no logs; self-navigation proven per engine (§3) |
| A frame is not a resource boundary; direct `parent.postMessage` bypasses the bridge (P1) | I3 restated as best effort and disclosed; window-message and port flood removal; one DOM app per page (§1, §5) |
| Parent CSP lacks `frame-src`; `CORP: same-site` wrong for an opaque document (P1) | `frame-src` for the app host only; `CORP: cross-origin` on the data-free shell; no subresources (§3) |
| Handshake not bound to a document generation (P1) | Generation per mount, one-shot handshake with timeout, parent-held identity, revocation on any later load (§4) |
| Probes could pass silently; phase A admission reused (P1) | Positive controls, payload/DNS observation, own-host and telemetry probes; separate, initially empty admission table (§7) |
| 32 KiB code bound and runtime field break the contract (P2) | New `dom_app` node type, phase A bounds, identity and send check include the type (§6) |
| Registry, suspend and dirty semantics do not fit a real iframe (P2) | One DOM app per page, inspector-or-row ownership, no suspend, conservative focus-based dirty, prompts never delay removal (§5) |

Revision 2 (Astra Max design review, `call-97039bf2b3294d7f04bccc4e`): the
design is viable; implementation waits for steps 1a and 1b. Changes in
revision 3: worklets follow `script-src` and are exercised, not assumed
refused (§7); a request after the shell fails admission whatever the host
answers (§3); removal is best effort (§1); the prototype's conclusions are
narrowed (§7.1); unknown node types fall back as unsupported (§6).

Revision 4 (owner decision, 2026-11-11): the separate site is dropped for
phase B after §7.3 showed the same isolation and egress results with a
same-origin shell; the price is a Controller route with its own headers and
the rule that the frame is never `allow-same-origin` (§3). Phase C trust
tiers recorded (§10).

Revision 5 (review of revision 4, `call-26918e60062dfe76ed7d5b86`):
process isolation is not attested on the person's client, so DOM apps start
only when tapped and the suite adds a forced-disabled negative control (§1,
§8); first admission keeps the full network evidence through a disposable,
net-logged Chrome (§8); phase C trust tiers are deferred until trust is a
host-owned assignment per resource and capability (§10).
