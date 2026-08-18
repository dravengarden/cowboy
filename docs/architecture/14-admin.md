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
| `GET /api/auth/status` | public | `{ registration: RegistrationPublicStatus, me? }` |
| `POST /api/auth/register` | public; policy-gated | create user + `cowboy_user` cookie |
| `POST /api/auth/login` | public | cookie + `me` |
| `POST /api/auth/logout` | cookie optional | clear cookie |
| `GET /api/auth/me` | product cookie | current principal |
| `GET /api/admin/users` | admin operator+ | list users + `role_for` |
| `POST /api/admin/users` | admin operator+ | create user + grant |
| `POST /api/admin/users/{id}/disable` | admin operator+ | disable + revoke sessions/tokens |
| `POST /api/admin/users/{id}/password` | admin owner | set password |

There is no `cowboy.auth.mode` and no lan/hybrid exposure switch. Product
`/ws` enforcement lands in a later PR; this slice does not install product
middleware on `/ws` or `/api/sessions`.
