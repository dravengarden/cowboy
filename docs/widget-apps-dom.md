# Widget apps, phase B: full-DOM apps on a separate site

Status: design, revision 3 (2026-11-09). Extends [`widget-apps.md`](widget-apps.md)
(phase A, interpreter apps), whose consent broker and outbox this phase reuses.
Owner decisions (2026-11-09): the app content site is the dedicated
registrable domain `thundersparrow.top`, which carries nothing trusted,
reachable only on the Stormbird overlay. Revision 2 answers the design review
of revision 1 (§10).

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
  per-app CPU, memory, GPU or IPC quota, and removing a frame does not
  synchronously kill a renderer. Phase B therefore promises only: the app runs
  in a renderer process separate from Cowboy's on enabled engines, Cowboy can
  always remove it, floods of messages remove it, and at most one DOM app runs
  per page. Removal is best effort: under memory or GPU exhaustion it may come
  late, and a hostile app can slow the device meanwhile. This is weaker than
  phase A and is stated to the person (§6).
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

The app host serves exactly one file per release: the shell document.
Nothing the app does can make a second request that the host or any other
server answers usefully.

- `GET https://<label>.thundersparrow.top/` (exact path, no query) returns the
  shell; every other path, any query string and every other method get an
  empty 404. Caddy logs nothing for this site.
- The shell's only script is inline and authorized by its hash. It loads
  nothing over the network: the parent sends the runtime bytes (React, ReactDOM,
  Three, Sucrase, the bridge) over the verified port, and the shell creates its
  own `blob:` URLs for them and for the compiled app module.
- Response headers:
  - `Content-Security-Policy: default-src 'none'; script-src 'sha256-<shell>'
    blob: 'wasm-unsafe-eval'; style-src 'unsafe-inline' blob:; img-src blob:
    data:; font-src blob: data:; media-src blob: data:; connect-src 'none';
    frame-src 'none'; child-src 'none'; worker-src 'none'; manifest-src
    'none'; form-action 'none'; base-uri 'none'; frame-ancestors
    https://cowboy.stormbird.xyz; webrtc 'block'`
  - `Connection-Allowlist: ()` (empty: no destination, WebRTC included; no
    `response-origin` exception)
  - `Permissions-Policy` listing every feature the targeted engines support
    with an empty allowlist (camera, microphone, geolocation, display-capture,
    usb, serial, hid, bluetooth, midi, payment, publickey-credentials-*,
    clipboard-read/write, fullscreen, screen-wake-lock, speaker-selection,
    autoplay, idle-detection, local-fonts, …); the doc of the release lists
    them exactly. Disabling media permissions does not disable WebRTC data
    channels; Connection-Allowlist does.
  - `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`,
    `Cache-Control: no-store`, `Cross-Origin-Resource-Policy: cross-origin`
    (the shell is framed cross-site; it carries no data).
- Self-navigation (the frame navigating itself, possibly with data in the
  URL) is a request Connection-Allowlist must govern; §7 proves it per engine.
  An empty 404 and disabled logging do not make such a request harmless (its
  URL has already left the page): any request after the shell fails
  admission.
- Cowboy's App CSP gains `frame-src 'self' https://*.thundersparrow.top`
  (its `default-src 'self'` otherwise forbids the frame); nothing else in the
  Cowboy policy widens.

## 4. Frame, handshake and generations

- The parent mounts `<iframe sandbox="allow-scripts" allow="<every feature
  ()>" referrerpolicy="no-referrer" credentialless src="https://<label>…/">`
  with a fresh random `<label>` per mount (process grouping on Chromium
  follows the precursor origin; measured in §7), a fresh nonce and a fresh
  generation. Opaque origin: no cookies, storage or service worker.
- Handshake, accepted once, with a 5 s timeout:
  1. The shell posts `ready` to `parent` with target origin
     `https://cowboy.stormbird.xyz`; the parent accepts it only from
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

## 8. Infrastructure (owner confirmation before activation)

- Zone `thundersparrow.top` is registered in Columbus
  (`resources/cloudflare/dns/thundersparrow-top`).
- Private DNS: `*.thundersparrow.top` → Hawk's overlay address (Stormbird
  config project); no public records for app names.
- TLS: Caddy wildcard certificate by DNS-01 with a token scoped to the zone
  (Zone.DNS:Edit), stored like `/etc/cloudflare/caddy.env`.
- Caddy vhost: the shell at `/` exactly with §3 headers, empty 404 otherwise,
  no access log.
- Egress sink (Hawk overlay plus a public host) for the suite only.

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
2. Infrastructure (§8) after owner confirmation; the suite against the real
   host; the first admission row.
3. Contract, Controller validation, Web frame host and registry admission.

## 10. Review record

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
