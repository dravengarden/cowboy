# Authentication Plugins

Status: normative public API for product-login extensions.

Cowboy Authentication Providers are signed, data-only `.cowboy-plugin`
packages. A package declares an identity protocol and presentation; it does not
execute in the Controller, read the database, receive a client secret, choose a
local account, or issue a Cowboy session. Cowboy's built-in drivers retain all
of those effects.

This is intentionally the same Plugin publication boundary used by Agent
Providers and code intelligence. `authentication_provider` is a payload kind,
not a parallel plugin system.

## Trust and configuration split

The public package contains:

- an exact plugin ID, version, publisher, component release, and contract
  fingerprint;
- display and button labels;
- HTTPS protocol endpoints, scopes, allowed client-authentication methods,
  accepted ID-token algorithms, and non-reserved authorization parameters; and
- no credentials, account mapping, email address, private key, or deployment
  secret.

The protected Controller configuration contains:

- an exact `(plugin_id, plugin_version, artifact_digest)` selection from the
  signed Plugin Catalog;
- the registered client ID and exact redirect URI;
- one exact upstream `sub` mapped to one existing Cowboy account; and
- only the credential fields required by the selected method, referenced by
  absolute mode-`0600` files.

The Controller rejects symlinks, group/world-readable files, unknown fields,
unpinned or unsigned packages, algorithm downgrade, duplicate methods, reserved
authorization-parameter overrides, more than 16 configured providers, a
redirect URI outside the selected provider callback, and configurations with
no login method.

Example server configuration:

```json
{
  "schema": "dravengarden.cowboy.authentication/v1",
  "password": { "enabled": true },
  "login_method_order": ["google", "password"],
  "passkeys": {
    "enabled": true,
    "prompt_after_login": true,
    "session_refresh_enabled": true
  },
  "session": {
    "activity_sliding_enabled": true,
    "idle_timeout_ms": 86400000,
    "passkey_max_age_ms": 259200000,
    "passkey_warning_ms": 1800000,
    "primary_max_age_ms": 2592000000,
    "primary_warning_ms": 86400000
  },
  "providers": [
    {
      "plugin_id": "google",
      "plugin_version": "1.0.0",
      "artifact_digest": "sha256:<exact signed release digest>",
      "oidc": {
        "client_id": "<registered client id>",
        "redirect_uri": "https://cowboy.example/api/auth/providers/google/callback",
        "subject": "<exact Google sub, never an email address>",
        "account": "owner",
        "admin_account": "owner",
        "client_authentication": {
          "method": "client_secret_post",
          "client_secret_file": "/run/credentials/cowboy/google-client-secret"
        }
      }
    }
  ]
}
```

Set `COWBOY_AUTH_CONFIG` to this protected file. The legacy
`COWBOY_CARDEA_OIDC_CONFIG` remains accepted and is projected as provider ID
`cardea`; it can coexist during migration but does not override a signed
`cardea` selection.

Password login is enabled when `password` is omitted. It may be disabled only
when at least one signed provider is configured. Initial instance setup still
creates the local account and grant to which upstream subjects are mapped.
When present, `login_method_order` must list `password` (when enabled) and every
configured provider ID exactly once. Without it, Cardea is first when available,
then Password, followed by the remaining providers in stable ID order. Servers
without Cardea retain Password first. A malformed, duplicate, missing, or unknown
entry fails Controller startup instead of silently hiding a login method.

## OpenID Connect driver

The `open_id_connect` driver always uses Authorization Code plus S256 PKCE,
random single-use state, and a nonce. The package cannot override those
parameters. The callback consumes its transaction before exchanging a code and
binds the transaction to the exact provider ID.

An Authentication Provider may declare an HTTPS
`pushed_authorization_request_endpoint`. Cowboy sends the complete request
directly to that endpoint, authenticates it with the configured client method,
accepts only a bounded JSON `request_uri` plus expiry, and navigates the browser
with only `client_id` and `request_uri`. This is the generic path for approval
gateways such as Cardea: the browser needs no pre-existing provider cookie to
create a request, while an unauthenticated caller cannot manufacture requests
for a registered client.

The driver currently supports these closed methods:

| Method | Intended use | Secret behavior |
|---|---|---|
| `private_key_jwt_ed25519` | Cardea or a compatible private-key client | Signs a five-minute client assertion from a protected OKP JWK |
| `client_secret_post` | Google and compatible confidential clients | Reads the client secret from a protected file |
| `apple_client_secret_es256` | Sign in with Apple | Generates a five-minute Apple client-secret JWT from a protected P-256 key; no long-lived generated JWT is stored |

ID tokens accept only a package-declared `EdDSA` or `RS256` algorithm. Cowboy
validates the exact issuer, audience membership, nonce, expiry, bounded issued
time, signature, and configured subject. JWKS responses are HTTPS-only and
size-bounded. A provider email claim is descriptive and never becomes the
identity key.

