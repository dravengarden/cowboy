# Mandatory device authentication and HTTPS — 2026-10-03

Option 1 is published and activated at
`b06c3153ee2a12e9434ae59a2279624509ba1b41`. See the
[exact receipts and HTTP observations](device-transport-security-2026-10-03.json)
and [protocol/deployment contract](../device-transport-security.md).

External access requires HTTPS/WSS through the local TLS terminator. Browser
account/admin cookies are bound durably to a proven P-256 device key; copied
cookies alone cannot authorize a request. CLI/ACP retain their signed Ed25519
device flow, with redirects disabled and legacy bearer-only credentials retired.
Browser/PWA, service worker, WebSocket, OIDC handoff and protected image paths use
the proof protocol. Same-host signed IPC and deployment probes keep their narrow
local process boundary.

The machine-owned activator committed both releases without recovery:

| Component | Immutable artifact | Transaction |
| --- | --- | --- |
| Web | `/nix/store/hrzlm56pydrkar3s37x3dkj6a73y7v5b-cowboy-web-release` | `1790993365038911002-b06c3153ee2a` |
| Controller | `/nix/store/f2cdh9yn4dm4k1krvv7wkblfwcpjhhiz-cowboy-controller-release` | `1790993374749845259-b06c3153ee2a` |

Public `/healthz`, `/version`, SPA and signer return 200. SPA version/ETag is
`f054e84cb531dffd6c8f1865f75b2be5`, HTML and signer are `no-store`, and the
service worker is `cowboy-v1809`. Successful external responses carry HSTS.
An unsigned login returns 401 with a pre-dispatch proof challenge; retired token
creation returns 410. A request from macbook-air to Hawk's plain HTTP listener,
including a forged `X-Forwarded-Proto: https`, returns 426.

Only the Controller daemon restarted. Hawk Machine PID `1223771`, its start time
and `worker-eed1d8105af00846771d` generation remained unchanged across this
activation; the Machine reconnected and reports online. An independent earlier
Machine release is outside this transaction.

macOS Manager **0.1.6 (7)** was built from `305b910c`, verified and installed at
`/Applications/Cowboy Manager.app` on macbook-air. It retains the existing
separately built `ca7a73c2` bootstrap executables and does not activate them.
The previous Manager bundle and the new build receipt remain in its local
`Library/Application Support/Cowboy/releases` directory. This is a local signed
application update, not a notarized public distribution. No iOS IPA was shipped.

## Verification and limits

The full pinned `just check-compact` gate passed at the security implementation
revision, including all-feature Rust, feature slices, dependency/lint/type
checks, Web tests and isolated PostgreSQL. After integrating main and the small
Clear-focus correction, all **1,993 Web tests**, Web types/lint/build, ten
installer unit tests and the real installer CLI integration passed. Real
Firefox WebCrypto/IndexedDB/HTTPS/WSS conformance passed **16 cases**, including
copied-cookie rejection, replay/target binding, OIDC form POST and Controller
restart with durable bindings. All **43 Manager Swift tests** and bundle
signature checks passed; protocol tests use disposable keys without unlocking
the user's login Keychain.

iOS 26.5 Simulator checks used an exclusive device and synthetic account/session
data. The unchanged shell and a standalone WKWebView harness running the real
native keyboard/clipboard code exercised protected image loading/lightbox,
native Pinyin on marker lines and after images, exact-range text/image paste in
compact/fullscreen, provider-backed clipboard images, source/markdown paths,
selection transfer, and a single submit. Clear's compatibility mouse-down focus
loss was reproduced and corrected; open/cancel/confirm now retain the native
textarea and visible software keyboard, clearing only on confirmation.

Do not describe this as full iOS acceptance. Native long-press menu automation
and the complete Queue/Draft decision/focus sequence remained inconclusive in
the isolated harness. A pasted `**` pair is not evidence for typed auto-pair
deletion, and the image-delete probe first consumed a trailing spacer before
selecting the image. Physical WeType/WeChat IME and the known painted-caret
pitfall #69 remain unverified/open. These limits do not certify an IPA release
or changes to those editor behaviors.

Existing browser/PWA sessions need a hard reload and one fresh primary login to
bind a new session to their local key. Clearing browser storage also requires a
fresh login. The independently scoped Option 2
[research and probe](../wireguard-transport.md) follow this release; they do not
enable a production VPN.
