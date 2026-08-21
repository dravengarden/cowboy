# Product login and self-host identity

Cowboy is a **self-hosted, single-instance** control plane. One household or
company runs one Service. There is no multi-tenant SaaS, no vendor IdP, and
no Service-side E2EE. Whoever operates that instance is inside the trust
boundary: the Service already commands Machines to run agents and already
stores plaintext transcripts.

Accounts are required on `/` and product APIs. There is no anonymous product
path. The published HTTPS origin in the URL bar is the Web trust source. The
packaged app will store that origin on a Source page (not shipped in this
chapter). `/admin` stays a separate identity plane and cookie. This stage
is **single-user**: first-run on `/` proves the host setup code, then
creates the only user (and the matching admin owner). Extra users, invites,
and open registration fail closed.

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
| `ProductPrincipal` | `cowboy_user` cookie or `Authorization: Bearer cow_…` | PWA REST, `/ws`, product APIs |
| `MachinePrincipal` | enrollment token, then signed challenge | machine enroll / connect |

A same handle on admin and product is coincidence, not a link.

Admin login is the break-glass plane: argon2id (legacy SHA-256 upgraded on
login), dummy verify, 12-hour `SameSite=Strict` cookie, HTTPS via the
loopback proxy only, a one-time host setup token, and rate-limited
setup/bootstrap/login. See
[Admin](14-admin.md).

## Passkeys (optional step-up)

Password login stays first. After sign-in a product user may register a
discoverable WebAuthn Passkey (`POST /api/auth/passkeys/register/*`). The
PWA default is to lock the **view after 15 minutes idle** (no pointer,
key, or visible tab) when a Passkey exists. Settings can turn that lock
off (`PUT /api/auth/passkeys/reauth`). No Passkey means the lock never
engages. The cookie and `/ws` stay valid. Admin uses the same split with
a **5-minute idle** window. Modeled on Cardea's password-then-Passkey
split, not its login-blocking factor ticket.

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
| `POST` | `/api/auth/logout` | cookie optional | clear cookie |
| `GET` | `/api/auth/me` | product | current principal |
| `POST` | `/api/auth/tokens` | product operator+ | create own `cow_…` token |
| `GET` | `/api/auth/tokens` | product (own rows) | list prefixes |
| `DELETE` | `/api/auth/tokens/{id}` | product (own row; else 404) | revoke |
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

`serve-acp` requires `--token` / `COWBOY_USER_TOKEN` and **exits** on
401/403. There is no anonymous loopback product bypass.

Admin routes are listed in [Admin](14-admin.md). Settings redaction is
[Server](06-server-api.md) (`settings_for_product_clients`).

## PWA gate

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
4. Mint a `cow_…` token. Set `COWBOY_USER_TOKEN` on every
   `agent_servers.cowboy-*` `serve-acp`.
5. Hard-reload the PWA (`sw.js` VERSION + `/version`). A WS reconnect keeps
   stale JS.
6. This instance stays single-user. Extra-user APIs return 403.
7. Prefer `COWBOY_PUBLIC_ORIGIN=https://<instance>` on the same activate.

`/admin` remains the break-glass after first-run. Rollback is a previous
controller generation, not a mode flag.

Hygiene-only settings redaction (allow-list `Outbound::Settings`) is the
only controller activate that is both safe and useful by itself.

## Related

- [Admin](14-admin.md) — registration policy, admin HTTP
- [Server & wire API](06-server-api.md) — route table, `/metrics`, `/ws`
- [Zed ACP](../integrations/zed.md) — `COWBOY_USER_TOKEN`