After verification, the built-in driver normalizes only security evidence that
Cowboy can independently validate: Provider ID, exact issuer and subject,
ID-token issuance time, optional `auth_time`, optional `acr`, and a bounded set
of unique `amr` values. Missing `auth_time` remains unknown; token issuance time
must never be substituted as proof of a recent upstream authentication. These
claims are audit and policy inputs only. The package still cannot select a
Cowboy account, assert a role, or issue or extend a Cowboy session.

Apple `form_post` callbacks are accepted in addition to query callbacks.
Provider discovery is deliberately not trusted at runtime in v1: endpoints are
reviewable signed package data, while rotating public signing keys still come
from the signed `jwks_uri` location.

## Human approval over OIDC

Cardea keeps the public Cowboy driver standard: its package uses OpenID Connect
PAR and adds the reviewed `approval_mode=manual` authorization parameter. The
registered Cowboy backend signs the pushed request, the browser confirms that
exact request without entering Cardea credentials, Cardea publishes it to the
assigned identity's approval inbox, and only a separate Cardea approval may
release the one-time OIDC code. A Cardea login session alone is not approval.
Other approval systems should expose the same security properties behind OIDC
rather than asking Cowboy to execute provider code or accept a
provider-specific session token.

## Custom logic and provider state

Authentication Provider packages remain data-only in v1. A custom email,
Google, Apple, enterprise, or private identity system should expose an OIDC
facade and keep its provider-owned database behind that service. This gives the
provider full freedom over enrollment, mail delivery, approval, account
recovery, and upstream credentials without placing that code or its database
authority inside Cowboy's session-signing process.

Cowboy deliberately does not expose generic `save JSON`, `get JSON`, SQL, or
database-handle APIs to these packages. Such an API would turn a presentation
and protocol manifest into Controller code execution, make package compromise a
Cowboy database compromise, and create an ambiguous second source of truth for
accounts and sessions.

If a future protocol has a demonstrated need that cannot be represented by a
built-in driver or a provider-side OIDC facade, its executable adapter must be
isolated out of process or in a restricted WebAssembly runtime. The only
eligible persistent interface is a host-owned, versioned state service with all
of these properties:

- the namespace is fixed by plugin ID, exact artifact digest, deployment, and
  optional upstream subject; the adapter cannot choose or enumerate another
  namespace;
- the closed operations are `get`, compare-and-swap `put`, and
  compare-and-swap `delete`; no SQL, joins, prefix scans, or database handles;
- keys and canonical JSON values are size-bounded, writes have a per-plugin
  quota, and authentication-transaction records require a short expiry;
- values are encrypted and integrity-bound to the namespace by the host, while
  every mutation is audited without logging secrets;
- Cowboy account, grant, role, credential, token, Passkey, and session records
  are never addressable; and
- stored JSON is always untrusted state. It can never itself mean
  `authenticated`, select a local user, or authorize session issuance.

This state service is a reserved extension boundary, not a v1 package
capability. It will be added only with a concrete non-OIDC protocol, a separate
versioned contract, quotas, lifecycle semantics, and adversarial tests. Until
then, a provider service owns provider state and Cowboy owns Cowboy state.

## Login UI

`GET /api/auth/status` returns `password_enabled`, `login_method_order`, the
Passkey and session server policies, and every enabled provider's stable ID,
display label, button label, and start URL. When more than one method exists,
Web renders a generic tab per method in the server-provided order and selects
the first one.
The built-in fallback gives Cardea priority when it is available. Web rejects an
incomplete or duplicate response order and reconstructs the same safe default so
a malformed response cannot hide an enabled login method. Native clients can
consume the same explicit order without inferring it from the Provider array.
Cowboy owns the full-page gate, tabs, warning bar, responsive dialog or bottom
sheet, locked backdrop, cancellation, and recovery path. A package contributes
signed protocol and presentation data only; it cannot render arbitrary UI or
cover Cowboy's security state.

Native clients derive all follow-up routes from the provider's fixed returned
start URL; packages cannot supply a callback or return destination. Cowboy
Manager uses its existing PKCE query and the provider-scoped
`/native/exchange`. The iOS browser shell uses `client=browser-shell` with two
independent S256 challenges, opens the start URL outside the app's sole
`WKWebView`, and retains the provider-scoped `/native/poll` only as the
one-time cookie exchange. Waiting uses a provider-scoped `/native/events`
WebSocket whose URL contains no proof; the window sends both raw PKCE proofs
only as its first WebSocket message. A `ready` event is only an invalidation
signal, after which the window performs exactly one `/native/poll` exchange to
consume the handoff and receive its Cowboy cookies.
Closing the native authorization browser dispatches a shell event; the
original window uses both proofs to cancel the server handoff and clear the
waiting UI immediately. Only the original window retains those secrets and
receives Cowboy cookies; the authorization browser receives neither. Legacy
Cardea keeps the equivalent fixed `/api/auth/oidc/*` aliases.

