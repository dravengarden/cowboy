# Admin console and product registration

Cowboy's product UI is the session PWA at `/`. The **admin plane** stores
operator policy in Hub settings (`cowboy.registration`,
`cowboy.permissions`, `cowboy.session_limits`, `cowboy.admin.identities`).
Product login is a separate identity plane (`users` + `cowboy_user`).
Those blobs are service-internal. They must not appear on product `/ws`.

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

Synapse-shaped service control, stored at `cowboy.registration`:

| Cowboy | Matrix / Synapse |
|---|---|
| `enabled = false` (default) | `enable_registration: false` |
| `mode = token` | `registration_requires_token` + MSC3231 tokens |
| `mode = open` | open registration |
| admin-issued tokens | `/_synapse/admin/v1/registration_tokens` |

`RegistrationPolicy::accepts_registration()` is `enabled && mode != disabled`.
`POST /api/auth/register` **fails closed** when that is false
(`403` `"registration is disabled by the service"`).

Token consume is a **pure** mutation (`consume_registration_token`). The
register handler INSERTs the user first, then re-reads the policy under the
settings mutex, increments `uses_count` with `commit_setting_locked`, drops
the lock, and `publish_setting`. A losing locker DELETEs the user it just
inserted. Open mode ignores any token field.

Public clients receive `RegistrationPublicStatus`
(`{ enabled, mode, accepts_registration }`) from `GET /api/auth/status`.
They must not receive the invite table (`public_view()` stays admin-only).

## Product users

Admin operators create the first product user even when registration is
closed:

1. `INSERT` the `users` row (no role column).
2. Under the settings lock, upsert `cowboy.permissions.grants` for that
   lowercase account. Default role is `operator`. Only an admin owner may
   grant `owner`.
3. Public register does **not** write a grant and therefore inherits
   `default_role` (`viewer`).

`PermissionPolicy::apply_patch` lowercases grant accounts so `Draven`
matches `draven`.

## API

| Route | Auth | Purpose |
|---|---|---|
| `GET /api/admin/auth` | public | `{ authenticated, bootstrap_required, account?, role? }` |
| `POST /api/admin/auth/bootstrap` | public; first owner only | create owner + `cowboy_admin` |
| `POST /api/admin/auth/login` | public | `cowboy_admin` cookie |
| `POST /api/admin/auth/logout` | cookie optional | clear admin cookie |
| `GET /api/admin/overview` | admin viewer+ | health, persistence, registration |
| `GET /api/admin/sessions` | admin viewer+ | live session list |
| `GET /api/admin/machines` | admin viewer+ | enrolled machines |
| `GET /api/admin/accounts` | admin viewer+ | admin operators (no hashes) |
| `POST /api/admin/accounts` | admin owner | create admin operator |
| `GET /api/admin/registration` | admin viewer+ | policy + token table (no hashes) |
| `PUT /api/admin/registration` | admin operator+ | enable / mode |
| `POST /api/admin/registration/tokens` | admin operator+ | issue invite; secret shown once |
| `DELETE /api/admin/registration/tokens/{id}` | admin operator+ | disable invite |
| `GET /api/admin/permissions` | admin viewer+ | grants + default role |
| `PUT /api/admin/permissions` | admin owner | replace grants |
| `GET /api/admin/session-limits` | admin viewer+ | controller session limits |
| `PUT /api/admin/session-limits` | admin operator+ | replace limits |
| `GET /api/admin/providers` | admin viewer+ | catalog entries + root |
| `POST /api/admin/providers/refresh` | admin operator+ | rescan external releases |
| `GET /api/auth/status` | public | `{ registration: RegistrationPublicStatus, me? }` |
| `POST /api/auth/register` | public; policy-gated | create user + `cowboy_user` cookie |
| `POST /api/auth/login` | public | cookie + `me` |
| `POST /api/auth/logout` | cookie optional | clear cookie |
| `GET /api/auth/me` | product cookie or Bearer | current principal |
| `POST /api/auth/tokens` | product operator+ | create own `cow_…` token; secret shown once |
| `GET /api/auth/tokens` | product (own rows) | list prefixes, names, timestamps |
| `DELETE /api/auth/tokens/{id}` | product (own row; else 404) | revoke |
| `GET /api/admin/users` | admin operator+ | list users + `role_for` |
| `POST /api/admin/users` | admin operator+ | create user + grant |
| `POST /api/admin/users/{id}/disable` | admin operator+ | disable + revoke sessions/tokens |
| `POST /api/admin/users/{id}/password` | admin owner | set password |

Signup is **live** and still policy-gated: default closed, invite tokens,
open mode, and admin-created users. The first product user is created on
this console. Product `/ws` and product APIs require a product principal.
Admin writes go through `require_admin_role` (operator+ or owner); viewers
keep admin read.

## Admin login

`/admin` is a separate site. The document is public HTML; every
`/api/admin/*` route except `GET/POST /api/admin/auth*` requires a
`cowboy_admin` cookie. Product cookies never become an admin principal.

Hardening:

- New admin passwords are argon2id PHC (12–128 characters, not the account).
  Existing iterated SHA-256 hashes verify once and upgrade on login.
- Unknown-account login dummy-verifies so timing does not enumerate owners.
- Cookie: `HttpOnly`, `SameSite=Strict`, 12-hour absolute TTL, `Secure` on HTTPS.
  A new login replaces every other session for that account.
- Login and bootstrap require HTTPS as advertised by a **loopback** reverse
  proxy (`X-Forwarded-Proto: https`). Direct non-loopback HTTP is refused.
  Loopback without forwarded headers stays allowed for `just dev`.
- Login/bootstrap share the in-process failure limiter. Failures log
  `admin_login ok=false` without passwords or hashes.
- Service worker never caches `/api/admin/*` or `/admin` as the PWA shell.

Admin Passkeys follow the product split: password login first, then optional
registration. With a Passkey registered, `/admin` locks the view after
**5 minutes idle** unless the operator turns that off on Accounts. Routes
live under `/api/admin/passkeys*`.

See [Product login](16-product-auth.md) for planes, the capability matrix,
and the Hawk enablement checklist.
