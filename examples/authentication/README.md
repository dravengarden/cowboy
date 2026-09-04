# Authentication Provider examples

These examples use Cowboy's signed, data-only Authentication Provider API.
Package files are public. Client credentials, upstream subject mappings, and
Cowboy account mappings belong only in the protected Controller runtime file.

- `password/` is the bundled local-password host plugin (login slot, no tables).
- `passkey/` is the bundled WebAuthn host plugin (plugin-owned PG schema or SQLite).
- `oidc-login.js` is the shared login.method module copied into each OIDC example's `ui/`.
- `google/` covers Google Accounts, including accounts whose mailbox is Gmail.
- `apple/` covers Sign in with Apple for a Services ID.
- `cloudflare-email/` documents a Cloudflare Email Service identity-authority
  boundary. It intentionally cannot be deployed until the operator supplies
  atomic transaction storage and signing-key custody.

Build packages with the `cowboy-plugin-pack` binary from the matching component
release, then write the host/UI sidecar with
`just example-auth-bundle <id>`. Publish the `.cowboy-plugin`, `.release.json`,
and `.hostbundle.json` together. The sidecar is bound to the signed package
digest so a Catalog cannot attach UI to a different artifact. A host bundle
must be signed with the publisher Ed25519 key in namespace
`cowboy-plugin-hostbundle-v1`; Catalog refresh rejects unsigned UI. Then
exact-pin the plugin version and artifact digest in `COWBOY_AUTH_CONFIG`.

The enclosing server file uses
`dravengarden.cowboy.authentication/v2`. Provider examples here show only one
entry for its `providers` array; capacity, logout, automation, Passkey, session,
and login-order policy remain server-owned. The complete default policy is in
[`docs/architecture/18-auth-capacity-sso.md`](../../docs/architecture/18-auth-capacity-sso.md).
