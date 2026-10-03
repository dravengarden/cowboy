# Mandatory device authentication and HTTPS (Option 1)

Option 1 is the current delivery baseline for both public and private networks.
The [security priorities and roadmap](secure-connectivity-design.md) defer
Option 2 until PWA delivery, bootstrap trust and full application integration
have a reviewed design. No WireGuard product mode is enabled.

Cowboy authenticates the account and the device independently. Stormbird or a
VPN is not required by this protocol. Every external connection uses HTTPS/WSS;
the client validates the Service certificate using its platform TLS trust store.
Cowboy never accepts a remote HTTP request, including one with a forged
`X-Forwarded-Proto: https` header.

## Deployment boundary

The Controller requires `COWBOY_PRODUCT_AUTH_ENABLED=true` (the default) and
at least one HTTPS `COWBOY_PUBLIC_ORIGIN`. Authentication-off startup fails.
TLS terminates at a **same-host reverse proxy**; only a loopback transport peer
can attest HTTPS. Configure that proxy to overwrite forwarded headers. A proxy
on another machine is not supported by this deployment mode. The proxy must
serve the configured certificate and forward WebSocket upgrades.

Same-host Machine control, health/metrics, the public shell health probe and
Machine deployment-health probe, and signed CLI IPC can still use
loopback HTTP without forwarded headers. Browser account/admin cookies are
removed on these local paths. This is a local process trust boundary, not an
external plaintext transport mode; access from any remote IP is rejected.
Hawk already uses a same-host Caddy HTTPS terminator. No WireGuard interface is
created by Option 1.

## Device keys

* CLI/ACP retain their Ed25519 device keys and browser approval flow, with
  rotating refresh tokens and signed access requests. Bearer-only API tokens
  are retired; new token creation returns 410. Use `cowboy login <HTTPS URL>`.
* Browser/PWA/WebView generate P-256 keys through WebCrypto. The private
  `CryptoKey` is non-exportable, stored in origin-local IndexedDB and shared
  across tabs and the service worker. The public key accompanies signed
  requests. This identifies a browser storage profile, not physical hardware.
* macOS Manager uses CryptoKit P-256 and an origin-specific Keychain item with
  `ThisDeviceOnly` accessibility. This is a local software key, not an attested
  Secure Enclave identity.

After a successful primary account login, the Controller binds the newly
issued HttpOnly account/admin cookie hash to the proven public key. This
mapping is durable in SQLite/PostgreSQL. A copied cookie, another key, or a
public key without its account session grants no access. Session revocation,
expiry, roles and Passkey policies continue to apply in the owning handlers.
Cookie rotation creates a fresh binding. Old expired binding rows are pruned
when a new binding is recorded.

Proofs bind the Controller boot epoch, HTTPS origin, public key, method,
path/query, timestamp and random nonce. Signatures are ECDSA P-256/SHA-256 in
raw 64-byte form. Nonces are single-use within a 90-second clock window. A
Controller restart changes the epoch; the browser refreshes its challenge and
retries only a pre-dispatch proof rejection. Enrollment and registered devices
have separate bounded replay budgets. TLS protects request bodies and responses.

Browser fetches use a header; WebSockets carry a proof in an additional
subprotocol while retaining the application protocol. Proofs do not enter
query strings. Native image elements use authenticated fetch and temporary
Blob URLs. OIDC navigations first load a same-origin signing bridge, then
complete the existing authorization transaction with proof; the bridge itself
does not issue account credentials. Password, setup and native handoff cookie
issuance reject missing proofs before dispatch.

The Service operator and same-origin scripts remain trusted. Non-exportability
does not make an origin safe from XSS, and this is not mutual TLS or E2EE.

## Upgrade and verification

Ship the Web signer before activating the enforcing Controller. Existing
unbound browser cookies require one fresh primary login; copying an old cookie
must never silently enroll a new key. Clearing browser storage similarly
requires a new login. A SW version bump delivers the matching frontend.
The macOS Manager binary must include the signer before it can log in to an
enforcing Controller. iOS remote web changes do not constitute an IPA release.

From the pinned shell, run
`just device-browser-conformance <pinned-firefox> <Rust-test-binary> <pinned-certutil>`.
The runner exercises real WebCrypto/IndexedDB, HTTPS and WSS against a disposable
Rust fixture in a private network namespace. It imports an ephemeral fixture CA
only into its temporary Firefox profile and keeps `acceptInsecureCerts=false`.
Both HTTPS and WSS must reject untrusted certificates and wrong hostnames before
application dispatch; the trusted connection then exercises login and device
proofs as a positive control. The host trust store and normal browser profiles
are untouched.

The [2026-10-03 follow-up receipt](experiments/option1-browser-security-2026-10-03.json)
records **22 passing browser checks**, including the original 16 device/login/
restart cases and six TLS validation/context checks, with source and test-binary
hashes. This strengthens the test harness; the earlier fixture's certificate
exception was not a production TLS setting. The follow-up changes no production
runtime and does not constitute Safari/physical-device acceptance.

Unit tests cover forged/tampered proofs, nonce reuse, epoch changes, stolen
cookies and retired credentials. Full repository and PostgreSQL gates remain
part of production release verification.

## Option 2 boundary

Option 2 is **roadmap only**. Its intended scope is a reachable internal Service
with simple domain/IP configuration, excluding VPN discovery, NAT traversal,
mesh/relay infrastructure, system routing and a central network control plane.
Peer provisioning, browser bootstrap and the product transport remain undecided.

The browser experiment uses trusted HTTPS to load the page and WSS to carry WG
packets. It does not establish how a PWA could securely install, cold-start or
update without HTTPS, nor does an address by itself authenticate a Server key.
Those questions, HttpOnly session binding and supported-platform acceptance
must be resolved before resuming implementation. The current HTTPS and device
requirements remain mandatory throughout.

The [browser research](browser-wireguard-transport.md) and
[native interoperability investigation](wireguard-transport.md) retain their
isolated evidence. Static profiles, native helpers and browser carrier proposals
in those notes are research candidates, not accepted product configuration or
a dependency of Option 1 delivery.
