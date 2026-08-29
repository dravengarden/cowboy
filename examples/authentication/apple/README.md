# Sign in with Apple Authentication Provider

This package uses an Apple Services ID, `form_post`, RS256 ID tokens, and a
fresh five-minute ES256 client-secret JWT created inside Cowboy for each token
exchange.

1. Configure Sign in with Apple for a Services ID and exact Cowboy HTTPS return
   URL.
2. Download the one-time `.p8` private key and install it as an absolute
   mode-`0600` runtime file; never add it to this package or source control.
3. Copy `runtime.example.json` into the protected server authentication config.
4. Map Apple's stable `sub`, not the relay email address.
5. Package, sign, publish, and exact-pin the release artifact.

Apple may return the user's name and email only during the first authorization.
Cowboy deliberately ignores those claims for authorization and uses only the
configured subject mapping.
