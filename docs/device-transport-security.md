# Mandatory device authentication and HTTPS (Option 1)

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

`just device-browser-conformance <pinned-firefox> <Rust-test-binary>` runs real
WebCrypto/IndexedDB, HTTPS and WSS against a disposable Rust fixture in a private
network namespace. Its temporary certificate exception applies only to that
disposable profile. Unit tests cover forged/tampered proofs, nonce reuse, epoch
changes, stolen cookies and retired credentials. Full repository and PostgreSQL
gates remain part of release verification.

## Option 2 boundary

Option 2 is scoped to a statically configured WireGuard data plane on a LAN
where devices can reach the server UDP listener. Its endpoint can be a hostname
or IP; a pinned peer public key identifies the server. WireGuard keys remain
separate from browser/CLI account keys, and HTTPS/device authentication remain
mandatory inside the tunnel.

Network administration owns peer profiles, address assignment, routes and VPN
peer removal. Cowboy does not own discovery, central enrollment, NAT traversal,
relays or a peer/address control plane. An existing system tunnel can supply
connectivity without a new Cowboy transport implementation; an optional Rust
helper would manage only a local static tunnel's lifecycle. The present release
does not implement that helper or promise browser access without a VPN client.

The initial Rust implementation comparison, configuration proposal and successful
isolated interoperability probe are recorded in
[Self-managed WireGuard transport](wireguard-transport.md).
