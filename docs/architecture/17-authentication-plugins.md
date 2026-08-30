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
  "passkeys": {
    "enabled": true,
    "prompt_after_login": true,
    "session_refresh_enabled": true
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

## Login UI

`GET /api/auth/status` returns `password_enabled`, the Passkey server policy,
and every enabled provider's stable ID, display label, button label, and start
URL. When more than one method exists, Web renders a generic tab per method.
Password is selected first when enabled; no provider ID is hard-coded in Web.

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
and permits the user to turn session refresh on later. The per-user refresh
toggle remains off by default. Adding or deleting a credential requires a
session-local login or Passkey step-up from the last five minutes; a long-lived
cookie alone cannot replace credential state. Enabling refresh does not extend
a session until the separate WebAuthn assertion succeeds.

If that five-minute window has elapsed, Web presents the server-configured
Passkey, password, and Authentication Provider methods. Verification completed
inside a native shell or the current browser window repeats the pending
operation once; a full-page provider redirect asks the user to repeat it after
returning. Cancellation and failed verification leave credential state
unchanged. Cowboy never converts the HTTP 428 gate into a client-side bypass,
and a provider response for a different account clears cached product data
instead of continuing the original operation under another identity.

Without a Passkey assertion, password and provider sessions last one day. A
successful explicit WebAuthn user-verification ceremony may atomically replace
only the current browser cookie with a session lasting at most 30 days. It does
not extend other devices. The configured 1, 7, or 14 day interval is a maximum
age for the next assertion, not an unattended background refresh.

Repeated renewal is safe only because every renewal is a new, origin-bound,
user-verifying WebAuthn assertion and the old cookie is revoked during rotation.
Cowboy does not treat the presence of a credential, a background timer, or an
upstream Cardea session as proof of user presence. Disabling server refresh
immediately stops enforcing old per-user refresh settings and stops creating
30-day replacements; revoking the Passkey does not revive an expired session.

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
