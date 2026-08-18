# Admin console

Cowboy's product UI is the session PWA at `/`. The **admin plane** stores
operator policy in Hub settings (`cowboy.registration`,
`cowboy.permissions`, `cowboy.session_limits`, `cowboy.admin.identities`).
Those blobs are service-internal. They must not appear on product `/ws`.

## Settings redaction

`Hub::settings_for_product_clients` is the only constructor of
`Outbound::Settings` (connect bootstrap and the live `set_setting`
broadcast). The product map is an **allow-list**:

- `session.autoResume.default`
- `session.autoResume.template`

`cowboy.admin.identities` (password salts/hashes and admin session token
hashes), `cowboy.permissions` (every account + role),
`cowboy.session_limits`, and the registration invite table are dropped.
Filtering only bootstrap is not enough: `persist_admin_identities` and
other admin writes call `Hub::set_setting`, which persists the full key
and then broadcasts the allow-list.

`Inbound::SetSetting` from a product socket is rejected unless the key is
one of the two auto-resume keys. Admin HTTP stays on `/api/admin/*`.

## Registration public view

`RegistrationPublicStatus` is `{ enabled, mode, accepts_registration }`
only. Do not reuse a view that lists invite tokens (id, name, prefix,
uses, expiry). Product clients must not receive the token table. Account
creation is not live yet; signup must read this policy and fail closed
when `enabled` is false.

The stored registration switch is Synapse-shaped (`enabled` default
false; `mode` is `token` or `open`). Consume lands with product register.
