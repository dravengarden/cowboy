# Passkey plugin

WebAuthn credentials and external ceremonies live in this plugin's own
PostgreSQL schema (`plugin_passkey`) or SQLite file. Cowboy core still owns
the user row, session cookies, and reauth policy flags.

The signed `host.json` claims `account.panel` and selects the closed
`account-passkeys-v1` renderer. Cowboy Web owns that renderer and its WebAuthn
interaction code; the plugin supplies data, storage, and the signed
`webauthn` native-capability claim, never browser-executable UI code.

Core `user_passkeys` / `admin_passkeys` tables are not modified. On first
activation Cowboy copies existing rows into the plugin namespace, then treats
the plugin store as authoritative.

`plugin.json` and the schema-2 `authentication.json` select the Controller's
`webauthn` driver with empty configuration. Build independently with
`nix develop -c just example-auth-bundle passkey`, then follow the generic
Plugin release workflow when publication is authorized. The release-bound
host must declare the account renderer, native capability, and storage; no
collector or RPC code is allowed. Switching from bootstrap to this signed
package preserves the namespace and exact migration bytes. Relying-party and
session policy stay in protected Controller configuration; this Plugin is
not installed on a Machine.
