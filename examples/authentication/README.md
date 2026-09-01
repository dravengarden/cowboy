# Authentication Provider examples

These examples use Cowboy's signed, data-only Authentication Provider API.
Package files are public. Client credentials, upstream subject mappings, and
Cowboy account mappings belong only in the protected Controller runtime file.

- `google/` covers Google Accounts, including accounts whose mailbox is Gmail.
- `apple/` covers Sign in with Apple for a Services ID.
- `cloudflare-email/` documents a Cloudflare Email Service identity-authority
  boundary. It intentionally cannot be deployed until the operator supplies
  atomic transaction storage and signing-key custody.

Build packages with the `cowboy-plugin-pack` binary from the matching component
release, publish the package and signed release through the ordinary Cowboy
Plugin Catalog, then exact-pin its plugin version and artifact digest in
`COWBOY_AUTH_CONFIG`.

The enclosing server file uses
`dravengarden.cowboy.authentication/v2`. Provider examples here show only one
entry for its `providers` array; capacity, logout, automation, Passkey, session,
and login-order policy remain server-owned. The complete default policy is in
[`docs/architecture/18-auth-capacity-sso.md`](../../docs/architecture/18-auth-capacity-sso.md).
