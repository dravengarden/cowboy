# Product login and self-host identity

Cowboy is a **self-hosted, single-instance** control plane. One household or
company runs one Service. There is no multi-tenant SaaS and no Service-side
E2EE. The instance may accept its local password or one explicitly pinned
Cardea authorization provider; Cardea never creates Cowboy users or chooses
their roles. Whoever operates the Cowboy instance remains inside the trust
boundary: the Service already commands Machines to run agents and already
stores plaintext transcripts.

Accounts are required on `/` and product APIs when
`COWBOY_PRODUCT_AUTH_ENABLED=true`. A deliberate `false` value restores the
trusted-network synthetic local owner without weakening the separate admin
plane. The published HTTPS origin in the URL bar is the Web trust source.
`/admin` stays a separate identity plane and cookie. This stage is
**single-user**: first-run on `/` proves the host setup code, then creates the
only user (and the matching admin owner). Extra users, invites, and open
registration fail closed.

See [Admin](14-admin.md) for the setup-code protocol.

## Why the Service is not E2EE

A Service-side MLS / blind-forwarder design was considered and rejected.

Even if the Service stored only ciphertext, it still tells `cowboy-machine`
which agent to start and which prompt to send. The operator of the Service
can command a replay on the user's PC. Encryption of the transcript does
not close that channel. Trust is therefore **whoever operates this
instance**, not a cryptographic promise that the Service cannot read.

This chapter documents the implemented login product. It does not add
OpenMLS, mailboxes, TUF, or a second "blind" Service.

## Three principals

Never mix these in one cookie.

| Principal | Proof | Scope |
|---|---|---|
| `AdminPrincipal` | `cowboy_admin` cookie (12h, `SameSite=Strict`) | `/api/admin/*` only |
| `ProductPrincipal` | policy-bounded `cowboy_user` cookie or `Authorization: Bearer cow_…` | PWA REST, `/ws`, product APIs |
| `MachinePrincipal` | enrollment token, then signed challenge | machine enroll / connect |

A same handle on admin and product is coincidence, not a link.

Admin login is the break-glass plane: argon2id (legacy SHA-256 upgraded on
login), dummy verify, 12-hour `SameSite=Strict` cookie, HTTPS via the
loopback proxy only, a one-time host setup token, and rate-limited
setup/bootstrap/login. See
[Admin](14-admin.md).

## Cardea authorization (optional provider)

Local Cowboy password login is enabled by default and may be disabled only
when a configured provider remains available. When the legacy
`COWBOY_CARDEA_OIDC_CONFIG` points to a protected consumer profile,
`GET /api/auth/status` advertises Cardea as an additional sign-in choice.
Cowboy uses Authorization Code flow with two independent PKCE boundaries:

- Browser login binds a five-minute transaction to a callback-only HttpOnly
  cookie, random state, nonce, and S256 verifier. The callback is the fixed
  `/api/auth/oidc/callback`; no request supplies a return destination.
- Cowboy authenticates to Cardea's token endpoint with a short-lived Ed25519
  client assertion. Redirects are disabled and response time, type, and size
  are bounded.
- The five-minute ID token is verified against the exact issuer, audience,
  nonce, signing key id, pinned Ed25519 public key, and configured subject.
  That subject maps only to the exact pre-existing Cowboy user. An optional
  admin mapping issues the separate admin cookie only for an exact,
  pre-existing admin account.
- Cowboy Manager adds a second S256 challenge. After the browser callback,
  Cowboy returns only a 60-second, single-use handoff code to the fixed
  reverse-domain `xyz.stormbird.cowboy.manager://auth/callback` URL. The app
  rejects HTTP redirects and never receives or stores a
  Cardea token or client private key.
- The iOS Cowboy shell never navigates its sole `WKWebView` to Cardea. It opens
  the provider in a system Safari sheet with independent S256 code and handoff
  challenges. Safari owns only the five-minute OIDC transaction cookie; after
  approval the callback marks the bounded handoff ready and redirects to a
  fixed, no-store completion page. The original Cowboy window waits for a
  provider-scoped WebSocket invalidation and then performs exactly one exchange
  with both retained random secrets before Controller issues product and
  optional admin cookies into that window. The handoff is single-use and a denial,
  expiry, provider mismatch, Origin mismatch, or either PKCE mismatch fails
  closed. Neither raw secret, Cowboy cookie, Cardea token, nor client key is
  transferred through the authorization URL.