## Passkey and session lifetime

Passkeys are a server feature flag and remain optional for each user. The
default server policy enables registration, recommends setup once after login,
and permits the user to turn periodic verification on later. The per-user
toggle remains off by default. Adding or deleting a credential requires a
session-local login or Passkey step-up from the last five minutes; a long-lived
cookie alone cannot replace credential state. Enabling the policy performs an
immediate, separate WebAuthn assertion before it becomes effective.

If that five-minute window has elapsed, Web presents the server-configured
Passkey, password, and Authentication Provider methods. Verification completed
inside a native shell or the current browser window repeats the pending
operation once; a full-page provider redirect asks the user to repeat it after
returning. Cancellation and failed verification leave credential state
unchanged. Cowboy never converts the HTTP 428 gate into a client-side bypass,
and a provider response for a different account clears cached product data
instead of continuing the original operation under another identity.

The Controller applies three independent deadlines. Trusted visible-document
input may slide the idle window, which defaults to 24 hours. An enabled
periodic Passkey policy requires a fresh assertion by the user's selected
interval, bounded by the service maximum (seven days by default). Password or
provider login has a non-sliding hard maximum of 30 days by default. Service
configuration also owns the warning windows: 30 minutes for Passkey and one day
for primary login. Values are validated at startup and published read-only to
clients.

Repeated Passkey verification is safe only because every refresh is a new,
origin-bound, user-verifying WebAuthn assertion and the old cookie is revoked
during rotation. The new cookie retains the original primary-authentication
timestamp, so Passkey never extends the service's full-login hard cap and never
extends another device. Cowboy does not treat a stored credential, background
timer, agent output, WebSocket heartbeat, or upstream provider session as proof
of user presence. Disabling periodic verification or revoking the final
Passkey removes that deadline but does not rewrite the primary cap.

Controller enforces the earliest deadline for REST and WebSocket access and
pushes per-cookie deadline snapshots over the authenticated product socket.
The client records real input at most once per minute, updates a single local
timer, and performs no session-policy polling. The responsive warning and lock
surfaces are Cowboy-owned: mobile respects safe areas and avoids automatic
keyboard focus, while Desktop uses a compact dialog and direct password focus.
At expiry the product view is blurred and locked, but agents continue and local
drafts plus queued prompts remain intact until verification succeeds or the
user signs out.

The iOS native shell runs the WebAuthn prompt in a system
`SFSafariViewController`, not its embedded `WKWebView`. This keeps Passkeys
available to SideStore-signed builds that cannot obtain the Associated Domains
entitlement and preserves the normal origin-bound Safari ceremony. The handoff
does not transfer the product cookie:

1. the authenticated Cowboy window creates an S256 PKCE challenge and starts a
   120-second transaction bound to the exact user and cookie-session hash;
2. only a random transaction token is placed in the fragment of the fixed
   `/passkey.html` URL, so it is absent from HTTP requests and referrers;
3. system Safari performs WebAuthn and can only stage the verified result; and
4. the original window must present the verifier from the same exact session
   before Cowboy persists a registration or rotates that window's cookie.

The browser page omits credentials on its API calls, removes the fragment from
history before invoking WebAuthn, uses a restrictive CSP and no-referrer policy,
and cannot choose an account, return URL, or session. Transactions are bounded,
single-use, and fail closed on expiry, cancellation, origin mismatch, session
mismatch, or PKCE mismatch. Ordinary Safari, installed PWAs, and desktop
browsers continue to use direct WebAuthn.

## Public examples

- [`../../examples/authentication/google`](../../examples/authentication/google)
  follows Google's published OIDC endpoints, RS256 JWKS verification, and
  `sub` identity.
- [`../../examples/authentication/apple`](../../examples/authentication/apple)
  uses Apple's Services ID flow, `form_post`, RS256 ID tokens, and a freshly
  generated ES256 client-secret JWT.
- [`../../examples/authentication/cloudflare-email`](../../examples/authentication/cloudflare-email)
  is a deployment skeleton for an allow-listed Cloudflare Email OIDC façade.
  Replace its example origin and complete key custody plus one-time transaction
  storage before packaging. Cloudflare Email Service sender/domain and
  destination verification rules still apply; the example does not turn Email
  Routing into an unrestricted mail sender.

Reference protocol sources:

- [OpenID Connect Core](https://openid.net/specs/openid-connect-core-1_0-18.html)
- [Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)
- [Sign in with Apple REST API](https://developer.apple.com/documentation/signinwithapplerestapi)
- [Cloudflare Email Service bindings](https://developers.cloudflare.com/email-service/configuration/send-bindings/)
- [Web Authentication Level 3](https://www.w3.org/TR/webauthn-3/)
