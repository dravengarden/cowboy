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

The sibling `../tauri/tauri.macos.conf.json` is merged automatically by
Tauri's macOS build and applies the tracked Associated Domains entitlement.
Once the macOS bundle is signed by the declared Apple team, its WKWebView uses
the same direct browser transport as the PWA; an unsigned/ad-hoc development
bundle fails closed to the system-browser transport.