The provider profile and client private JWK must both be regular, non-symlink
files inaccessible to group and others. The profile pins all trust material;
loading any malformed or over-broad file fails controller startup. Cardea
proves identity only. Cowboy owns its cookie lifetime, revocation, role, and
trusted-network auth-off switch.

Every product cookie records the primary method that created that browser
session: `password` or one exact Authentication Provider ID. A primary
reauthentication rotates that cookie only when the same method succeeds. It
cannot silently substitute another provider or a password, even when both map
to the same Cowboy account. Signing out deletes the binding; the next fresh
login may choose any enabled method. Sessions created before this field existed
retain `NULL` and bind exactly once on their next successful primary
reauthentication because their original source cannot be reconstructed safely.

## Session protection and optional Passkeys

Controller configuration owns three independent browser-session deadlines.
The defaults are a sliding 24-hour idle window, a three-day maximum between
explicit Passkey checks when the user enables periodic verification, and a
30-day hard maximum before a password or Authentication Provider login is
required again. The service warns 30 minutes before a Passkey deadline and one
day before the primary-login deadline. The Account surface publishes these
effective service values so clients never have to infer policy.

Only trusted input in a visible Cowboy document slides the idle deadline.
Pointer, touch, and keyboard activity is coalesced to at most one authenticated
WebSocket activity message and durable database write per minute. Agent output,
streaming events, heartbeats, background tabs, and timer wakeups never count as
human activity. Activity never moves the Passkey or primary-login hard
deadline.

After sign-in a product user may register a discoverable WebAuthn Passkey
(`POST /api/auth/passkeys/register/*`). Periodic verification is off by default.
When enabled, the user chooses a supported interval up to the server maximum;
the closed options are 1, 2, 3, 4, 6, or 12 hours and 1 (the default), 2, or 3
days. The default server maximum is three days, so it filters out longer
choices. Every refresh requires a new,
origin-bound, user-verifying assertion. It rotates only the current browser's
cookie and records new Passkey proof, but preserves the original primary-login
timestamp. A Passkey can therefore restore the idle window without silently
turning one primary login into an unbounded session.

The earliest applicable deadline wins. When due, Controller rejects protected
REST and `/ws` with `428 Precondition Required`; the PWA lock is presentation,
not the security boundary. A per-cookie `AuthSession` WebSocket message pushes
new deadlines immediately after activity or verification, so the UI uses one
local timeout rather than network polling. During the warning window Cowboy
shows a responsive top reminder. Once due it blurs and locks the product view
inside Cowboy-owned chrome while running agents, drafts, and queued prompts
continue in the background. Mobile uses a safe-area bottom sheet without
automatic keyboard focus; Desktop uses a compact dialog and focuses the
password field when password login is selected.

If periodic Passkey verification is unavailable or disabled, an idle deadline
requires a configured primary login method instead. Disabling the policy or
deleting the final Passkey removes the Passkey deadline; it does not rewrite the
primary hard cap. Status, logout, Passkey assertion, and password/provider login
remain available for recovery. Admin keeps its separate five-minute idle-view
lock and 12-hour cookie.

The SideStore iOS shell cannot rely on the Associated Domains entitlement for
WebAuthn inside `WKWebView`. It therefore opens the fixed `/passkey.html` page
in a system authentication session. For an assertion, the page invokes WebAuthn
as soon as the initiating Cowboy tap opens it, so there is no second page-level
Continue action before Face ID, Touch ID, or the device passcode. Registration
keeps its explicit page gesture. An authenticated start binds a 120-second
transaction to the exact Cowboy user and cookie session plus an S256 challenge.
Safari sees only an opaque token in the URL fragment and can stage a verified
credential; it cannot persist the Passkey or extend a cookie. The original
Cowboy window must finalize with the retained verifier and the same session.
Success closes the sheet; cancellation, expiry, session replacement, and PKCE
mismatch fail closed. Browser dismissal and foreground recovery perform a
bounded finalize so a suspended WebKit callback cannot leave Cowboy busy. The
system session is presentation only: all account mutation and cookie rotation
remain Controller-owned.

