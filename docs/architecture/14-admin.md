# Admin console and product registration

Cowboy's product UI is the session PWA at `/`. This stage is **single-user**:
the first visit to `/` on an empty instance asks for the host setup code,
then creates the only product user (and the matching admin owner). Extra
users, invites, and open registration are not available.

The **admin plane** stores operator policy in Hub settings
(`cowboy.registration`, `cowboy.permissions`, `cowboy.session_limits`,
`cowboy.admin.identities`). Product login is a separate identity plane
(`users` + `cowboy_user`). Those blobs are service-internal. They must
not appear on product `/ws`. `/admin` does not create the first user.

## Settings redaction

`Hub::settings_for_product_clients` is the only constructor of
`Outbound::Settings` (connect bootstrap and the live `set_setting`
broadcast). The product map is an **allow-list**:

- `session.autoResume.default`
- `session.autoResume.template`

`cowboy.admin.identities`, `cowboy.permissions`, `cowboy.session_limits`,
and the registration invite table are dropped. `Inbound::SetSetting` from
a product socket is rejected unless the key is one of the two auto-resume
keys. Admin HTTP stays on `/api/admin/*`.

## Registration

This stage has no public registration. `GET /api/auth/status` always
returns `accepts_registration: false`. Invite tokens, open mode, extra
admin operators, extra product users, and permission grants all **fail
closed** (`403` `"this instance is single-user"`).

First-run uses a Portainer-style host setup code, not a calendar-expiring
invite:

1. With no product users, the controller writes a one-time `cow_setup_…`
   token to `$COWBOY_DATA_DIR/admin-setup.token` (`0600`) and logs it once.
   The HTTP API never returns the secret. The file stays until the only
   user is created; there is no wall-clock expiry.
2. `POST /api/auth/setup` proves the code, rate-limits failures, and sets
   a 10-minute `cowboy_admin_setup` cookie. That cookie is the setup
   session, not the code itself.
3. `POST /api/auth/register` with that cookie creates the only product
   user as owner, creates the matching admin owner if identities are
   empty, then deletes the setup file and clears setup tickets.
4. A second register, extra-user create, or `/admin` bootstrap is `403`.

Public clients still receive `RegistrationPublicStatus`
(`{ enabled, mode, accepts_registration }`) so older PWAs can parse
status. They must not receive a setup secret or invite table.

## Product users

The only product user is created on `/` after the setup code:

1. `INSERT` the `users` row (no role column).
2. Under the settings lock, upsert `cowboy.permissions.grants` for that
   lowercase account as `owner`.
3. If admin identities are empty, create the matching admin owner with
   the same account and password. Cookies stay separate.

`PermissionPolicy::apply_patch` still lowercases grant accounts so
`Draven` matches `draven`. The patch API is fail-closed in this stage.

## API

