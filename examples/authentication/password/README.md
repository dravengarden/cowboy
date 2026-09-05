# Password login plugin

Cowboy's local password method is a host plugin. The Controller still owns the
`users` password hash and session cookies; this package owns the login slot.

No plugin-owned tables. Identity stays in the product user plane.

`plugin.json` and the schema-2 `authentication.json` select the Controller's
`local_password` driver with empty configuration. Build independently with
`nix develop -c just example-auth-bundle password`, then use the generic
`plugin-set-published-artifact-url`, `plugin-sign`, `plugin-verify`, and
`plugin-publish` workflow when a release is authorized. The host bundle is
required and bound by the same signature. No passwords or password policy
belong in these public files. Controller `password.enabled` still owns
enablement; this is not an OIDC provider entry or a Machine installation.