Roles reuse the serde names `owner` / `operator` / `viewer`. Product roles
live only in `cowboy.permissions` (`role_for`); there is no `users.role`
column. Admin roles live on `cowboy.admin.identities`.

## Capability matrix

| Capability | Product viewer | Product operator | Product owner | Admin viewer | Admin operator | Admin owner |
|---|---|---|---|---|---|---|
| Read own transcripts on `/` | yes | yes | yes | `/admin` index | `/admin` | `/admin` |
| Create / prompt / delete own sessions | no | yes | yes | no | no | no |
| See other users' sessions | no | no | yes | index | index | index |
| Unowned legacy sessions | read | read + mutate | read + mutate | index | index | index |
| Provider Service login (PWA) | no | yes | yes | no | no | no |
| Catalog refresh | no | no | no | no | yes | yes |
| Machine enroll / revoke | no | no | no | no | yes | yes |
| Registration + invites | no | no | no | no | no | no |
| `cowboy.permissions` | no | no | no | no | no | read |
| Create admin operators | no | no | no | no | no | no |
| Create extra product users | no | no | no | no | no | no |

Unowned `sessions.owner_user_id IS NULL` rows are a shared household pool.
There is no adopt / reassign API.

## Hub ACL

```text
can_see(principal, session_id) -> bool
can_mutate(principal, session_id) -> bool
```

- `can_see`: product viewer+ sees own rows and unowned rows; product owner
  grant sees every row. An admin cookie is **not** a product principal and
  does not open `/ws`.
- `can_mutate`: viewer → false; operator → own or unowned; owner grant → any.

Applied to WS bootstrap, session-scoped `Outbound`, session REST families,
and `Inbound::Sync` (`order` / `title` / `queue:<id>` / `mobile-review:<id>`).
`GET /api/artifacts/{name}` requires a product principal; the hash is the
capability.

Global `title` / `order` SyncPatches are projected to visible ids. Reorder
**merges**: submitted ids only permute names they include; an omitted
visible id is never dropped (`[A,B,C]` + `[C,A]` → `[C,B,A]`).