| Route | Auth | Purpose |
|---|---|---|
| `GET /api/admin/auth` | public | `{ authenticated, bootstrap_required, setup_pending, account?, role? }` — never the setup token |
| `POST /api/admin/auth/setup` | public | `403` `"complete setup on /"` — first-run is the product PWA |
| `POST /api/admin/auth/bootstrap` | public | `403` `"complete setup on /"` |
| `POST /api/admin/auth/login` | public | `cowboy_admin` cookie |
| `POST /api/admin/auth/logout` | cookie optional | clear admin cookie |
| `GET /api/admin/overview` | admin viewer+ | health, persistence, registration |
| `GET /api/admin/sessions` | admin viewer+ | live session list |
| `GET /api/admin/machines` | admin viewer+ | enrolled machines |
| `GET /api/admin/accounts` | admin viewer+ | admin operators (no hashes) |
| `POST /api/admin/accounts` | admin owner | `403` `"this instance is single-user"` |
| `GET /api/admin/registration` | admin viewer+ | policy + token table (no hashes); always closed |
| `PUT /api/admin/registration` | admin operator+ | `403` `"this instance is single-user"` |
| `POST /api/admin/registration/tokens` | admin operator+ | `403` `"this instance is single-user"` |
| `DELETE /api/admin/registration/tokens/{id}` | admin operator+ | `403` `"this instance is single-user"` |
| `GET /api/admin/permissions` | admin viewer+ | grants + default role |
| `PUT /api/admin/permissions` | admin owner | `403` `"this instance is single-user"` |
| `GET /api/admin/session-limits` | admin viewer+ | controller session limits |
| `PUT /api/admin/session-limits` | admin operator+ | replace limits |
| `GET /api/admin/providers` | admin viewer+ | catalog entries + root |
| `POST /api/admin/providers/refresh` | admin operator+ | rescan external releases |
| `GET /api/auth/status` | public | `{ registration, setup_required, setup_pending, me? }` |
| `POST /api/auth/setup` | public; HTTPS | prove host setup code; 10-minute setup cookie |
| `POST /api/auth/register` | public; setup cookie | create the only user + `cowboy_user` cookie |
| `POST /api/auth/login` | public | cookie + `me` |
| `POST /api/auth/logout` | cookie optional | clear cookie |
| `GET /api/auth/me` | product cookie or Bearer | current principal |
| `POST /api/auth/tokens` | product operator+ | create own `cow_…` token; secret shown once |
| `GET /api/auth/tokens` | product (own rows) | list prefixes, names, timestamps |
| `DELETE /api/auth/tokens/{id}` | product (own row; else 404) | revoke |
| `GET /api/admin/users` | admin operator+ | list users + `role_for` |
| `POST /api/admin/users` | admin operator+ | `403` `"this instance is single-user"` |
| `POST /api/admin/users/{id}/disable` | admin operator+ | disable + revoke sessions/tokens |
| `POST /api/admin/users/{id}/password` | admin owner | set password |

Signup is first-run only: setup code on `/`, then one owner. Product
`/ws` and product APIs require that product principal. Admin writes go
through `require_admin_role` (operator+ or owner); viewers keep admin
read. Extra-user writes fail closed.

## Admin login

`/admin` is a separate site. The document is public HTML; every
`/api/admin/*` route except `GET/POST /api/admin/auth*` requires a
`cowboy_admin` cookie. Product cookies never become an admin principal.

Hardening:

- New admin passwords are argon2id PHC (15–128 characters, not the account).
  Hand-chosen secrets need uppercase, lowercase, and a digit. Hyphenated
  Chrome / Apple generated passwords (three or more alphanumeric groups)
  are accepted even when they are lowercase-only. Setup tells the operator
  this origin can run agents and to prefer a generated password.
  Existing iterated SHA-256 hashes verify once and upgrade on login.
- Unknown-account login dummy-verifies so timing does not enumerate owners.
- Cookie: `HttpOnly`, `SameSite=Strict`, 12-hour absolute TTL, `Secure` on HTTPS.
  A new login replaces every other session for that account.
- Login, setup, and bootstrap require HTTPS as advertised by a **loopback** reverse
  proxy (`X-Forwarded-Proto: https`). Direct non-loopback HTTP is refused.
  Loopback without forwarded headers stays allowed for `just dev`.
- First start with no product user writes a one-time `cow_setup_…` token to
  `$COWBOY_DATA_DIR/admin-setup.token` (`0600`) and logs `admin_setup_token`
  once. The HTTP API never returns the secret. The file has no calendar
  expiry; after the only user is created the file is deleted.
- `POST /api/admin/auth/setup` and `POST /api/admin/auth/bootstrap` are
  `403` `"complete setup on /"` so `/admin` cannot consume the setup
  code. Wrong product setup codes dummy-compare and share the in-process
  failure limiter.
- Login/bootstrap failures log `ok=false` without passwords, hashes, or the
  setup secret.
- Service worker never caches `/api/admin/*` or `/admin` as the PWA shell.

Admin Passkeys follow the product split: password login first, then optional
registration. With a Passkey registered, `/admin` locks the view after
**5 minutes idle** unless the operator turns that off on Accounts. Routes
live under `/api/admin/passkeys*`.

See [Product login](16-product-auth.md) for planes, the capability matrix,
and the Hawk enablement checklist.
