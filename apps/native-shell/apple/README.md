# Apple native shell overlay

Cowboy's web bundle owns account state, server ceremonies, and the standard
WebAuthn request/response contract. Apple shells may contribute only a native
ceremony transport under `Sources/`; they do not own sessions or authentication
provider state.

`CowboyPasskeyBridge.mm` is copied into Tauri's generated Apple project before
a native build. It reports native Passkeys as available only when the app's
`Info.plist` contains the exact relying-party ID in
`CowboyNativePasskeyRelyingPartyIdentifiers`. A production build that enables
that key must also be signed with the matching Associated Domains entitlement,
and the Cowboy origin must publish the matching `webcredentials` association.
AuthenticationServices independently enforces both requirements.

SideStore/free-team builds leave the key absent. The shared web transport
registry then uses Cowboy's PKCE-bound system-Safari ceremony. This is an
intentional secure fallback, not a local-biometric substitute for WebAuthn.
The same bridge owns that fallback through `ASWebAuthenticationSession` so the
fixed Cowboy callback closes the sheet after success or cancellation even when
the underlying WKWebView is suspended. Assertions start immediately from the
user's tap in Cowboy, so the next visible interaction is the system Passkey
prompt rather than a second web-page button. Registration retains an explicit
gesture on the trusted Cowboy page. Only an Associated-Domains-capable build
can omit the system authentication browser entirely and invoke
AuthenticationServices from the app.

The sibling `../tauri/tauri.macos.conf.json` is merged automatically by
Tauri's macOS build. Development bundles use Tauri's `-` pseudo-identity and
carry no restricted entitlement, so every default build remains launchable and
fails closed to the system-browser transport.

An official Passkey-enabled macOS release additionally merges
`../tauri/tauri.macos.passkeys.conf.json` and overrides the signing identity:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: ..." \
  cargo tauri build --bundles app \
  --config src-tauri/tauri.macos.passkeys.conf.json
```

That opt-in layer applies the tracked Associated Domains entitlement. Once the
bundle is signed by the declared Apple team, its WKWebView uses the same direct
browser transport as the PWA. Apple rejects an ad-hoc app carrying this
restricted entitlement, so the two configurations must remain separate.