## Product HTTP

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/auth/status` | public | `{ registration, setup_required, setup_pending, me? }` — no invite table |
| `POST` | `/api/auth/setup` | public; HTTPS | prove host setup code; 10-minute setup cookie |
| `POST` | `/api/auth/register` | public; setup cookie | create the only user + `cowboy_user` |
| `POST` | `/api/auth/login` | public | cookie + `me` |
| `GET` | `/api/auth/oidc/start` | public | fixed Cardea Authorization Code + PKCE start |
| `GET` | `/api/auth/oidc/callback` | public; transaction cookie | verify Cardea response and issue Cowboy session |
| `POST` | `/api/auth/oidc/native/exchange` | same-origin; one-time handoff + PKCE | issue Manager product/admin cookies |
| `POST` | `/api/auth/oidc/native/poll` | same-origin; two retained secrets | poll and consume an iOS browser-shell handoff; issue product/admin cookies only in the original app window |
| `GET` | `/api/auth/oidc/native/complete` | public | fixed no-store Safari completion page; carries no account or handoff data |
| `POST` | `/api/auth/logout` | cookie optional; recent proof for `all` / `provider` | revoke `current`, `all`, or current-Provider Cowboy sessions; optionally return the pinned Provider logout URL |
| `GET` | `/api/auth/logout/complete` | public | fixed no-store RP-logout return that clears the local cookie and redirects to `/` |
| `POST` | `/api/auth/providers/{id}/backchannel-logout` | signed Provider Logout Token | replay-safe Provider-scoped session revocation |
| `GET` | `/api/auth/me` | product | current principal |
| `GET` | `/api/auth/sessions` | fresh product session | list current account sessions, active leases, effective limits, and fencing tokens |
| `DELETE` | `/api/auth/sessions/{id}` | recent product login + Origin | revoke one own logical session |
| `DELETE` | `/api/auth/active-clients/{id}?fencing_token=…` | fresh product session + Origin | reclaim one own active lease only when the fencing token still matches |
| `POST` | `/api/auth/automation/credentials` | recent product operator + Origin; feature-gated | issue one audited, scoped, sender-constrained credential bounded by server policy |
| `POST` | `/api/auth/passkeys/assert/*` | product session | verify and rotate the current browser session within its primary hard cap |
| `POST` | `/api/auth/passkeys/external/start` | exact product cookie + Origin | start a session-bound native Safari ceremony |
| `POST` | `/api/auth/passkeys/external/options` | public; Origin + opaque transaction | return only staged WebAuthn options to system Safari |
| `POST` | `/api/auth/passkeys/external/complete` | public; Origin + opaque transaction | verify and stage Safari's credential without changing account state |
| `POST` | `/api/auth/passkeys/external/finalize` | exact product cookie + Origin + PKCE | persist or rotate only after the original window proves the handoff |
| `PUT` | `/api/auth/passkeys/reauth` | fresh product session | opt in/out and set the bounded interval |
| `POST` | `/api/auth/device/authorizations` | public, rate-limited | start a PKCE- and public-key-bound request |
| `POST` | `/api/auth/device/authorizations/inspect` | approval capability | show the client name and full public-key fingerprint |
| `POST` | `/api/auth/device/authorizations/approve` | recent product login + Origin | approve the one-time device request |
| `GET` | `/api/auth/device/authorizations/events` | PKCE-bound WebSocket | push approval or denial without polling |
| `POST` | `/api/auth/device/exchange` | approved request + key proof | issue sender-constrained access and rotating refresh credentials |
| `POST` | `/api/auth/device/refresh` | refresh bearer + key proof | rotate once; replay revokes the device |
| `GET` | `/api/auth/devices` | fresh product session | list own authorized devices, never secrets |
| `DELETE` | `/api/auth/devices/{id}` | recent product login + Origin | revoke own device and active access |
| `POST`/`GET`/`DELETE` | `/api/auth/tokens[/…]` | legacy product boundary | migration-only personal token compatibility |
| `ANY` | `/ws` | product; cookie also Origin | 401 before upgrade; later revoke closes **4001** |
| `POST` | `/api/sessions` | product operator+ | stamps `owner_user_id` |
| `GET` | `/api/sessions/{id}/*` | product + `can_see` | session REST |
| `POST`/`PUT`/`DELETE` | session families | product + `can_mutate` | mutate |
| `GET` | `/api/artifacts/{name}` | product | hash capability |
| `POST` | `/api/machines/enrollment` | product operator+ or admin operator+ | mint one-time machine enrollment; `machine_id` is optional and auto-assigned |
| `DELETE` | `/api/machines/enrollment` | product operator+ or admin operator+ | atomically discard an unconsumed enrollment token before returning to setup details |
| `GET` | `/metrics` | scrape-only | loopback peer, no forwarded headers, else 404 |

Cookie POST/PUT/DELETE and cookie `/ws` upgrades run the Origin allow-list
(`Host`, loopback `X-Forwarded-Host`, `COWBOY_PUBLIC_ORIGIN`). Vite origins
are debug-only. Bearer skips Origin.

Product passwords are argon2id PHC strings. Admin passwords stay iterated
SHA-256 in this slice. Cookie `Secure` is set when the request is HTTPS or
`X-Forwarded-Proto: https`.

Product cookies use the Controller's configured primary-login hard maximum.
A successful Passkey assertion rotates the current token rather than extending
it in place, records the assertion time on that session only, and retains the
original primary-login timestamp. One device can never refresh another
device's cookie or move another device's deadlines.

`serve-acp` normally uses browser-approved, sender-constrained device
credentials. First use opens the configured login and explicit fingerprint
approval; subsequent access uses a 10-minute access token and a rotating
30-day refresh token stored with its private Ed25519 key in a mode-0600 local
file. A loopback API connection uses the configured `COWBOY_PUBLIC_ORIGIN` for
the browser approval page; clients accept that cross-origin handoff only when
the page is HTTPS, while non-loopback API connections remain same-origin. A
refresh replay revokes the whole device. `--token` /
`COWBOY_USER_TOKEN` remain hidden migration inputs only. When auth is enabled,
there is no anonymous loopback product bypass; when the explicit auth-off flag
is deployed, `/api/auth/status` authoritatively preserves synthetic local-owner
access.

Admin routes are listed in [Admin](14-admin.md). Settings redaction is
[Server](06-server-api.md) (`settings_for_product_clients`).

## PWA gate

Product login is guarded by the controller feature flag
`--product-auth-enabled <true|false>` / `COWBOY_PRODUCT_AUTH_ENABLED`. It
defaults to `false`: the controller exposes a synthetic local owner, keeps the
PWA and product APIs available without a cookie, and leaves the separate admin
authentication plane intact. Set it to `true` only when the complete login
stack is ready and deploy Web plus controller together. Keeping the flag is an
intentional emergency rollback and trusted-intranet mode, not a UI-only switch.

Cardea is independently optional. The provider file uses schema
`dravengarden.cowboy.cardea-oidc/v1` and pins `issuer`, `client_id`,
`client_key_id`, `client_private_key_file`, `id_token_key_id`,
`id_token_public_key_jwk`, `subject`, `account`, optional `admin_account`, and
the exact HTTPS `redirect_uri`. Private key material stays outside Git and the
Nix store.

`ProductAuthGate` wraps Desktop/Mobile in `web/src/main.tsx`.
`web/src/auth/*` must not import `web/src/store.ts` (`subscribe()` opens
`/ws`). HTTP 200 + `me == null` → login. 404/501 → "controller too old /
activating", not login. Logout / `me` change deletes `HISTORY_CACHE` and
must stop WS reconnect (`cowboy:product-sign-out`).

Handshake `/me` probe: **200** reconnect; **401/403** or WS **4001** remount
login; **network / 5xx / 404** keep the cookie.

## App Source page

Not implemented in this chapter. The packaged shell will store a self-host
HTTPS origin and navigate the webview there. The installed browser PWA
never shows Source. Do not hard-code `cowboy.stormbird.xyz` as a product
default.

## Plugins

User-facing "Provider" becomes **Plugin** in a later PR (copy, dual GET
routes, install ≠ enable). Login does not depend on that rename. Plugin
bytes are Service-ingested signed packages; Machines download from Cowboy
CAS, not from npm or client-direct GitHub.

## Hawk enablement checklist

Do **not** switch `cowboy.service` until this stack is the intended
activate and the operator has read this list. The first login-required
controller activate is a **planned outage** of `/` and Zed.

1. Read the one-time `cow_setup_…` token from the journal (`admin_setup_token`)
   or `$COWBOY_DATA_DIR/admin-setup.token`. Open `/`, enter the code, then
   create the only user. Prefer a Chrome or Apple generated password
   (15+ characters). Write it down. `/admin` login uses the same account.
2. Activate **web + controller together** so `GET /api/auth/status` is 200
   and the PWA serves `ProductAuthGate`. Do not pair a new gate with an old
   controller, or a new controller with old `store.ts`.
3. Sign in on `/`. `/admin` is login-only after first-run.
4. Start each `agent_servers.cowboy-*` bridge (or run `cowboy login` once),
   compare the full `SHA256:` fingerprint, and approve the client in the normal
   Cowboy login. Do not create or copy a personal token.
5. Hard-reload the PWA (`sw.js` VERSION + `/version`). A WS reconnect keeps
   stale JS.
6. This instance stays single-user. Extra-user APIs return 403.
7. Prefer `COWBOY_PUBLIC_ORIGIN=https://<instance>` on the same activate.

`/admin` remains the break-glass after first-run. The auth-off flag is the
bounded product-plane rollback; a previous controller generation remains the
code rollback.

Hygiene-only settings redaction (allow-list `Outbound::Settings`) is the
only controller activate that is both safe and useful by itself.

## Related

- [Admin](14-admin.md) — registration policy, admin HTTP
- [Server & wire API](06-server-api.md) — route table, `/metrics`, `/ws`
- [Zed ACP](../integrations/zed.md) — browser-approved device authorization
