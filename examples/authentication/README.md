# Authentication Provider examples

These examples use Cowboy's signed, data-only Authentication Provider API.
Package files are public. Client credentials, upstream subject mappings, and
Cowboy account mappings belong only in the protected Controller runtime file.

- `password/` is a schema-2 local-password Plugin (login slot, no tables).
- `passkey/` is a schema-2 WebAuthn Plugin (plugin-owned PG schema or SQLite).
- each login host selects a closed Cowboy-owned `login.method` renderer in
  `host.json`; no example ships browser code.
- `google/` covers Google Accounts, including accounts whose mailbox is Gmail.
- `apple/` covers Sign in with Apple for a Services ID.
- `cloudflare-email/` documents a Cloudflare Email Service identity-authority
  boundary. It intentionally cannot be deployed until the operator supplies
  atomic transaction storage and signing-key custody.

Build packages with the `cowboy-plugin-pack` binary from the matching component
release; `build` emits the `.cowboy-plugin`, `.release.json`, and, when
`host.json` exists, `.hostbundle.json` in one SDK-only operation.
`just example-auth-bundle <id>` is the repository wrapper;
`just example-auth-build-all` verifies every example, including Password and
Passkey. Publish all three
together. Release schema 2 binds the SHA-256 of the
exact host-data sidecar into the outer Plugin `artifact_digest` and publisher
signature, so it cannot be replaced or attached to a different release. The
sidecar contains `host.json` plus optional collector programs and rejects
`ui/**`. Authentication bundles additionally forbid collector files or process
grants. The SDK verifies that the host renderer, storage, and native capability
match the payload's closed protocol at build, signing, and verification.

For OIDC, exact-pin the plugin version and artifact digest in
`COWBOY_AUTH_CONFIG`. Local Password and WebAuthn select existing Controller
drivers with `configuration: {}`; they do not go in the OIDC `providers` array.
Their enablement and session policy remain server-owned. Exact host activation
is configured separately through `COWBOY_PLUGIN_HOST_CONFIG`, with OIDC pins
automatically merged from `COWBOY_AUTH_CONFIG`. Publishing a new version must
not silently replace the configured login renderer or run its storage migrations.
The opt-in `catalog_only` policy requires every enabled method and the WebAuthn
storage used by Product/admin authentication before startup. It disables all
source fallback and durably records a one-way cutover marker only after
activation succeeds. Keep the same Plugin ID and historical migration bytes
to preserve existing Passkey credentials. See the complete
[Controller host activation contract](../../docs/plugin-packages.md#controller-host-activation)
for the private configuration, readiness checks, and restart semantics.
Before an authorized restart, add `--check-plugin-hosts` to the candidate
Controller's intended `serve` command, retaining the same Service environment
and authentication/database flags. It verifies configuration and signed
selections without creating state or connecting to the database. Its JSON
report lists the runtime, migration and live-login checks it has not performed;
success is not an activation receipt.
This repository change performs no production publication or cutover; existing
deployments without the new policy remain in bootstrap mode.

The enclosing server file uses
`dravengarden.cowboy.authentication/v2`. Provider examples here show only one
entry for its `providers` array; capacity, logout, automation, Passkey, session,
and login-order policy remain server-owned. The complete default policy is in
[`docs/architecture/18-auth-capacity-sso.md`](../../docs/architecture/18-auth-capacity-sso.md).
