# Core security client boundary

This is the eleventh slice of the [spatiotemporal design](plugin-spatiotemporal-design.md),
and the first client-side part of P1. It moves local authentication presentation
out of Plugin host selection and gives the existing native Passkey ceremony a
closed, Cowboy-owned client port. It does **not** complete CoreSecurity storage
ownership, remove the historical SDK/native ABI, or meet all P1 exit criteria.

## Local security is not a Plugin surface

`web/src/auth/coreSecurity.ts` projects Service policy into local UI decisions.
It cannot authorize a request; Controller authentication remains authoritative.

- Password login, recent-password verification, setup-code entry and sole-account
  creation use core-owned field labels. A signed host cannot replace them.
- Ordinary password login renders/submits only when enabled and selected. Stale
  selection during a policy update and Enter on an OIDC surface cannot submit
  retained password fields. Server validation remains required.
- Initial setup is a separate core ceremony even when the ordinary password
  method is disabled. A default external identity cannot replace the setup-code
  or account-creation form. No permission, account or password policy changes.
- Desktop, Mobile and Machine setup mount `ProductAccountSecurity` directly from
  the explicit Service Passkey policy. Empty, missing or duplicate Plugin panel
  descriptors cannot hide or multiply the local Passkey panel. Unknown/disabled
  policy does not enable it.
- External OIDC keeps its signed Plugin selection and presentation. The two old
  local renderer IDs remain in the readable SDK registry but render nothing;
  no current core caller passes local credential context through a Plugin slot.

This removes the client's presentation dependency, **not** the Controller's
current exact host/storage prerequisites. Empty Catalog startup on an already
cut-over Controller is still unsupported until the separate storage migration.

## Closed native port and truthful effect outcomes

`web/src/coreNativeBridge.ts` owns only three operations: a no-effect capability
query and explicit registration/assertion. There is no Plugin ID, native name,
Catalog fetch, registration hook or generic public invoke in that module.
The authentication flow adapts its server JSON to this port through
`auth/passkeyNative.ts`.

The port deliberately uses the installed `__cowboyNativePasskey` v1 ABI. Only
version 1 is accepted, not an unbounded future version interval. Creation and
assertion have distinct typed request/result shapes. Runtime decoding rejects
arrays, missing/wrong fields, mixed success/failure, unknown native error codes,
malformed credential fields and oversized binary-text fields. Successful results
are projected into fresh owned records; native error text is not reflected.
The Controller still verifies the actual challenge, signature and authentication
policy. This client decoder is neither a WebAuthn verifier nor a new trust root.

Only one effectful ceremony may be in flight in the Web realm. The captured port
must still match when its reply arrives; a replaced bridge cannot supply a late
success to the new scope. Completion/failure releases this local exclusion. It
does not claim to cancel an OS ceremony or enforce a distributed lease.

Missing bridge **before** dispatch, or the v1 native driver's explicit
`not_configured`/`unsupported_os` refusal, may use the existing secure fallback.
A promise rejection **after** dispatch is `outcome_unknown`, not unavailable:
native may already have created a credential. It never automatically begins
another native/browser ceremony. Cancellation, busy, malformed replies and other
failures also do not fall back. A capability query may safely fail unavailable
because it never starts a ceremony. No client error payload is treated as proof
that an effect was reversed.

## Verification and release boundary

The deterministic suite covers the core policy matrix, actual server-rendered
React login forms with no Plugin inventory, default-SSO bootstrap, disabled local
methods and untrusted local labels. Native fixtures exercise both v1 result
shapes, closed decoding, unsupported versions, explicit fallback codes, bounded
fields, concurrent calls and replaced-port replies. Tests drive the actual
registration/assertion flow with both native and external transports available
and prove that a lost native result starts exactly one native ceremony, no
external transaction and no completion request.

These are hermetic client tests, not a physical iOS Passkey/login receipt.
Native code, Associated Domains, entitlements and all wire ABI bytes are
unchanged. This slice requires only a Web release and PWA version bump. It does
not publish/bump Plugins or the component matrix, restart Controller/Machine,
change host selection, move credentials, run data migrations or install software
on a Machine. The existing Controller and rollback reader can consume the same
public authentication protocol.

## Still required for P1

The subsequent [storage bridge](core-security-storage-bridge.md) now provides a
typed core persistence port, shared write-once binding and atomic first import
at the existing namespace. The next [ownership path](core-security-ownership.md)
implements core-only startup and durable handoff without local Catalog
prerequisites. Its production reader rollout does not perform the host-policy
cutover or finish supported-client/native acceptance below.

1. Accept the implemented local authentication/storage handoff into CoreSecurity without copying
   stale credential rows over current Plugin-namespace data. Preserve applied
   SQL bytes, credential IDs/public keys, sessions and monotonic security state.
2. Verify a compatible ownership handoff and cold/rollback reader floor before
   retiring local Authentication packages or their production pins/markers.
3. The [later Web host slice](core-web-plugin-host.md) removes the application's
   dependency on the mixed Plugin runtime, with closed typed slots and owned
   observations. Complete the public authoring SDK migration and native artifact
   retirement separately. The historical `invokeNativePluginCapability`, signed native claims
   and native `__COWBOY_NATIVE_PLUGIN_HOST` remain readable migration inputs;
   this slice does not pretend they have been removed from shipped artifacts.
4. Before changing/removing native ABI, run the native conformance/product-shell
   gates and real password/device/Passkey/session/origin/gesture acceptance on
   supported clients. Do not delete authority markers or credentials to bypass
   a missing migration or unavailable platform.
