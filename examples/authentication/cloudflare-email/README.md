# Cloudflare Email Authentication Provider reference

Cloudflare Email Service is mail delivery, not an identity provider. The public
package therefore targets a separate OIDC authority that an operator must finish
and own. `worker.ts` is deliberately fail-closed until all of these
deployment-specific controls exist:

- exact confidential-client registration and mode-`0600` client secrets;
- an allow-listed, normalized mailbox-to-immutable-subject mapping;
- generic start responses and per-IP plus per-mailbox rate limits;
- a random, hashed, single-use magic-link credential with a short expiry;
- a Durable Object transaction that atomically consumes the link and later the
  authorization code; KV is not an authorization transaction store;
- S256 PKCE, exact redirect URI, state and nonce binding;
- an HSM/service-bound or protected PKCS#8 RS256 signing key and public JWKS;
- an onboarded sending domain and Cloudflare Email destination policy; and
- audit events that contain no mailbox, token, code, secret, or full request.

After implementing and independently reviewing that authority, replace the
example origin in `authentication.json`, package and sign it, and use
`runtime.example.json` for Cowboy's protected client configuration. Cowboy maps
the authority's opaque stable `sub`, not the mailbox.
