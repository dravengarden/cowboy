# Google Account Authentication Provider

This package follows Google's OpenID Connect Authorization Code flow. A Gmail
address is a Google Account presentation attribute; Cowboy maps the immutable
Google `sub` claim, never the email address.

1. Create a confidential Web application in Google Cloud.
2. Register only the exact Cowboy callback
   `https://<cowboy-origin>/api/auth/providers/google/callback`.
3. Put the client secret in an absolute mode-`0600` file outside the repository.
4. Copy `runtime.example.json` into the protected server authentication config,
   replacing the client ID, exact `sub`, local account, and secret path.
5. Package and sign `plugin.json`; configure its exact signed artifact digest.

Do not add arbitrary redirect URIs, map by email, or place the client secret in
this public package.
